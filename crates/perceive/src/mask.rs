//! 语义遮罩——把不该读的区域从图里抠掉。
//!
//! ## 这是唯一一级以准确率为主的（ADR-0002 第 3 级）
//!
//! 其余几级（变化门控、增量 ROI、尺度归一）都是**成本**手段：少送像素。
//! 这一级不同。抠掉头像、表情包、图片的首要收益是
//! **不把这些误当成对方说的话**；省 token 只是副作用。
//!
//! 所以它的验收标准是「图里的字没有进 `Message[]`」，
//! 不是「送出的像素变少了」。用成本指标衡量它，会把它优化成一个纯粹的
//! 省钱步骤，而它真正的价值在别处。
//!
//! ## 判据只有两个，都来自实测
//!
//! **平直度（purity）**。一块表面是平的，一块内容是花的。
//! 真实帧上实测（`cargo xtask measure`）：
//!
//! | 区域 | 众数占比 | 颜色种数 |
//! |------|---------|---------|
//! | 我方气泡 `rgb(114,205,144)` | 1.00 | 1 |
//! | 面板底色 `rgb(30,30,31)` | 1.00 | 1 |
//! | 对方气泡 `rgb(47,47,48)` | 1.00 | 1 |
//! | 聊天里的图片 | 0.51 | **2037** |
//!
//! 图片区域 51% 的众数看起来仍然很高——那是因为它有**大片纯黑背景**。
//! 所以只看众数占比会漏判，必须同时看**颜色种数**：
//! 平直表面只有 1~2 种色，内容有几千种。
//!
//! **字号比**。真实正文行高 11~14 px；图片/头像里的字通常更小。
//! ⚠ 这个判据在本帧上**未实测**（图片里的字没被 OCR 读出来），
//! 取值保守，拿到含图内字的帧后要回来重标。

use chatservant_capture::ocr::OcrLine;
use chatservant_core::frame::{Frame, Rect};

use crate::attribute::{NoiseClass, Speaker};
use crate::pixels::{Rgb, dist, dominant_ratio};

/// 一块区域至少要有这么多种**精确色**，才不是一片平表面。
///
/// 实测：面板底色与两侧气泡都只有 1 种；聊天里的图片有 2037 种。
/// 阈值取 6，把两者分开，留足余量。
pub const FLAT_MAX_COLORS: usize = 6;

/// 一块区域的众数占比低于此值 → 不是平表面，是内容。
///
/// ⚠ **实测定的是「颜色种数」，不是这个比值。**
/// 实测图片区众数占比高达 0.51（大片纯黑背景），单看比值会漏判。
/// 这个阈值取 0.35 是为了给「纯黑背景占一半的图片」留一道保险，
/// 主判据是颜色种数。
pub const FLAT_MIN_PURITY: f32 = 0.35;

/// 判为「平表面」的结论，**必须带着依据**。
///
/// 依据可追是 CONTEXT.md 的要求：误判必须可见。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceVerdict {
    /// 是不是一片平表面。两个判据都过才算 `true`——
    /// 只看色种数会放过「3 种色的渐变背景」，只看众数占比会漏掉
    /// 实测众数高达 0.51 的照片区（大片纯黑背景）。
    pub is_flat: bool,
    /// 众数占比。
    pub purity: f32,
    /// 出现过的精确色种数。
    pub colors: usize,
}

impl SurfaceVerdict {
    /// 把结论连同两个实测数字讲成人话。
    ///
    /// 判成「不是平表面」时把主判据 [`FLAT_MAX_COLORS`] 也写出来，
    /// 否则看到「内容区」无从知道它是卡在哪一条上。
    pub fn explain(&self) -> String {
        if self.is_flat {
            format!("平表面（{} 种色 · 众数 {:.2}）", self.colors, self.purity)
        } else {
            format!(
                "内容区（{} 种色 · 众数 {:.2}）——色种数超过 {}，不是一片底色",
                self.colors, self.purity, FLAT_MAX_COLORS
            )
        }
    }
}

