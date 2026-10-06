//! 本地离线 OCR：Windows 自带的 `Windows.Media.Ocr`。
//!
//! ## 为什么用它
//!
//! 系统自带、零模型体积、纯离线、零 token。
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
//! 中间**不经过 PNG、不落盘**。
//!
//! WinRT 异步用 `IAsyncOperation::join()` 阻塞等待（**不是 `get()`**）。

use chatservant_core::frame::{Frame, Rect};

/// 一个 OCR 词及其外框。
#[derive(Debug, Clone, PartialEq)]
pub struct OcrWord {
    pub text: String,
    pub rect: Rect,
}

/// 一行 OCR 结果。`rect` 是本行所有词的**并集**。
///
/// 坐标以帧左上角为原点，与 `Frame` 一致——不搞第二套坐标系。
#[derive(Debug, Clone, PartialEq)]
pub struct OcrLine {
    pub text: String,
    pub rect: Rect,
    pub words: Vec<OcrWord>,
}

impl OcrLine {
    /// 行的垂直中心。
    ///
    /// 用它做同一气泡内的多行归并，比用 `rect.y0` 稳：`y0` 会随行内
    /// 字形高度浮动（同一行的字高低不齐时框会抖）。
    pub fn center_y(&self) -> f32 {
        self.rect.y0 as f32 + self.rect.height() as f32 / 2.0
    }

    pub fn height(&self) -> i32 {
        self.rect.height()
    }
}

/// 识别结果。`text` 是整图文字，`lines` 是逐行结果（含坐标）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OcrText {
    pub text: String,
    pub lines: Vec<OcrLine>,
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

    let _ = unsafe {
        windows::Win32::System::WinRT::RoInitialize(
            windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
        )
    };

    let (w, h) = (frame.width() as i32, frame.height() as i32);

    // RGB8 → BGRA8。OCR 要 Bgra8，字节序与我们的 Frame 相反。
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
        if let Some(parsed) = parse_line(&line) {
            lines.push(parsed);
        }
    }
    lines.sort_by_key(|l| (l.rect.y0, l.rect.x0));

    Ok(OcrText { text, lines })
}

/// WinRT 的 `BoundingRect` 是以**逻辑**像素给的，要换算成物理像素，
/// 否则在 125%/150% DPI 下整个框会缩。
///
/// OCR 引擎报告的坐标相对于它接收的那张位图；我们喂进去的位图就是
/// 帧的原始物理像素，所以这里只做轴对齐取整，不做 DPI 换算——
/// 换算会引入我们无法验证的假设。
fn parse_line(line: &windows::Media::Ocr::OcrLine) -> Option<OcrLine> {
    let words_view = line.Words().ok()?;

    let mut words = Vec::with_capacity(words_view.Size().unwrap_or(0) as usize);
    let mut acc: Option<Rect> = None;

    for w in words_view {
        // IVectorView 迭代直接给出 OcrWord，不包 Result。
        let Ok(t) = w.Text() else { continue };
        if t.is_empty() {
            continue; // 空词进了下游只会变成一条空消息，污染去重集合
        }
        let Some(r) = w.BoundingRect().ok().map(denorm) else {
            continue;
        };
        acc = Some(match acc {
            None => r,
            Some(prev) => union(prev, r),
        });
        words.push(OcrWord {
            text: t.to_string(),
            rect: r,
        });
    }

    let rect = acc?;
    if words.is_empty() {
        return None;
    }
    Some(OcrLine {
        text: line.Text().ok()?.to_string(),
        rect,
        words,
    })
}

fn denorm(r: windows::Foundation::Rect) -> Rect {
    // WinRT Rect 可能是「空矩形」(Infinity)，先夹成有限值
    let fin = |v: f32| {
        if v.is_finite() { v as i32 } else { 0 }
    };
    Rect::new(fin(r.X), fin(r.Y), fin(r.X + r.Width), fin(r.Y + r.Height))
}

fn union(a: Rect, b: Rect) -> Rect {
    Rect::new(
        a.x0.min(b.x0),
        a.y0.min(b.y0),
        a.x1.max(b.x1),
        a.y1.max(b.y1),
    )
}

fn pipeline(e: windows::core::Error) -> OcrError {
    OcrError::Pipeline(e.to_string())
}
