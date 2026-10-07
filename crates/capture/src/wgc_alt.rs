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
/// ## 会话是长驻的，不是每帧新建
///
/// WGC 会话包含 DispatcherQueue、D3D11 设备、Direct2D 上下文、帧池与捕获会话
/// ——**一整套**。早先这里每次调用都 `thread::spawn` 一个线程、在这条线程上
/// 建一整套会话、取一帧就退出。
///
/// 实测（2026-10-07，`cargo xtask thread-probe`，无 UI / 无模型 / 无 OCR）：
///
/// ```text
/// 抓帧前线程数 = 4
/// 第 1 帧 … 22      第 6 帧 … 82
/// 第 10 帧 … 130    ← 每帧 +12，线性增长，永不回收
/// ```
///
/// **每抓一帧泄漏 12 个线程。** 应用每秒抓一帧，跑十几分钟就会到
/// 两千多个线程。现在改成**一条长驻工作线程 + 一个会话**，
/// 会话只在目标窗口变化或会话失效时才重建。
///
/// 会话对象不是 `Send`，所以它必须**待在**那条线程上不出来——
/// 跨边界的仍然只有 `(w, h, Vec<u8>)` 和错误。
pub fn capture_one_frame(hwnd_raw: isize) -> Result<Frame, AltCaptureError> {
    let (tx, rx) = mpsc::channel::<RawCapture>();
    worker()
        .send(Job {
            hwnd_raw,
            reply: tx,
        })
        .map_err(|e| AltCaptureError::Frame(format!("抓帧工作线程不在：{e}")))?;

    match rx.recv_timeout(FIRST_FRAME_TIMEOUT) {
        Ok(RawCapture::Err(e)) => Err(e),
        Ok(RawCapture::Ok {
            width,
            height,
            rgba,
        }) => assemble(width, height, rgba),
        // 超时时把工作线程从消息循环里叫醒。它醒来后会**丢弃当前会话**
        // 并重新建一个——被叫醒的代价是重建一次会话，远小于泄漏 12 个线程。
        Err(mpsc::RecvTimeoutError::Timeout) => {
            worker().poke();
            Err(AltCaptureError::Timeout {
                secs: FIRST_FRAME_TIMEOUT.as_secs(),
            })
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(AltCaptureError::Frame(
            "抓帧工作线程提前结束且没有回传结果".into(),
        )),
    }
}

/// 一次抓帧请求。**只传纯数据**，WGC 对象一律不过这个边界。
struct Job {
    hwnd_raw: isize,
    reply: mpsc::Sender<RawCapture>,
}

/// 长驻抓帧线程的句柄。
struct Worker {
    jobs: mpsc::Sender<Job>,
    /// 工作线程自己的线程号——超时时靠它投 WM_QUIT 把人叫醒。
    /// 用共享格子而不是 `Receiver`：`Receiver` 不是 `Sync`，静态里放不了。
    tid: std::sync::Arc<std::sync::Mutex<Option<u32>>>,
}

static WORKER: std::sync::OnceLock<Worker> = std::sync::OnceLock::new();

fn worker() -> &'static Worker {
    WORKER.get_or_init(spawn_worker)
}

fn spawn_worker() -> Worker {
    let (jobs_tx, jobs_rx) = mpsc::channel::<Job>();
    let tid_cell = std::sync::Arc::new(std::sync::Mutex::new(None));
    let tid_cell_bg = tid_cell.clone();
    std::thread::Builder::new()
        .name("cs-capture".into())
        .spawn(move || run_worker(jobs_rx, tid_cell_bg))
        .expect("起抓帧工作线程失败");
    Worker {
        jobs: jobs_tx,
        tid: tid_cell,
    }
}

impl Worker {
    fn send(&self, job: Job) -> Result<(), mpsc::SendError<Job>> {
        self.jobs.send(job)
    }

