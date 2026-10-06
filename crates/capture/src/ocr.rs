//! 本地离线 OCR：Windows 自带的 `Windows.Media.Ocr`。
//!
//! ## 为什么用它
//!
//! 系统自带、零模型体积、纯离线、零 token。目前只回答一件事：
//! **帧里的字认不认得出来**。逐框的 `OcrLine` 定位、说话人归类
//! 留给 ticket 03。
//!
//! ## 像素路径
//!
//! `DataWriter::new()` 自带一个 `IBuffer`，写完 `DetachBuffer()` 拿出来，
//! 直接喂 `SoftwareBitmap::CreateCopyFromBuffer`。
//!
//! ```text
//! Frame(RGB8) → 补成 BGRA8 → DataWriter::WriteBytes → DetachBuffer
//!              → CreateCopyFromBuffer(Bgra8) → RecognizeAsync → Text
//! ```
//!
//! 中间**不经过 PNG、不落盘**。早先以为要用 `SoftwareBitmap::LoadAsync` +
//! `InMemoryRandomAccessStream`，查了 windows-rs 0.62.2 的生成源码才发现
//! 那条路绕了一圈，而 `DetachBuffer` 一步到位。
//!
//! WinRT 异步用 `IAsyncOperation::join()` 阻塞等待（**不是 `get()`**，
//! windows-future 0.3.2 里没有 `get()`）。

use chatservant_core::frame::Frame;

/// 识别结果。`text` 是整图文字，`lines` 是逐行文字。
#[derive(Debug, Clone, Default)]
pub struct OcrText {
    pub text: String,
    pub lines: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    #[error(
        "取不到可用的 OCR 引擎。多半是系统没装对应语言的 OCR 组件：\n     \
         设置 → 时间和语言 → 语言和区域 → 中文(简体) → 语言选项 → 安装「光学字符识别」"
    )]
    NoEngine,

    #[error("构造 BGRA 像素缓冲失败：{0}")]
    Pixels(String),

    #[error("OCR 链路失败：{0}")]
    Pipeline(String),
}

/// 识别一帧里的文字。纯本地，不出网。
pub fn recognize(frame: &Frame) -> Result<OcrText, OcrError> {
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::DataWriter;

    // WinRT 必须在用 WinRT API 之前初始化。当前线程若是全新的没有套间，
    // 这里就地初始化；已初始化过则返回 S_FALSE，忽略即可。
    let _ = unsafe {
        windows::Win32::System::WinRT::RoInitialize(
            windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
        )
    };

    let (w, h) = (frame.width() as i32, frame.height() as i32);

    // RGB8 → BGRA8。OCR 要 Bgra8，字节序和我们的 Frame 相反。
    let rgb = frame.as_bytes();
    let mut bgra = Vec::with_capacity(rgb.len() + (w as usize) * (h as usize));
    for px in rgb.chunks_exact(3) {
        bgra.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }

    let writer = DataWriter::new().map_err(pipeline)?;
    writer.WriteBytes(&bgra).map_err(pipeline)?;
    let buffer = writer.DetachBuffer().map_err(pipeline)?;

    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, w, h)
        .map_err(|e| OcrError::Pixels(e.to_string()))?;

    let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| OcrError::NoEngine)?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(pipeline)?
        .join()
        .map_err(pipeline)?;

    let text = result.Text().map_err(pipeline)?.to_string();

    let view = result.Lines().map_err(pipeline)?;
    let mut lines = Vec::with_capacity(view.Size().unwrap_or(0) as usize);
    for line in view {
        if let Ok(t) = line.Text() {
            lines.push(t.to_string());
        }
    }

    Ok(OcrText { text, lines })
}

fn pipeline(e: windows::core::Error) -> OcrError {
    OcrError::Pipeline(e.to_string())
}
