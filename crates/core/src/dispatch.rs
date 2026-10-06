//! 投递档位状态机——纯逻辑，不碰 Win32。
//!
//! ## 为什么不是一个布尔值
//!
//! 注入失败有两种完全不同的含义：
//!
//! - **确定没发**：连剪贴板都没写成。重放无害。
//! - **可能已经发了**：剪贴板写成功了，但回读超时的那 0.4 秒里，用户
//!   可能自己按了回车，或者微信的发送动画已经播完了。
//!
//! 第二种情况下**重放会发出两条一样的消息，而发出去的东西无法撤销**。
//! 所以 [`DeliveryReceipt`] 必须能表达「不知道」，并且这个「不知道」
//! 要**永久禁止自动重放**。
//!
//! ## 升档只能由人触发
//!
//! 程序的任何内部状态变化——失败、超时、回读对不上——**都不能**把档位
//! 升上去或降下去。档位只被两个东西改：人的显式操作，和会话结束。
//! 会话结束一律回落到 [`Dispatch::Manual`]，不管之前在哪一档。

use std::fmt;

/// 一次投递的结果。
///
/// **不是布尔值。** 见模块文档。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryReceipt {
    /// 确定没发（剪贴板都没写成）。**可以安全重放。**
    NotSent,
    /// 送达且已回读验证。
    Sent,
    /// 可能已经发了——**禁止自动重放**。
    PossiblySent,
    /// 明确失败，可按退避重试。
    Failed(String),
    /// 超时，结果未知，按 `PossiblySent` 对待。
    TimedOut,
}

impl DeliveryReceipt {
    /// 重放安不安全。
    ///
    /// 只有**确定没发**才安全。`TimedOut` 与 `PossiblySent` 同等对待——
    /// 超时恰恰是最容易「其实已经发出去了」的那种情况。
    pub fn replay_safe(&self) -> bool {
        matches!(self, Self::NotSent | Self::Failed(_))
    }

    /// 要不要挡住自动重试。
    pub fn blocks_automatic_retry(&self) -> bool {
        !self.replay_safe()
    }

    /// 界面上给人看的一句话。
    pub fn explain(&self) -> &'static str {
        match self {
            Self::NotSent => "确定没发，可以重试",
            Self::Sent => "已送达并回读验证",
            Self::PossiblySent => "可能已经发出去了——不再自动重放，请自己看一眼聊天窗口",
            Self::Failed(_) => "明确失败，可以按退避重试",
            Self::TimedOut => "回读超时，按「可能已发送」对待——不再自动重放",
        }
    }
}

/// 发送档位。**默认且终态最低是 `Manual`。**
///
/// 变体顺序即档位高低：靠前的更低。`Default` 落在 `Manual` 上，
/// `#[derive(Default)]` 只能在末位变体上生效，所以手写。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dispatch {
    /// 只填入输入框，停在光标处。默认值，会话结束回落到此档。
    Manual,
    /// 填入后二次确认才发送。
    Confirm,
    /// 填入并发送，不打断。
    Auto,
}

impl Default for Dispatch {
    /// `Manual`——**最低档**。派生实现会默认到末位（`Auto`），
    /// 正好是这里最不能默认的那一档，所以必须手写。
    fn default() -> Self {
        Self::Manual
    }
}

impl Dispatch {
    /// 这个档位会不会自己发出去。
    pub fn sends_without_asking(&self) -> bool {
        matches!(self, Self::Auto)
    }
}

impl fmt::Display for Dispatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Manual => "Manual（只填入）",
            Self::Confirm => "Confirm（确认后发送）",
            Self::Auto => "Auto（直接发送）",
        })
    }
}

/// 指数退避的上限与次数。
pub const MAX_RETRIES: u32 = 3;
pub const BASE_BACKOFF_MS: u64 = 800;
pub const MAX_BACKOFF_MS: u64 = 8_000;

/// 第 `attempt` 次重试要等多久（`attempt` 从 0 起）。
///
/// 有次数上限，是为了让「一直失败」变成一个**停下来等人**的状态，
/// 而不是永远转圈。
pub fn backoff_ms(attempt: u32) -> Option<u64> {
    if attempt >= MAX_RETRIES {
        return None;
    }
    Some((BASE_BACKOFF_MS << attempt).min(MAX_BACKOFF_MS))
}

/// 当前档位 + 上一次结果 → 下一步该做什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// 到点了，可以再投一次。
    Retry { after_ms: u64, attempt: u32 },
    /// 停下来等**人**。程序不许自己继续。
    Halt { reason: String },
    /// 投递失败，但要降档重试——必须由**人**确认后才继续。
    NeedsConfirmation { proposed: Dispatch, reason: String },
}

impl Next {
    /// 界面上要显示的完整理由。**每一步都必须有话可说。**
    pub fn explain(&self) -> String {
        match self {
            Self::Retry { after_ms, attempt } => {
                format!("第 {attempt} 次重试，等 {after_ms} ms")
            }
            Self::Halt { reason } => format!("停下等人工：{reason}"),
            Self::NeedsConfirmation { proposed, reason } => {
                format!("需要人工确认后继续（拟切到 {proposed}）：{reason}")
            }
        }
    }
}

