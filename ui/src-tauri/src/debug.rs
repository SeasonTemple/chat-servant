//! 调试画布的数据——**把每个识别框按归类结果上色**。
//!
//! ## 为什么值得单独一个模块
//!
//! 「为什么这条没进消息流」是调试时最高频的问题。答案散在
//! `Attribution::noise` 与 `evidence` 里，前端要自己翻译一遍。
//! 翻译规则放在**读得到那些数据的一侧**（Rust），才不会两边各写一套、
//! 慢慢分叉。
//!
//! ## 六种颜色，不是两种
//!
//! 只有「归类 / 丢弃」两种颜色时，看到框消失还是变色，判断不出是
//! 「字太小」还是「当成图片」——而这两者的修法完全不同。
//! 每条丢弃路径都必须有自己的颜色（ADR-0002：拒绝理由要能定位到级）。

use chatservant_core::frame::Rect;
use chatservant_perceive::{Attribution, NoiseClass, Speaker};
use serde::Serialize;

/// 一个框的归类结果。**前端只渲染，不重算。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BoxKind {
    /// 我方正文
    Me,
    /// 对方正文
    Them,
    /// 会话名、群公告等**标题**
    Header,
    /// 时间戳、引用块、发言人名等**灰字**
    Gray,
    /// 当成图片丢掉的（图内字、头像）
    DroppedAsImage,
    /// 字太小被当成碎片丢掉的
    DroppedAsFragment,
    /// 归成系统但不是噪声
    System,
}

impl BoxKind {
    /// 十六进制色值。前端直接用，不做映射。
    pub fn css(self) -> &'static str {
        match self {
            Self::Me => "#72cd90",                // 我方绿
            Self::Them => "#7ab8f0",              // 对方蓝
            Self::Header => "#f5d76e",            // 标题 黄
            Self::Gray => "#8d8d94",              // 灰字 暗灰
            Self::DroppedAsImage => "#f6a04d",    // 当图丢掉 橙
            Self::DroppedAsFragment => "#c77dd6", // 太小丢掉 紫
            Self::System => "#e8e8ea",            // 系统 白
        }
    }

    /// 画在框角上的中文标签。
    pub fn label(self) -> &'static str {
        match self {
            Self::Me => "我",
            Self::Them => "对方",
            Self::Header => "标题",
            Self::Gray => "灰字",
            Self::DroppedAsImage => "当图丢掉",
            Self::DroppedAsFragment => "太小丢掉",
            Self::System => "系统",
        }
    }

    /// 从归类结果推出该画什么框。**这是唯一的翻译点。**
    pub fn of(a: &Attribution) -> Self {
        match a.noise {
            NoiseClass::Text => match a.speaker {
                Speaker::Me => Self::Me,
                Speaker::Them => Self::Them,
                Speaker::System => Self::System,
            },
            // 标题与灰字**必须分开**：两者都是「不进消息流」，但
            // 一个是对话名（有用，值得留），一个是时间戳（噪声）。
            // 合成一种颜色时，看到框消失判断不出是哪一类。
            NoiseClass::Header => Self::Header,
            NoiseClass::Gray => Self::Gray,
            NoiseClass::ImageText => Self::DroppedAsImage,
            NoiseClass::Fragment => Self::DroppedAsFragment,
        }
    }
}

/// 给前端的形状。**颜色与标签一起发**——前端不该再自己映射。
#[derive(Debug, Clone, Serialize)]
pub struct DebugBox {
    pub rect: Rect,
    pub text: String,
    pub evidence: String,
    pub css: &'static str,
    pub label: &'static str,
    /// 是不是正文（前端「只看被丢弃的」用它过滤）。
    pub is_text: bool,
}

/// 一帧的开销账。回答 user story 13：「这一帧到底有多少像素被送去识别、
/// 花了多少视觉 token」——不然用户没法判断该调哪一档。
///
/// ## 三个像素数**不是一回事**，别只报一个
///
/// 这是本结构存在的全部理由：
///
/// | 字段 | 是什么 | 谁花的
/// |---|---|---|
/// | `ocr_pixels` | **真的送进 OCR 的**——放大后的**整帧** | OCR 引擎
/// | `roi_pixels` | 水位线以下的增量带 | 只是「哪些行算数」
/// | `pane_pixels` | 整块消息区 | 分母
///
/// ⚠ **`ocr_pixels` 恒等于整帧，不是 ROI。** 这是 2026-10-07 实测逼出来的
/// 做法（`watch.rs` 里那段注释）：`Windows.Media.Ocr` 需要行周围有内容才收，
/// 裁得越紧漏得越多——实测同一块画面，裁 ROI 反而读不到「乐」那条消息。
/// 所以**识别精度/成本滑块对 OCR 开销无效**，省 OCR 只能靠变化门控
/// （画面没变时**一次都不跑**）。
///
/// 这个真相对用户有用：它把「我该调哪一档」这个问题从猜变成有依据。
/// 只报一个数字的话，用户会以为调滑块能省 OCR——而那调了没反应。
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameCost {
    /// 送进 OCR 的像素数（放大后的整帧）。**静默期是 0**——没跑过。
    pub ocr_pixels: u64,
    /// 水位线以下的增量带面积。`None` = 这一帧没读。
    pub roi_pixels: Option<u64>,
    /// 整块消息区面积。没认出锚点时是 0。
    pub pane_pixels: u64,
    /// 这一帧调了几次 OCR。**静默期必须是 0。**
    pub ocr_calls: u32,
    /// 这一帧花了多少视觉 token。**默认配置下恒为 0**（图片随文本进 B）。
    pub vision_tokens: u64,
}

