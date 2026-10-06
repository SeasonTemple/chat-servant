//! Windows Graphics Capture 单帧捕获。
//!
//! 用 `windows-capture`（MIT OR Apache-2.0）而不是自己实现 WGC。
//! 见 docs/adr/0003-dependency-strategy.md。
//!
//! **硬约束：帧只在内存里。** 本模块除了 `Frame` 什么都不返回，
//! 写盘是 `xtask probe` 显式做的事。

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows_capture::capture::GraphicsCaptureApiHandler;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

use chatservant_core::frame::Frame;

/// 拿到第一帧之前等多久。WGC 正常在几十毫秒内送第一帧；
/// 超过这个时间还没来，基本可以判定会话起不来，再等也是浪费。
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("窗口尺寸无效（{0}x{1}），无法建立捕获会话")]
    BadWindowSize(i32, i32),

    #[error("捕获会话启动失败：{0}")]
    Start(String),

    #[error(
        "捕获会话在送来第一帧之前就关闭了。会话线程报的原始错误：{detail}\n会话在跑但一帧没来：会话线程报的原始错误：{detail}"
    )]
    ClosedBeforeFirstFrame { detail: String },

    #[error("等待第一帧超时（{secs} 秒）。会话在跑，但一帧都没送过来")]
    Timeout { secs: u64 },

    #[error("捕获到了帧，但缓冲区长度对不上尺寸：期望 {expected} 字节，实得 {actual}")]
    BufferSizeMismatch { expected: usize, actual: usize },

    #[error("读取帧缓冲区失败：{0}")]
    Buffer(String),

    #[error("抓到的是全黑帧。这是 GPU 合成窗口捕获失败的典型症状，不是成功")]
    BlackFrame,
}

/// 拿到一帧就停，并且如实记录"发生了什么"——
/// 尤其是会话提前关闭和超时，这两种失败长得完全不一样，
/// 混成一句"没抓到帧"会让排查无从下手。
struct OneShot {
    slot: Arc<Mutex<Option<Result<Frame, CaptureError>>>>,
    done: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
}

impl GraphicsCaptureApiHandler for OneShot {
    type Flags = (i32, i32);
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(_ctx: windows_capture::capture::Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            slot: Arc::new(Mutex::new(None)),
            done: Arc::new(AtomicBool::new(false)),
            closed: Arc::new(AtomicBool::new(false)),
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut windows_capture::frame::Frame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        // WGC 可能在停止生效前再推几帧，只认第一帧。
        if self.done.swap(true, Ordering::SeqCst) {
            control.stop();
            return Ok(());
        }
        let out = to_core_frame(frame);
        if let Ok(mut guard) = self.slot.lock() {
            *guard = Some(out);
        }
        control.stop();
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// `ColorFormat::Rgba8` → RGB8 紧凑排列，并检测全黑。
///
/// 全黑是 GPU 合成窗口用错捕获路径时的稳定症状。报成成功会让下游
/// 拿到一张黑图继续跑，比直接失败难排查得多。
fn to_core_frame(frame: &mut windows_capture::frame::Frame) -> Result<Frame, CaptureError> {
    let (w, h) = (frame.width(), frame.height());
    let mut scratch = Vec::new();
    let rgba = frame
        .buffer()
        .map_err(|e| CaptureError::Buffer(e.to_string()))?
        .as_nopadding_buffer(&mut scratch)
        .to_vec();

    let expected = (w as usize) * (h as usize) * 4;
    if rgba.len() != expected {
        return Err(CaptureError::BufferSizeMismatch {
            expected,
            actual: rgba.len(),
        });
    }

    let mut rgb = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for px in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&px[..3]);
    }

    let frame = Frame::from_rgb(w, h, rgb).ok_or(CaptureError::BufferSizeMismatch {
        expected: (w as usize) * (h as usize) * 3,
        actual: 0,
    })?;
    if frame.is_black() {
        return Err(CaptureError::BlackFrame);
    }
    Ok(frame)
}

/// 抓取 `hwnd_raw` 的**一帧**，返回内存里的 `Frame`。全程不落盘。
///
/// 用 `start_free_threaded` 而不是 `start`：后者会把当前线程交给
/// Win32 消息循环，没法加超时，也观测不到会话是不是提前关闭了。
/// 这里自己管超时，好把"起不来"和"起来了但没帧"分开报。
pub fn capture_one_frame(
    hwnd_raw: isize,
    expected_size: (i32, i32),
) -> Result<Frame, CaptureError> {
    let (w, h) = expected_size;
    if w <= 0 || h <= 0 {
        return Err(CaptureError::BadWindowSize(w, h));
    }

    let item = Window::from_raw_hwnd(hwnd_raw as *mut c_void);
    let settings = Settings::new(
        item,
        // 这四项全部走 Default。显式设 WithoutCursor / WithoutBorder 会去调
        // Win10 2004（build 19041）才有的接口，低版本上整个会话会直接被关掉，
        // 症状是"启动成功但一帧不来"。先用默认值把变量排掉。
        CursorCaptureSettings::Default,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        // 原生提供变化区域。ADR-0002 的第 1、2 级建立在它之上，
        // 不自己写脏矩形逻辑。
        DirtyRegionSettings::Default,
        ColorFormat::Rgba8,
        (w, h),
    );

    let handler = OneShot {
        slot: Arc::new(Mutex::new(None)),
        done: Arc::new(AtomicBool::new(false)),
        closed: Arc::new(AtomicBool::new(false)),
    };
    let slot = Arc::clone(&handler.slot);
    let closed = Arc::clone(&handler.closed);

    let control =
        OneShot::start_free_threaded(settings).map_err(|e| CaptureError::Start(e.to_string()))?;

    let deadline = Instant::now() + FIRST_FRAME_TIMEOUT;
    let outcome = loop {
        if let Some(r) = slot.lock().ok().and_then(|mut g| g.take()) {
            break Some(r);
        }
        if control.is_finished() {
            break None;
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    // 收尾：无论拿到什么都要把会话停掉，别留一个野线程。
    // stop 会吃掉 control，所以状态必须在它之前读。
    // stop/wait 的返回值里**才有**会话线程的真实错误——on_closed 是
    // 空回调，光看它只知道"关了"，不知道"为什么关"。这个信息不能丢。
    let session_finished = control.is_finished();
    let detail = control
        .stop()
        .err()
        .map(|e| e.to_string())
        .unwrap_or_else(|| "无（会话线程没报错就结束了）".into());

    match outcome {
        Some(r) => r,
        // 会话自己结束 = 窗口不可捕获 / 已销毁 / 不走普通合成路径。
        None if closed.load(Ordering::SeqCst) || session_finished => {
            Err(CaptureError::ClosedBeforeFirstFrame { detail })
        }
        // 会话还活着但一帧没来。
        None => Err(CaptureError::Timeout {
            secs: FIRST_FRAME_TIMEOUT.as_secs(),
        }),
    }
}

/// 确认 `hwnd` 仍是有效窗口。窗口句柄会失效，抓之前要查一次——
/// 拿着过期句柄去开捕获会话会得到一个含义不明的错误。
pub fn is_alive(hwnd_raw: isize) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;
    unsafe { IsWindow(Some(HWND(hwnd_raw as *mut c_void))).as_bool() }
}
