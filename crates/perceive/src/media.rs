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

use crate::mask::{FLAT_MAX_COLORS, distinct_colors};
use crate::pixels::dominant_ratio;

/// 横条的条高（像素）。取 6 是为了比正文行高（实测 11~14 px）细：
/// 行与行之间才有空隙可分。
/// ⚠ **未实测**：行高 11~14 是实测的，条高 6 本身是按它推的，没有在含媒体的
/// 帧上量过。拿到后用 `cargo xtask media <png>` 重新量。
pub const BAND_H: i32 = 6;

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
    /// 检出区域。横向是「有内容」的包围盒，纵向是合并后的横条带——
    /// 候选横条本来横贯整个消息区宽度，直接用会把整条面板宽度算进来。
    pub rect: Rect,
    /// 归成了哪类。只按长边与长宽比分头像/图片，分不出就留 [`MediaKind::Unknown`]。
    pub kind: MediaKind,
    /// 判定依据之一：色种数。
    pub colors: usize,
    /// 判定依据之一：众数占比。
    pub purity: f32,
}

/// 媒体种类。区分不出来就 [`MediaKind::Unknown`]——**不要瞎猜成 Photo**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum MediaKind {
    /// 照片/图片。当前归类只有这一档与头像两档，其余全落到 [`MediaKind::Unknown`]。
    Photo,
    /// 表情包。**当前实现还分不出来**——判据只有尺寸与色种数，
    /// 表情与照片在这两项上没有实测差异，所以不会被判成这里。
    Sticker,
    /// 头像。判据是长边 ≤ [`AVATAR_MAX`] 且长短边差 ≤ [`AVATAR_SQUARE`]。
    ///
    /// ⚠ [`AVATAR_MAX`] / [`AVATAR_SQUARE`] 本身未实测：手头的帧里没有头像
    /// （夹具已把它们刷成块底色），所以这一档目前是在无样本的情况下定的。
    Avatar,
    /// 分不出来。宁可留未知，也不要瞎猜——猜错的代价是把这块的内容
    /// 按错误的类别描述给下游。
    Unknown,
}

impl MediaKind {
    /// 中文短标签，给界面与调试输出用。
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
    /// 检出的区域，**顺序即自上而下**。
    ///
    /// 空 vec 是合法结果——纯文字聊天就该是空的，调用方别把它当失败。
    pub regions: Vec<MediaRegion>,
}

/// 在聊天区里找媒体区域。
///
/// 流程：切横条 → 找**不平直**的条 → 合并（允许 ≤ `BAND_GAP` 条间隔）
/// → 丢掉过小的 → **条带探针排除纯文字气泡** → 归类。
///
/// ## 不收 `Palette`
///
/// 早先的签名收 `&Palette`、内部按「异于面板底色」判占用。那条判据在真实帧
/// 上**整张图漏检**（照片的深色背景与底色几乎一样），所以两处都换成了
/// 平直度判据，`panel_bg` 也就没有用处了——**留一个用不到的参数比删掉更糟**。
pub fn find(frame: &Frame, pane: Rect) -> Found {
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
        if band_is_content(frame, band) {
            match open {
                // 上一条候选已经结束，且空隙超过 BAND_GAP：先收口再开新的。
                Some((b0, b1)) if y - b1 > BAND_GAP * BAND_H => {
                    push_region(frame, Rect::new(x_start, b0, x_end, b1), &mut regions);
                    open = Some((y, band.y1));
                }
                Some((b0, _)) => open = Some((b0, band.y1)),
                None => open = Some((y, band.y1)),
            }
        }
        y += BAND_H;
    }
    if let Some((b0, b1)) = open {
        push_region(frame, Rect::new(x_start, b0, x_end, b1), &mut regions);
    }

    Found { regions }
}

