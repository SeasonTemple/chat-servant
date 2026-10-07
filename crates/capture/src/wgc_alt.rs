//! WGC 交叉验证：用 `wgc` 2.0.1 再试一次同一套 Windows Graphics Capture。
//!
//! ## 为什么要两个实现
//!
//! `windows-capture` 2.0.1 在本机上对**微信窗口、记事本窗口、主显示器**
//! 三者表现完全一致：会话启动后立刻关闭、不报错、一帧不来，而
//! `GraphicsCaptureSession::IsSupported()` 明确返回 `true`（见 ADR-0004）。
//!
//! 区分"环境问题"和"这个 crate 的问题"只有一个办法：**换一个独立实现再测一次**。
//! `wgc`（MIT OR Apache-2.0，github.com/Atliac/wgc）是另一个 WGC 封装，
//! 事件循环、D3D 设备创建、会话管理都是自己写的，与 `windows-capture` 无代码关联。
//!
//! 它和 `windows-capture` 用的是**同一个 `windows` crate** 的 `HWND`，
//! 所以不需要跨绑定的类型桥接。
//!
//! ## 它的迭代器语义正好适合诊断
//!
//! `Wgc` 自己实现 `Iterator`，`next()` 内部跑 `GetMessage` 消息循环。
//!
//! ## `next()` 返回 `None` 到底意味着什么（2026-10-07 查明）
//!
//! 读源码定死：`next()` 返回 `None` 的**唯一**分支是 `GetMessageW`
//! 返回 0，也就是消息队列里取到了 **WM_QUIT**（`capture.rs:203`）。
//! 而本文件用到的这条路径里，WM_QUIT 的**唯一来源**是 `Wgc::new` 给
//! `GraphicsCaptureItem` 注册的 `Closed` 事件回调——
//! 回调体第一句就是 `PostQuitMessage(0)`（`capture.rs:137-143`）。
//!
//! 所以这个症状的正确解读是：**capture item 被系统 Closed 了**，
//! 不是「帧还没来」，也不是 COM 初始化或消息循环有问题。
//!
//! ⚠ **别把它当成「环境失效」的证据。** 曾据此推断「两个实现同病，
//! 所以是环境问题」，并一路追到窗口选择上，全错。真正的原因是
//! `cargo xtask probe frame` 的默认方法指向 `windows-capture` 那条
//! 已知不工作的路——同一个窗口、同一台机器，换 `wgc` crate 立刻正常
//! （342 ms，ADR-0004）。
//!
//! 想直接看见这一行，开 `wgc=trace`：`Item closed, stopping capture`
//! 是 `debug!`，`info` 级别看不到。

use std::sync::mpsc;
use std::time::Duration;

use chatservant_core::frame::Frame;

/// 等第一帧的超时。WGC 正常在几十毫秒内送第一帧。
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum AltCaptureError {
    #[error("为 hwnd 创建 GraphicsCaptureItem 失败：{0}")]
    CreateItem(String),

    #[error("建立 WGC 会话失败：{0}")]
    Open(String),

    #[error("WGC 迭代器立刻结束（next() 返回 None），没等来任何帧")]
    IteratorEndedImmediately,

    #[error("等待第一帧超时（{secs} 秒）。迭代器在阻塞——说明消息循环是活的——但帧没来")]
    Timeout { secs: u64 },

    #[error("取帧失败：{0}")]
    Frame(String),

    #[error("抓到了帧，但缓冲区长度对不上尺寸：期望 {expected} 字节，实得 {actual}")]
    BufferSizeMismatch { expected: usize, actual: usize },

    #[error("抓到的是全黑帧")]
    BlackFrame,
}

/// 跨线程只传这个：纯数据 + 错误。
enum RawCapture {
    Ok {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Err(AltCaptureError),
}

/// 抓取 `hwnd_raw` 的一帧，用 `wgc` 实现。全程不落盘。
///
/// **整个 WGC 会话都活在那个线程里**，一个 D3D/WinRT 对象都不跨线程。
/// 跨边界的只有 `(w, h, Vec<u8>)` 和错误。
/// （`Wgc` / `GraphicsCaptureItem` / `Frame` 都含 `NonNull<c_void>`，不是 `Send`，
/// 把它们 move 进 `thread::spawn` 编译不过。）
pub fn capture_one_frame(hwnd_raw: isize) -> Result<Frame, AltCaptureError> {
    let (tx, rx) = mpsc::channel::<RawCapture>();
    std::thread::spawn(move || {
        let _ = tx.send(capture_inside(hwnd_raw));
    });

    match rx.recv_timeout(FIRST_FRAME_TIMEOUT) {
        Ok(RawCapture::Err(e)) => Err(e),
        Ok(RawCapture::Ok {
            width,
            height,
            rgba,
        }) => assemble(width, height, rgba),
        // 收不到 = 线程还卡在 next() 里 = 消息循环阻塞着。
        // 线程随进程退出而结束，这里不 join。
        Err(mpsc::RecvTimeoutError::Timeout) => Err(AltCaptureError::Timeout {
            secs: FIRST_FRAME_TIMEOUT.as_secs(),
        }),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(AltCaptureError::Frame(
            "抓帧线程提前结束且没有回传结果".into(),
        )),
    }
}

/// 必须在目标线程内调用：创建会话、跑迭代器、取出纯数据。
fn capture_inside(hwnd_raw: isize) -> RawCapture {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HWND;

    let hwnd = HWND(hwnd_raw as *mut c_void);
    let item = match wgc::window::new_item_from_hwnd(hwnd) {
        Ok(i) => i,
        Err(e) => return RawCapture::Err(AltCaptureError::CreateItem(e.to_string())),
    };

    // WgcSettings 派生 SmartDefault，默认就是 RGBA8 + 不抓光标不画边框。
    let mut settings = wgc::WgcSettings::default();
    settings.pixel_format = wgc::PixelFormat::RGBA8;

    let mut wgc = match wgc::Wgc::new(item, settings) {
        Ok(w) => w,
        Err(e) => return RawCapture::Err(AltCaptureError::Open(e.to_string())),
    };

    // next() 返回 None = 消息循环立刻退出；Some(Err) = 迭代器内部出错。
    match wgc.next() {
        None => RawCapture::Err(AltCaptureError::IteratorEndedImmediately),
        Some(Err(e)) => RawCapture::Err(AltCaptureError::Frame(e.to_string())),
        Some(Ok(frame)) => {
            let size = match frame.size() {
                Ok(s) => s,
                Err(e) => return RawCapture::Err(AltCaptureError::Frame(e.to_string())),
            };
            match frame.pixels() {
                Ok(rgba) => RawCapture::Ok {
                    width: size.width,
                    height: size.height,
                    rgba,
                },
                Err(e) => RawCapture::Err(AltCaptureError::Frame(e.to_string())),
            }
        }
    }
}

/// 纯数据组装：RGBA8 → RGB8，顺带全黑检测与尺寸校验。
fn assemble(w: u32, h: u32, rgba: Vec<u8>) -> Result<Frame, AltCaptureError> {
    let expected = (w as usize) * (h as usize) * 4;
    if rgba.len() != expected {
        return Err(AltCaptureError::BufferSizeMismatch {
            expected,
            actual: rgba.len(),
        });
    }

    let mut rgb = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for px in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&px[..3]);
    }
    let out = Frame::from_rgb(w, h, rgb).ok_or(AltCaptureError::BufferSizeMismatch {
        expected: 0,
        actual: 0,
    })?;
    if out.is_black() {
        return Err(AltCaptureError::BlackFrame);
    }
    Ok(out)
}
