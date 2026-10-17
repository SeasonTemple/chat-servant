//! 管线接缝：一张 `Frame` 进去，`Message[]` 出来。
//!
//! ## 为什么要有这一层
//!
//! 这是整个系统里最贵的调试杠杆。中间任何一步改了颜色阈值，都能在
//! [`read`] 上一眼看出生效范围；没有它，每次改动都得开着微信窗口
//! 手点一遍完整链路。
//!
//! ## 为什么 OCR 是注入的、其余全部真跑
//!
//! **可入库的夹具里没有墨色像素。** 真实帧的脱敏会把文字、头像、图片
//! 全部刷成所在块的底色——这正是它必须做到的事。于是
//! [`probe_bubble_bg`] 要测的 `ink` / `contrast` 在夹具上恒为零，
//! 分类判据无从测量。
//!
//! 这不是缺陷，是**脱敏的必然后果**：分类判据量的正是我们绝不能提交的东西。
//! 所以接缝拆在这里：
//!
//! | 步骤 | 测试里怎么跑 |
//! |------|--------------|
//! | [`anchor::derive`] 锚定 | 真跑，读真实像素 |
//! | [`Palette::learn`] 学配色 | 真跑，读真实像素 |
//! | OCR 取文本框 | **注入** [`LineSource`]——唯一带隐私的一步 |
//! | [`classify_all`] 归类 | 真跑，读真实像素（外圈众数 + 框内墨色） |
//! | [`group`] 归并 | 真跑 |
//!
//! 生产用 [`SystemOcr`]，测试用 [`RecordedLines`]。
//! 夹具 + manifest + 一组录制好的行框，就是完整的黄金用例。

use std::borrow::Cow;

use chatservant_capture::anchor::{self, AnchorError, PaneAnchor};
use chatservant_capture::ocr::{self, OcrError, OcrText};
use chatservant_core::frame::{Frame, Rect};

use crate::attribute::{Attribution, Palette};
use crate::merge::{self, Message};

/// 一次完整读取的结果。中间态也留着——调试视图与回归测试都要看。
#[derive(Debug, Clone)]
pub struct Read {
    /// 这一帧量出来的锚点：消息区矩形、输入框上沿、会话名位置。
    ///
    /// 留着是因为它**每帧都可能变**（窗口缩放、字号改、切会话），
    /// 下游必须拿同一次读取的锚点，不能复用上一帧的。
    pub anchor: PaneAnchor,
    /// 从这一帧学到的配色基准。整个归类都靠它定「在不在气泡里」，
    /// 所以它和 [`Read::attributions`] 必须来自同一帧。
    pub palette: Palette,
    /// 逐行判定，含被判成噪声的。调试视图按这个上色。
    pub attributions: Vec<Attribution>,
    /// 归并后的消息流，**不含噪声**。
    pub messages: Vec<Message>,
}

