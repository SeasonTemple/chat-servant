//! 域类型与不依赖平台的纯逻辑。这里不放任何 Win32 或网络代码。

pub mod dispatch;
pub mod frame;
pub mod redact;
pub mod scale;

pub use dispatch::{DeliveryReceipt, Dispatch, Dispatcher, Next};
pub use frame::{Frame, Rect};
pub use scale::{Billing, Fitted, Unfit, fit, logical_size};