    /// 给工作线程投一条 WM_QUIT，让它退出 `GetMessage` 循环。
    ///
    /// `GetMessageW` 只在收到 WM_QUIT 时返回 0，而 `next()` 随即返回 `None`
    /// （`wgc-2.0.1/src/capture.rs:203`）。工作线程醒来后会**丢弃并重建会话**，
    /// 下一帧自然又能抓。
    fn poke(&self) {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        let tid = *self.tid.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tid) = tid {
            unsafe {
                let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
    }
}

/// 工作线程的主体：**一个会话，反复取帧**。
fn run_worker(jobs: mpsc::Receiver<Job>, tid_cell: std::sync::Arc<std::sync::Mutex<Option<u32>>>) {
    use windows::Win32::System::Threading::GetCurrentThreadId;
    *tid_cell.lock().unwrap_or_else(|e| e.into_inner()) = Some(unsafe { GetCurrentThreadId() });

    // 当前会话绑在哪个窗口上。窗口变了（微信重开、句柄被复用）就重建。
    let mut session: Option<(isize, wgc::Wgc)> = None;

    while let Ok(job) = jobs.recv() {
        // 会话不存在，或者绑的不是这个窗口（微信重开、句柄被复用）→ 重建。
        if session.as_ref().is_none_or(|(h, _)| *h != job.hwnd_raw) {
            match open_session(job.hwnd_raw) {
                Ok(w) => session = Some((job.hwnd_raw, w)),
                Err(e) => {
                    let _ = job.reply.send(RawCapture::Err(e));
                    session = None;
                    continue;
                }
            }
        }
        // `take_one_frame` 需要 `&mut`，借完之后才能改 `session`，
        // 所以先把结果算出来，再决定要不要丢会话。
        let taken = session.as_mut().map(|(_, w)| take_one_frame(w));
        match taken {
            Some(Some(raw)) => {
                let _ = job.reply.send(raw);
            }
            _ => {
                // `next()` 返回 None：会话失效（item 被 Closed，或被人叫醒）。
                // 丢掉它，下一轮重建。
                tracing::debug!(target: "chatservant_capture::wgc", "会话结束，准备重建");
                session = None;
                let _ = job
                    .reply
                    .send(RawCapture::Err(AltCaptureError::IteratorEndedImmediately));
            }
        }
    }
}

/// 建会话。**与原先一样在建之前就把 border/cursor 关掉**。
fn open_session(hwnd_raw: isize) -> Result<wgc::Wgc, AltCaptureError> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HWND;

    let hwnd = HWND(hwnd_raw as *mut c_void);
    let item = wgc::window::new_item_from_hwnd(hwnd)
        .map_err(|e| AltCaptureError::CreateItem(e.to_string()))?;

    let mut settings = wgc::WgcSettings::default();
    settings.pixel_format = wgc::PixelFormat::RGBA8;
    // `None` = 系统默认 = **在用户窗口上画捕获边框** + **把鼠标指针画进帧里**。
    // 观察循环每秒建一次会话，于是用户的微信窗口每秒闪一次黄框（用户先发现的）。
    if wgc::capabilities::is_border_configurable().unwrap_or(false) {
        settings.display_border = Some(false);
    }
    if wgc::capabilities::is_cursor_configurable().unwrap_or(false) {
        settings.capture_cursor = Some(false);
    }
    wgc::Wgc::new(item, settings).map_err(|e| AltCaptureError::Open(e.to_string()))
}

/// 从会话里取一帧并转成纯数据。`None` = 会话结束。
fn take_one_frame(w: &mut wgc::Wgc) -> Option<RawCapture> {
    match w.next() {
        None => None,
        Some(Err(e)) => Some(RawCapture::Err(AltCaptureError::Frame(e.to_string()))),
        Some(Ok(frame)) => {
            let size = match frame.size() {
                Ok(s) => s,
                Err(e) => {
                    return Some(RawCapture::Err(AltCaptureError::Frame(e.to_string())));
                }
            };
            match frame.pixels() {
                Ok(rgba) => Some(RawCapture::Ok {
                    width: size.width,
                    height: size.height,
                    rgba,
                }),
                Err(e) => Some(RawCapture::Err(AltCaptureError::Frame(e.to_string()))),
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