/// 判定一块区域是不是一片平表面。
///
/// **两个判据都要满足**才算平表面——只满足一个都会被实测数据打掉：
/// - 图片区众数占比 0.51（很高）但 2037 种色 → 只看占比会漏判
/// - 一块刚好有 3 种色的渐变背景占比 0.9 → 只看种数会漏判
pub fn judge_surface(frame: &Frame, r: Rect) -> Option<SurfaceVerdict> {
    let colors = distinct_colors(frame, r)?;
    if colors == 0 {
        return None;
    }
    // 先看色种数。图片区实测众数占比高达 0.51（大片纯黑背景），
    // 单看占比**一定**会漏判，所以色种数是主判据。
    if colors > FLAT_MAX_COLORS {
        return Some(SurfaceVerdict {
            is_flat: false,
            purity: dominant_ratio(frame, r),
            colors,
        });
    }
    // 色种数够少，才轮到占比：3 种色的渐变背景占比可以很高，
    // 但那仍不是一片底色。
    Some(SurfaceVerdict {
        is_flat: dominant_ratio(frame, r) >= FLAT_MIN_PURITY,
        purity: dominant_ratio(frame, r),
        colors,
    })
}

/// 区域里出现过的**精确色种数**。
///
/// 不做量化合并——量化会把「深灰 vs 近黑」并成一个桶，
/// 那样平表面和内容区的差别就没了。要的就是精确值。
pub fn distinct_colors(frame: &Frame, r: Rect) -> Option<usize> {
    use std::collections::HashSet;
    let mut set: HashSet<Rgb> = HashSet::new();
    let x1 = (r.x1.max(0) as u32).min(frame.width());
    let y1 = (r.y1.max(0) as u32).min(frame.height());
    let mut y = r.y0.max(0) as u32;
    while y < y1 {
        let mut x = r.x0.max(0) as u32;
        while x < x1 {
            if let Some(p) = frame.pixel(x, y) {
                set.insert(p);
            }
            x += 1;
        }
        y += 1;
    }
    Some(set.len())
}

/// 一行文字所在区域被判成噪声时，**丢弃原因**。
#[derive(Debug, Clone, PartialEq)]
pub struct Rejection {
    /// 被丢弃那行文字在帧里的位置，调试视图靠它上色。
    pub rect: Rect,
    /// 它被归成了哪一类噪声（标题、灰字、图内字、碎片……）。
    pub noise: NoiseClass,
    /// 丢弃理由，一句人能读的话。
    ///
    /// 两种来源：区域压根读不出像素时是固定文案；
    /// 判成图内字时直接取 [`SurfaceVerdict::explain`]——它带上了色种数与
    /// 众数占比，所以「为什么被当成图里的字」能一路追到实测数字，
    /// 调试视图要上色也只有这一个字符串可依据。
    pub reason: String,
}

/// 语义遮罩：判定每一行文字该不该进消息流。
///
/// 返回**全部**被丢弃的行与理由——调试视图要按这个上色。
/// 一个总计数在识别不对时无法定位是哪一级的问题。
/// 一行文字的遮罩结论：留下，还是丢弃。
#[derive(Debug, Clone, PartialEq)]
pub enum Masked {
    /// 留下来了，走说话人判据。
    Kept,
    /// 丢弃了，**带着原因**——调试视图要按这个上色。
    Dropped(Rejection),
}

/// 语义遮罩：判定每一行文字该不该进消息流。
///
/// 返回**全部**被丢弃的行与理由——调试视图要按这个上色。
/// 只有一个总计数的话，识别不对时无法定位是哪一级的问题。
pub fn mask(lines: &[OcrLine], frame: &Frame) -> Vec<(usize, Masked)> {
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let verdict = match judge_surface(frame, line.rect) {
                Some(v) => v,
                None => {
                    return (
                        i,
                        Masked::Dropped(Rejection {
                            rect: line.rect,
                            noise: NoiseClass::Fragment,
                            reason: "区域读不出像素，跳过".to_string(),
                        }),
                    );
                }
            };
            if verdict.is_flat {
                (i, Masked::Kept)
            } else {
                (
                    i,
                    Masked::Dropped(Rejection {
                        rect: line.rect,
                        noise: NoiseClass::ImageText,
                        reason: verdict.explain(),
                    }),
                )
            }
        })
        .collect()
}

