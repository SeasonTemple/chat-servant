//! 说话人与噪声归类。
//!
//! ## 阈值从哪来
//!
//! 全部来自 `cargo xtask measure` 对**真实微信帧**的实测
//! （深色主题，1262×958，chat pane 304..1260），不是照搬参考项目的数字——
//! 那是别人机器、别人微信版本的实测值。
//!
//! ```text
//! 面板底色        rgb( 30,  30,  31)   彩度  1
//! 我方气泡底色    rgb(114, 205, 144)   彩度 91   墨色 rgb( 22, 22, 22)  对比 162.6
//! 对方气泡底色    rgb( 47,  47,  48)   彩度  0   墨色 rgb(225,225,229)  对比 178.4
//! 面板灰字/时间戳  bg == 面板底色        墨色 rgb(123,123,128)        对比  93.5
//! 会话名（标题）   bg == 面板底色        墨色 rgb(225,225,229)        对比 195.4
//! 正文行高        11~14 px
//! ```
//!
//! 两帧分别取自「G8W」（我方气泡侧）与置顶的「小号」会话（两侧都有文字气泡）。
//!
//! ### ⚠ 上面这一整套**全是深色主题量出来的**，浅色主题另有一套（2026-10-14 补）
//!
//! ```text
//! 浅色主题（target/probe/frame-1791974144.png，1267×930）
//!   面板底色        rgb(250,250,250)
//!   我方气泡底色    rgb(157,242,159)   彩度 85
//!   对方气泡底色    rgb(238,238,240)   彩度  2
//! ```
//!
//! **对方气泡与面板底色只差 12, 12, 10**，而深色那套差 17, 17, 17。
//! 这就是为什么「在不在气泡里」不能吃 `COLOR_TOL = 12` 那个宽松度：
//! 浅色下 `12 <= 12` 成立，对方气泡被判成面板底色，那句话**不会变成消息**，
//! 系统等于认为**对方没说过话**。改用 `is_panel_bg`（容差 2）后两个主题都成立。
//!
//! ⇒ **凡是本文件里的实测数字，先问一句「哪个主题量的」。**
//!
//! ## 三条判据，全部实测支撑
//!
//! 1. **在不在气泡里 = 底色是否等于面板底色**（相等性，不是距离）。
//!    两侧气泡（彩度 91 / 0）与面板（彩度 1）底色互不相同，
//!    而灰字、时间戳、会话名的底色**就是**面板底色。
//! 2. **不在气泡里时，用对比度切「标题 vs 灰字」**。
//!    面板上只观测到 93.5 与 195.4 两档，阈值取 140 两端都留 45% 余量。
//! 3. **在气泡里时，用底色彩度切「我方 / 其余」**。
//!    实测我方 `rgb(114,205,144)` 彩度 **91**，其余全是低彩度：
//!    对方气泡 `rgb(47,47,48)` 彩度 **1**、面板底色 `rgb(30,30,31)` 彩度 **1**。
//!
//!    ⚠ 注意：**彩度只分得出「我方饱和」与「其余」，分不出对方与面板**——
//!    两者彩度都是 1。对方是靠第 1 条（底色 `rgb(47,47,48)` ≠ 面板 `rgb(30,30,31)`）
//!    进的气泡分支，不是靠彩度。这个区别不能搞混。
//!    阈值 30 离 91 与 1 都远，留足余量。
//!
//! ## 判据顺序是被数据逼出来的，不是设计出来的
//!
//! **对方气泡文字的对比度 178.4，比我方气泡正文的 162.6 还高，
//! 也超过了标题阈值 140。** 如果代码先看对比度，它会被判成「标题」——
//! 正因为先判了「在不在气泡里」，它才走对分支。
//! 这两个顺序反过来就会错，所以顺序必须钉死，不能靠直觉重排。

use chatservant_capture::ocr::OcrLine;
use chatservant_core::frame::{Frame, Rect};

use crate::pixels::{BubbleHit, Rgb, chroma, dist, dominant_color, is_panel_bg, probe_bubble_bg};

/// 测文字框外圈时向外扩多少像素。
///
/// 外圈必须**大于抗锯齿溢出范围、又不至于碰到相邻气泡**。
/// 实测相邻两行间距远大于 12，所以 6 有余量。
const RING_PAD: i32 = 6;

