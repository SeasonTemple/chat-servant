//! 感知层：把 OCR 框归类成对话人与噪声类。
//!
//! 这一层只做**测量与判定**，不出网、不发请求、不写盘。
//! 输入是 [`Frame`] + 一行 OCR 的几何，输出是每行一个 [`Attribution`]，
//! **连同判定依据**——依据可追，认错了才能一眼看出是哪一步的锅。

pub mod attribute;
pub mod fixture;
pub mod merge;
pub mod pixels;

pub use attribute::{Attribution, Evidence, NoiseClass, Palette, Speaker, classify, classify_all};
pub use fixture::{keep_colors, redact_with};
pub use merge::{Message, group};
pub use pixels::{
    BubbleHit, dominant_color, dominant_ratio, ink_ratio, mean_color, near, probe_bubble_bg,
};

#[cfg(test)]
mod tests;