/// [`read`] 在哪一步失败。
///
/// 全部可恢复：调用方拿到它只该**这一帧不算数**，不该中止整个会话。
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// 推不出锚点。微信改版或窗口被折叠时会发生，重试可能就好了。
    #[error("锚点推不出来：{0}")]
    Anchor(#[from] AnchorError),

    /// 聊天区太平，学不出面板底色。多半是空会话。
    ///
    /// **上层应降级，绝不退回硬编码颜色**——换主题时硬编码会静默失效，
    /// 失败得比认错颜色明显得多。
    #[error("学不到配色（聊天区太平，可能是空会话）——上层应降级，不许退回硬编码颜色")]
    NoPalette,

    /// 锚点推出来了，却裁不出合法的消息区。
    ///
    /// 锚点本身有效但矩形退化（宽或高 ≤ 0），通常是帧比锚点先来的时序问题。
    #[error("按锚点裁不出消息区")]
    NoPane,

    /// OCR 这一步失败。前面的锚点与配色都已经算好了，可以直接重试整条链路。
    #[error(transparent)]
    Ocr(#[from] OcrError),
}

/// 取文本框这一步的来源。
///
/// 抽出来只有一个理由：**它是整条链路上唯一带隐私的步骤**。
/// 其余每一步都在读像素，而像素是可以脱敏后提交的。
pub trait LineSource {
    /// `view` 是**给 OCR 看的画面**，行框坐标以它的左上角为原点。
    ///
    /// 它是整帧还是消息区裁剪件，**由 [`LineSource::view_policy`] 决定**——
    /// 两个引擎实测要的画面不一样，见 [`ViewPolicy`]。
    /// 行坐标必须换算回消息区、并按消息区过滤之后才轮到下游。
    fn read(&self, view: &Frame) -> Result<OcrText, OcrError>;

    /// 这个引擎要看哪一块画面、放大几倍。**默认是 WinRT 实测要的那一组。**
    fn view_plan(&self) -> ViewPlan {
        ViewPlan::WINRT
    }
}

/// OCR 看到的画面该有多大、放大几倍。**两个引擎要的不是一个。**
///
/// | | WinRT | PP-OCRv6 |
/// |---|---|---|
/// | 画面 | **必须整帧** | **只给消息区** |
/// | 放大 | **2×** | **1×** |
/// | 依据 | 裁掉侧边栏后「乐」消失；1× 读不到对方短消息 | 整帧 1146ms/50 行 vs 裁剪 455ms/5 行且逐字更准；1× 已零漏检 |
///
/// ⚠ 这不是「谁更讲究」，是**两个识别器对上下文与分辨率的要求不同**。
/// 它曾经是三个写死的常量（画面=整帧、放大=2、换引擎时手改），
/// 而「换引擎」与「换那两个」隔着两个 crate——**漏掉任何一个都不报错**，
/// 只会变成「又慢又不准」。合成一个结构体之后，它们至少一起被审。
///
/// ⚠ 仍然**由引擎自己决定**，不由调用方传参：e2e 探针和产品各传各的话，
/// 探针就证明不了产品那条路。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewPlan {
    /// 整帧还是消息区裁剪件。
    pub policy: ViewPolicy,
    /// 喂给 OCR 之前放大几倍。
    pub upscale: u32,
}

/// OCR 看到的画面该有多大。**两个引擎要的不是一个，这是实测逼出来的分叉。**
///
/// | | WinRT | PP-OCRv6 |
/// |---|---|---|
/// | 画面 | **必须整帧** | **只给消息区** |
/// | 依据 | 裁掉侧边栏后「乐」消失（`read_lines` 的实测） | 整帧 1146ms/50 行 vs 裁剪 455ms/5 行，且逐字更准（`ocr::crops_to_pane`） |
///
/// ⚠ 这不是「谁更讲究」，是**两个识别器对上下文的要求不同**。
/// 把它做成调用方的一个参数会立刻分叉：e2e 探针和产品各传各的，
/// 于是探针证明不了产品那条路。所以决定权在**引擎自己**手里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewPolicy {
    /// 放大后的整帧。
    WholeFrame,
    /// 放大后的消息区裁剪件。
    ChatPane,
}

impl ViewPlan {
    /// WinRT 实测要的那一组，也是默认值。
    pub const WINRT: ViewPlan = ViewPlan {
        policy: ViewPolicy::WholeFrame,
        upscale: 2,
    };

    /// PP-OCRv6 实测要的那一组。
    pub const PPOCR: ViewPlan = ViewPlan {
        policy: ViewPolicy::ChatPane,
        upscale: 1,
    };
}

/// 真的调 Windows OCR。纯本地，不出网。
pub struct SystemOcr;

impl LineSource for SystemOcr {
    fn read(&self, view: &Frame) -> Result<OcrText, OcrError> {
        ocr::recognize(view)
    }

    /// 跟着实际选中的引擎走，不看环境变量自己猜一遍——
    /// 猜错的后果有两个：给 WinRT 裁了图（对方短消息整条消失），
    /// 或者给 PP-OCR 放大 2 倍（慢一倍、还一条都不多读）。
    fn view_plan(&self) -> ViewPlan {
        if ocr::crops_to_pane() {
            ViewPlan::PPOCR
        } else {
            ViewPlan::WINRT
        }
    }
}

