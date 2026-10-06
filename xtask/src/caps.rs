//! 采集能力探针：回答「这台机器到底支不支持 WGC」。
//!
//! WGC 会话静默死掉有好几种可能，必须先把「系统层面就不支持」和
//! 「支持但本 crate 走到某条不通的路上」区分开。
//! `GraphicsCaptureSession::IsSupported()` 就是这个分界线。

use windows::Graphics::Capture::GraphicsCaptureSession;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

/// `None` = 没测出来；`Some(Ok(false))` = 系统明确说不支持；
/// `Some(Err(..))` = WinRT 调用本身失败。
pub type Support = Option<Result<bool, String>>;

pub struct Caps {
    pub os_build: String,
    pub wgc_supported: Support,
}

/// 不抓窗口，纯查能力，不落盘。
pub fn probe() -> Caps {
    // WinRT 必须在用 WinRT API 之前初始化。没初始化的话 IsSupported()
    // 会直接抛异常而不是返回 false——这两种情况必须分开报。
    let wgc_supported: Support = match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        Ok(()) => Some(GraphicsCaptureSession::IsSupported().map_err(|e| e.to_string())),
        Err(e) => Some(Err(format!("RoInitialize 失败：{e}"))),
    };

    Caps {
        os_build: os_build(),
        wgc_supported,
    }
}

/// 用 `RtlGetVersion` 而不是 `GetVersion`：后者在没有 manifest 声明时
/// 会谎报成 6.2，拿到的版本号是假的。`RtlGetVersion` 在 ntdll 里，
/// windows-rs 没绑，所以自己声明。
fn os_build() -> String {
    #[repr(C)]
    struct RtlOsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
    }
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut RtlOsVersionInfo) -> i32;
    }

    let mut info = RtlOsVersionInfo {
        size: std::mem::size_of::<RtlOsVersionInfo>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        service_pack: [0; 128],
    };
    // NTSTATUS >= 0 即成功
    if unsafe { RtlGetVersion(&raw mut info) } >= 0 {
        format!("{}.{}.{}", info.major, info.minor, info.build)
    } else {
        "未知".into()
    }
}
