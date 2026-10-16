//! 感知层：把 OCR 框归类成对话人与噪声类。
//!
//! 这一层只做**测量与判定**，不出网、不发请求、不写盘。
//! 输入是 [`Frame`] + 一行 OCR 的几何，输出是每行一个 [`Attribution`]，
//! **连同判定依据**——依据可追，认错了才能一眼看出是哪一步的锅。

// 见 `crates/core/src/lib.rs` 同一条注释：AGENTS.md 声称这里 forbid(unsafe_code)，
// 实际全仓都没有这个属性。「实测 0 处」不等于「写不进」，2026-10-17 补上守卫。
#![forbid(unsafe_code)]

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
pub use merge::{MESSAGE_WINDOW, Message, conversation_name, group, merge_into_stream};
pub use pipeline::{LineSource, OCR_UPSCALE, Read, ReadError, SystemOcr, read, read_lines};
pub use pixels::{
    BubbleHit, dominant_color, dominant_ratio, ink_ratio, mean_color, near, probe_bubble_bg,
};
pub use stream::{Decision, PaneWatcher};

#[cfg(test)]
mod tests;