/// 档位守则的判定。**纯函数，没有副作用**——这样每条不变量都能测。
#[derive(Debug, Clone, Copy)]
pub struct Dispatcher {
    tier: Dispatch,
}

impl Dispatcher {
    /// 永远是 `Manual`。没有「默认 Auto」这种东西。
    pub fn new() -> Self {
        Self {
            tier: Dispatch::Manual,
        }
    }

    pub fn tier(&self) -> Dispatch {
        self.tier
    }

    /// 人显式设档。**唯一的升档入口。**
    pub fn set_tier(&mut self, tier: Dispatch) {
        self.tier = tier;
    }

    /// 会话结束。**一律回落到 `Manual`**，不管之前在哪一档。
    pub fn end_session(&mut self) {
        self.tier = Dispatch::Manual;
    }

    /// 上一次投递结果是 `receipt`、已经重试了 `attempt` 次 → 下一步。
    pub fn next(&self, receipt: DeliveryReceipt, attempt: u32) -> Next {
        // 可能已经发出去了 —— 无论什么档位，都不许自动重放。
        // 这条检查必须在最前面：Auto 档也不许。
        if receipt.blocks_automatic_retry() {
            return Next::Halt {
                reason: receipt.explain().to_string(),
            };
        }

        // 成功了就没事了。
        if matches!(receipt, DeliveryReceipt::Sent) {
            return Next::Halt {
                reason: "已送达，无需后续动作".to_string(),
            };
        }

        let Some(after_ms) = backoff_ms(attempt) else {
            return Next::Halt {
                reason: format!("重试 {MAX_RETRIES} 次仍失败，停下等人工"),
            };
        };

        // Auto 档下投递失败：不能自动降级重试再发，必须人要确认。
        //
        // 理由：Auto 是「不打断」的承诺。承诺的兑现方式是失败后**停下来问**，
        // 而不是自己改主意。用户以为在 Auto 模式，结果程序偷偷降了档又发一次，
        // 那比直接失败更糟。
        if self.tier == Dispatch::Auto {
            return Next::NeedsConfirmation {
                proposed: Dispatch::Confirm,
                reason: format!(
                    "Auto 档投递失败（{}），降档重试需要人工确认",
                    receipt.explain()
                ),
            };
        }

        Next::Retry { after_ms, attempt }
    }

    /// 人在 `NeedsConfirmation` 上点了「继续」。**只有人能走到这里。**
    pub fn confirm_downgrade(&mut self, proposed: Dispatch) {
        self.tier = proposed;
    }
}

impl Default for Dispatcher {
    /// 与 `new()` 同义：默认档 `Manual`。`Dispatcher` 没有别的默认构造。
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 重放一条已经发出的消息无法撤销。这条不变量必须锁死。
    #[test]
    fn possibly_sent_is_never_replayed() {
        let d = Dispatcher::new();
        for tier in [Dispatch::Manual, Dispatch::Confirm, Dispatch::Auto] {
            let mut d = d;
            d.set_tier(tier);
            let n = d.next(DeliveryReceipt::PossiblySent, 0);
            assert!(
                matches!(n, Next::Halt { .. }),
                "{tier} 档下 PossiblySent 必须停下，实际 {n:?}"
            );
        }
    }

    /// 超时是最容易「其实已经发出去」的那种情况，按 PossiblySent 对待。
    ///
    /// 比的是**下一步动作**，不是理由文案——两者的提示语故意写得不一样，
    /// 用户看到「回读超时」才知道该去查什么。
    #[test]
    fn timed_out_is_treated_exactly_like_possibly_sent() {
        let mut d = Dispatcher::new();
        d.set_tier(Dispatch::Auto);
        let a = d.next(DeliveryReceipt::PossiblySent, 0);
        let b = d.next(DeliveryReceipt::TimedOut, 0);
        let kind = |n: &Next| matches!(n, Next::Halt { .. });
        assert!(kind(&a) && kind(&b), "两者都必须停下");
        assert_eq!(
            DeliveryReceipt::TimedOut.blocks_automatic_retry(),
            DeliveryReceipt::PossiblySent.blocks_automatic_retry()
        );
        assert_eq!(
            DeliveryReceipt::TimedOut.replay_safe(),
            DeliveryReceipt::PossiblySent.replay_safe()
        );
        // 提示语不同：用户看到哪个词，就知道去查哪件事
        assert_ne!(a.explain(), b.explain());
        assert!(b.explain().contains("超时"));
    }

    /// 会话结束一定回落到 Manual，不管之前在哪一档。
    #[test]
    fn ending_a_session_always_falls_back_to_manual() {
        for tier in [Dispatch::Auto, Dispatch::Confirm, Dispatch::Manual] {
            let mut d = Dispatcher::new();
            d.set_tier(tier);
            d.end_session();
            assert_eq!(d.tier(), Dispatch::Manual, "{tier} 档结束会话后应回落");
        }
    }

