//! 推理层：`Message[]` + 配置 → `DraftSet`。
//!
//! **这是唯一允许出网做模型调用的阶段**（CONTEXT.md 的系统边界）。
//! 但「唯一允许」不等于「必须出网」——传输是 [`model::Reasoner`] trait，
//! 测试用假实现，于是整个推理接缝的测试**不联网、不花钱、可确定性重跑**。
//!
//! 三段式（判断 → 起草 → **纯本地**排序）与降级路径见 [`seam`]。

// 见 `crates/core/src/lib.rs` 同一条注释：AGENTS.md 声称这里 forbid(unsafe_code)，
// 实际全仓都没有这个属性。「实测 0 处」不等于「写不进」，2026-10-17 补上守卫。
#![forbid(unsafe_code)]

pub mod clean;
pub mod context;
pub mod model;
pub mod peer;
pub mod seam;
pub mod transport;
pub mod vision;
pub mod voice;

#[cfg(test)]
mod gate;
#[cfg(test)]
mod mock_upstream;
#[cfg(test)]
mod quiet;

pub use context::{Context, Summarizer, Trimmed};
pub use model::{Candidate, DraftSet, Intent, ReasonError, Reasoner, StageCalls, TurnContext};
pub use peer::{same_peer, similarity};
pub use seam::{Turn, turn};
pub use transport::{
    HttpReasoner, ModelCell, ModelGroup, Protocol, Provider, ProxyCell, Route, probe_route, redact,
};
pub use vision::{Describer, ImagePayload, VisionBridge, VisionMode};
pub use voice::{VOICE_MAX_CHARS, VOICE_MAX_SAMPLES, voice_samples};
