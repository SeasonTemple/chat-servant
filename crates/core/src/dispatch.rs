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
//!
//! ## 档位集合为什么是这两档（2026-10-15 改）
//!
//! **历史走过的三档**是 `Manual` → `Confirm` → `Auto`（填入即发、不打断）。
//! 2026-10-13 曾整档取消 `Auto`，理由是它与 `Confirm` 的差别只是
//! 「少一次点击」，而那一次点击正是硬约束第 5 条的闸门。
//!
//! 2026-10-15 **用户重新拍板：砍掉的是 `Confirm`，`Auto` 回来。**
//! 现行两档：
//!
//! - [`Dispatch::Manual`]——只填入输入框，停在光标处。**默认档**。
//! - [`Dispatch::Auto`]——**点一下就填入并发送，没有第二次确认**。
//!
//! **这拆掉了「发送前必须再问一次」那道闸**，是安全模型的实质变更，
//! 不是排版改动。守卫测试相应地不再是「没有哪一档会不问就发」
//! （`no_tier_sends_without_asking` 已随之删除），而是
//! [`Dispatch`] 只允许这两档、且升档只能由人触发。
//!
//! **`Auto` 档只保证「回落到 `Manual`」，不保证「发得对」。**
//! 概率是模型的自评，不是校准过的真实性；降级候选（`blind`）的概率更不可信。
//! 「挑最高置信度那条自动发」这条策略的局限见
//! `ui/src-tauri/src/watch.rs` 里自动发送那段注释。

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
    /// **文本已经进输入框了，但没有按回车——确定没发出去。**
    ///
    /// ## 为什么必须有它（2026-10-15）
    ///
    /// 在这个变体出现之前，`fill_input_box` 的成功路径返回
    /// [`Self::PossiblySent`]。那是**撒谎**：它从头到尾没按过回车，
    /// 而 `PossiblySent` 的意思恰恰是「按了回车、回读没确认、
    /// 不知道发没发」。真机实测的结果是面板对「仅填入」档打出一句
    /// 假警报「可能已经发出去了——请自己看一眼聊天窗口」，
    /// 而对话流里根本没有新消息。
    ///
    /// 同一声称被用在两种相反的事实上，比没有回执更糟：
    /// 人要么被吓去检查聊天窗口，要么学不会读这个提示。
    ///
    /// ## 和 `Sent` 的区别不是程度，是方向
    ///
    /// `Sent` 说「它已经离开这台设备」，`Filled` 说「它就停在你面前」。
    /// 后者是**本枚举里唯一关于「装填完成」的正面回执**，
    /// 而 `fill_and_send` 需要它来当「可以按回车了」的内部信号
    /// （在它出现之前，`fill_and_send` 是反过来拿 `PossiblySent` 当这个信号，
    /// 于是同一个错误同时污染了「仅填入」的提示和 `Auto` 档的判据）。
    Filled,
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
    ///
    /// [`Self::Filled`] 也**不在**安全这一侧，尽管它同样「没发出去」。
    /// 判据不是「发没发」，是「**重来一遍会不会变成两条**」：
    /// 输入框里已经有那段字了，再填一次就是追加成两份——
    /// 那正是 2026-10-09 `Confirm` 两段式发出去两条拼接消息的机制。
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
            Self::Filled => "已填进输入框并停在光标处——没有发出去，要发你自己按回车",
            Self::PossiblySent => "可能已经发出去了——不再自动重放，请自己看一眼聊天窗口",
            Self::Failed(_) => "明确失败，可以按退避重试",
            Self::TimedOut => "回读超时，按「可能已发送」对待——不再自动重放",
        }
    }
}

/// 发送档位。**默认且终态最低是 `Manual`。**
///
/// **只有两档**（2026-10-15 用户拍板，砍掉的是 `Confirm`）：
/// `Manual` 只填入、`Auto` 点一下就填入并发送。见模块文档。
///
/// `Auto` **会在不二次确认的情况下发出去**。这是显式选择的结果，
/// 守卫是 [`Dispatch::ALL`] 恰好两项与 `the_tier_set_is_exactly_two_and_named`。
///
/// 变体顺序即档位高低：靠前的更低。`Default` 落在 `Manual` 上，
/// `#[derive(Default)]` 只能在末位变体上生效（现在是 `Auto`），
/// 所以必须手写。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dispatch {
    /// 只填入输入框，停在光标处。默认值，会话结束回落到此档。
    Manual,
    /// 填入并发送。**没有第二次确认**——人点一次，就发出去了。
    Auto,
}

impl Default for Dispatch {
    /// `Manual`——**最低档**。派生实现会默认到末位（`Auto`），
    /// 而这里宁可手写也不要那个默认值：万一将来再加一档，
    /// 派生实现会**静默地把默认档升上去**，而默认档必须是最低的那档。
    fn default() -> Self {
        Self::Manual
    }
}

