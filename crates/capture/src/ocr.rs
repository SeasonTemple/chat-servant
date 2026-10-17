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
//!
//! ## `--scale` 是最近邻 ⇒ 「放大 N 倍还是乱码」不能当排除依据
//!
//! `cargo xtask ocr --scale N` 走 `Frame::upscaled_nearest`
//! （`crates/core/src/frame.rs`），只复制像素、不插值。2026-10-17 在
//! 真机帧 `target/probe/actrow-v3.png`（不入库）上对**同一块裁剪**做过
//! 两种 4× 的 A/B：
//!
//! ```text
//! <按硬约束移除的 OCR 碎片>
//! <按硬约束移除的 OCR 碎片>
//! 原文       ✓ <按硬约束移除的 OCR 碎片>
//! ```
//!
//! ⇒ 用 `--scale` 验「是不是字太小」时必须记住这条：**放大到 4× 仍乱码，
//! 只能说明最近邻没救回来，不能当成「与字号无关」。** 换平滑插值在这行上
//! 确实明显变好，但那**不是净赢**：同一帧的正文气泡上双三次反而少读出
//! 20% 的字（非空白字符 128 → 103，行数 8 → 7）。
//!
//! 最近邻是 `frame.rs` 里记着的**有意选择**（字形不被抹平，结论才站得住）。
//! 要不要改插值方式牵连 perceive 的取色与阈值，是跨 crate 的取舍——
//! 不在 `ocr.rs` 里单方面改。

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
///
/// ## 两条路，默认走 PP-OCR（2026-10-18 翻的第三次）
///
/// | 引擎 | 何时用 | 代价 |
/// |---|---|---|
/// | PP-OCRv6 small（[`crate::ocr_pp_pipeline`]） | **默认**——没设 `CHATSERVANT_OCR` 就是它 | 权重 31 MB，推理后端默认 ort（[`crate::ocr_pp::backend`]） |
/// | `Windows.Media.Ocr` | `CHATSERVANT_OCR=winrt` 显式点名 | 系统自带，零模型体积 |
///
/// ⚠ **本段曾经写着相反的话**：标题是「默认走 WinRT」、表里把 WinRT 标成
/// **默认**、模型写的是 PP-OCR**v4**、权重写的是 15 MB。四条在同一天内
/// 全部过期，而文档比代码读得快——**下一个人读到的是这几行，不是 `engine_choice()`**。
/// ⇒ 翻默认值时**连这段一起改**，并在 [`engine_choice`] 记下为什么翻。
///
/// **PP-OCR 跑不通仍然继续用 WinRT**，而不是在界面上报错（ADR-0002）：
/// 一台没下模型的机器照样要能聊天。
///
/// 缺权重或推理失败会回落并 **`warn!` 报一次**（不是每帧一条）；
/// 要强制走另一条用 `CHATSERVANT_OCR`，不必重编。
///
/// ## 回落只报一次
///
/// 回落是**持续状态**而不是事件：PP-OCR 挂掉时每一帧都会走到这里。
/// 早先按帧记 `info!`，回落期间日志会被刷爆——而刷爆的真正代价是
/// **第一条被淹掉之后没人再看见它**。
static PPOCR_FALLBACK_REPORTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn recognize(frame: &Frame) -> Result<OcrText, OcrError> {
    use std::sync::atomic::Ordering;

    match engine_choice() {
        Engine::Ppocr => {
            if let Some(t) = crate::ocr_pp_pipeline::recognize(frame) {
                // 恢复了就把闸复位，下次再坏时照样报。
                PPOCR_FALLBACK_REPORTED.store(false, Ordering::Relaxed);
                return Ok(t);
            }
            if !PPOCR_FALLBACK_REPORTED.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    "PP-OCR 不可用（无权重或推理失败），本轮起回落 Windows OCR —— \
                     若这不是你预期的，先查权重目录与 bundle.resources 的映射"
                );
            } else {
                tracing::debug!("仍在回落 Windows OCR");
            }
        }
        Engine::Winrt => return recognize_winrt(frame),
    }
    recognize_winrt(frame)
}

