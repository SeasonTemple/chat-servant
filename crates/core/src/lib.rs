//! 域类型与不依赖平台的纯逻辑。这里不放任何 Win32 或网络代码。

// 这个 crate 一行 unsafe 都不该有。AGENTS.md 写着「三处 forbid(unsafe_code)
// 是真的守住了」，而在此之前**那句是假的**——2026-10-17 实测全仓 `crates/`
// 下一个 `forbid(unsafe_code)` 都没有，「0 处 unsafe」靠的是自觉。
// 「实测为 0」与「写不进 unsafe」差着一个编译期守卫，这里补上后者。
#![forbid(unsafe_code)]

pub mod dispatch;
pub mod frame;
pub mod redact;
pub mod scale;

pub use dispatch::{DeliveryReceipt, Dispatch, Dispatcher, Next};
pub use frame::{Frame, Rect};
pub use scale::{Billing, Fitted, Unfit, fit, logical_size};