impl Dispatch {
    /// 全部档位。**测试与界面用它枚举，不用手写 `match`。**
    ///
    /// 为什么留着这个而不是只删枚举变体：档位集合是**硬约束**的一部分，
    /// 「一共有几档、分别是谁」必须有单一出处，前端、托盘、
    /// IPC 校验都从这里取，而不是各自写一份 `match`。
    ///
    /// 加一档的话**这个数组必须同步改**——
    /// `the_tier_set_is_exactly_two_and_named` 会在你忘了改的时候红。
    pub const ALL: [Dispatch; 2] = [Dispatch::Manual, Dispatch::Auto];
}

impl fmt::Display for Dispatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Manual => "Manual（仅填入）",
            Self::Auto => "Auto（自动发送）",
        })
    }
}

/// 指数退避的上限与次数。
pub const MAX_RETRIES: u32 = 3;
/// 第一次重试等多久（毫秒）。之后每翻一倍。
pub const BASE_BACKOFF_MS: u64 = 800;
/// 退避的封顶。再翻下去用户等不起，等不起就会以为程序死了。
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
///
/// **这里没有「发送」这一项**，这是有意为之：`Next` 只回答
/// 「投递失败或成功之后，程序还能自己做什么」，**发不发由人**。
/// 想加一个「直接发出去」的变体之前，先去读根 `AGENTS.md` 硬约束第 5 条。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// 到点了，可以再投一次。
    Retry {
        /// 距现在至少要等这么久。
        after_ms: u64,
        /// 这是第几次重试，从 0 起。用来把界面上「又失败一次」说清楚。
        attempt: u32,
    },
    /// 停下来等**人**。程序不许自己继续。
    Halt {
        /// 为什么停下来。**必须是人能读懂的一句话**，不是错误码。
        reason: String,
    },
    // `NeedsConfirmation` 于 2026-10-13 删除。
    //
    // 它表达的是「程序提议降档，等人点头再降」，而两档之下**没有降档**可做
    // （最低就是 `Manual`）。留着就是一个永远构造不出来的死变体，
    // 而死变体比缺功能更糟——它让人以为这条路径是通的。
    // 当时的判断逻辑原文见本文件顶部的模块文档。
}
impl Next {
    /// 界面上要显示的完整理由。**每一步都必须有话可说。**
    pub fn explain(&self) -> String {
        match self {
            Self::Retry { after_ms, attempt } => {
                format!("第 {attempt} 次重试，等 {after_ms} ms")
            }
            Self::Halt { reason } => format!("停下等人工：{reason}"),
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

    /// 当前档位。**只读**——升档的唯一入口是 [`Dispatcher::set_tier`]。
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
    ///
    /// **这个函数与档位无关。** 判定全部只看 [`DeliveryReceipt`]：
    /// 能不能重试、该不该停，取决于「发出去没有」，不取决于「人在哪一档」。
    /// ⇒ **升到 `Auto` 档并不会让失败变得可以静默重试**。
    pub fn next(&self, receipt: DeliveryReceipt, attempt: u32) -> Next {
        // 可能已经发出去了 —— **任何档位**都不许自动重放。
        //
        // 这条检查必须在最前面：它是「重发一条已经发出去的消息」的
        // 最后一道拦，而发出去的东西撤不回来。
        //
        // [`DeliveryReceipt::Filled`] 也走这一支，但理由不同：
        // 它确定**没发**，可输入框里已经有那段字了，重放就是追加成两份。
        // 停下等人在这里是对的——该干的活干完了，剩下的是人读、人改、人按回车。
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

        // 走到这里说明**确定没发出去**（`replay_safe()` 为真），
        // 所以重试是安全的。发不发由档位与人决定，不由这里决定。
        Next::Retry { after_ms, attempt }
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
        for tier in Dispatch::ALL {
            let mut d = d;
            d.set_tier(tier);
            let n = d.next(DeliveryReceipt::PossiblySent, 0);
            assert!(
                matches!(n, Next::Halt { .. }),
                "{tier} 档下 PossiblySent 必须停下，实际 {n:?}"
            );
        }
    }

    /// 「装填完成」既不会被重放，也**不会谎称自己可能已经发出去**。
    ///
    /// 后半句守的是 2026-10-15 的一个实机假警报：
    /// 「仅填入」档点候选后，面板显示「可能已经发出去了——
    /// 请自己看一眼聊天窗口」，而对话流里根本没有新消息。
    /// 根因是 `fill_input_box` 没按回车却返回了 `PossiblySent`。
    ///
    /// 修法是给回执加上 [`DeliveryReceipt::Filled`]，并让
    /// **`PossiblySent` 这个说法只出现在真的按过回车的路径上**。
    /// 这条测试把那个说法的可出现范围钉死：将来谁再让「只填入」的路径
    /// 借用它，会在这里红，而不是等到真机骗了人才发现。
    #[test]
    fn filled_is_never_replayed_and_never_claims_it_might_have_been_sent() {
        let mut d = Dispatcher::new();
        for tier in Dispatch::ALL {
            d.set_tier(tier);
            let n = d.next(DeliveryReceipt::Filled, 0);

            assert!(
                matches!(n, Next::Halt { .. }),
                "{tier} 档下 Filled 必须是终态（输入框里已经有字了），实际 {n:?}"
            );

            // 停下是对的，但**理由必须说真话**：说「可能已经发出去了」
            // 就是把一条确定没发生的事报成不确定，人会白跑一趟去查。
            let said = n.explain();
            assert!(
                !said.contains("可能已经发出去了"),
                "{tier} 档的 Filled 回执不得声称可能已发送，实际：{said}"
            );
            assert!(
                said.contains("没有发出去"),
                "{tier} 档的 Filled 回执要明说「没发」，实际：{said}"
            );
        }

        // 另一半：`PossiblySent` 那句警告必须**只**留给真的按过回车的路径。
        assert!(
            DeliveryReceipt::PossiblySent
                .explain()
                .contains("可能已经发出去了"),
            "PossiblySent 才是那句警告的正主；`Filled` 顶替它之后它不能被改写掉"
        );
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
        for tier in Dispatch::ALL {
            let mut d = Dispatcher::new();
            d.set_tier(tier);
            d.end_session();
            assert_eq!(d.tier(), Dispatch::Manual, "{tier} 档结束会话后应回落");
        }
    }

    /// 默认档就是最低档。没有「默认高档」这种东西。
    #[test]
    fn the_default_tier_is_the_lowest_one() {
        assert_eq!(Dispatcher::new().tier(), Dispatch::Manual);
        assert_eq!(Dispatch::default(), Dispatch::Manual);
    }

    /// **档位集合是硬约束，不是实现细节。**
    ///
    /// 2026-10-15 用户拍板砍掉 `Confirm`、恢复 `Auto`（模块文档有完整来龙去脉）。
    /// 这条测试是「档位没有悄悄长回来」的唯一守卫。
    ///
    /// 为什么单列一条：加一档是一个**看起来很小**的改动——
    /// 枚举加一个变体、`Display` 加一支、前端 `TIERS` 加一项，
    /// 每一处都编译得过、每处都合理。而「不该有第三档」这件事
    /// **没有任何编译器会提醒**，它散在规范里、散在 `AGENTS.md` 里。
    #[test]
    fn the_tier_set_is_exactly_two_and_named() {
        let ids: Vec<String> = Dispatch::ALL.iter().map(|d| d.to_string()).collect();
        assert_eq!(
            ids,
            vec![
                "Manual（仅填入）".to_string(),
                "Auto（自动发送）".to_string()
            ],
            "档位集合变了。2026-10-15 的决定是「砍 Confirm、留 Auto」；\
             要加回来先去改 docs/设计规范.md §10 第 3b 条与根 AGENTS.md 硬约束第 5 条"
        );
        assert_eq!(Dispatch::ALL.len(), 2);
        // 最低档必须排在最前 —— `Ord` 派生的比较方向依赖这一点。
        assert!(Dispatch::Manual < Dispatch::Auto);
    }

    /// **`Auto` 档会在不二次确认的情况下发出去——这条是已知的、被接受的。**
    ///
    /// 2026-10-15 之前这里有一条 `no_tier_sends_without_asking`，
    /// 它断言「没有任何一档会不问就发出去」。**那条随 `Confirm` 一起删掉了**，
    /// 因为砍掉 `Confirm` 之后这个边界**不再成立**，`Auto` 档正是
    /// 「不问就发」的那一档。
    ///
    /// 删它不是因为它测错了，而是因为它守的东西被**显式决定**改掉了。
    /// 留着一条恒假的测试比删掉更糟：它会让人以为闸还在。
    ///
    /// 现在替代它守安全模型的是这三条：
    /// · [`the_tier_set_is_exactly_two_and_named`]——档位集合恰好两档；
    /// · [`the_default_tier_is_the_lowest_one`]——默认档是最不危险的那档；
    /// · [`ending_a_session_always_falls_back_to_manual`]——会话结束自动回落。
    ///
    /// **它们合起来仍不保证「发得对」**，那是 `Auto` 档的定义本身。
    /// 「挑最高置信度自动发」的局限见 `watch.rs` 里自动发送那段注释。
    #[test]
    fn auto_is_the_one_tier_that_sends_without_asking() {
        // 这条测试存在的意义是**把已经改掉的事实写在代码里**，
        // 免得下一个读代码的人以为还有一道「必须确认」的闸。
        assert!(
            matches!(Dispatch::ALL[1], Dispatch::Auto),
            "最高档必须是 Auto——它是唯一会不问就发的那档"
        );
        // 而最低档必须只填入：默认状态下程序不会替人说话。
        assert!(
            matches!(Dispatch::ALL[0], Dispatch::Manual),
            "最低档必须是 Manual"
        );
    }

    /// 投递失败可以退避重试——**前提是确定没发出去**。
    ///
    /// 这条取代了已删除的
    /// `auto_failure_needs_human_confirmation_instead_of_downgrading`。
    /// 那条守的是「`Auto` 档失败不许自己降档重试又发一次」，
    /// 而 `Auto` 取消后**没有降档可做**，这条边界随之消失。
    ///
    /// 现在的边界是另一条，而且更要紧：确定没发出去 ⇒ 重试安全；
    /// 不确定 ⇒ 一律停下问人。判据是 [`DeliveryReceipt::replay_safe`]，
    /// **与档位无关**。
    #[test]
    fn retry_is_allowed_exactly_when_nothing_was_sent() {
        for tier in Dispatch::ALL {
            let mut d = Dispatcher::new();
            d.set_tier(tier);

            // 确定没发出去的 ⇒ 按退避重试
            for receipt in [
                DeliveryReceipt::NotSent,
                DeliveryReceipt::Failed("x".into()),
            ] {
                let n = d.next(receipt.clone(), 0);
                assert!(
                    matches!(n, Next::Retry { after_ms, attempt: 0 } if after_ms == BASE_BACKOFF_MS),
                    "{tier} 档 / {receipt:?} 应可重试，实际 {n:?}"
                );
            }

            // 不确定发没发出去的 ⇒ 一次都不许重试，**任何档位都一样**
            for receipt in [
                DeliveryReceipt::Sent,
                DeliveryReceipt::TimedOut,
                DeliveryReceipt::PossiblySent,
            ] {
                let n = d.next(receipt.clone(), 0);
                assert!(
                    matches!(n, Next::Halt { .. }),
                    "{tier} 档 / {receipt:?} 必须停下问人，实际 {n:?}"
                );
            }
        }
    }

    /// 人工确认之前，重试一次都不许发生。
    ///
    /// 「停下等人工」在两档之下的具体形态是 [`Next::Halt`]。
    #[test]
    fn no_retry_happens_after_a_halt() {
        let mut d = Dispatcher::new();
        d.set_tier(Dispatch::Auto);
        // 停过一次之后，同一个回执在任意重试次数下都还是停——
        // 「停下」不是「第 N 次可以再试」。
        for attempt in 0..MAX_RETRIES + 2 {
            let n = d.next(DeliveryReceipt::PossiblySent, attempt);
            assert!(
                matches!(n, Next::Halt { .. }),
                "第 {attempt} 次不该出现 {n:?}"
            );
        }
    }

    /// 重试有次数上限，不无限重试。
    #[test]
    fn retries_are_bounded_and_back_off_exponentially() {
        let mut d = Dispatcher::new();
        d.set_tier(Dispatch::Auto);
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
    ///
    /// 顺带覆盖**降档**：`next()` 是 `&self`，它**没有任何办法**改档位——
    /// 这一点由签名保证，而这条测试把「从两档各出发」都走一遍，
    /// 防止将来有人给 `next` 加个 `&mut self` 就悄悄改了档。
    #[test]
    fn the_program_never_raises_its_own_tier() {
        let receipts = [
            DeliveryReceipt::Sent,
            DeliveryReceipt::NotSent,
            DeliveryReceipt::Failed("x".into()),
            DeliveryReceipt::TimedOut,
            DeliveryReceipt::PossiblySent,
        ];
        for start in Dispatch::ALL {
            let mut d = Dispatcher::new();
            d.set_tier(start);
            for r in &receipts {
                for attempt in 0..MAX_RETRIES + 2 {
                    let before = d.tier();
                    let n = d.next(r.clone(), attempt);
                    assert_eq!(
                        d.tier(),
                        before,
                        "{r:?} / 第 {attempt} 次把档位从 {before} 改掉了（{n:?}）"
                    );
                }
            }
            assert_eq!(d.tier(), start);
        }
    }

    /// 每一步都要有可读的理由——界面上要能显示「为什么不重试了」。
    #[test]
    fn every_step_carries_a_readable_reason() {
        let mut d = Dispatcher::new();
        d.set_tier(Dispatch::Auto);
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
