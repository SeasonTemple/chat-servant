//! 窗口发现与 Windows Graphics Capture。只认识 HWND，不认识任何聊天软件语义。

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
