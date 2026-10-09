//! 窗口发现与 Windows Graphics Capture。只认识 HWND，不认识任何聊天软件语义。

// ⚠ **`missing_docs` 在这个 crate 上不适用。**
//
// 工作区的 lint 策略里有 `missing_docs = "warn"`，而 `gate.ps1` 跑的是
// `clippy -- -D warnings`，于是它在这个 crate 上会变成 75 条 error——
// 全部是 `AnchorError::TooSmall(u32, u32)`、`WgcError::Buffer(String)`
// 这类**错误枚举的变体**。
//
// 这正是那条策略自己反对的事：「一上来全 deny 会产生几百条噪声，
// **噪声本身会让人忽略警告**」。
//
// 为什么在这里特别不适用：本 crate 是一层 **Win32 FFI 绑定**。
// `TooSmall`、`NoPanelBg`、`Buffer(String)` 这些名字本身就是文档——
// 它们逐字对应 Win32 的错误语义，补 75 段注释只会让真正的警告更难看见。
//
// 纯逻辑的那三个 crate（`core` / `perceive` / `reason`）**没有**这条豁免，
// 它们的公开 API 仍然受 `missing_docs` 约束。
#![allow(missing_docs)]

pub use chatservant_core::frame::Frame;

pub mod anchor;
pub mod anchor_diag;
pub mod inject;
pub mod media;
pub mod ocr;
pub mod printwindow;
pub mod region;
pub mod surface;
pub mod wgc;
pub mod wgc_alt;
pub mod window;

pub use surface::{ALL, ChatSurface, WECHAT};
pub use wgc::{CaptureError, capture_one_frame, is_alive};
pub use window::{
    WindowError, WindowHit, enumerate_for, find_chat_window, restore_without_activating,
    select_main,
};

/// 从一张 PNG 读一帧。
///
/// ## 为什么住在 capture 而不是 app
///
/// `Frame` 的家在这里，`image` 也已经是本 crate 的依赖。放 app 里就得
/// 为同一件事再加一条依赖，而且「怎么把一张图变成帧」在两个地方各写一遍
/// 迟早分叉（一个转 RGB、一个转 RGBA，最后颜色通道对不上）。
///
/// ## 它为什么重要
///
/// 没有它，**整个应用只能在微信真的开着、并且窗口摆对的时候跑**。
/// 于是改一次界面就得去点一次微信、截一次图、确认一次像素，
/// 而这些步骤每次结果都不一样——「界面能不能跑通」这件事就没法被可靠验证。
///
/// 有它之后，OCR、归类、会话锁定、起草、事件推送、界面渲染一条都不少，
/// 只有「这一帧从哪来」变了。见 `app/watch::FrameSource`。
pub fn frame_from_png(path: &std::path::Path) -> Result<Frame, String> {
    let img = image::open(path)
        .map_err(|e| format!("打开 PNG 失败：{e}"))?
        .to_rgb8();
    // `RgbImage` 自带 width/height，不需要 `GenericImageView` 那个 trait。
    let (w, h) = (img.width(), img.height());
    Frame::from_rgb(w, h, img.into_raw())
        .ok_or_else(|| format!("PNG 声明 {w}x{h}，但缓冲区对不上——解码出来的像素数量与尺寸不符"))
}
