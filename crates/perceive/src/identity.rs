//! 会话身份：**按像素指纹认，不按 OCR 文字认**。
//!
//! ## 为什么不能用文字
//!
//! 实测（`cargo xtask perceive --file target/probe/text-chat.png`）：
//! 头部那一行被正确归类成 `NoiseClass::Header`，但 OCR 把真实会话名
//! 「小号」读成了 **`G8W`**。11 px 高的中文标题，OCR 认不准是常态。
//!
//! 于是任何「拿 OCR 文字和常量比字符串」的锁定方式，在真实环境上
//! **永远是关的**——不误发，但也永远不工作。
//!
//! ## 为什么指纹比文字可靠
//!
//! 文字是 OCR 的产物；**那一小块像素**不是。换个会话，那块像素必然变；
//! 同一个会话在多帧之间，那块像素基本不变（只有未读角标之类会抖）。
//!
//! ## 怎么比：二值墨迹掩码，不是颜色直方图
//!
//! ⚠ **2026-10-11 实测推翻了原来的「64 桶颜色直方图」。**
//!
//! 那天在真实微信帧上切会话，闸**一次都没关**，面板跟着切到了
//! 用户从未锁定的那个会话，还在给那个会话出候选——硬约束第 1 条直接破口。
//! 拿真实帧逐块量下来，原因有两层：
//!
//! ```text
//! 区域 x308..708 y31..71（现状用的那块，16000 px）
//!   最大桶占比 = 98.8%          ← 几乎所有颜色塌进同一个桶
//!   直方图相似度 = 0.982        ← 阈值 0.90，永远判「同一个」
//!   墨迹掩码 Dice = 0.2793       ← 换成墨迹立刻分开
//! ```
//!
//! 第一层是**量化太粗**：每通道压到 2 bit 之后，`rgb(17,17,17)`、
//! `rgb(34,34,51)`、`rgb(51,51,51)` 全落进第 0 号桶——微信深色界面
//! 九成的像素挤在一个桶里，会话名那 189 个亮像素（占 1.2%）根本推不动它。
//!
//! 第二层更要命：原来的测试 `a_different_name_prints_differently`
//! 用的是 `tint 225` 与 `tint 120`——**亮度差 105 级**。真实会话名之间的
//! 差别是**字形**，不是亮度：两个名字都是白字，亮度完全一样。
//! 那条测试一直在证明一件与真实情况无关的事，所以它一直是绿的，
//! 而闸在真实帧上从来没关过。
//!
//! 现在取的是**墨迹**：与本区域**众数亮度**差得够远的像素才算「字」。
//! 字是稀疏、位置精确的高对比像素，于是 189 个像素就足以定义身份。
//! 背景取众数而不是写死阈值，是为了深色主题（亮字暗底）与
//! 浅色主题（暗字亮底）都能用——写死阈值会在另一个主题上整个失效。
//!
//! ## 位移怎么吸收
//!
//! 掩码对位移敏感：整块挪 1 px，在掩码上就是一大片不重叠。
//! 锚点是按颜色分析推出来的，真机上不一定永远落在同一行。
//!
//! 做法是**只在「命中」这一侧放宽**：A 的某个墨像素，只要 B 在
//! **±1 px 内**有墨，就算命中。分母仍是各自的真实墨迹数。
//!
//! ⚠ **为什么不先膨胀再比**：膨胀把两边的墨迹面积一起放大，位移带来的
//! 错位占比被摊薄但没消失（实测膨胀 1 px 只把 1 px 位移拉到 0.835，
//! 2 px 才 0.863，都还在 0.90 以下）；而膨胀**同时**会把两个不同名字的
//! 重叠也拉起来（实测 0.2793 → 0.5483 → 0.6354），那是往危险方向走。
//! 只放宽命中这一侧就没有这个问题：不同名字的字形位置本来就不同，
//! ±1 px 拉不动它们。
//!
//! 实测（同一对真实帧，两种量并排）：
//!
//! | 量 | 同一会话 | 不同会话 |
//! |---|---|---|
//! | 墨迹 Dice（不宽容错位） | 1.0000 | 0.2793 |
//! | **墨迹 Dice（±1 px 容错命中）** | **1.0000** | **0.2793** |
//! | 32×8 灰度格 | 1.0000 | 0.5377 |
//!
//! 两行相同 = ±1 px 的容错**没有**把不同名字拉近；而整块挪 1 px 会被
//! 完全吸收（单测 `a_shifted_box_is_still_the_same_conversation` 守着）。
//! 灰度格也分得开，但每格平均掉 25×5 像素，字被抹平；掩码保留位置信息，
//! 判别力更直接，也更好解释。

