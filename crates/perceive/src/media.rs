//! 媒体区域检测——找出「这里有一张图」的**区域**。
//!
//! ## 为什么需要它
//!
//! [`crate::mask`] 是**逐行**的：它对每一个 OCR 读出来的文本行判定
//! 「这行是不是图里的字」。但实测帧里对方发的那张图片，**OCR 一行字都没读
//! 出来**——于是遮罩对那张图完全不存在、完全看不见。
//! [`crate::merge::Message`] 里也没有「图」这个概念。
//!
//! 所以送图给视觉模型的第一步必须是**区域级**检测。这一层只负责找区域，
//! **不裁图、不缩放、不出网**。
//!
//! ## 最重要的一条不变量
//!
//! **纯文字气泡绝不能被判成媒体区域。** 把用户自己的文字气泡当成「图」
//! 送进外部模型是隐私事故。第 5 步的条带探针就是为它存在的：
//! 微信气泡里的文字垂直居中，所以候选区的顶部 20% 与底部 20% 都没字，
//! 两条都平就说明那是一块平底色，不是一张图。
//!
//! ## 阈值纪律
//!
//! 本项目有条规矩：**没有实测数据的判据必须显式标注「未实测」**。
//! 本文件里除 `FLAT_MAX_COLORS`（实测：平表面 1 种色 / 图片 2037 种）外，
//! 其余阈值全部标了「未实测」——手头没有含媒体的真实帧可以标定。

use chatservant_core::frame::{Frame, Rect};

use crate::attribute::Palette;
use crate::mask::{FLAT_MAX_COLORS, distinct_colors};
use crate::pixels::{dist, dominant_ratio};

/// 横条的条高（像素）。取 6 是为了比正文行高（实测 11~14 px）细：
/// 行与行之间才有空隙可分。
/// ⚠ **未实测**：行高 11~14 是实测的，条高 6 本身是按它推的，没有在含媒体的
/// 帧上量过。拿到后用 `cargo xtask media <png>` 重新量。
pub const BAND_H: i32 = 6;

/// 一条横条里「非面板底色」像素占比超过此值，这条就算「有东西」。
/// ⚠ **未实测**：取值保守（0.05）是为了让浅色气泡边框、抗锯齿边缘也能触发
/// 一条，避免图片边缘被削掉。
pub const BAND_MIN_INK: f32 = 0.05;

/// 像素算不算「面板底色」的距离容差。
/// 实测面板底色在帧内**完全一致**（`rgb(30,30,31)` 到处一样），12 有很大余量
/// （与 `pixels::COLOR_TOL` 同源）。
pub const BG_TOL: f32 = 12.0;

/// 允许合并的两条「有东西的条」之间最多空几条。
/// ⚠ **未实测**：文字行之间会有空隙，取 2 是保守值。
pub const BAND_GAP: i32 = 2;

/// 媒体区域的最小宽 / 高（像素）。
///
/// ✅ **高度已实测**（`cargo xtask media target/probe/text-chat.png`——那张含照片的
/// 真实帧）：40 这个值会把**文字行**判成媒体，误检出的三条全是 42/42/54px 高的
/// 横条；而照片本身 140px 高以上（票 07 记的 140×160）。
/// 取 80 落在「实测最高误检 54」与「实测最低真图 140」中间。
pub const MIN_MEDIA_W: i32 = 40;
/// ⚠ **宽度未实测**：实测里误检横条宽 167~238、真图宽 156 起——**宽度单独
/// 完全分不开**，所以它只当一个下限兜底，主判据是高度与色种数。
pub const MIN_MEDIA_H: i32 = 80;

/// 媒体区域的最小色种数。
///
/// ✅ 两端都是实测值：真图 830~2237 种（同一帧上四块检出的区域），
/// 脱敏后残留过渡色的文字条只有 **9** 种（`wechat-dark-2side` 实测）。
/// 24 落在中间，两边都留了余量。
pub const MEDIA_MIN_COLORS: usize = 24;