/// **WinRT 那条路的放大倍数。** 产品链路上它由
/// [`ViewPlan::WINRT`] 提供；这里保留一份给探针与直接引用它的代码。
///
/// ## 两个引擎要的倍数相反，2026-10-18 实测
///
/// 同一批真实微信帧（`target/probe/wechat-chat.png` 7 条、`gws-chat.png` 10 条），
/// 详见 [`docs/adr/0006-ocr-engine-evaluation.md`](../../../docs/adr/0006-ocr-engine-evaluation.md)：
///
/// ```text
///                      1x      2x
/// WinRT             13/17   14/17    <- 少一条，多放大才补回来
/// PP-OCRv6 small    17/17   17/17    <- 原生分辨率就吃得下 14px 小字
/// ```
///
/// ⇒ **它曾经是个全局常数，而两个引擎要的相反。** 换引擎与改这一个数
/// 隔着两个 crate、一次提交，漏掉任何一个都不报错，只变成「又慢又不准」。
/// 现在它是 [`ViewPlan`] 的一个字段，**跟着引擎一起被决定**。
///
/// ⚠ 本常量是 **WinRT 的值（2）**，因为 WinRT 仍是默认引擎。
/// 把它当「全局的放大倍数」用会让 PP-OCR 那条路慢一倍。
///
/// ## WinRT 为什么需要 2（2026-10-07 实测，深色主题，1161x893）
///
/// 对方那条只有一个「乐」的气泡，19x19 px 白字灰底，**放大前一律读不到**，
/// 放大后稳定读到。1x 时同一张图上 h=12~14 的行却能正常读出来——
///
/// ```text
/// 整帧 1x   26 行  无「乐」
/// 整帧 2x   40 行  有「乐」   <- WinRT 的默认值
/// 整帧 3x   37 行  有「乐」
/// 整帧 4x   32 行  有「乐」   （4644x3572，没撞上任何尺寸上限）
/// ```
///
/// 所以不是「字太小」那么简单，但放大对 WinRT 是**必要条件**，
/// 在所有被测配置下都成立。
pub const OCR_UPSCALE: u32 = ViewPlan::WINRT.upscale;

/// 把 OCR 读到的行换算回**消息区坐标**，并丢掉落在消息区之外的行。
///
/// `origin` 是**要把视图坐标减掉的那个偏移**：
/// 整帧视图传 `(chat.x0, chat.y0)`，裁剪视图传 `(0, 0)`。
///
/// ⚠ 它不是可有可无的参数。`read_lines` 改用裁剪件之后，
/// 视图坐标**已经是**消息区相对的；这里若还去减 `chat.x0/y0`，
/// **每一行都会被平移掉整个消息区的偏移**，而症状是
/// 「行都在，但全被判成区外丢掉」——空消息流，不报错。
///
/// ## 为什么必须丢掉区外的行
///
/// 吃整帧时（WinRT 那条路）侧边栏的联系人名、标题栏、输入框里的字
/// 全都会被读出来。它们绝不能进消息流——联系人名混进消息里，
/// 产品就会凭空「听到」对方说的话。那是正确性事故，不是精度问题。
fn lines_into_pane(ocr: OcrText, chat: Rect, scale: u32, origin: (i32, i32)) -> OcrText {
    let s = scale as i32;
    let lines = ocr
        .lines
        .into_iter()
        .filter_map(|mut line| {
            // 放大后的坐标先退回视图坐标，再平移到消息区原点。
            let local = Rect::new(
                line.rect.x0 / s - origin.0,
                line.rect.y0 / s - origin.1,
                line.rect.x1 / s - origin.0,
                line.rect.y1 / s - origin.1,
            ); // 与消息区没有重叠 = 在侧边栏 / 标题栏 / 输入框里，丢掉。
            if local.x1 <= 0
                || local.y1 <= 0
                || local.x0 >= chat.width()
                || local.y0 >= chat.height()
            {
                return None;
            }
            line.rect = local;
            for w in &mut line.words {
                w.rect = Rect::new(
                    w.rect.x0 / s - origin.0,
                    w.rect.y0 / s - origin.1,
                    w.rect.x1 / s - origin.0,
                    w.rect.y1 / s - origin.1,
                );
            }
            Some(line)
        })
        .collect();
    OcrText {
        text: String::new(),
        lines,
    }
}

