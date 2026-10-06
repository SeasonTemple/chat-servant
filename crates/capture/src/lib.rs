//! 窗口发现与 Windows Graphics Capture。只认识 HWND，不认识任何聊天软件语义。

pub use chatservant_core::frame::Frame;

pub mod anchor;
pub mod anchor_diag;
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
