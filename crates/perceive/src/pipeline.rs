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

use chatservant_capture::anchor::{self, AnchorError, PaneAnchor};
use chatservant_capture::ocr::{self, OcrError, OcrText};
use chatservant_core::frame::Frame;

use crate::attribute::{Attribution, Palette};
use crate::merge::{self, Message};

/// 一次完整读取的结果。中间态也留着——调试视图与回归测试都要看。
#[derive(Debug, Clone)]
pub struct Read {
    pub anchor: PaneAnchor,
    pub palette: Palette,
    /// 逐行判定，含被判成噪声的。调试视图按这个上色。
    pub attributions: Vec<Attribution>,
    /// 归并后的消息流，**不含噪声**。
    pub messages: Vec<Message>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("锚点推不出来：{0}")]
    Anchor(#[from] AnchorError),

    #[error("学不到配色（聊天区太平，可能是空会话）——上层应降级，不许退回硬编码颜色")]
    NoPalette,

    #[error("按锚点裁不出消息区")]
    NoPane,

    #[error(transparent)]
    Ocr(#[from] OcrError),
}

/// 取文本框这一步的来源。
///
/// 抽出来只有一个理由：**它是整条链路上唯一带隐私的步骤**。
/// 其余每一步都在读像素，而像素是可以脱敏后提交的。
pub trait LineSource {
    /// `pane` 是**按锚点裁好的消息区**，行框坐标以它为原点。
    fn read(&self, pane: &Frame) -> Result<OcrText, OcrError>;
}

/// 真的调 Windows OCR。纯本地，不出网。
pub struct SystemOcr;

impl LineSource for SystemOcr {
    fn read(&self, pane: &Frame) -> Result<OcrText, OcrError> {
        ocr::recognize(pane)
    }
}

/// 跑完整条链路。`source` 只负责取文本框，其余全真跑。
pub fn read(frame: &Frame, source: &impl LineSource) -> Result<Read, ReadError> {
    let anchor = anchor::derive(frame)?;
    let palette = Palette::learn(frame, anchor.chat).ok_or(ReadError::NoPalette)?;
    let pane = frame.crop(anchor.chat).ok_or(ReadError::NoPane)?;
    let ocr = source.read(&pane)?;
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
    const MY_BUBBLE: [u8; 3] = [114, 205, 144];
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

    /// 把帧坐标换算成**消息区坐标**（锚点裁剪之后坐标系变了）。
    fn to_pane(r: Rect, origin: Rect) -> Rect {
        Rect::new(
            r.x0 - origin.x0,
            r.y0 - origin.y0,
            r.x1 - origin.x0,
            r.y1 - origin.y0,
        )
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

    fn recorded(origin: Rect) -> RecordedLines {
        RecordedLines(
            [
                (MY_TEXT, "我方那条"),
                (THEIR_TEXT, "对方那条"),
                (GRAY_TEXT, "昨天 13:44"),
            ]
            .into_iter()
            .map(|(r, text)| {
                let p = to_pane(r, origin);
                OcrLine {
                    text: text.to_string(),
                    rect: p,
                    words: vec![],
                }
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
        let origin = anchor::derive(&frame).expect("锚点").chat;
        let r = read(&frame, &recorded(origin)).expect("整条链跑通");

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

        let origin = anchor::derive(&frame).expect("锚点").chat;
        let pane_line = OcrLine {
            text: "图片里的字".to_string(),
            rect: to_pane(photo_text, origin),
            words: vec![],
        };
        let src = RecordedLines(vec![pane_line]);
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
    ///   我方 `rgb(114,205,144)` 彩度 91 · 墨色 `rgb(22,22,22)` 对比 162.6
    ///   对方 `rgb(47,47,48)` 彩度 1 · 墨色 `rgb(225,225,229)` 对比 178.4
    ///   面板灰字 `rgb(123,123,128)` 对比 93.5
    ///
    /// ⚠ 对方气泡的对比度 178.4 **高于**标题阈值 140——所以判据顺序
    /// 必须先判「在不在气泡里」，反过来会把对方正文误判成标题。
    #[test]
    fn every_attribution_carries_the_measured_numbers() {
        let frame = synthetic_frame();
        let origin = anchor::derive(&frame).expect("锚点").chat;
        let r = read(&frame, &recorded(origin)).expect("整条链跑通");
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
        assert!(me.contrast > 140.0, "我方正��对比 {}", me.contrast);

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