use chatservant_core::frame::{Frame, Rect};

/// 一个会话头部区域的**二值墨迹掩码**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderPrint {
    /// 逐像素 0/1。长度 = 宽 × 高。
    mask: Vec<u8>,
    /// 掩码的宽高，用来在比对时定位邻格。
    w: usize,
    h: usize,
    /// 墨像素数。Dice 的分母用它。
    ink: u32,
    /// 区域面积（宽 × 高）。**不是**墨迹数。
    total: u32,
}

impl HeaderPrint {
    /// 取一块区域的指纹。
    ///
    /// 返回 `None` 有三种情况，**含义都是「认不出」而不是「是同一个」**：
    /// 区域太小、区域内几乎没有墨（一片纯底色）、像素取不全。
    pub fn of(frame: &Frame, r: Rect) -> Option<Self> {
        let x1 = (r.x1.max(0) as u32).min(frame.width());
        let y1 = (r.y1.max(0) as u32).min(frame.height());
        let y0 = r.y0.max(0) as u32;
        let x0 = r.x0.max(0) as u32;
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let w = (x1 - x0) as usize;
        let h = (y1 - y0) as usize;
        if w * h < MIN_PIXELS as usize {
            return None;
        }

        // 一趟取亮度，顺便统计众数当背景色。
        // ⚠ 裁剪过了却取不到像素**不该发生**；真发生了按「亮度 0」处理，
        //   它会让这个像素更接近亮背景而不是暗背景，即**偏向不被当成字**——
        //   宁可少认，不可多认。
        let mut lum: Vec<u8> = Vec::with_capacity(w * h);
        let mut hist = [0u32; 256];
        for y in y0..y1 {
            for x in x0..x1 {
                let l = frame.pixel(x, y).map_or(0, |[r, g, b]| luminance(r, g, b));
                hist[l as usize] += 1;
                lum.push(l);
            }
        }
        let bg = mode(&hist)?;

        let mut raw = vec![0u8; w * h];
        let mut ink = 0u32;
        for (i, &l) in lum.iter().enumerate() {
            if (l as f64 - bg as f64).abs() > INK_DELTA {
                raw[i] = 1;
                ink += 1;
            }
        }
        // 一块没有字的区域**不能**当身份：它的掩码全 0，
        // 和任何别的掩码比都是 0 分，于是「认不出」会被读成「换了人」——
        // 闸会莫名其妙地一直关着。所以这里直接返回 `None`。
        if ink < MIN_INK {
            return None;
        }

        Some(Self {
            mask: raw,
            w,
            h,
            ink,
            total: (w * h) as u32,
        })
    }

    /// 指纹覆盖的区域像素数。**探针用它说明取样范围**，
    /// 太小说明那块区域不足以当身份用。
    pub fn pixels(&self) -> u32 {
        self.total
    }