/// 不在气泡里时，对比度高于此值算「标题/会话名」，否则算灰字噪声。
///
/// 实测面板上只有两档：灰字 93.5、标题 195.4。140 在中间，
/// 距两端都超过 45%。
pub const HEADER_MIN_CONTRAST: f32 = 140.0;

/// 气泡底色彩度高于此值 = 饱和色 = 我方气泡。
///
/// 实测：我方 `rgb(114,205,144)` 彩度 **91**；对方 `rgb(47,47,48)` 与
/// 面板 `rgb(30,30,31)` 彩度都是 **1**。所以它只负责把「我方饱和气泡」
/// 从其余情况里摘出来——对方与面板的区分靠底色相等性，不靠彩度。
pub const SATURATED_MIN_CHROMA: i32 = 30;

/// 气泡外圈众数占比低于此值 = 这片是图不是气泡底色。
///
/// ⚠ **尚未实测**。取值偏保守（宁可把图判成噪声，也不要把头像当气泡）。
/// 拿到含头像/表情包的帧后要回来重标。
pub const RING_PURITY_MIN: f32 = 0.35;

/// 行高低于此值算「不是正文」。
///
/// 实测所有真实正文都是 11~12 px。小于此的是字形碎片或图标。
/// ⚠ **尚未实测噪声侧的分布**，取值同样保守。
pub const MIN_TEXT_HEIGHT: i32 = 8;

/// 谁说的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Speaker {
    /// 我方。判据是气泡底色**饱和**（彩度 ≥ [`SATURATED_MIN_CHROMA`]）。
    ///
    /// 实测我方气泡 `rgb(114,205,144)` 彩度 **91**，是面板底色与对方气泡
    /// （两者都是 1）之间唯一分得开的一档——所以「我方」只能靠饱和度认。
    Me,
    /// 对方。判据是**在气泡里**（底色 ≠ 面板底色）且**不饱和**。
    ///
    /// ⚠ 对方与面板的彩度都是 1，分开靠的是第 1 条判据（底色相等性），不是彩度。
    /// 又因为对方正文对比度 178.4 **高过**标题阈值 140，判据顺序一旦反过来，
    /// 它就会被判成标题——先判「在不在气泡里」是钉死的。
    Them,
    /// 不属于任何一方（系统、时间戳、引用块……）
    System,
}

/// 这行文字是什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseClass {
    /// 正常对话正文
    Text,
    /// 会话名、群公告等标题
    Header,
    /// 时间戳、引用块、发言人名、链接描述等灰字
    Gray,
    /// 图片/头像/表情包里的字
    ImageText,
    /// 字形碎片：太小、太散，不是一句话
    Fragment,
}

/// 从一帧学到的配色基准。
///
/// **只存面板底色。** 曾经还存「气泡色」，但那是多余的：
/// 「在不在气泡里」看底色是否等于面板底色，「我方还是对方」看彩度——
/// 两个判断都不需要先知道气泡色是什么。何况两侧气泡像素数常常打平，
/// 强行学「那个」气泡色只会引入不确定性（实测就随机学成了对方那侧）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// 学到的面板底色，取自聊天区的众数。
    ///
    /// 「在不在气泡里」全靠它与外圈底色**相等**判定（不是距离），
    /// 所以必须是精确的面板色。学不到时 [`Palette::learn`] 返回 `None`、
    /// 上层降级，**不退回硬编码颜色**——换主题/换版本时硬编码会静默失效。
    pub panel_bg: Rgb,
}

impl Palette {
    /// 从聊天区学面板底色。
    ///
    /// 学不到（区域太空，比如空会话）返回 `None`——上层据此降级，
    /// **不退回硬编码的颜色**。硬编码在换主题/换版本时会静默失效。
    pub fn learn(frame: &Frame, chat: Rect) -> Option<Self> {
        Some(Self {
            panel_bg: dominant_color(frame, chat)?,
        })
    }
}

/// 判定依据。**每条 Attribution 都带着它**，认错了能立刻看出是哪一步。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Evidence {
    /// 文字框外圈的主色
    pub bg: Rgb,
    /// 框内与 bg 距离最大的像素
    pub ink: Rgb,
    /// ink 与 bg 的感知距离
    pub contrast: f32,
    /// 底色是否等于面板底色（= 不在气泡里）
    pub on_panel: bool,
    /// 底色彩度
    pub chroma: i32,
    /// 外圈众数占比
    pub purity: f32,
    /// 文字框行高。低于 [`MIN_TEXT_HEIGHT`] 直接判成 [`NoiseClass::Fragment`]。
    ///
    /// 实测所有真实正文都是 11~12 px，比这更矮的是字形碎片或图标。
    pub height: i32,
}