/// 按产品规则取文本框：**放大后的整帧**喂给 OCR，行坐标换算回消息区，
/// 落在消息区之外的行丢掉。
///
/// ## 为什么要抽出来
///
/// 端到端探针（`xtask e2e`）本来是**自己复制了一遍**裁剪 + OCR 的，
/// 于是产品链路改了它没跟着改，探针还在测一条已经不存在的旧路——
/// 和 `probe frame` 默认路径指向坏实现是同一类病：
/// **诊断命令必须和产品走同一条代码路径**，否则它证明不了任何事。
///
/// 两个调用方现在共用这一个入口。
pub fn read_lines(
    frame: &Frame,
    chat: Rect,
    source: &impl LineSource,
) -> Result<OcrText, ReadError> {
    // 给 OCR 看哪一块画面，**由引擎决定**（[`ViewPolicy`]）。两条都是实测：
    //
    // ## WinRT：必须整帧
    //
    // 1. 不放大：对方短消息稳定读不到（见 [`OCR_UPSCALE`]）。
    // 2. 只给裁好的消息区：同样读不到。同一帧、同为 2x，把左侧
    //    侧边栏裁掉之后「乐」就消失；用纯色补白模拟不出来，
    //    必须是真实文字才生效。
    //
    // ```text
    // 裁剪 x0=0    （含侧边栏）2x -> 有「乐」
    // 裁剪 x0=190  （含侧边栏）2x -> 有「乐」
    // 裁剪 x0=210  （含侧边栏）2x -> 无「乐」
    // 裁剪 x0=304  （纯消息区）2x -> 无「乐」   <- 原来是这一档
    // ```
    //
    // **代价**：OCR 输入变大、变慢（实测 223ms -> 480ms）。但观察循环
    // 只在画面**变化**时才 OCR（`PaneWatcher` 逐字节比对，没变就
    // `Quiet`），所以静默期不付这份成本。
    //
    // **Windows OCR 为什么这样，没查清。** 上面这些数据只支持
    // 「放大 + 左侧有文字上下文」两个条件同时成立，机制本身未知。
    // 这是按实测兜住的行为，不是按原理推出来的结论。
    //
    // ## PP-OCR：必须裁
    //
    // 反过来。同一张真实整窗（`target/probe/full-win.png` 1162×893）：
    // 整帧 1146ms / 50 行，裁到消息区 455ms / 5 行，
    // 而且**逐字更准**。见 [`crate::capture::ocr::crops_to_pane`]。
    let plan = source.view_plan();
    // 视图左上角在**消息区坐标系**里的位置。
    //
    // ⚠ 方向别搞反（2026-10-18 第一次就写反了，被这两条测试当场打回）：
    // - 整帧视图：视图坐标**就是**整帧坐标，换到消息区要减掉消息区原点；
    // - 裁剪视图：视图坐标**已经是**消息区相对坐标，一个都不用减。
    //
    // 写反的症状不是坐标偏一点，而是**整批行被判成区外丢掉** ⇒ 空消息流。
    let origin = match plan.policy {
        ViewPolicy::WholeFrame => (chat.x0, chat.y0),
        ViewPolicy::ChatPane => (0i32, 0i32),
    };
    let base = match plan.policy {
        ViewPolicy::WholeFrame => Cow::Borrowed(frame),
        ViewPolicy::ChatPane => Cow::Owned(frame.crop(chat).ok_or(ReadError::NoPane)?),
    };
    let view = base
        .upscaled_nearest(plan.upscale)
        .ok_or(ReadError::NoPane)?;
    let ocr = source.read(&view)?;
    Ok(lines_into_pane(ocr, chat, plan.upscale, origin))
}