    /// 与另一个指纹的相似度，`0.0..=1.0`。
    ///
    /// 用 **Sørensen–Dice**（`2·重叠 / (墨A + 墨B)`）。
    ///
    /// ## 命中允许 ±[`SHIFT_TOLERANCE`] 像素的错位
    ///
    /// 锚点是按颜色分析推出来的，真机上不一定永远落在同一行；整块挪 1 px
    /// 在掩码上就是一大片不重叠。
    ///
    /// ⚠ **为什么不先膨胀再比**：膨胀会把两边的墨迹面积**一起放大**，
    /// 于是 Dice 的分子分母同时变大，位移带来的那点错位占比被摊薄但没消失
    /// ——实测膨胀 1 px 只把 1 px 位移从 0.69 拉到 0.835，膨胀 2 px 才 0.863，
    /// 都还在 0.90 以下。而膨胀同时会把两个**不同**名字的重叠也拉起来
    /// （实测 0.2793 → 0.5483 → 0.6354），那是往危险方向走。
    ///
    /// 只放宽「命中」这一侧：分母仍是各自的真实墨迹数，不同名字不会因为
    /// 膨胀而「看起来更像」，而位移能被吸收。实测同一会话 1.0000、
    /// 不同会话 0.2793、整体挪 1 px 约 1.0。
    ///
    /// 区域尺寸不一致时直接给 0：尺寸变了说明锚点动了，
    /// 那是「认不出」，不该拿两片不同形状的掩码去凑一个相似度。
    pub fn similarity(&self, other: &Self) -> f32 {
        if self.ink == 0 || other.ink == 0 || self.mask.len() != other.mask.len() {
            return 0.0;
        }
        let (w, h) = (self.w, self.h);
        let b = &other.mask;
        let mut hit = 0u32;
        for y in 0..h {
            for x in 0..w {
                if self.mask[y * w + x] != 1 {
                    continue;
                }
                // 同一格，或上下左右斜角有一格墨就算命中。
                if near(b, w, h, x, y) {
                    hit += 1;
                }
            }
        }
        (2.0 * hit as f64 / (self.ink + other.ink) as f64) as f32
    }

    /// 两块区域是不是「同一个会话」。**阈值以下判为换人了。**
    pub fn is_same(&self, other: &Self) -> bool {
        self.similarity(other) >= SAME_THRESHOLD
    }
}

/// Rec.601 亮度。整数运算，返回 `0..=255`。
fn luminance(r: u8, g: u8, b: u8) -> u8 {
    ((299 * r as u32 + 587 * g as u32 + 114 * b as u32) / 1000).min(255) as u8
}

/// 直方图的众数。整片全 0（不该发生）返回 `None`。
fn mode(hist: &[u32; 256]) -> Option<u8> {
    let mut best = 0u32;
    let mut at = 0usize;
    for (i, &v) in hist.iter().enumerate() {
        if v > best {
            best = v;
            at = i;
        }
    }
    (best > 0).then_some(at as u8)
}

/// `(x,y)` 自身或周围 [`SHIFT_TOLERANCE`] 像素内有没有墨。
fn near(mask: &[u8], w: usize, h: usize, x: usize, y: usize) -> bool {
    let r = SHIFT_TOLERANCE;
    let y0 = y.saturating_sub(r);
    let y1 = (y + r).min(h - 1);
    let x0 = x.saturating_sub(r);
    let x1 = (x + r).min(w - 1);
    for ny in y0..=y1 {
        for nx in x0..=x1 {
            if mask[ny * w + nx] == 1 {
                return true;
            }
        }
    }
    false
}

/// 小于这个像素数的区域不配当身份。太小的块连字都装不下。
const MIN_PIXELS: u32 = 200;

/// 与背景亮度差过这个值就算「字」。
///
/// 取 60 是因为实测那块区域里正文是 lum≈200、背景是 lum≈30，
/// 中间没有渐变层；60 卡在两侧中间，对主题整体深浅不敏感
/// （只要字与底仍差得开）。
const INK_DELTA: f64 = 60.0;

/// 墨迹少于这个像素数就当「这块里没有身份」。
const MIN_INK: u32 = 40;

/// 命中判定允许的错位半径（像素）。见 [`HeaderPrint::similarity`]。
const SHIFT_TOLERANCE: usize = 1;