/// 头像的最大边长与长宽比容差（像素）。
/// ⚠ **未实测**：微信默认头像是正方形的小图，但本帧里没有头像
/// （夹具已脱敏刷掉），取值只能先按常见尺寸给。
pub const AVATAR_MAX: i32 = 48;
/// ⚠ **未实测**：见 [`AVATAR_MAX`]。
pub const AVATAR_SQUARE: i32 = 12;

/// 一块媒体区域。**依据必须可追**（CONTEXT.md 的要求：误判要看得见）。
///
/// 只 derive `PartialEq` 不 derive `Eq`：`purity` 是 `f32`，`Eq` 编不过。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct MediaRegion {
    pub rect: Rect,
    pub kind: MediaKind,
    /// 判定依据之一：色种数。
    pub colors: usize,
    /// 判定依据之一：众数占比。
    pub purity: f32,
}

/// 媒体种类。区分不出来就 [`MediaKind::Unknown`]——**不要瞎猜成 Photo**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum MediaKind {
    Photo,
    Sticker,
    Avatar,
    Unknown,
}

impl MediaKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Photo => "图片",
            Self::Sticker => "表情",
            Self::Avatar => "头像",
            Self::Unknown => "未知",
        }
    }
}

/// 检出结果。**空的 vec 是合法结果**，不是错误。
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub regions: Vec<MediaRegion>,
}

/// 在聊天区里找媒体区域。
///
/// 流程：切横条 → 找「有东西」的条 → 合并（允许 ≤ `BAND_GAP` 条间隔）
/// → 丢掉过小的 → **条带探针排除纯文字气泡** → 归类。
pub fn find(frame: &Frame, pane: Rect, palette: &Palette) -> Found {
    let mut regions = Vec::new();
    let y_start = pane.y0.max(0);
    let y_end = pane.y1.min(frame.height() as i32);
    let x_start = pane.x0.max(0);
    let x_end = pane.x1.min(frame.width() as i32);
    if x_end <= x_start || y_end <= y_start {
        return Found { regions };
    }

    // 1+2. 逐条判「有没有东西」，再 3. 合并。
    let mut open: Option<(i32, i32)> = None; // (y0, y1) 未闭合的候选
    let mut y = y_start;
    while y < y_end {
        let band = Rect::new(x_start, y, x_end, (y + BAND_H).min(y_end));
        if band_ink_ratio(frame, band, palette.panel_bg) > BAND_MIN_INK {
            match open {
                // 上一条候选已经结束，且空隙超过 BAND_GAP：先收口再开新的。
                Some((b0, b1)) if y - b1 > BAND_GAP * BAND_H => {
                    push_region(
                        frame,
                        Rect::new(x_start, b0, x_end, b1),
                        palette,
                        &mut regions,
                    );
                    open = Some((y, band.y1));
                }
                Some((b0, _)) => open = Some((b0, band.y1)),
                None => open = Some((y, band.y1)),
            }
        }
        y += BAND_H;
    }
    if let Some((b0, b1)) = open {
        push_region(
            frame,
            Rect::new(x_start, b0, x_end, b1),
            palette,
            &mut regions,
        );
    }

    Found { regions }
}