    /// 默认档就是最低档。没有「默认 Auto」这种东西。
    #[test]
    fn the_default_tier_is_the_lowest_one() {
        assert_eq!(Dispatcher::new().tier(), Dispatch::Manual);
        assert_eq!(Dispatch::default(), Dispatch::Manual);
        assert!(!Dispatcher::new().tier().sends_without_asking());
    }

    /// Auto 档下投递失败不自动降级重试，必须人工确认。
    #[test]
    fn auto_failure_needs_human_confirmation_instead_of_downgrading() {
        let mut d = Dispatcher::new();
        d.set_tier(Dispatch::Auto);
        let n = d.next(DeliveryReceipt::Failed("焦点被拒".into()), 0);
        let Next::NeedsConfirmation { proposed, .. } = &n else {
            panic!("Auto 档失败必须停下问人，实际 {n:?}")
        };
        assert_eq!(*proposed, Dispatch::Confirm);
        assert_eq!(d.tier(), Dispatch::Auto, "问人不等于自己降档——档位没变");

        // 人点了确认才降
        d.confirm_downgrade(*proposed);
        assert_eq!(d.tier(), Dispatch::Confirm);
    }

    /// 人工确认之前，重试一次都不许发生。
    #[test]
    fn no_retry_happens_before_human_confirmation() {
        let mut d = Dispatcher::new();
        d.set_tier(Dispatch::Auto);
        for attempt in 0..MAX_RETRIES {
            let n = d.next(DeliveryReceipt::Failed("x".into()), attempt);
            assert!(
                matches!(n, Next::NeedsConfirmation { .. }),
                "确认前不该出现 {n:?}"
            );
        }
    }

    /// 重试有次数上限，不无限重试。
    #[test]
    fn retries_are_bounded_and_back_off_exponentially() {
        let mut d = Dispatcher::new();
        d.set_tier(Dispatch::Confirm);
        let mut waits = Vec::new();
        for attempt in 0..MAX_RETRIES {
            let Next::Retry { after_ms, .. } = d.next(DeliveryReceipt::Failed("x".into()), attempt)
            else {
                panic!("第 {attempt} 次应可重试")
            };
            waits.push(after_ms);
        }
        // 退避递增且有上限
        assert!(waits.windows(2).all(|w| w[1] >= w[0]), "{waits:?} 未递增");
        assert!(
            waits.iter().all(|&w| w <= MAX_BACKOFF_MS),
            "{waits:?} 超出上限"
        );

        // 再往后就没了
        assert!(backoff_ms(MAX_RETRIES).is_none());
        assert!(matches!(
            d.next(DeliveryReceipt::Failed("x".into()), MAX_RETRIES),
            Next::Halt { .. }
        ));
    }

    /// 升档只能由人触发。程序的状态变化一律不许升档。
    #[test]
    fn the_program_never_raises_its_own_tier() {
        let mut d = Dispatcher::new();
        d.set_tier(Dispatch::Manual);

        // 各种失败、超时、可能已发送，全都不许升档
        let receipts = [
            DeliveryReceipt::Sent,
            DeliveryReceipt::NotSent,
            DeliveryReceipt::Failed("x".into()),
            DeliveryReceipt::TimedOut,
            DeliveryReceipt::PossiblySent,
        ];
        for r in receipts {
            for attempt in 0..MAX_RETRIES + 2 {
                let before = d.tier();
                let n = d.next(r.clone(), attempt);
                assert_eq!(d.tier(), before, "{r:?} 把档位从 {before} 改掉了");
                match n {
                    Next::Retry { .. } | Next::Halt { .. } => {}
                    Next::NeedsConfirmation { .. } => {
                        // 这只是**提议**，档位没动；确认才动
                        assert_eq!(d.tier(), before);
                    }
                }
            }
        }
        assert_eq!(d.tier(), Dispatch::Manual);
    }

    /// 每一步都要有可读的理由——界面上要能显示「为什么不重试了」。
    #[test]
    fn every_step_carries_a_readable_reason() {
        let mut d = Dispatcher::new();
        d.set_tier(Dispatch::Confirm);
        for (receipt, attempt) in [
            (DeliveryReceipt::Failed("x".into()), 0u32),
            (DeliveryReceipt::PossiblySent, 0),
            (DeliveryReceipt::Sent, 0),
            (DeliveryReceipt::TimedOut, 0),
            (DeliveryReceipt::NotSent, 0),
        ] {
            let n = d.next(receipt.clone(), attempt);
            let s = n.explain();
            assert!(!s.is_empty(), "{receipt:?} 没有给出理由");
            assert!(s.chars().count() > 6, "理由太短，没有信息量：{s}");
        }
    }

    #[test]
    fn only_sent_is_not_replay_safe() {
        assert!(DeliveryReceipt::NotSent.replay_safe());
        assert!(DeliveryReceipt::Failed("x".into()).replay_safe());
        assert!(!DeliveryReceipt::Sent.replay_safe());
        assert!(!DeliveryReceipt::PossiblySent.replay_safe());
        assert!(!DeliveryReceipt::TimedOut.replay_safe());
    }
}
