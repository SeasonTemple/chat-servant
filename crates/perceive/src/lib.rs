//! 感知层：把 OCR 框归类成对话人与噪声类。
//!
//! 这一层只做**测量与判定**，不出网、不发请求、不写盘。
//! 输入是 [`Frame`] + 一行 OCR 的几何，输出是每行一个 [`Attribution`]，
//! **连同判定依据**——依据可追，认错了才能一眼看出是哪一步的锅。

pub mod attribute;
pub mod fixture;
pub mod identity;
pub mod mask;
pub mod media;
pub mod merge;
pub mod pipeline;
pub mod pixels;
pub mod stream;

pub use attribute::{Attribution, Evidence, NoiseClass, Palette, Speaker, classify, classify_all};
pub use fixture::{keep_colors, redact_with};
pub use mask::{Masked, Rejection, SurfaceVerdict, judge_surface, mask, reject_image_text};
pub use media::{Found, MediaKind, MediaRegion, find};
pub use merge::{Message, conversation_name, group};
pub use pipeline::{LineSource, OCR_UPSCALE, Read, ReadError, SystemOcr, read, read_lines};
pub use pixels::{
    BubbleHit, dominant_color, dominant_ratio, ink_ratio, mean_color, near, probe_bubble_bg,
};
pub use stream::{Decision, PaneWatcher};

#[cfg(test)]
mod tests;