/// 4+5+6：过小就丢；顶部/底部 20% 两条都平就丢；剩下才归类。
fn push_region(frame: &Frame, banded: Rect, palette: &Palette, out: &mut Vec<MediaRegion>) {
    // 候选横条是整条消息区宽度，真实图片只占一部分。先把「有东西」的
    // 横向范围收出来——否则一次统计会覆盖整个面板宽度，色种数必然爆炸。
    let r = match content_bounds(frame, banded, palette.panel_bg) {
        Some(r) => r,
        None => return,
    };
    if r.width() < MIN_MEDIA_W || r.height() < MIN_MEDIA_H {
        return;
    }

    // **防误判的关键一步**：微信气泡里的文字垂直居中，所以顶部/底部 20%
    // 两条对气泡来说是没字的。思路同 `pixels::probe_bubble_bg`
    // （「框内被字形污染，不能用」）。两条都平 → 那是一块平底色，不是图。
    let h = r.height();
    let cap = (h / 5).max(1);
    let top = Rect::new(r.x0, r.y0, r.x1, r.y0 + cap);
    let bottom = Rect::new(r.x0, r.y1 - cap, r.x1, r.y1);
    let top_flat = distinct_colors(frame, top).is_none_or(|n| n <= FLAT_MAX_COLORS);
    let bottom_flat = distinct_colors(frame, bottom).is_none_or(|n| n <= FLAT_MAX_COLORS);
    if top_flat && bottom_flat {
        return;
    }

    let colors = distinct_colors(frame, r).unwrap_or(0);
    // 脱敏后的真实纯文字帧上，条带探针**拦不住**残留的麻点过渡色：
    // 那两块各只有 9 种色、众数 0.92，混在图片的量级（实测 2037 种）里
    // 根本不是一回事。所以再加一道色种数下限。
    if colors < MEDIA_MIN_COLORS {
        return;
    }
    let purity = dominant_ratio(frame, r);
    let long = r.width().max(r.height());
    let short = r.width().min(r.height());
    let kind = if long <= AVATAR_MAX && (long - short).abs() <= AVATAR_SQUARE {
        MediaKind::Avatar
    } else {
        MediaKind::Photo
    };
    out.push(MediaRegion {
        rect: r,
        kind,
        colors,
        purity,
    });
}

/// 候选条里「与面板底色不同」的像素占比。
fn band_ink_ratio(frame: &Frame, r: Rect, panel_bg: [u8; 3]) -> f32 {
    let x1 = (r.x1.max(0) as u32).min(frame.width());
    let y1 = (r.y1.max(0) as u32).min(frame.height());
    let mut inked = 0usize;
    let mut total = 0usize;
    for y in (r.y0.max(0) as u32)..y1 {
        for x in (r.x0.max(0) as u32)..x1 {
            if let Some(p) = frame.pixel(x, y) {
                total += 1;
                if dist(p, panel_bg) > BG_TOL {
                    inked += 1;
                }
            }
        }
    }
    if total == 0 {
        return 0.0;
    }
    inked as f32 / total as f32
}