/// 走哪条 OCR 路。
///
/// **没有 `Auto` 变体**：`engine_choice()` 必须在**内部**就把 auto 解析成
/// 具体某条。早先留了个 `Auto` 分支，于是 `recognize` 的 match 不穷尽、
/// 编译器把「以后再加变体忘了处理」这件事留到了运行期——而 OCR 选错路的后果
/// 是**静默读到别的文字**，不是报错。
///
/// ## 「回落」曾经**真的**看不见，现在看得见了（2026-10-15 踩，2026-10-18 修）
///
/// [`recognize`] 回落时只有一句 `tracing::warn!`。而 `xtask` 的日志过滤器
/// 曾经写成 `"xtask=debug,wgc=trace"`——**`EnvFilter` 没有全局档位时，
/// 未列出的 target 一律被丢掉**，于是 `chatservant_capture` 的记录整条看不见，
/// 包括这一句。
///
/// 后果实测过一次：PP-OCR 每帧都失败回落 WinRT，而 `xtask ocr` 打出的是
/// **WinRT 的逐字输出**，看上去像「PP-OCR 把整张图切成单字框」。
/// 排查方向因此整段跑偏——调了半天的 det 参数，而 det 一次都没真正跑起来。
///
/// ⇒ 过滤器现在带全局档位（`xtask` 的 `LOG_FILTER`），
/// 并由 `the_log_filter_has_a_global_level` 钉住，免得改回去。
///
/// **判据**（不依赖日志也能分辨）：PP-OCR 跑 det 要几秒（debug 构建十几秒），
/// WinRT 只要 300ms 量级。两个数字差一个量级，就说明你看到的是哪条路。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Engine {
    Ppocr,
    Winrt,
}

fn engine_choice() -> Engine {
    match std::env::var("CHATSERVANT_OCR").as_deref() {
        Ok("ppocr") => Engine::Ppocr,
        Ok("winrt") => Engine::Winrt,
        _ => {
            // **默认恒为 PP-OCR（2026-10-18 第三次改，这次是真翻了）。**
            //
            // ## 为什么前两次都没翻
            //
            // **第一次（早先）**的规则是「检测到 det 权重就选 PP-OCR」。
            // 那条规则有个致命后果：**装一次权重就把生产引擎换掉，
            // 而没有任何人做过那个决定。**
            // ⇒ 「装了权重」这件事**不构成**「换引擎」的授权。
            // 所以 2026-10-18 早上改成「一律 WinRT」——**当时是对的**：
            // 那一版 PP-OCR 的真帧 CER 是 **18.60%**，与 WinRT 的 18.25% 齐平，
            // 也就是说换引擎在字准上一点没赚，只赚了检出。
            //
            // **第二次（同日下午）**没有翻，因为准确度被修好了（2.38%），
            // 但「装包能不能跑」还没实测——**在没验过的路上翻默认值，
            // 失败时还会静默回落**，那正是要避免的失败面。
            //
            // ## 这一版翻的四个条件，逐条都有实测
            //
            // 1. **准确度**：真帧 CER 2.38%、17/17 全配对（修复前 18.60%）。
            //    见 ADR-0006 第（四）节——那里记着「0.79% 与 18.60% 从来
            //    不是同一个东西」那次自我订正。
            // 2. **延迟**：532~542 ms 稳态 / 664 ms 冷启动，TICK 是 2500 ms。
            // 3. **装机形态**：`pnpm tauri build --no-bundle` 实测
            //    `bundle.resources` **在 `--no-bundle` 下也会被拷**，
            //    落在 `<exe 目录>/models/ppocrv6-small/`，三个文件齐全、
            //    能加载、能跑、无回落告警。
            // 4. **失败面不再是静默**：没权重时会 `warn!` **报一次**
            //    （不是每帧一条，那是刷屏，刷屏等于把第一条淹掉），
            //    并指名要查权重目录与 `bundle.resources` 映射。
            //
            // ## 仍然保留的两条
            //
            // · **不按「权重在不在」自动选**——那是本条被推翻过一次的原因。
            //   权重只是**能不能用**的前提，不是**该不该用**的授权；
            //   授权写在这行代码里，而且它是**一次明确决定**。
            // · **PP-OCR 跑不通仍然继续用 WinRT**，功能不受影响
            //   （ADR-0002）。只是现在它会说话。
            //
            // 回退：`CHATSERVANT_OCR=winrt` 强制 Windows OCR，
            // 不必重编。
            Engine::Ppocr
        }
    }
}

