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

/// 一整帧的调试数据。
#[derive(Debug, Clone, Serialize)]
pub struct DebugPane {
    pub chat: Rect,
    pub width: u32,
    pub height: u32,
    pub boxes: Vec<DebugBox>,
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
}
