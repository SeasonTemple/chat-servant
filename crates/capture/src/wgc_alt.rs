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
    // 抓帧线程要先把**自己的线程号**报回来——超时时我们靠它把这个线程叫醒。
    let (tid_tx, tid_rx) = mpsc::channel::<u32>();

    let handle = std::thread::spawn(move || {
        use windows::Win32::System::Threading::GetCurrentThreadId;
        let _ = tid_tx.send(unsafe { GetCurrentThreadId() });
        let _ = tx.send(capture_inside(hwnd_raw));
    });
    let tid = tid_rx.recv_timeout(TID_TIMEOUT).ok();

    let out = match rx.recv_timeout(FIRST_FRAME_TIMEOUT) {
        Ok(RawCapture::Err(e)) => Err(e),
        Ok(RawCapture::Ok {
            width,
            height,
            rgba,
        }) => assemble(width, height, rgba),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // ⚠ **必须把抓帧线程收回来，否则它会永久泄漏。**
            //
            // 早先这里直接 `return Err(Timeout)`，把线程丢下不管。
            // 而它此刻正卡在 `next()` 里的 `GetMessageW` 循环上——
            // 那个循环只在取到 WM_QUIT 时才退出。这样每次超时的捕获都会
            // **永久留下一个线程和一个活着的 WGC 会话**。
            //
            // 实测（2026-10-07）：观察循环跑了几分钟后进程有 **2000+ 线程**，
            // 增速 **9 个/秒**，全是等待态。线程和捕获会话都会无限累积。
            reap_capture_thread(&handle, tid);
            return Err(AltCaptureError::Timeout {
                secs: FIRST_FRAME_TIMEOUT.as_secs(),
            });
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(AltCaptureError::Frame(
            "抓帧线程提前结束且没有回传结果".into(),
        )),
    };
    // 正常返回时线程已经结束，直接回收。
    let _ = handle.join();
    out
}

/// 等抓帧线程报上线程号的上限。它只做一件事，超过这个时间就当拿不到。
const TID_TIMEOUT: Duration = Duration::from_millis(500);

/// 把卡在消息循环里的抓帧线程叫醒并回收。
///
/// `GetMessageW` 只在收到 **WM_QUIT** 时返回 0，而 `next()` 随即返回 `None`
/// （见 `wgc-2.0.1/src/capture.rs:203`）。所以给那个线程投一条 WM_QUIT
/// 就能让它正常退出——这是 Win32 自己定义的退出方式，不是强杀。
fn reap_capture_thread(handle: &std::thread::JoinHandle<()>, tid: Option<u32>) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};

    if let Some(t) = tid {
        unsafe {
            let _ = PostThreadMessageW(t, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
    // 给它一点时间走完退出流程；还不走也不能无限等——那会把调用方也卡住。
    let deadline = std::time::Instant::now() + Duration::from_millis(500);
    while !handle.is_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
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

    // WgcSettings 派生 SmartDefault，`None` = **系统默认行为**。
    // 那两个默认值都会出问题，必须显式关掉：
    //
    // · `display_border = None` → WGC 会在**被捕获的窗口上画一圈绿/黄边框**
    //   （系统的隐私提示）。观察循环每秒建一次会话，于是用户的微信窗口上
    //   每秒闪一次黄框（实机 2026-10-07，用户先发现的）。
    //
    // · `capture_cursor = None` → **鼠标指针会被画进帧里**。指针正好压在
    //   聊天区上时，OCR 会读到箭头形状的噪声，归类也就跟着错。
    //
    // 两项都依赖 `is_border_configurable()` / `is_cursor_configurable()`
    // 这两个能力检查；老版本 Windows 上不支持时**不设置**，而不是报错——
    // 不支持就退回系统默认，仍然能抓到，只是不那么干净。
    let mut settings = wgc::WgcSettings::default();
    settings.pixel_format = wgc::PixelFormat::RGBA8;
    if wgc::capabilities::is_border_configurable().unwrap_or(false) {
        settings.display_border = Some(false);
    }
    if wgc::capabilities::is_cursor_configurable().unwrap_or(false) {
        settings.capture_cursor = Some(false);
    }

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
