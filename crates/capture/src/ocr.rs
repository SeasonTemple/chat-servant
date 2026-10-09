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

    // SAFETY: 初始化本线程的 WinRT（COM）运行时。
    // `RO_INIT_MULTITHREADED` 对应 `MTA`——`recognize` 会在 OCR 引擎的
    // 回调里跨 apartment 使用 `OcrResult`，用 STA 会出现"COM 对象
    // apartment 不匹配"。重复初始化返回 `RPC_E_CHANGED_MODE`，
    // 这里用 `let _` 忽略：说明本线程已经初始化过，不是错误。
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
        // ⚠ **不能用 `line.Text()`**：它把词一律用空格拼起来。
        // 中文会被逐字切成独立的 word，于是「古文山」变成「古 文 山」、
        // 「文件传输助手」变成六个孤字。实测这两个都会直接显示在面板标题上
        // ——标题正是「我现在看着谁」，那里出现空格就是在说谎。
        // 空格该不该有，看两个词**在图上隔多远**（见 [`join_words`]）。
        text: join_words(&words),
        rect,
        words,
    })
}

/// 按**几何间距**把词拼成一行，而不是照 `OcrLine.Text` 一律加空格。
///
/// ## 阈值是怎么定的（实测，不是拍脑袋）
///
/// `cargo xtask ocr <帧> --scale 2` 会把每一对相邻词的间隙和字高都打出来。
/// 2026-10-09 在真实微信帧上量到的分布：
///
/// ```text
/// 文│件│传│输│助│手   间隙 0~2px   字高 24   比值 0.00~0.08   相邻汉字
/// 来│了│吗            间隙 2~4px   字高 24   比值 0.08~0.17   相邻汉字
/// 卜│a2025│全         间隙 2px     字高 20   比值 0.10        中英紧排
/// 1│g│3               间隙 6px     字高 18   比值 0.33        误识别的连排
/// ]│h                 间隙 38px    字高 22   比值 1.73        真有空格
/// 口│.tgz             间隙 26px    字高 22   比值 1.18        真有空格
/// ```
///
/// 两堆之间是 **0.33 与 1.18** 之间的大空档，0.5 正落在中间。
///
/// 早先这里试过 1/2 又改成 1/3：1/3 会被那条 0.33 的误识别连排骗到，
/// 把「B0」拼成「B 0」。**降阈值是为了迁就一个自己编的测试几何，
/// 而真帧上的真实分布根本不需要那么松**——量出来的数才是判据。
///
/// ## 为什么不写死「中文之间永远不加空格」
///
/// 那条规则在「纯中文」上对，碰上中英混排就会把 `用 3D 建模` 粘成
/// `用3D建模`。间隙是图像里真实存在的量，比字符类别可靠。
fn join_words(words: &[OcrWord]) -> String {
    let mut out = String::new();
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            let prev = &words[i - 1];
            let gap = w.rect.x0 - prev.rect.x1;
            let h = w.rect.height().max(prev.rect.height());
            // 写成 `gap * 2 >= h` 而不是 `gap >= h / 2`：字高是奇数时后者会
            // 向下取整，21px 的字高会得到阈值 10 而不是 10.5，边界用例会飘。
            if gap * 2 >= h {
                out.push(' ');
            }
        }
        out.push_str(&w.text);
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个**宽 `adv`、高 `h`** 的词，左边缘在 `x`。
    ///
    /// ⚠ 宽高必须分开。早先的测试辅助把宽度也写成 `h`，于是「有空格」
    /// 的用例只能靠把 `x` 拉得很远来构造，间隙大到不像真帧——
    /// 阈值就是被这种编出来的几何带偏的（降到 1/3，然后真帧上
    /// 「B0」被拼成「B 0」）。下面的数字全部来自 `cargo xtask ocr`
    /// 在真实微信帧上的实测。
    fn word(text: &str, x: i32, adv: i32, h: i32) -> OcrWord {
        OcrWord {
            text: text.into(),
            rect: Rect::new(x, 100, x + adv, 100 + h),
        }
    }

    /// **逐字切开的中文不该被插空格。**
    ///
    /// 2026-10-08 实机事故：面板标题栏把「古文山」显示成「古 文 山」，
    /// 更早一次是「文件传输助手」六个孤字。根因就是照抄
    /// `OcrLine.Text()`——它把每个 word 用空格拼起来，而中文被逐字切分。
    /// 标题栏是「我现在看着谁」，那上面出现空格就是在说谎。
    ///
    /// 实测「文│件│传│输│助│手」：字高 24，间隙 0~2px。
    #[test]
    fn cjk_split_into_characters_keeps_no_spaces() {
        let ws = vec![
            word("文", 100, 24, 24),
            word("件", 124, 24, 24),
            word("传", 148, 24, 24),
            word("输", 172, 24, 24),
            word("助", 196, 24, 24),
            word("手", 221, 24, 24),
        ];
        assert_eq!(join_words(&ws), "文件传输助手");
    }

    /// **真空格要保住。** 实测 `口│.tgz`：间隙 26px / 字高 22（比值 1.18）。
    #[test]
    fn a_real_word_gap_becomes_one_space() {
        let ws = vec![word("口", 100, 18, 22), word(".tgz", 144, 30, 22)];
        assert_eq!(join_words(&ws), "口 .tgz");
    }

    /// **误识别的连排不该被拆开。**
    ///
    /// 实测真帧上一行被读成 `1 g 3`：中间那一对的间隙 6px / 字高 18
    /// （比值 0.33）。早先把阈值降到 1/3 就是被它骗到，把 `B0` 拼成
    /// `B 0`。这条测试守的就是**别再降**。
    #[test]
    fn a_misread_but_tight_run_stays_joined() {
        let ws = vec![word("B", 100, 12, 18), word("0", 118, 12, 18)];
        assert_eq!(join_words(&ws), "B0", "间隙 6px/字高 18=0.33，不够半个字高");
    }

    /// 中英紧排：实测 `卜│a2025│全` 间隙 2px / 字高 20（比值 0.10）。
    #[test]
    fn cjk_and_latin_without_a_space_stay_joined() {
        let ws = vec![
            word("卜", 100, 20, 20),
            word("a2025", 122, 40, 20),
            word("全", 164, 20, 20),
        ];
        assert_eq!(join_words(&ws), "卜a2025全");
    }

    /// 抗锯齿造成的 1px 缝隙**不算**空格。阈值若取 0，汉字之间那点缝隙
    /// 会全变成空格，退化成原来那个 bug。
    #[test]
    fn a_one_pixel_seam_is_not_a_space() {
        let ws = vec![word("信", 100, 24, 24), word("息", 125, 24, 24)];
        assert_eq!(join_words(&ws), "信息");
    }

    /// 重叠（间隙为负）也不能变成空格——滚动会让框轻微交叠。
    #[test]
    fn overlapping_boxes_stay_joined() {
        let ws = vec![word("一", 100, 20, 26), word("j", 118, 12, 26)];
        assert_eq!(join_words(&ws), "一j");
    }

    /// 只有一个词时不该凭空多个空格。
    #[test]
    fn a_single_word_is_returned_verbatim() {
        assert_eq!(join_words(&[word("G8W", 10, 30, 10)]), "G8W");
        assert_eq!(join_words(&[]), "");
    }
}