/// **这条引擎要不要先把画面裁到消息区再送进去。** 2026-10-18 实测。
///
/// 同一张真实微信整窗（`target/probe/full-win.png`，1162×893，
/// ONNX Runtime、4 线程、PP-OCRv6 small）：
///
/// ```text
/// 整帧        1086~1146 ms   50 行   （45 行是侧边栏与会话列表，事后被丢掉）
/// 只裁消息区   443~ 455 ms    5 行   （就是那 5 条消息）
/// ```
///
/// ⇒ **快 2.5 倍，而且读得更准**：整帧那 45 行是联系人名与别的会话预览，
/// 它们让 det 多算 10 倍面积，也让 rec 多编几十个宽度的图。
/// 文本质量也更好——整帧把「反正你那么喜爱」读成「反止你那么喜爱」，
/// 裁剪后逐字正确。
///
/// ⚠ **这与 WinRT 的结论相反**（见 `perceive::read_lines` 的实测）：
/// `Windows.Media.Ocr` 裁剪后会漏读，所以它仍要整帧。
/// 两个引擎要的画面**不一样**，所以这件事由引擎决定，不由调用方决定。
///
/// ⚠ 顺带一条硬约束上的好处：整帧会把**联系人名与其它会话的预览**一起读进来。
/// 那些读进来又被丢掉的行，仍然是「读了别人的东西」。
pub fn crops_to_pane() -> bool {
    matches!(engine_choice(), Engine::Ppocr)
}

/// **这条引擎要放大几倍。** 2026-10-18 实测。
///
/// ```text
///            1x      2x
/// WinRT     13/17   14/17    <- 2× 补回一条
/// PP-OCRv6  17/17   17/17    <- 1× 已经零漏检，2× 只是慢
/// ```
///
/// ⇒ **两个引擎要的倍数相反。** 早先它是 `perceive::OCR_UPSCALE`
/// 一个全局常数，于是「换引擎」和「换倍数」必须同时做，
/// 而这两件事隔着两个 crate、一次提交——**漏掉任何一个都不会报错，
/// 只会变成「又慢又不准」**。所以它现在是引擎自己的属性。
///
/// ⚠ `perceive::OCR_UPSCALE` 保留为 WinRT 的值，供那条路与探针使用。
pub fn upscale() -> u32 {
    match engine_choice() {
        Engine::Ppocr => 1,
        Engine::Winrt => 2,
    }
}

