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
//! ## 怎么比：颜色直方图，不是逐字节
//!
//! 逐字节比对太脆——OCR 框抖动一两个像素，内容就全变了。
//! 所以取**颜色直方图**：每通道压到 2 bit（4 级），共 64 个桶，
//! 统计像素数。小位移对直方图几乎没影响，而「换了名字」会明显改变它。
//!
//! 比较用**相似度**而不是相等——未读角标、时间戳这类小幅变化
//! 不该把闸关掉，��是**明显换人**才该关。

use chatservant_core::frame::{Frame, Rect};

/// 每通道 4 级 → 4×4×4 = 64 个桶。
const BUCKETS: usize = 64;

/// 一个会话头部区域的颜色直方图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderPrint {
    counts: [u16; BUCKETS],
    total: u32,
}

impl HeaderPrint {
    /// 取一块区域的指纹。区域太小或空时返回 `None`——
    /// **拿不到指纹不等于「是同一个会话」**，调用方必须当成「认不出」。
    pub fn of(frame: &Frame, r: Rect) -> Option<Self> {
        let x1 = (r.x1.max(0) as u32).min(frame.width());
        let y1 = (r.y1.max(0) as u32).min(frame.height());
        let y0 = r.y0.max(0) as u32;
        let x0 = r.x0.max(0) as u32;
        // 太小就没法当身份用：一片纯底色区域的指纹对任何会话都一样。
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        if (x1 - x0) * (y1 - y0) < MIN_PIXELS {
            return None;
        }

        let mut counts = [0u16; BUCKETS];
        let mut total = 0u32;
        for y in y0..y1 {
            for x in x0..x1 {
                if let Some([r, g, b]) = frame.pixel(x, y) {
                    counts[bucket(r, g, b)] += 1;
                    total += 1;
                }
            }
        }
        (total > 0).then_some(Self { counts, total })
    }

    /// 指纹覆盖的像素数。**探针用它说明取样范围**，
    /// 太小说明那块区域不足以当身份用。
    pub fn pixels(&self) -> u32 {
        self.total
    }

    /// 与另一个指纹的相似度，`0.0..=1.0`。
    ///
    /// 用**直方图交集**（Sørensen–Dice）：两个分布共有的像素占比。
    /// 选它而不是余弦相似度，是因为桶计数本身就是分布，
    /// 交集有「同分布不同尺度也高」这个我们正想要的性质。
    pub fn similarity(&self, other: &Self) -> f32 {
        if self.total == 0 || other.total == 0 {
            return 0.0;
        }
        let mut inter = 0f64;
        for i in 0..BUCKETS {
            inter += self.counts[i].min(other.counts[i]) as f64;
        }
        (2.0 * inter / (self.total + other.total) as f64) as f32
    }

    /// 两块区域是不是「同一个会话」。**阈值以下判为换人了。**
    pub fn is_same(&self, other: &Self) -> bool {
        self.similarity(other) >= SAME_THRESHOLD
    }
}

/// 小于这个像素数的区域不配当身份。太小的块直方图区分不出会话。
const MIN_PIXELS: u32 = 200;

/// 相似度阈值。
///
/// ⚠ **未实测到会话切换时的真实分布**，取值偏保守：
/// 宁可「换人了但没认出来」（闸多关一次，用户看得见），
/// 也不要「换了人但没认出来」（对着别人的聊天起草）。
pub const SAME_THRESHOLD: f32 = 0.90;

fn bucket(r: u8, g: u8, b: u8) -> usize {
    let q = |v: u8| (v >> 6) as usize; // 0..3
    q(r) * 16 + q(g) * 4 + q(b)
}

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

    const W: u32 = 300;
    const H: u32 = 200;
    const R: Rect = Rect::new(0, 0, W as i32, 40);

    /// 一块「写着名字」的头部：近黑底 + 高对比的白字条。
    fn header(tint: u8) -> Frame {
        frame_of(
            |x, y| {
                if !(12..=24).contains(&y) {
                    [30, 30, 31]
                } else if x % 7 < 4 {
                    [tint, tint, tint]
                } else {
                    [30, 30, 31]
                }
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

    /// **换个名字，指纹要明显不同。** 名字不同 → 白的像素比例与分布不同。
    #[test]
    fn a_different_name_prints_differently() {
        let a = HeaderPrint::of(&header(225), R).expect("取得到");
        let b = HeaderPrint::of(&header(120), R).expect("取得到");
        assert!(
            !a.is_same(&b),
            "不同会话被判成同一个：相似度 {}",
            a.similarity(&b)
        );
    }

    /// 整体亮度平移（未读角标、主题微调）**不该**被当成换人。
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
    /// OCR 框每帧都会抖一点点。逐字节比对会在这里疯狂误报，
    /// 直方图就是为这件事准备的。
    #[test]
    fn a_shifted_box_is_still_the_same_conversation() {
        let f = header(225);
        let a = HeaderPrint::of(&f, R).expect("取得到");
        let b = HeaderPrint::of(&f, Rect::new(1, 1, W as i32 + 1, 42)).expect("取得到");
        assert!(a.is_same(&b), "框抖 1px 被误判成换人了");
    }

    /// **取不到指纹必须返回 `None`，不能给一个「空指纹」。**
    ///
    /// 空指纹会让「认不出」看起来像「是同一个」，而那正好是最危险的读法。
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
        assert!(p.total > 0);
    }
}
