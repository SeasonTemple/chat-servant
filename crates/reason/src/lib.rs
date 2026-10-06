//! 推理层：`Message[]` + 配置 → `DraftSet`。
//!
//! **这是唯一允许出网做模型调用的阶段**（CONTEXT.md 的系统边界）。
//! 但「唯一允许」不等于「必须出网」——传输是 [`model::Reasoner`] trait，
//! 测试用假实现，于是整个推理接缝的测试**不联网、不花钱、可确定性重跑**。
//!
//! 三段式与降级路径见 [`seam`]。

pub mod clean;
pub mod model;
pub mod seam;

#[cfg(test)]
mod gate;
#[cfg(test)]
mod quiet;

pub use model::{Candidate, DraftSet, Intent, ReasonError, Reasoner, TurnContext};
pub use seam::{Turn, turn};