/// 一条横条里有没有**内容**——判据是「这条不平直」。
///
/// ## 为什么不能用「异于面板底色」
///
/// 早先的版本数「与 `panel_bg` 距离超过 `BG_TOL` 的像素占比」。在真实帧上
/// （`target/probe/text-chat.png`，`scripts/scancol.ps1` 实测）**整张图漏检**：
///
/// | Y 区间 | 精确色种数 | 是什么 |
/// |--------|-----------|--------|
/// | 240–320 | 50 ~ 125 | 照片本体（高约 85px） |
/// | 330–370 | 3 | 照片自己的深色背景 |
/// | 其余 | 1 ~ 11 | 文字行 |
///
/// 照片**内部**那些只有 3 种色的深色背景，与面板底色 `rgb(30,30,31)` 几乎
/// 一样，于是被判成空带，区域被切成碎片，每片都够不到 [`MIN_MEDIA_H`]。
///
/// 这与票 07 早就记下的「图片区 51% 纯黑背景」是**同一个病根**：
/// **平直度判据在图片上很灵，「异于底色」在图片上很钝**，
/// 两条判据不能共用一个思路。色种数是实测里唯一单调的信号。
fn band_is_content(frame: &Frame, r: Rect) -> bool {
    distinct_colors(frame, r).is_some_and(|n| n > FLAT_MAX_COLORS)
}

/// 4+5+6：过小就丢；顶部/底部 20% 两条都平就丢；剩下才归类。
fn push_region(frame: &Frame, banded: Rect, out: &mut Vec<MediaRegion>) {
    // 候选横条是整条消息区宽度，真实图片只占一部分。先把「有东西」的
    // 横向范围收出来——否则一次统计会覆盖整个面板宽度，色种数必然爆炸。
    let r = match content_bounds(frame, banded) {
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

/// 候选条里「有内容」的横向包围盒。整条都平返回 `None`。
///
/// 同样**不能用「异于面板底色」**：照片左右两侧的深色背景与底色几乎一样，
/// 用那种判据算包围盒会把照片**裁掉一截**。逐列判「这一列平不平直」，
/// 对深色背景免疫——照片里的一列再暗也是花的（几十种色），
/// 而纯底色的一列只有 1 种。
fn content_bounds(frame: &Frame, r: Rect) -> Option<Rect> {
    let x1 = (r.x1.max(0) as u32).min(frame.width());
    let y1 = (r.y1.max(0) as u32).min(frame.height());
    let y0 = r.y0.max(0) as u32;
    let mut min_x: Option<u32> = None;
    let mut max_x = 0u32;
    for x in (r.x0.max(0) as u32)..x1 {
        if band_is_content(
            frame,
            Rect::new(x as i32, y0 as i32, x as i32 + 1, y1 as i32),
        ) {
            min_x = min_x.or(Some(x));
            max_x = x + 1;
        }
    }
    min_x.map(|m| Rect::new(m as i32, r.y0, max_x as i32, r.y1))
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

    /// **不变量**：纯文字气泡绝不能被判成媒体。
    ///
    /// 把用户自己的文字气泡当成「图」送进外部模型是隐私事故。
    #[test]
    fn a_text_bubble_is_never_media() {
        let f = text_bubble();
        let found = find(&f, PANE);
        assert!(
            found.regions.is_empty(),
            "纯文字气泡被误判成媒体了：{:?}",
            found.regions
        );
    }

    #[test]
    fn a_noisy_block_is_media() {
        let f = photo();
        let found = find(&f, PANE);
        assert_eq!(found.regions.len(), 1, "噪声方块应被检出：{found:?}");
        assert_eq!(found.regions[0].kind, MediaKind::Photo);
    }

    #[test]
    fn an_empty_pane_finds_nothing() {
        let f = frame_of(|_, _| PANEL);
        let found = find(&f, PANE);
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
        let found = find(&f, PANE);
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
        let found = find(&f, PANE);
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
        let found = find(&frame, chat);
        assert!(
            found.regions.is_empty(),
            "纯文字真实帧检出了 {} 个媒体区域，是误判：{:?}",
            found.regions.len(),
            found.regions
        );
    }
}
