//! 桌面端不需要这个库。Tauri 2 要求 crate 里有一个 `[lib]`，是因为
//! iOS / Android 目标要把入口导出给原生工程；桌面端的主程序在 `main.rs`。
//!
//! 这里保持空，不要往里塞业务——业务放 `commands.rs`，它被 `main.rs` 使用。

/// 应用版本。与 `tauri.conf.json` 里的 `version` 保持一致。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 凭据存放的目标名（ticket 13）。**不是文件路径**——
/// API key 只从 Windows 凭据管理器或环境变量读，绝不落普通文件。
pub const CREDENTIAL_TARGET: &str = "chat-servant";