impl FrameCost {
    /// 增量带占整块消息区的百分比，`0.0..=100.0`。没读时 `None`。
    ///
    /// **前端不算这个除法**——它不是显示细节，它是一个判断：
    /// 接近 100% 说明这一帧几乎重读了整块，增量门控没在起作用。
    pub fn roi_pct(&self) -> Option<f32> {
        match (self.roi_pixels, self.pane_pixels) {
            (Some(r), p) if p > 0 => Some(r as f32 * 100.0 / p as f32),
            _ => None,
        }
    }
}

/// 手写序列化：**多带一个 `roiPct`，且它只在这一处算。**
///
/// 为什么不用 `#[derive(Serialize)]` 挂一个字段：那需要一个 `roi_pct`
/// 字段，于是谁构造谁就得记得填——**填漏了不会有任何报错**，只会
/// 界面上那个百分比悄悄变成 `null`。而在序列化时算，它和原始数字
/// 永远同源，**没有能漂移的第二份状态**。
impl Serialize for FrameCost {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("FrameCost", 6)?;
        st.serialize_field("ocrPixels", &self.ocr_pixels)?;
        st.serialize_field("roiPixels", &self.roi_pixels)?;
        st.serialize_field("panePixels", &self.pane_pixels)?;
        st.serialize_field("ocrCalls", &self.ocr_calls)?;
        st.serialize_field("visionTokens", &self.vision_tokens)?;
        st.serialize_field("roiPct", &self.roi_pct())?;
        st.end()
    }
}

/// 一整帧的调试数据。
#[derive(Debug, Clone, Serialize)]
pub struct DebugPane {
    pub chat: Rect,
    pub width: u32,
    pub height: u32,
    pub boxes: Vec<DebugBox>,
    /// 这一帧的开销账。**静默期也照发**（全 0）——
    /// 不发的话界面只能一直挂着上一帧的数字，用户看不出「这一帧什么都没做」，
    /// 而那恰恰是变化门控有没有生效的**唯一证据**。
    pub cost: FrameCost,
}