/// [`Windows.Media.Ocr`] 那条路。**默认。**
pub fn recognize_winrt(frame: &Frame) -> Result<OcrText, OcrError> {
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
        // **不能用 `line.Text()`**：它把词一律用空格拼起来。
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
///
/// **但票 41 的 ② 把这条推翻了，规则收窄成「两个单字汉字之间」**。
///
/// 真机 2× 实测有一档 1.15~1.18 的间隙会插出假空格：
///
/// ```text
/// 直│迭   间隙 14px  字高 12  比值 1.17
/// 幸│啡   间隙 13px  字高 11  比值 1.18
/// 的│言   间隙 15px  字高 13  比值 1.15
/// ```
///
/// 而**真空格那一档是 1.15~1.73**——两档**完全重叠**，所以
/// **比值本身分不开它们**，任何「把比值阈值往上/往下挪」的改法都必然
/// 吃掉真空格或者留着假空格。那次「降阈值迁就编出来的测试几何」的事故
/// 就是这么来的，这里不能再犯一次。
///
/// **票面写的成因（「检测把一个字拆成了两个框」）不成立**：
/// 汉字的字身框是方的（宽 ≈ 高 = 1 em），**一个字的左右两半之间
/// 不可能有 ≥1 em 的空隙**。比值 1.15~1.18 的间隙本身就 ≥ 一个字身框，
/// 所以那两框之间是**真的空**——要么原文有空格，要么识别器在那里丢了一个字。
/// ⇒ **不能按「框合并」把它们并成一个框**：并出来的框会横跨那片空白，
/// 几何上是错的，而且会把真空格一起吃掉。
///
/// 现在这条规则只做一件事：**两个都是单个汉字时，不插空格**。
/// 依据是排版事实——中文正文里字与字之间本来就不打空格，而实测里
/// 每一对**真空格**都至少有一侧不是汉字（`口│.tgz`、`]│h`），
/// 两条都保住。已知代价见 [`is_single_ideograph`] 的注释。
fn join_words(words: &[OcrWord]) -> String {
    let mut out = String::new();
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            let prev = &words[i - 1];
            if needs_space_between(prev, w) {
                out.push(' ');
            }
        }
        out.push_str(&w.text);
    }
    out
}

/// 两个相邻词之间该不该插一个空格。
fn needs_space_between(prev: &OcrWord, next: &OcrWord) -> bool {
    let gap = next.rect.x0 - prev.rect.x1;
    let h = next.rect.height().max(prev.rect.height());
    // 写成 `gap * 2 >= h` 而不是 `gap >= h / 2`：字高是奇数时后者会
    // 向下取整，21px 的字高会得到阈值 10 而不是 10.5，边界用例会飘。
    if gap * 2 < h {
        return false;
    }
    // 两个**单字汉字**之间不插空格（票 41 的 ②，理由见 [`join_words`]）。
    !is_single_ideograph(&prev.text) || !is_single_ideograph(&next.text)
}