/// 候选条里「有东西」的横向包围盒。整条都平返回 `None`。
fn content_bounds(frame: &Frame, r: Rect, panel_bg: [u8; 3]) -> Option<Rect> {
    let x1 = (r.x1.max(0) as u32).min(frame.width());
    let y1 = (r.y1.max(0) as u32).min(frame.height());
    let mut min_x = x1;
    let mut max_x = 0u32;
    for y in (r.y0.max(0) as u32)..y1 {
        for x in (r.x0.max(0) as u32)..x1 {
            let Some(p) = frame.pixel(x, y) else {
                continue;
            };
            if dist(p, panel_bg) > BG_TOL {
                min_x = min_x.min(x);
                max_x = max_x.max(x + 1);
            }
        }
    }
    if min_x >= max_x {
        return None;
    }
    Some(Rect::new(min_x as i32, r.y0, max_x as i32, r.y1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView as _;
    use std::path::Path;

    const PANEL: [u8; 3] = [30, 30, 31];
    const BUBBLE: [u8; 3] = [47, 47, 48];
    const W: u32 = 400;
    const H: u32 = 300;
    const PANE: Rect = Rect::new(0, 0, W as i32, H as i32);

    fn frame_of(f: impl Fn(u32, u32) -> [u8; 3]) -> Frame {
        let mut px = vec![0u8; (W * H * 3) as usize];
        for y in 0..H {
            for x in 0..W {
                let i = ((y * W + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&f(x, y));
            }
        }
        Frame::from_rgb(W, H, px).expect("尺寸与缓冲区一致")
    }

    /// 纯文字气泡：面板底色 + 一块对方气泡色 + 几行垂直居中的灰阶笔画。
    ///
    /// 笔画**垂直居中**是关键：顶部/底部 20% 探针要能证明「这里是底色」。
    fn text_bubble() -> Frame {
        frame_of(|x, y| {
            let in_bubble = (60..340).contains(&x) && (40..260).contains(&y);
            if !in_bubble {
                return PANEL;
            }
            // 行高 14、上下留白 40，正文落在 y=62..240 区间里垂直居中
            let ly = (y - 40) % 20;
            let stroke = in_bubble && y > 62 && y < 238 && ly < 3 && x > 80 && x < 320;
            if stroke { [225, 225, 229] } else { BUBBLE }
        })
    }

    /// 图片区域：51% 纯黑背景 + 几千种色（对照 `mask.rs` 里的实测数字）。
    fn photo() -> Frame {
        frame_of(|x, y| {
            if (100..300).contains(&x) && (60..240).contains(&y) {
                if (x / 7 + y / 3) % 3 == 0 {
                    [20, 20, 20]
                } else {
                    let h = x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503);
                    [
                        (h & 0xff) as u8,
                        ((h >> 8) & 0xff) as u8,
                        ((h >> 16) & 0xff) as u8,
                    ]
                }
            } else {
                PANEL
            }
        })
    }

    fn palette() -> Palette {
        Palette { panel_bg: PANEL }
    }

    /// **不变量**：纯文字气泡绝不能被判成媒体。
    ///
    /// 把用户自己的文字气泡当成「图」送进外部模型是隐私事故。
    #[test]
    fn a_text_bubble_is_never_media() {
        let f = text_bubble();
        let found = find(&f, PANE, &palette());
        assert!(
            found.regions.is_empty(),
            "纯文字气泡被误判成媒体了：{:?}",
            found.regions
        );
    }

    #[test]
    fn a_noisy_block_is_media() {
        let f = photo();
        let found = find(&f, PANE, &palette());
        assert_eq!(found.regions.len(), 1, "噪声方块应被检出：{found:?}");
        assert_eq!(found.regions[0].kind, MediaKind::Photo);
    }

    #[test]
    fn an_empty_pane_finds_nothing() {
        let f = frame_of(|_, _| PANEL);
        let found = find(&f, PANE, &palette());
        assert!(
            found.regions.is_empty(),
            "空面板不该检出媒体：{:?}",
            found.regions
        );
    }

    /// 依据可追：每个区域都带着真实的 colors / purity，不是默认值。
    #[test]
    fn every_region_carries_its_evidence() {
        let f = photo();
        let found = find(&f, PANE, &palette());
        for r in &found.regions {
            assert!(
                r.colors >= MEDIA_MIN_COLORS,
                "区域必须过了色种数下限 {}，实际只有 {} 种：{r:?}",
                MEDIA_MIN_COLORS,
                r.colors
            );
            assert!(
                r.purity > 0.0 && r.purity <= 1.0,
                "众数占比必须是合法比值，实际 {}：{r:?}",
                r.purity
            );
        }
    }

    /// 低于最小尺寸的方块直接丢掉。
    #[test]
    fn a_tiny_block_is_dropped() {
        let f = frame_of(|x, y| {
            if (100..120).contains(&x) && (100..120).contains(&y) {
                let h = x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503);
                [
                    (h & 0xff) as u8,
                    ((h >> 8) & 0xff) as u8,
                    ((h >> 16) & 0xff) as u8,
                ]
            } else {
                PANEL
            }
        });
        let found = find(&f, PANE, &palette());
        assert!(
            found.regions.is_empty(),
            "20x20 的方块低于最小尺寸 {}x{}，不该被检出：{found:?}",
            MIN_MEDIA_W,
            MIN_MEDIA_H
        );
    }

    /// 端到端防线：真实脱敏夹具是**纯文字聊天、没有图**，检出数必须是 0。
    #[test]
    fn a_real_text_only_frame_finds_nothing() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../capture/fixtures/wechat-dark-2side.png");
        let img = image::open(&path).unwrap_or_else(|e| panic!("读夹具失败：{e}"));
        let (w, h) = img.dimensions();
        let frame = Frame::from_rgb(w, h, img.into_rgb8().into_raw()).expect("尺寸一致");
        let chat = Rect::new(304, 31, 1260, 683);
        let pal = Palette::learn(&frame, chat).expect("学得到配色");
        let found = find(&frame, chat, &pal);
        assert!(
            found.regions.is_empty(),
            "纯文字真实帧检出了 {} 个媒体区域，是误判：{:?}",
            found.regions.len(),
            found.regions
        );
    }
}