/// 从归类结果造出画布数据。
pub fn build(chat: Rect, width: u32, height: u32, attrs: &[Attribution]) -> DebugPane {
    DebugPane {
        chat,
        width,
        height,
        boxes: attrs
            .iter()
            .map(|a| {
                let k = BoxKind::of(a);
                DebugBox {
                    rect: a.rect,
                    text: a.text.clone(),
                    evidence: a.evidence.to_string(),
                    css: k.css(),
                    label: k.label(),
                    is_text: a.noise == NoiseClass::Text,
                }
            })
            .collect(),
        // 由调用方在 OCR 之后用 [`FrameCost`] 覆盖——那时候才知道
        // 真正送进 OCR 的是多少像素。这里先给全 0，**不给假的估计值**。
        cost: FrameCost::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chatservant_perceive::Evidence;

    fn attr(speaker: Speaker, noise: NoiseClass) -> Attribution {
        Attribution {
            speaker,
            noise,
            text: "x".into(),
            rect: Rect::new(1, 2, 3, 4),
            evidence: Evidence {
                bg: [0, 0, 0],
                ink: [1, 1, 1],
                contrast: 1.0,
                on_panel: false,
                chroma: 0,
                purity: 1.0,
                height: 12,
            },
        }
    }

    /// 七种归类结果**必须**给出七种不同的颜色。
    ///
    /// 只有两种颜色时，看到框消失还是变色，判断不出是「太小」还是
    /// 「当成图片」，而这两者的修法完全不同。`Header` 与 `Gray` 同理：
    /// 一个是对话名（值得留），一个是时间戳（纯噪声）。
    #[test]
    fn every_outcome_gets_its_own_colour() {
        let kinds = [
            BoxKind::of(&attr(Speaker::Me, NoiseClass::Text)),
            BoxKind::of(&attr(Speaker::Them, NoiseClass::Text)),
            BoxKind::of(&attr(Speaker::System, NoiseClass::Text)),
            BoxKind::of(&attr(Speaker::System, NoiseClass::Gray)),
            BoxKind::of(&attr(Speaker::System, NoiseClass::Header)),
            BoxKind::of(&attr(Speaker::System, NoiseClass::ImageText)),
            BoxKind::of(&attr(Speaker::System, NoiseClass::Fragment)),
        ];
        assert_eq!(kinds.len(), 7, "七种归类结果都要有颜色");

        let mut css: Vec<&str> = kinds.iter().map(|k| k.css()).collect();
        css.sort_unstable();
        let before = css.len();
        css.dedup();
        assert_eq!(css.len(), before, "有归类结果共用颜色：{css:?}");

        let mut labels: Vec<&str> = kinds.iter().map(|k| k.label()).collect();
        labels.sort_unstable();
        let before = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), before, "有归类结果共用标签：{labels:?}");
    }

    #[test]
    fn build_carries_evidence_through() {
        let p = build(
            Rect::new(0, 0, 100, 100),
            200,
            200,
            &[attr(Speaker::Them, NoiseClass::Text)],
        );
        assert_eq!(p.boxes.len(), 1);
        assert_eq!(p.boxes[0].label, "对方");
        assert!(p.boxes[0].is_text);
        assert!(!p.boxes[0].evidence.is_empty(), "依据必须带过去");
    }

    /// 百分比**在序列化那一刻算**，和原始数字永远同源。
    ///
    /// 早期版本想挂一个 `roi_pct` 字段，但那样谁构造谁就得记得填——
    /// 填漏了**不会有任何报错**，界面上只是那个百分比悄悄变成 `null`。
    /// 这条断言的是「JSON 里那个数一定和原始数字对得上」。
    #[test]
    fn the_serialised_pct_always_matches_the_raw_pixels() {
        let cost = FrameCost {
            ocr_pixels: 400 * 100,
            roi_pixels: Some(100 * 50),
            pane_pixels: 100 * 800,
            ocr_calls: 1,
            vision_tokens: 0,
        };
        let v = serde_json::to_value(cost).expect("能序列化");
        assert_eq!(v["ocrPixels"], 40000u64);
        assert_eq!(v["roiPixels"], 5000u64);
        assert_eq!(v["panePixels"], 80000u64);
        assert_eq!(v["ocrCalls"], 1u64);
        // 5000 / 80000 = 6.25%
        assert!(
            (v["roiPct"].as_f64().unwrap() - 6.25).abs() < 1e-6,
            "百分比算错：{v}"
        );
    }

    /// **静默期也要报数**，而且报的是 0，不是「没有这一项」。
    ///
    /// 静默期零调用是硬约束第 6 条，而「这一帧 0 像素 / 0 次 OCR」
    /// 是它唯一可信的证据。报 `null` 等于把证据收起来了。
    #[test]
    fn a_quiet_frame_reports_zeroes_not_nothing() {
        let quiet = FrameCost::default();
        let v = serde_json::to_value(quiet).expect("能序列化");
        assert_eq!(v["ocrPixels"], 0u64);
        assert_eq!(v["ocrCalls"], 0u64);
        assert_eq!(v["visionTokens"], 0u64);
        assert!(v["roiPixels"].is_null(), "没读就没有增量带");
        assert!(
            v["roiPct"].is_null(),
            "没读就没有百分比——不是 0%，那会被读成「读了但一块都没读」"
        );
    }

    /// 没认出锚点时分母是 0，**不能除出 NaN 或 inf**。
    ///
    /// NaN 序列化过去是 `null`，而界面上一个 `null` 和「没读」长得一样，
    /// 于是锚点没找到这件事就被吞掉了。
    #[test]
    fn a_zero_pane_never_divides() {
        let c = FrameCost {
            ocr_pixels: 100,
            roi_pixels: Some(10),
            pane_pixels: 0,
            ocr_calls: 1,
            vision_tokens: 0,
        };
        assert_eq!(c.roi_pct(), None);
        let v = serde_json::to_value(c).expect("能序列化");
        assert!(v["roiPct"].is_null(), "不该把 NaN 发给界面：{v}");
    }

    /// 整数面积算百分比**不能溢出**。`u64` 的面积在 4K 帧放大 2 倍后
    /// 也就 3300 万，`u32` 也够——但这里锁的是「用 u64 传」这个决定，
    /// 免得以后有人顺手改成 u32 再在某台高 DPI 机器上绕回。
    #[test]
    fn large_frames_do_not_overflow() {
        let c = FrameCost {
            ocr_pixels: u64::from(u32::MAX) * 2,
            roi_pixels: Some(u64::from(u32::MAX)),
            pane_pixels: u64::from(u32::MAX) * 4,
            ocr_calls: 1,
            vision_tokens: 0,
        };
        let pct = c.roi_pct().expect("能算");
        assert!((0.0..=100.0).contains(&pct), "百分比越界了：{pct}");
        assert!((pct - 25.0).abs() < 1e-6, "算错了：{pct}");
    }
}