/// 是不是**一个**汉字（基本区 CJK 统一表意文字 U+4E00..=U+9FFF）。
///
/// 只认这一个区：全角标点 `（）、：` 在 U+FFxx、`讠` 类偏旁在 U+8BA0
/// ——后者**故意**算汉字，因为实测里 `讠` 确实是被拆出来的偏旁；
/// 前者不算，于是 `（ 好的 ）` 这种带空格的排版原样保留。
/// 拉丁字母、数字、`.tgz` 这类多字符词一律不算。
fn is_single_ideograph(s: &str) -> bool {
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => ('\u{4e00}'..='\u{9fff}').contains(&c),
        _ => false,
    }
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
    /// 宽高必须分开。早先的测试辅助把宽度也写成 `h`，于是「有空格」
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

    /// **假空格那一档不再产生空格**（票 41 的 ②）。
    ///
    /// 三组几何逐字取自 `cargo xtask ocr <帧> --scale 2` 的真机实测
    /// （票 41 §②），宽度按「汉字字身框 ≈ 字高」补齐：
    ///
    /// ```text
    /// 直│迭  间隙 14  字高 12  比值 1.17
    /// 幸│啡  间隙 13  字高 11  比值 1.18
    /// 的│言  间隙 15  字高 13  比值 1.15
    /// ```
    ///
    /// **自证会响**：把这三组几何交给旧逻辑（比值 ≥ 0.5 ⇒ 插空格），
    /// 得到的正是 `直 迭` / `幸 啡` / `的 言`——即票面记的那三个症状。
    #[test]
    fn the_measured_one_one_five_band_no_longer_produces_a_space() {
        let cases: [(Vec<OcrWord>, &str, f32); 3] = [
            (
                vec![word("直", 100, 12, 12), word("迭", 126, 12, 12)],
                "直迭",
                1.17,
            ),
            (
                vec![word("幸", 100, 11, 11), word("啡", 124, 11, 11)],
                "幸啡",
                1.18,
            ),
            (
                vec![word("的", 100, 13, 13), word("言", 128, 13, 13)],
                "的言",
                1.15,
            ),
        ];
        for (ws, want, measured_ratio) in cases {
            let gap = ws[1].rect.x0 - ws[0].rect.x1;
            let ratio = gap as f32 / ws[0].rect.height() as f32;
            assert!(
                (ratio - measured_ratio).abs() < 0.005,
                "这组夹具几何不是实测那一档（期望比值 {measured_ratio}，实际 {ratio:.2}），\
                 守卫就失去意义了"
            );
            assert_eq!(join_words(&ws), want, "比值 {ratio:.2} 不该插空格");
        }
    }

    /// **偏旁被拆出来的那一类也要并回去**（票 39 实测 `<按硬约束移除的真人聊天片段>`）。
    ///
    /// `讠`（U+8BA0）落在基本区之内，所以算单字汉字；它与右半边之间
    /// 那个 ≥1 em 的间隙照样不插空格。
    #[test]
    fn a_split_radical_does_not_get_a_space_either() {
        let ws = vec![word("\u{8ba0}", 100, 8, 12), word("就", 122, 12, 12)];
        assert_eq!(ws[1].rect.x0 - ws[0].rect.x1, 14, "夹具几何要与实测一致");
        assert_eq!(join_words(&ws), "讠就");
    }

    /// **实测的真空格一对都不能少**（票 39 的 `]│h`：间隙 38 / 字高 22）。
    ///
    /// 它的比值 1.73 落在「真空格 1.15~1.73」那一档的顶端，
    /// 而新判据只看**两侧是不是单个汉字**，所以它不受影响。
    #[test]
    fn the_measured_widest_real_space_survives() {
        let ws = vec![word("]", 100, 8, 22), word("h", 146, 12, 22)];
        assert_eq!(ws[1].rect.x0 - ws[0].rect.x1, 38, "夹具几何要与实测一致");
        assert_eq!(join_words(&ws), "] h");
    }

    /// **同一比值下，中英混排与汉字之间的结论必须不同**。
    ///
    /// `口│.tgz`（间隙 26 / 字高 22 = 1.18）与假空格那一档比值几乎一样，
    /// 却是**真**空格——它靠「右侧不是单个汉字」活下来。
    /// 这条同时守着「别把中英混排的空格一起吃掉」这个已知的坑。
    #[test]
    fn the_same_ratio_splits_cjk_pairs_from_mixed_script_pairs() {
        let false_space = [word("直", 100, 12, 12), word("迭", 126, 12, 12)];
        let real_space = [word("口", 100, 18, 22), word(".tgz", 144, 30, 22)];
        let ratio =
            |ws: &[OcrWord]| (ws[1].rect.x0 - ws[0].rect.x1) as f32 / ws[0].rect.height() as f32;
        assert!(
            (ratio(&false_space) - ratio(&real_space)).abs() < 0.05,
            "两组比值必须仍然重叠，否则这条守卫证明不了「比值分不开它们」"
        );
        assert_eq!(join_words(&false_space), "直迭");
        assert_eq!(join_words(&real_space), "口 .tgz");
    }

    /// **代价钉在这里**：用户真在两个汉字之间打了空格，会被吃掉。
    ///
    /// 中文正文里字间空格极罕见（排版事实），而实测里每一对真空格都
    /// 至少有一侧不是汉字，所以选了这条。**要改这个取舍，先改这里**，
    /// 别让它悄悄发生。
    #[test]
    fn a_real_space_between_two_chinese_characters_is_given_up_deliberately() {
        // 间隙 48 / 字高 12 = 4 个字身框——图上真的是一大片空。
        let ws = vec![word("直", 100, 12, 12), word("迭", 160, 12, 12)];
        assert_eq!(ws[1].rect.x0 - ws[0].rect.x1, 48);
        assert_eq!(join_words(&ws), "直迭", "已知代价：汉字之间的真空格被吃掉");
    }

    /// 全角标点**不算**汉字，所以带空格的排版原样保留。
    #[test]
    fn full_width_punctuation_keeps_its_space() {
        let ws = vec![word("\u{ff08}", 100, 12, 12), word("好", 130, 12, 12)];
        assert_eq!(join_words(&ws), "（ 好");
    }

    /// 单词判定本身：只认基本区的一个字。
    #[test]
    fn only_one_basic_ideograph_counts_as_an_ideograph() {
        assert!(is_single_ideograph("文"));
        assert!(
            is_single_ideograph("\u{8ba0}"),
            "偏旁也算，它是实测里被拆出来的"
        );
        assert!(!is_single_ideograph("文件"), "两个字的词不算");
        assert!(!is_single_ideograph("0"));
        assert!(!is_single_ideograph(".tgz"));
        assert!(!is_single_ideograph("\u{ff08}"), "全角标点不算");
        assert!(!is_single_ideograph(""), "空串不算");
    }

    /// **默认恒为 PP-OCR，目录里有没有权重都不改。**（2026-10-18 第三次改）
    ///
    /// 这条守卫的**判据形状没变，变的是它断言的值**——这正是它的价值：
    /// 两次改默认都是它当场红的，不是我事后想起来的。
    ///
    /// ## 三次默认值与各自的依据
    ///
    /// | 版本 | 默认 | 依据 |
    /// |---|---|---|
    /// | 早先 | 「检测到权重就选 PP-OCR」 | **无人拍板**——装个文件就换掉生产引擎 |
    /// | 2026-10-18 上午 | WinRT | PP-OCR 真帧 CER 18.60%，与 WinRT 齐平，换了没赚 |
    /// | 2026-10-18 傍晚 | **PP-OCR** | CER 2.38%、532~542ms、装机形态实测过、失败会 `warn!` |
    ///
    /// ## 为什么仍然要这条
    ///
    /// 它钉的是「**默认**」这件事**与本机状态无关**。
    /// 上一版的判据是「默认落在某条真路上，**而且**本机没权重时必须是 WinRT」——
    /// 后半句有条件：权重一装上断言就自动失效，于是那次换引擎**全程无人拍板**。
    /// 所以判据必须是无条件的：显式设 `CHATSERVANT_OCR` 才可能选到别的。
    #[test]
    fn the_default_engine_is_ppocr_whether_or_not_weights_are_installed() {
        // 测试进程自己没设过这个变量；若外层设了，那是在测别的东西，跳过并说明。
        if std::env::var("CHATSERVANT_OCR").is_ok() {
            eprintln!("跳过：外层设了 CHATSERVANT_OCR，测的已经不是默认值");
            return;
        }
        assert_eq!(
            engine_choice(),
            Engine::Ppocr,
            "默认必须是 PP-OCR。本机权重在不在都不该改这件事——\
             换引擎是一次决定，不是「目录里碰巧多了几个文件」"
        );
    }

    /// **不存在目录必须判定为「没权重」。**
    ///
    /// 这条判据被 [`crate::ocr_pp_pipeline::recognize_in`] 当作「跑不跑 PP-OCR」
    /// 的开关：它要是误报 true，一台没下模型的机器会走进 PP-OCR 分支、
    /// 白跑一次探测再回落。
    #[test]
    fn an_absent_directory_reports_no_weights() {
        let absent = std::env::temp_dir().join("chatservant-definitely-absent-dir");
        assert!(
            !crate::ocr_pp_pipeline::det_file_present(&absent),
            "不存在的目录被判成有权重"
        );
    }
}