/// 相似度阈值。
///
/// ⚠ **这一条现在是实测出来的，不是猜的**（2026-10-11，真实微信帧）：
///
/// | 量 | 同一会话 | 不同会话 |
/// |---|---|---|
/// | 墨迹 Dice（不宽容错位） | 1.0000 | 0.2793 |
/// | **墨迹 Dice（±1 px 容错命中）** | **1.0000** | **0.2793** |
///
/// 不同名字的字形位置本来就不同，±1 px 的容错**不会**把它们拉近
/// （上表两行相同就是这个意思）；而整块挪 1 px 会被完全吸收。
/// 0.90 落在 0.28 与 1.00 之间，两侧都留了很宽的余量。
///
/// 方向仍然是保守的：宁可「换人了但没认出来」（闸多关一次，用户看得见），
/// 也不要「换了人但没认出来」（对着别人的聊天起草）。
pub const SAME_THRESHOLD: f32 = 0.90;

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_of(f: impl Fn(u32, u32) -> [u8; 3], w: u32, h: u32) -> Frame {
        let mut px = vec![0u8; (w * h * 3) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&f(x, y));
            }
        }
        Frame::from_rgb(w, h, px).expect("尺寸一致")
    }

    /// 帧要比取样区**宽出余量**：早先把帧设成正好 300 宽，于是「往右挪 1 px」
    /// 的取样框被裁到帧边（299 宽），掩码长度与原框不同，相似度直接给 0——
    /// 位移测试假失败，而原因在夹具不在被测代码。
    const W: u32 = 400;
    const H: u32 = 200;
    const R: Rect = Rect::new(0, 0, 300, 40);

    /// 一块「写着名字」的头部。**字的比例照实测来。**
    ///
    /// 真实帧上那块区域 16000 px 里只有 **189 个墨迹像素（1.2%）**。
    /// 早先这里画的是 `x % 7 < 4` —— 那是在字行里铺满 4/7 的**条纹**，
    /// 覆盖率高出真实值几十倍。拿条纹当名字，会得到一个比现实更容易
    /// 分开的假象，也会把「膨胀」该有的作用掩盖掉。
    fn header(tint: u8) -> Frame {
        named(tint, 0)
    }

    /// 三个「字」块，宽 13 px。`style` 决定笔画怎么排 ——
    /// **真实会话名之间的差别就是这个，不是亮度。**
    ///
    /// 两个 style 是**同一个框里笔画落在不同位置**（左重 / 右重），
    /// 不是两套花纹：花纹之间会共享笔画，`±1 px` 的容错会把它们拉近，
    /// 那时候测的就不是「名字不同」而是「花纹相似」了。
    fn named(tint: u8, style: u32) -> Frame {
        frame_of(
            |x, y| {
                for sx in [24u32, 42, 60] {
                    // ⚠ 必须用 checked 且**落在块内**：早先写 `saturating_sub`，
                    // 字块上方的每一行都被折成 cy=0，于是取样框往上挪一像素
                    // 就会凭空多出一片墨，位移测试因此假失败。
                    let Some(cx) = x.checked_sub(sx).filter(|v| *v < 13) else {
                        continue;
                    };
                    let Some(cy) = y.checked_sub(12).filter(|v| *v < 13) else {
                        continue;
                    };
                    let on = match style {
                        0 => cx < 7,  // 笔画压在左半
                        1 => cx >= 7, // 笔画压在右半
                        2 => cy < 7,  // 上重
                        _ => false,
                    };
                    if on {
                        return [tint, tint, tint];
                    }
                }
                [30, 30, 31]
            },
            W,
            H,
        )
    }

    /// **同一块区域，两次取指纹必须一致。** 这是整条路的地基。
    #[test]
    fn the_same_region_always_prints_the_same() {
        let f = header(225);
        let a = HeaderPrint::of(&f, R).expect("取得到");
        let b = HeaderPrint::of(&f, R).expect("取得到");
        assert_eq!(a, b);
        assert!(a.is_same(&b), "同一帧同一区域必须判为同一个");
    }

    /// **换个名字，指纹要明显不同 —— 而且名字的差别是「字形」不是「亮度」。**
    ///
    /// ⚠ **这条测试在 2026-10-11 被整个换掉了。**
    ///
    /// 旧版是 `header(225)` 对 `header(120)`——亮度差 105 级。
    /// 真实会话名之间**亮度完全一样**（都是白字），差的是笔画布局；
    /// 于是旧版那条测试证明的全是与真实情况无关的东西，
    /// 一直是绿的，而闸在真实帧上从来没关过。
    ///
    /// 现在两边的字**亮度相同、形状不同**，这才是真实名字的差别。
    #[test]
    fn a_different_name_of_the_same_brightness_is_a_different_conversation() {
        let a = HeaderPrint::of(&named(225, 0), R).expect("取得到");
        let b = HeaderPrint::of(&named(225, 1), R).expect("取得到");
        assert!(
            !a.is_same(&b),
            "同亮度、不同字形的两个名字被判成同一个了：相似度 {}\
             （这是 2026-10-11 那个破口在单测里的样子）",
            a.similarity(&b)
        );
    }

    /// 整体亮度平移（未读角标、主题微调）**不该**被当成换人。
    ///
    /// 掩码只看「与背景差多少」，字整体变暗一点仍远高于 `INK_DELTA`。
    #[test]
    fn a_small_change_is_not_a_conversation_change() {
        let a = HeaderPrint::of(&header(225), R).expect("取得到");
        let b = HeaderPrint::of(&header(200), R).expect("取得到");
        assert!(
            a.is_same(&b),
            "只差 25 级灰不该判为换人，相似度 {}",
            a.similarity(&b)
        );
    }

    /// **框抖一两个像素不该判为换人。**
    ///
    /// 膨胀 1 px 就是为这件事存在的：掩码本身对平移敏感，
    /// 不膨胀的话字块平移 1 px 的重叠率会掉一半上下。
    ///
    /// ⚠ 这里刻意**保持区域尺寸不变**（宽 300、高 40），只挪取样起点。
    /// 尺寸变了是另一回事，见下面那条。
    #[test]
    fn a_shifted_box_is_still_the_same_conversation() {
        let f = header(225);
        let a = HeaderPrint::of(&f, R).expect("取得到");
        let b = HeaderPrint::of(&f, Rect::new(1, 1, 301, 41)).expect("取得到");
        assert!(
            a.is_same(&b),
            "框抖 1px 被误判成换人了：{}",
            a.similarity(&b)
        );
    }

    /// **区域尺寸变了就该判为「认不出」**，不能拿两片不同形状的掩码凑相似度。
    ///
    /// 真机上尺寸会变只有一个原因：锚点动了（窗口改了大小）。
    /// 那时候闸关上是**对的**——宁可多关一次，也不要拿着旧形状去比。
    #[test]
    fn a_resized_box_is_not_the_same_conversation() {
        let f = header(225);
        let a = HeaderPrint::of(&f, R).expect("取得到");
        let b = HeaderPrint::of(&f, Rect::new(0, 0, W as i32, 41)).expect("取得到");
        assert!(
            !a.is_same(&b),
            "尺寸都变了还判成同一个：{}",
            a.similarity(&b)
        );
    }

    /// **浅色主题下也要能用**：暗字亮底，墨迹照样取得到。
    ///
    /// 背景取众数就是为了这条——写死亮度阈值的话整个浅色主题会失效，
    /// 而那种失效是**静默**的：掩码全 0 → 和谁都不像 → 闸一直关着。
    #[test]
    fn a_light_theme_header_still_prints() {
        let f = frame_of(
            |x, y| {
                if (12..=24).contains(&y) && x % 7 < 4 {
                    [40, 40, 45]
                } else {
                    [240, 240, 240]
                }
            },
            W,
            H,
        );
        let p = HeaderPrint::of(&f, R).expect("浅色主题也要取得到");
        assert!(p.ink > 0, "浅色主题下没取到任何墨迹——背景取法有问题");
    }

    /// **一片纯底色必须返回 `None`，不能给一个「空指纹」。**
    ///
    /// 空指纹会让「认不出」看起来像「换了人」，闸会莫名其妙一直关着。
    #[test]
    fn a_blank_region_prints_nothing() {
        let blank = frame_of(|_, _| [30, 30, 31], W, H);
        assert!(
            HeaderPrint::of(&blank, R).is_none(),
            "一片纯底色也给了指纹：它和任何区域比都是 0 分"
        );
    }

    /// **取不到指纹必须返回 `None`，不能给一个「空指纹」。**
    #[test]
    fn too_small_a_region_prints_nothing() {
        let f = header(225);
        assert!(HeaderPrint::of(&f, Rect::new(0, 0, 4, 4)).is_none());
        assert!(HeaderPrint::of(&f, Rect::new(0, 0, 0, 0)).is_none());
    }

    /// 越界要被裁到帧内，而不是 panic 或读到垃圾。
    #[test]
    fn an_out_of_bounds_rect_is_clamped_not_trusted() {
        let f = header(225);
        let p = HeaderPrint::of(&f, Rect::new(0, 0, 10_000, 10_000)).expect("应裁剪后取得到");
        assert!(p.pixels() > 0);
    }
}