/// 跑完整条链路。`source` 只负责取文本框，其余全真跑。
pub fn read(frame: &Frame, source: &impl LineSource) -> Result<Read, ReadError> {
    let anchor = anchor::derive(frame)?;
    let palette = Palette::learn(frame, anchor.chat).ok_or(ReadError::NoPalette)?;
    let pane = frame.crop(anchor.chat).ok_or(ReadError::NoPane)?;
    let ocr = read_lines(frame, anchor.chat, source)?;

    let mut attributions = crate::attribute::classify_all(&ocr.lines, &pane, &palette);
    // 语义遮罩（ADR-0002 第 3 级）：**准确率**手段，不是成本手段。
    // 图里的字绝不能算成任何一方说的话。
    crate::mask::reject_image_text(&mut attributions);
    let messages = merge::group(&attributions);
    Ok(Read {
        anchor,
        palette,
        attributions,
        messages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attribute::{NoiseClass, Speaker};
    use chatservant_capture::ocr::OcrLine;
    use chatservant_core::frame::Rect;

    const SIDEBAR: [u8; 3] = [34, 34, 51];
    const PANEL: [u8; 3] = [30, 30, 31];
    const CHROME: [u8; 3] = [51, 51, 51];
    const MY_BUBBLE: [u8; 3] = [53, 210, 141];
    const MY_INK: [u8; 3] = [22, 22, 22];
    const THEIR_BUBBLE: [u8; 3] = [47, 47, 48];
    const THEIR_INK: [u8; 3] = [225, 225, 229];
    const GRAY_INK: [u8; 3] = [123, 123, 128];

    /// 帧尺寸与 `anchor::tests::anchor_tracks_window_size` 用的完全一致，
    /// 所以锚点结果也是那份测试断言过的 `(192,27)-(800,426)`。
    const W: u32 = 800;
    const H: u32 = 600;
    const SIDEBAR_W: u32 = 192;
    const HEADER_H: u32 = 27;
    const INPUT_TOP: u32 = 426;

    /// 三个文本框的**帧坐标**。
    const MY_TEXT: Rect = Rect {
        x0: 540,
        y0: 104,
        x1: 700,
        y1: 116,
    };
    const THEIR_TEXT: Rect = Rect {
        x0: 240,
        y0: 164,
        x1: 400,
        y1: 176,
    };
    const GRAY_TEXT: Rect = Rect {
        x0: 400,
        y0: 250,
        x1: 500,
        y1: 262,
    };

    /// 造一张「微信式」帧，并把三段真墨色画进去。
    ///
    /// 墨色是 **2 px 细条纹**，不是实心块：真实字形就这个粗细。
    /// 画成实心块的话外圈众数会被拉偏，测的就不是真实情形了。
    fn synthetic_frame() -> Frame {
        // 必须是**定长**零缓冲：`put` 走的是下标写入，`with_capacity` 给的是空 Vec。
        let mut px = vec![0u8; (W * H * 3) as usize];
        for y in 0..H {
            for x in 0..W {
                let c = if x < SIDEBAR_W {
                    SIDEBAR
                } else if y < HEADER_H || y == INPUT_TOP {
                    CHROME
                } else {
                    PANEL
                };
                put(&mut px, x, y, c);
            }
        }

        // 我方气泡 + 深字
        fill(
            &mut px,
            Rect {
                x0: 520,
                y0: 96,
                x1: 720,
                y1: 124,
            },
            MY_BUBBLE,
        );
        stroke(&mut px, MY_TEXT, MY_INK);
        // 对方气泡 + 亮字
        fill(
            &mut px,
            Rect {
                x0: 224,
                y0: 156,
                x1: 416,
                y1: 184,
            },
            THEIR_BUBBLE,
        );
        stroke(&mut px, THEIR_TEXT, THEIR_INK);
        // 面板上的灰字（时间戳）
        stroke(&mut px, GRAY_TEXT, GRAY_INK);

        Frame::from_rgb(W, H, px).expect("synthetic")
    }

    fn put(px: &mut [u8], x: u32, y: u32, c: [u8; 3]) {
        let i = ((y * W + x) * 3) as usize;
        px[i..i + 3].copy_from_slice(&c);
    }

    fn fill(px: &mut [u8], r: Rect, c: [u8; 3]) {
        for y in (r.y0.max(0) as u32)..(r.y1 as u32) {
            for x in (r.x0.max(0) as u32)..(r.x1 as u32) {
                put(px, x, y, c);
            }
        }
    }

    /// 在框内画两道 2 px 横笔。
    fn stroke(px: &mut [u8], r: Rect, c: [u8; 3]) {
        for y in [r.y0 + 3, r.y0 + 7] {
            for dy in 0..2i32 {
                for x in (r.x0 + 4)..(r.x1 - 4) {
                    let i = (((y + dy) as u32 * W + x as u32) * 3) as usize;
                    px[i..i + 3].copy_from_slice(&c);
                }
            }
        }
    }

    /// 把帧坐标换算成**OCR 视图坐标**（放大后的整帧）。
    ///
    /// 注入契约跟着 [`LineSource::read`] 一起变了：测试给的行框现在
    /// 必须落在**放大后的整帧**坐标系里，换算回消息区是 `read()` 的事。
    /// 夹具本身仍按真实帧坐标书写，只是过一遍这个函数。
    fn to_view(r: Rect) -> Rect {
        let s = OCR_UPSCALE as i32;
        Rect::new(r.x0 * s, r.y0 * s, r.x1 * s, r.y1 * s)
    }

    struct RecordedLines(Vec<OcrLine>);

    impl LineSource for RecordedLines {
        fn read(&self, _pane: &Frame) -> Result<OcrText, OcrError> {
            Ok(OcrText {
                text: self.0.iter().map(|l| l.text.as_str()).collect(),
                lines: self.0.clone(),
            })
        }
    }

    /// 一个**声明自己要裁到消息区**的假引擎：它按**视图坐标**给行框，
    /// 于是能验「裁剪件的坐标换算回消息区」对不对。
    struct CroppedLines(OcrText);

    impl LineSource for CroppedLines {
        fn read(&self, _view: &Frame) -> Result<OcrText, OcrError> {
            Ok(self.0.clone())
        }

        fn view_plan(&self) -> ViewPlan {
            ViewPlan::PPOCR
        }
    }

    /// **裁剪件的坐标换算回消息区，行要留住、位置要准。**
    ///
    /// 这是 [`ViewPlan::ChatPane`] 唯一容易搞错的地方：视图原点已经不是
    /// 整帧的 (0,0)，而是消息区的左上角。若换算时仍减去 `chat.x0/y0`，
    /// **每一行都会被平移掉整个消息区的偏移**，而症状是
    /// 「行都被判成区外丢掉」⇒ 空消息流，**全程不报错**。
    ///
    /// 2026-10-18：真帧上 `xtask ocr` 逐行 0 行与这条是同一类失败。
    #[test]
    fn lines_read_from_a_cropped_view_land_inside_the_pane() {
        let (fw, fh) = (600u32, 400u32);
        let chat = Rect::new(120, 40, 600, 360);
        let f = Frame::from_rgb(fw, fh, vec![30u8; (fw * fh * 3) as usize]).expect("frame");
        // 视图坐标：消息区里的两行（PP-OCR 是 1×，所以视图坐标 = 消息区相对坐标）
        let lines = OcrText {
            text: "甲\n乙".into(),
            lines: vec![
                OcrLine {
                    text: "甲".into(),
                    rect: Rect::new(10, 20, 200, 40),
                    words: Vec::new(),
                },
                OcrLine {
                    text: "乙".into(),
                    rect: Rect::new(10, 200, 300, 220),
                    words: Vec::new(),
                },
            ],
        };
        let got = read_lines(&f, chat, &CroppedLines(lines)).expect("读行失败");
        assert_eq!(got.lines.len(), 2, "裁剪件的行被整批丢掉了：{got:?}");
        assert_eq!(
            got.lines[0].rect,
            Rect::new(10, 20, 200, 40),
            "第一行的位置偏了——换算时多半是把 chat.x0/y0 多减了一次"
        );
        assert_eq!(got.lines[1].rect, Rect::new(10, 200, 300, 220));
    }

    /// **整帧那条路的换算不能被裁剪改动带歪。**
    ///
    /// 默认策略是 [`ViewPlan::WINRT`]，而 `read_lines` 现在多了一段
    /// 坐标换算。上一条只证明裁剪那条对；这条钉住整帧那条**没变**——
    /// 夹具的行框本来就写在放大后的整帧坐标系里。
    #[test]
    fn the_whole_frame_policy_maps_into_the_pane_exactly_as_before() {
        let (fw, fh) = (600u32, 400u32);
        let chat = Rect::new(120, 40, 600, 360);
        let f = Frame::from_rgb(fw, fh, vec![30u8; (fw * fh * 3) as usize]).expect("frame");
        let src = RecordedLines(vec![OcrLine {
            text: "对方那条".into(),
            rect: to_view(Rect::new(130, 60, 300, 80)),
            words: Vec::new(),
        }]);
        let got = read_lines(&f, chat, &src).expect("读行失败");
        assert_eq!(got.lines.len(), 1, "整帧路径的行被丢掉了：{got:?}");
        assert_eq!(
            got.lines[0].rect,
            Rect::new(10, 20, 180, 40),
            "整帧路径的坐标换算变了：{:?}",
            got.lines[0].rect
        );
    }

    fn recorded() -> RecordedLines {
        RecordedLines(
            [
                (MY_TEXT, "我方那条"),
                (THEIR_TEXT, "对方那条"),
                (GRAY_TEXT, "昨天 13:44"),
            ]
            .into_iter()
            .map(|(r, text)| OcrLine {
                text: text.to_string(),
                rect: to_view(r),
                words: vec![],
            })
            .collect(),
        )
    }

    /// 期望的锚点。标题带是 `y < 27`，`low_dominance_bands` 记的是**带内最后
    /// 一行**，所以 `header_top = 26` 而不是 27。
    fn expected_chat() -> Rect {
        Rect::new(SIDEBAR_W as i32, 26, W as i32, INPUT_TOP as i32)
    }

    /// **端到端**：一张帧进去，`Message[]` 出来。
    ///
    /// 锚定、裁剪、学配色、量外圈、量墨色、归类、归并——**一步都不 mock**，
    /// 只有取文本框那一步是注入的（理由见模块文档）。
    #[test]
    fn frame_in_messages_out() {
        let frame = synthetic_frame();
        let r = read(&frame, &recorded()).expect("整条链跑通");

        assert_eq!(r.anchor.chat, expected_chat(), "锚点不对");
        assert_eq!(r.palette.panel_bg, PANEL, "面板底色学错了");

        let speakers: Vec<Speaker> = r.messages.iter().map(|m| m.speaker).collect();
        assert_eq!(
            speakers,
            vec![Speaker::Me, Speaker::Them],
            "消息流与顺序错了：{:#?}",
            r.messages
        );
        assert_eq!(r.messages[0].text, "我方那条");
        assert_eq!(r.messages[1].text, "对方那条");

        // 灰字必须被判成噪声、且**不进消息流**。
        let gray = r
            .attributions
            .iter()
            .find(|a| a.text == "昨天 13:44")
            .expect("灰字那一行要有判定");
        assert_eq!(gray.noise, NoiseClass::Gray, "依据：{}", gray.evidence);
        assert_eq!(gray.speaker, Speaker::System);
        assert!(
            r.messages.iter().all(|m| m.text != "昨天 13:44"),
            "灰字不该进消息流"
        );
    }

    /// **语义遮罩的端到端验收**：聊天区里贴一张「图片」，
    /// 图片里有 OCR 读出来的字——那行字不许进 `Message[]`。
    ///
    /// 这是 ADR-0002 第 3 级的验收标准，**不是成本指标**：
    /// 「送出的像素变少了」衡量不了它，只有「图里的字没进消息流」才能。
    #[test]
    fn text_inside_an_embedded_photo_never_reaches_the_message_stream() {
        let mut px = vec![0u8; (W * H * 3) as usize];
        for y in 0..H {
            for x in 0..W {
                let i = ((y * W + x) * 3) as usize;
                let base = if x < SIDEBAR_W {
                    SIDEBAR
                } else if y < HEADER_H || y == INPUT_TOP {
                    CHROME
                } else {
                    PANEL
                };
                px[i..i + 3].copy_from_slice(&base);
            }
        }
        // 在对方气泡的位置贴一张"照片"：大片深色背景 + 散乱的色
        for y in 160..320u32 {
            for x in 224..420u32 {
                let i = ((y * W + x) * 3) as usize;
                let h = x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503);
                px[i..i + 3].copy_from_slice(&[
                    (h & 0xff) as u8,
                    ((h >> 8) & 0xff) as u8,
                    ((h >> 16) & 0xff) as u8,
                ]);
            }
        }
        // 图片里的一道"文字"
        let photo_text = Rect {
            x0: 260,
            y0: 220,
            x1: 400,
            y1: 232,
        };
        for y in [223u32, 227] {
            for x in 266..396u32 {
                let i = ((y * W + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&THEIR_INK);
            }
        }
        let frame = Frame::from_rgb(W, H, px).expect("frame");

        let view_line = OcrLine {
            text: "图片里的字".to_string(),
            rect: to_view(photo_text),
            words: vec![],
        };
        let src = RecordedLines(vec![view_line]);
        let r = read(&frame, &src).expect("整条链跑通");

        assert!(
            r.messages.is_empty(),
            "图里的字进了消息流：{:?}",
            r.messages
        );
        let a = &r.attributions[0];
        assert_eq!(a.noise, NoiseClass::ImageText, "依据：{}", a.evidence);
        assert_eq!(a.speaker, Speaker::System, "图里的字不能算任何一方说的");
    }

    /// 说话人判据的三个数必须来自**实测**，不能是拍脑袋。
    ///
    /// 实测（深色主题真实帧）：
    ///   我方 `rgb(53,210,141)` 彩度 157 · 墨色 `rgb(22,22,22)` 对比 133.2
    ///   对方 `rgb(47,47,48)` 彩度 1 · 墨色 `rgb(225,225,229)` 对比 178.4
    ///   面板灰字 `rgb(123,123,128)` 对比 93.5
    ///
    /// 对方气泡的对比度 178.4 **高于**标题阈值 140——所以判据顺序
    /// 必须先判「在不在气泡里」，反过来会把对方正文误判成标题。
    #[test]
    fn every_attribution_carries_the_measured_numbers() {
        let frame = synthetic_frame();
        let r = read(&frame, &recorded()).expect("整条链跑通");
        let by_text = |t: &str| {
            r.attributions
                .iter()
                .find(|a| a.text == t)
                .unwrap_or_else(|| panic!("缺 {t} 的判定"))
                .evidence
        };

        let me = by_text("我方那条");
        assert!(!me.on_panel, "我方在气泡里");
        assert!(me.chroma >= 30, "我方气泡彩度 {}", me.chroma);
        assert!(me.contrast > 140.0, "我方正块对比 {}", me.contrast);

        let them = by_text("对方那条");
        assert!(!them.on_panel, "对方在气泡里");
        assert!(
            them.chroma < 30,
            "对方靠**底色≠面板色**进的气泡分支，彩度只有 {}",
            them.chroma
        );
        assert!(
            them.contrast > 140.0,
            "对方对比 {} 高于标题阈值 140——这正是判据顺序不能反的原因",
            them.contrast
        );

        let gray = by_text("昨天 13:44");
        assert!(gray.on_panel, "灰字直接落在面板上");
        assert!(
            gray.contrast < 140.0,
            "灰字对比 {} 应低于标题阈值 140",
            gray.contrast
        );
    }

    /// ── 2026-10-07 实机回归：OCR 视野改成整帧之后的两条不变量 ──
    ///
    /// OCR 现在吃放大后的整帧，所以侧边栏的联系人名也会被读出来。
    /// 它们**绝不能**进消息流——否则产品会凭空「听到」并不存在的话。

    #[test]
    fn sidebar_text_never_reaches_the_message_stream() {
        // 一行「联系人名」落在侧边栏里（帧坐标 x < SIDEBAR_W）。
        let sidebar_line = OcrLine {
            text: "某个联系人".to_string(),
            rect: to_view(Rect::new(80, 120, 200, 136)),
            words: vec![],
        };
        let mut src = recorded();
        src.0.push(sidebar_line);
        let frame = synthetic_frame();
        let r = read(&frame, &src).expect("整条链跑通");

        assert!(
            !r.messages.iter().any(|m| m.text.contains("某个联系人")),
            "侧边栏的联系人名混进消息流了：{:?}",
            r.messages.iter().map(|m| &m.text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn input_box_text_never_reaches_the_message_stream() {
        // 输入框在消息区**下方**（y >= INPUT_TOP），是用户自己打的字。
        let typed = OcrLine {
            text: "我正在打的字".to_string(),
            rect: to_view(Rect::new(400, 450, 600, 470)),
            words: vec![],
        };
        let mut src = recorded();
        src.0.push(typed);
        let frame = synthetic_frame();
        let r = read(&frame, &src).expect("整条链跑通");

        assert!(
            !r.messages.iter().any(|m| m.text.contains("我正在打的字")),
            "输入框里的字混进消息流了：{:?}",
            r.messages.iter().map(|m| &m.text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn view_coordinates_are_mapped_back_into_the_pane() {
        // 换算必须精确：`THEIR_TEXT` 在消息区里的位置要和帧坐标减掉
        // 消息区原点完全一致。差一个像素都会让归类读到错的地方。
        let frame = synthetic_frame();
        let r = read(&frame, &recorded()).expect("整条链跑通");
        let chat = r.anchor.chat;
        let a = r
            .attributions
            .iter()
            .find(|a| a.text == "对方那条")
            .expect("对方那条应被归类");
        assert_eq!(
            a.rect,
            Rect::new(
                THEIR_TEXT.x0 - chat.x0,
                THEIR_TEXT.y0 - chat.y0,
                THEIR_TEXT.x1 - chat.x0,
                THEIR_TEXT.y1 - chat.y0
            ),
            "坐标没从视图坐标系换算回消息区"
        );
    }

    /// 推不出来就**报错**，不许猜一个框。
    ///
    /// 猜出来的框会让后面每一步都在错误的区域上工作，比报失败难查得多。
    #[test]
    fn uniform_frame_fails_loudly_instead_of_guessing() {
        let flat: Vec<u8> = [30u8, 30, 31].repeat((W * H) as usize);
        let f = Frame::from_rgb(W, H, flat).unwrap();
        let err = read(&f, &RecordedLines(vec![])).expect_err("纯色帧应推不出标题栏");
        assert!(
            matches!(err, ReadError::Anchor(_)),
            "应报锚点失败，实际 {err}"
        );
    }
}