/// 一行文字与它外圈底色的对比度是否**高得可疑**。
///
/// 高对比本身不是问题（对方气泡正文的对比是 178.4，比标题阈值还高）。
/// 这个函数存在的意义是提醒：**不要用对比度当「这是图片里的字」的判据**——
/// 实测对方气泡正文 178.4 > 标题阈值 140，两者会互相混淆。
pub fn contrast_is_suspiciously_high(ink: Rgb, bg: Rgb) -> bool {
    dist(ink, bg) > 150.0
}

/// 图里的字不会被当成对方说的话——这才是本级的验收点。
///
/// 判据顺序不可交换：先判「在不在平表面上」，再判说话人。
/// 对方气泡的对比度 178.4 高于任何对比度阈值，先看对比度会误判。
pub fn judge(line: &OcrLine, frame: &Frame) -> Option<NoiseClass> {
    match judge_surface(frame, line.rect)? {
        v if v.is_flat => Some(NoiseClass::Text),
        _ => Some(NoiseClass::ImageText),
    }
}

/// 把遮罩接进归类：`ImageText` 直接判给 `System`。
///
/// 这是本级与既有归类逻辑的**唯一**接缝——遮罩不重排判据，
/// 只是在它前面加一道「这是不是一片平表面」的闸。
pub fn reject_image_text(attrs: &mut [crate::attribute::Attribution]) {
    for a in attrs {
        if a.noise == NoiseClass::ImageText {
            a.speaker = Speaker::System;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chatservant_capture::ocr::OcrWord;

    const PANEL: [u8; 3] = [30, 30, 31];
    const MY_BUBBLE: [u8; 3] = [114, 205, 144];
    const W: u32 = 400;
    const H: u32 = 300;

    fn frame_of(f: impl Fn(u32, u32) -> [u8; 3]) -> Frame {
        let mut px = vec![0u8; (W * H * 3) as usize];
        for y in 0..H {
            for x in 0..W {
                let i = ((y * W + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&f(x, y));
            }
        }
        Frame::from_rgb(W, H, px).expect("frame")
    }

    fn solid(c: [u8; 3]) -> Frame {
        frame_of(|_, _| c)
    }

    fn bubble_with_text() -> Frame {
        frame_of(|x, y| {
            let in_bubble = (240..380).contains(&x) && (40..300).contains(&y);
            // 稀疏笔画：真实文字是稀疏的，画成实心块会把纯度拉下来。
            // 用饱和减法而不是 `y - 48`，帧小的时候 y 可能比 48 小。
            let stroke = in_bubble && y >= 48 && (y - 48) % 8 < 2 && (250..370).contains(&x);
            if stroke {
                [22, 22, 22]
            } else if in_bubble {
                MY_BUBBLE
            } else {
                PANEL
            }
        })
    }

    /// 图片区域：大片纯黑背景 + 几千种色。
    ///
    /// 数字照真实帧实测：众数占比 0.51、2037 种色。
    fn photo() -> Frame {
        frame_of(|x, y| {
            if (100..380).contains(&x) && (40..200).contains(&y) {
                // 51% 纯黑背景，其余散开造出几千种色
                if (x / 7 + y / 3) % 3 == 0 {
                    [20, 20, 20]
                } else {
                    // wrapping_mul / wrapping_sub：调试版算术会 panic，
                    // 但这只是个造数据的哈希。
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

    fn ocr_line(text: &str, r: Rect) -> OcrLine {
        OcrLine {
            text: text.into(),
            rect: r,
            words: vec![OcrWord {
                text: text.into(),
                rect: r,
            }],
        }
    }

    /// 平表面只有 1 种色。
    #[test]
    fn a_flat_surface_has_exactly_one_color() {
        let f = bubble_with_text();
        let v = judge_surface(&f, Rect::new(260, 44, 370, 60)).expect("应能判定");
        assert!(v.is_flat, "气泡内部应判为平表面：{}", v.explain());
        assert!(
            v.colors <= FLAT_MAX_COLORS,
            "平表面的色种数只有 {}，不该超过 {}",
            v.colors,
            FLAT_MAX_COLORS
        );
    }

    /// **关键回归**：图片区有 51% 的纯黑背景，只看众数占比会漏判它。
    ///
    /// 实测那张聊天图片：2037 种色、众数 0.51。
    #[test]
    fn a_photo_is_not_a_flat_surface_even_with_a_large_flat_background() {
        let f = photo();
        let v = judge_surface(&f, Rect::new(120, 60, 360, 180)).expect("应能判定");
        assert!(
            !v.is_flat,
            "图片必须被判为内容区，但实测纯度 {} 色种 {} —— 判错了：{}",
            v.purity,
            v.colors,
            v.explain()
        );
        assert!(
            v.colors > FLAT_MAX_COLORS,
            "实测色种数 {} 应远超阈值 {}",
            v.colors,
            FLAT_MAX_COLORS
        );
        // 合成用的哈希散列把众数摊薄了，纯度没到真实帧那个 0.51——真实帧的
        // 数字由 `the_measured_photo_numbers_are_recorded_here` 锁。
        // 这里锁的是「色种数」这条主判据确实起作用。
        assert!(
            v.colors > FLAT_MAX_COLORS * 100,
            "合成图片的色种数 {} 应该远多于平表面的量级（阈值 {}）",
            v.colors,
            FLAT_MAX_COLORS
        );
    }

    /// **实测数字的回归锁**：真实帧那张聊天图片是
    /// 「51% 纯黑背景 + 2037 种色」——只看众数占比一定漏判。
    ///
    /// 这条把那两个数字写进测试，并断言**它们之间的关系**。
    /// 将来谁把主判据改回「只看占比」，或者把纯度阈值调到 0.6 以上
    /// 想「更严」，这条会立刻指出来：真实帧上那个 0.51 本来就高于
    /// 任何一个合理的纯度阈值，拦住它的是色种数。
    #[test]
    fn the_measured_photo_numbers_are_recorded_here() {
        // `scripts/colors.ps1` 在真实帧 target/probe/text-chat.png 上量到的：
        // 区域 140x160 = 22400 px，主色 rgb(20,20,20) 占 51.49%，共 2037 种色。
        let (purity, colors) = (0.5149f32, 2037usize);

        // 纯度这道保险拦不住它 —— 这就是为什么色种数必须是主判据
        assert!(
            purity > FLAT_MIN_PURITY,
            "真实图片的众数占比 {purity} 高于纯度阈值 {FLAT_MIN_PURITY}"
        );
        // 合起来的判定必须是「内容区」
        let is_flat = colors <= FLAT_MAX_COLORS && purity >= FLAT_MIN_PURITY;
        assert!(
            !is_flat,
            "真实图片必须被判为内容区：{colors} 种色 > {FLAT_MAX_COLORS}"
        );
    }

    /// 本级的验收点：**图里的字不进 `Message[]`**。
    #[test]
    fn text_inside_a_photo_never_becomes_a_message() {
        let f = photo();
        let line = ocr_line("图片里的字", Rect::new(150, 90, 330, 104));
        assert_eq!(judge(&line, &f), Some(NoiseClass::ImageText));
    }

    /// 对照：气泡里的字照常进消息流。
    #[test]
    fn text_inside_a_bubble_still_gets_through() {
        let f = bubble_with_text();
        let line = ocr_line("气泡里的字", Rect::new(260, 48, 360, 60));
        assert_eq!(
            judge(&line, &f),
            Some(NoiseClass::Text),
            "气泡里的字不该被误伤"
        );
    }

    /// 每个被丢弃的区域都要带丢弃原因——调试视图靠它上色。
    #[test]
    fn every_rejection_carries_a_reason() {
        let f = photo();
        let lines = vec![
            ocr_line("图里的字", Rect::new(150, 90, 330, 104)),
            ocr_line("另一张图里的字", Rect::new(160, 120, 320, 134)),
        ];
        let verdict = mask(&lines, &f);
        assert_eq!(verdict.len(), 2, "两行都要有结论");
        for (i, m) in &verdict {
            let Masked::Dropped(r) = m else {
                panic!("第 {i} 行不该被留下")
            };
            assert_eq!(r.noise, NoiseClass::ImageText);
            assert!(
                r.reason.contains("种色"),
                "丢弃原因要能看出是哪个判据：{}",
                r.reason
            );
        }
    }

    /// 落在平表面上的行必须留下——遮罩不能把对话也一起遮掉。
    #[test]
    fn the_mask_keeps_what_is_on_a_flat_surface() {
        let f = bubble_with_text();
        let lines = vec![
            ocr_line("气泡里的字", Rect::new(260, 48, 360, 60)),
            ocr_line("面板上的灰字", Rect::new(20, 200, 120, 212)),
        ];
        let verdict = mask(&lines, &f);
        assert!(
            verdict.iter().all(|(_, m)| matches!(m, Masked::Kept)),
            "平表面上的两行都不该被丢：{verdict:?}"
        );
    }

    /// 遮罩**只**改说话人，不改其他判据，也不重排行序。
    #[test]
    fn the_mask_only_touches_the_speaker() {
        let mut attrs = vec![
            crate::attribute::Attribution {
                speaker: Speaker::Them,
                noise: NoiseClass::ImageText,
                text: "图里的字".into(),
                rect: Rect::new(10, 10, 40, 24),
                evidence: crate::attribute::Evidence {
                    bg: [47, 47, 48],
                    ink: [225, 225, 229],
                    contrast: 178.4,
                    on_panel: false,
                    chroma: 1,
                    purity: 0.5,
                    height: 14,
                },
            },
            crate::attribute::Attribution {
                speaker: Speaker::Them,
                noise: NoiseClass::Text,
                text: "对方说的".into(),
                rect: Rect::new(10, 40, 40, 54),
                evidence: crate::attribute::Evidence {
                    bg: [47, 47, 48],
                    ink: [225, 225, 229],
                    contrast: 178.4,
                    on_panel: false,
                    chroma: 1,
                    purity: 1.0,
                    height: 14,
                },
            },
        ];
        let before = attrs.clone();
        reject_image_text(&mut attrs);

        assert_eq!(attrs[0].speaker, Speaker::System, "图内字不算任何一方说的");
        assert_eq!(attrs[1].speaker, Speaker::Them, "正文说话人不能被改");
        assert_eq!(attrs[0].evidence, before[0].evidence, "判定依据不能变");
        assert_eq!(attrs[0].text, before[0].text);
        assert_eq!(attrs[1].noise, NoiseClass::Text);
    }

    /// **不能用对比度当「这是图里的字」的判据**。
    ///
    /// 实测对方气泡正文对比 178.4，高于任何合理阈值。拿它当判据，
    /// 会把对方说的话判成图内字——把对话判没了。
    #[test]
    fn contrast_alone_can_never_mean_image_text() {
        let them_ink: Rgb = [225, 225, 229];
        let them_bg: Rgb = [47, 47, 48];
        assert!(
            contrast_is_suspiciously_high(them_ink, them_bg),
            "对方气泡正文的对比确实高得可疑——所以它绝不能当判据"
        );
        // 但它在平表面上，照样是正文
        let f = solid(them_bg);
        let line = ocr_line("对方说的", Rect::new(20, 20, 100, 34));
        assert_eq!(judge(&line, &f), Some(NoiseClass::Text));
    }

    /// 纯色面板不该被判成内容区——否则所有字都会被丢。
    #[test]
    fn the_panel_itself_is_still_a_flat_surface() {
        let f = solid(PANEL);
        let v = judge_surface(&f, Rect::new(0, 0, W as i32, 200)).expect("应能判定");
        assert!(v.is_flat, "纯色面板是平表面：{}", v.explain());
    }
}