impl std::fmt::Display for Evidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "bg=rgb({},{},{}) ink=rgb({},{},{}) 对比={:.1} {} 彩度={} 纯度={:.2} 高={}",
            self.bg[0],
            self.bg[1],
            self.bg[2],
            self.ink[0],
            self.ink[1],
            self.ink[2],
            self.contrast,
            if self.on_panel {
                "在面板上"
            } else {
                "在气泡里"
            },
            self.chroma,
            self.purity,
            self.height,
        )
    }
}

/// 一行文字的归类结果。
#[derive(Debug, Clone, PartialEq)]
pub struct Attribution {
    /// 谁说的。只有 [`NoiseClass::Text`] 才可能是 [`Speaker::Me`] / [`Speaker::Them`]，
    /// 其余一律 [`Speaker::System`]——噪声不参与消息流。
    pub speaker: Speaker,
    /// 这行算正文还是噪声。只有 [`NoiseClass::Text`] 会进 [`crate::merge::group`]。
    pub noise: NoiseClass,
    /// OCR 认出来的原文。归并成消息时要用，不能丢。
    pub text: String,
    /// 文字框在帧内的位置，坐标系与传入的 `Frame` 一致（左上角为原点）。
    ///
    /// 归并（按 [`crate::merge::MERGE_GAP_RATIO`] 量垂直间距）与媒体检测
    /// 都要用它，所以坐标系错一步后面全都错。
    pub rect: Rect,
    /// 这一行是**凭什么**这么判的。认错了要能立刻定位到是哪条判据错的，
    /// 所以 [`classify`] 把每一步的中间量都留在了这里。
    pub evidence: Evidence,
}

/// 归类一行。
pub fn classify(line: &OcrLine, palette: &Palette, probe: BubbleHit) -> Attribution {
    let on_panel = is_panel_bg(probe.bg, palette.panel_bg);
    let evidence = Evidence {
        bg: probe.bg,
        ink: probe.ink,
        contrast: probe.contrast,
        on_panel,
        chroma: chroma(probe.bg),
        purity: probe.purity,
        height: line.height(),
    };

    let (speaker, noise) = if evidence.height < MIN_TEXT_HEIGHT {
        // 太矮，不是一行话
        (Speaker::System, NoiseClass::Fragment)
    } else if evidence.purity < RING_PURITY_MIN {
        // 外圈颜色杂乱 = 这是张图，不是有底色的气泡
        (Speaker::System, NoiseClass::ImageText)
    } else if on_panel {
        // 不在气泡里。面板上只有标题与灰字两档。
        if evidence.contrast >= HEADER_MIN_CONTRAST {
            (Speaker::System, NoiseClass::Header)
        } else {
            (Speaker::System, NoiseClass::Gray)
        }
    } else if evidence.chroma >= SATURATED_MIN_CHROMA {
        // 在气泡里，且是饱和色 = 我的气泡
        (Speaker::Me, NoiseClass::Text)
    } else {
        // 在气泡里，但是近灰色 = 对方气泡（实测彩度 0）
        (Speaker::Them, NoiseClass::Text)
    };

    Attribution {
        speaker,
        noise,
        text: line.text.clone(),
        rect: line.rect,
        evidence,
    }
}

/// 量一行并归类。`frame` 与 `line` 必须同一坐标系（都相对帧左上角）。
pub fn classify_in_frame(line: &OcrLine, frame: &Frame, palette: &Palette) -> Option<Attribution> {
    let probe = probe_bubble_bg(frame, line.rect, RING_PAD)?;
    Some(classify(line, palette, probe))
}

/// 测量并归类整个聊天区。
pub fn classify_all(lines: &[OcrLine], frame: &Frame, palette: &Palette) -> Vec<Attribution> {
    lines
        .iter()
        .filter_map(|l| classify_in_frame(l, frame, palette))
        .collect()
}

/// 供调试用：某行文字到面板底色的色距。
pub fn panel_distance(bg: Rgb, palette: &Palette) -> f32 {
    dist(bg, palette.panel_bg)
}
