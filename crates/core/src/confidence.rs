//! 置信度**分档**——把起草段自评的概率映射成三档，供界面画笔迹。
//!
//! ## 为什么分档这件事住在 Rust 侧
//!
//! 概率是起草段（`SYS_DRAFT`）让模型自评的「你认为用户会选哪条」。
//! 界面**不显示百分数**了（2026-10-25 用户定案：首选行只有勾，
//! 备选行改成一道**笔迹轻重**表示置信程度）⇒ 概率不再直接上屏，
//! 取而代之的是「高 / 中 / 低」三档笔迹。
//!
//! ⚠ **档位与概率的对应关系只有这一份**。如果前端自己按阈值分档，
//! 那阈值就有了第二份拷贝，而后端改阈值时前端不会跟着改 ⇒
//! 面板画出来的笔迹和后端以为的对不上，而且**没有任何编译器会告诉你**。
//! ⇒ 分档在这里算，界面只按后端给的档位选笔形
//! （`ui/src/lib/PaperPanel.svelte` 里的 `row.confidence`）。
//!
//! ## 阈值为什么是绝对值，不是分位
//!
//! 分位（按本轮候选的数量取前 1/3）能保证三档都出现，但会**谎报差别**：
//! 三条候选的概率是 0.9 / 0.89 / 0.88 时，分位会说「第一条高、第二条中」，
//! 而它们其实一样高。绝对阈值不会——三条会落在同一档。
//!
//! ⇒ 用绝对阈值：同一个概率在任何一轮都落在同一档，
//! **跨轮次可比**，也不会把「差不多」画成「差很多」。
//!
//! ⚠ 代价写在 [`ConfidenceTier::from_probability`] 的文档里：
//! 首选行不显示置信度 ⇒ 备选的概率天然比首选低 ⇒ **「高」档不常见**
//! （它只在候选之间差距很小、首选本身也不高的时候出现）。

use serde::Serialize;

/// 置信度档位。**三档，没有第四档。**
///
/// 序列化给前端时是 `"high"` / `"mid"` / `"low"`（小写）。
///
/// ⚠ **声明顺序就是强度顺序**（低 → 中 → 高）⇒ 派生的 `Ord` 可直接用来
/// 比较「谁更强」。写成 High 在前会让「更强的档位 Ord 更小」，
/// 而那正是本文件里那条单调性判据要抓的方向错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfidenceTier {
    /// 把握小。
    Low,
    /// 中等。
    Mid,
    /// 把握大。
    High,
}

impl ConfidenceTier {
    /// 「高」的下界：概率 ≥ 此值算 [`ConfidenceTier::High`]。
    pub const HIGH_AT: f32 = 0.5;
    /// 「中」的下界：概率 ≥ 此值算 [`ConfidenceTier::Mid`]，否则 [`ConfidenceTier::Low`]。
    pub const MID_AT: f32 = 0.2;

    /// 按概率分档。
    ///
    /// * `p >= 0.5` ⇒ 高
    /// * `p >= 0.2` ⇒ 中
    /// * 其余 ⇒ 低
    ///
    /// ## 边界与非法输入
    ///
    /// * 阈值本身算**上一档**（`0.5` 是高，`0.2` 是中）——
    ///   「达到」就是够到那一档。
    /// * `NaN` ⇒ 落**低**档。`NaN` 跟任何阈值比较都是 false，
    ///   若按「小于中界」处理，NaN 会静悄悄变成低档；这里显式写出来，
    ///   免得哪天有人改顺序时把它变成高。
    /// * 超出 0..=1 的值按数值本身比较（模型给 1.3 就当高）。
    pub fn from_probability(p: f32) -> Self {
        if p.is_nan() {
            return Self::Low;
        }
        if p >= Self::HIGH_AT {
            Self::High
        } else if p >= Self::MID_AT {
            Self::Mid
        } else {
            Self::Low
        }
    }

    /// 档位的机器可读名（与序列化一致）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Mid => "mid",
            Self::High => "high",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **阈值边界**：达到阈值就算够到那一档。
    #[test]
    fn the_thresholds_land_on_the_higher_tier() {
        assert_eq!(ConfidenceTier::from_probability(0.5), ConfidenceTier::High);
        assert_eq!(ConfidenceTier::from_probability(0.2), ConfidenceTier::Mid);
        assert_eq!(ConfidenceTier::from_probability(0.19), ConfidenceTier::Low);
        assert_eq!(ConfidenceTier::from_probability(0.49), ConfidenceTier::Mid);
    }

    /// **两端的极限**也要有答案，不能 panic、不能空。
    #[test]
    fn the_extremes_are_defined() {
        assert_eq!(ConfidenceTier::from_probability(0.0), ConfidenceTier::Low);
        assert_eq!(ConfidenceTier::from_probability(1.0), ConfidenceTier::High);
    }

    /// **`NaN` 落低档**，且**不 panic**。
    ///
    /// ⚠ 这条单测看起来像废话，但它是刻意的：概率来自模型的自评，
    /// 而模型可能给出一个解析不出来的数。分档函数**绝不能**因为它崩掉。
    #[test]
    fn a_nan_probability_lands_on_low_instead_of_panicking() {
        assert_eq!(
            ConfidenceTier::from_probability(f32::NAN),
            ConfidenceTier::Low
        );
    }

    /// **档位随概率单调不降**——概率更大，档位不会更弱。
    ///
    /// ⚠ 这条是「同一个概念只有一把尺」的机器判据：
    /// 分档函数里如果哪天有人把两个阈值写反了，这里立刻红。
    #[test]
    fn the_tier_never_gets_weaker_as_the_probability_grows() {
        let mut prev = ConfidenceTier::Low;
        for step in 0..=100 {
            let p = step as f32 / 100.0;
            let tier = ConfidenceTier::from_probability(p);
            assert!(
                tier >= prev,
                "概率 {p} 落到了 {tier:?}，比上一档 {prev:?} 更弱"
            );
            prev = tier;
        }
    }

    /// **三档都真的会出现**——不是「定义了但永远轮不到」。
    #[test]
    fn all_three_tiers_are_reachable() {
        assert_eq!(ConfidenceTier::from_probability(0.7), ConfidenceTier::High);
        assert_eq!(ConfidenceTier::from_probability(0.2), ConfidenceTier::Mid);
        assert_eq!(ConfidenceTier::from_probability(0.1), ConfidenceTier::Low);
    }
}
