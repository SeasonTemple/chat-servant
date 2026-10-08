//! 「静默期零模型调用」的守卫。
//!
//! ## 为什么这条要单独一个文件
//!
//! 它是 AGENTS.md 的硬约束第 6 条：**没有新的对方消息时不调任何模型**。
//! 而它**极容易在重构时被悄悄破坏**——比如有人在 `PaneWatcher` 与
//! `reason::turn` 之间加一层缓存、或把判断改成「每帧都刷新摘要」，
//! 测试全绿而这条约束已经作废。
//!
//! 所以它不能靠"顺手看一眼"，得有**计数断言**：一次调用都不许有。

use crate::gate::{GateReasoner, counted};
use crate::{ReasonError, Reasoner, TurnContext, turn};
use chatservant_core::frame::Rect;
use chatservant_perceive::{Message, Speaker};

/// 造几段消息，模拟「对方说了话」。
fn messages() -> Vec<Message> {
    vec![
        Message {
            id: "Them:今晚有空吗".into(),
            speaker: Speaker::Them,
            text: "今晚有空吗".into(),
            rect: Rect::new(0, 0, 10, 10),
            evidence: "测试".into(),
        },
        Message {
            id: "Me:我看一下".into(),
            speaker: Speaker::Me,
            text: "我看一下".into(),
            rect: Rect::new(0, 20, 10, 30),
            evidence: "测试".into(),
        },
    ]
}

/// 静默期：**一次模型调用都不许发生。**
///
/// 这条是本文件的核心，也是 AGENTS.md 硬约束第 6 条。
///
/// 它**抓到过一个真缺陷**：早期 `turn()` 没有这道闸门，空对话照样
/// 跑满三段——静默期反而是唯一在烧调用的场景。把它放在 seam 里而不是
/// 调用方，才谈得上「任何调用方都无法违反」。
#[test]
fn quiet_period_makes_zero_model_calls() {
    let mut r = counted(GateReasoner::new());
    let ctx = TurnContext::text(vec![], "小号".into());
    let err = turn(&mut r, &ctx).expect_err("静默期不该产出候选");
    assert_eq!(err, ReasonError::NothingToDo);
    assert_eq!(
        r.calls(),
        0,
        "静默期不该有任何模型调用，实际调了 {} 次",
        r.calls()
    );
}

/// 有新消息时才允许调用，且**每段各一次**（不多不少）。
///
/// 「不多」同样重要：一次刷新调三遍就是钱。
#[test]
fn a_new_message_makes_exactly_three_calls() {
    let mut r = counted(GateReasoner::new());
    let ctx = TurnContext::text(
        messages().iter().map(|m| m.text.clone()).collect(),
        "小号".into(),
    );
    let t = turn(&mut r, &ctx).expect("推理成功");
    assert_eq!(r.calls(), 3, "判断+起草+排序 = 3 次，实际 {} 次", r.calls());
    assert!(t.recommended().is_some(), "该有推荐");
}

/// 判断失败时降级成盲拟，**调用次数不增加**——仍然是三次。
///
/// 降级不是「多试几次」。多试几次会让限流更严重。
#[test]
fn degradation_does_not_retry_more() {
    let mut r = counted(GateReasoner::judge_fails());
    let ctx = TurnContext::text(vec!["在吗".into()], "小号".into());
    let t = turn(&mut r, &ctx).expect("判断失败不该让整轮失败");
    assert!(!t.judged);
    assert_eq!(r.calls(), 3, "降级路径不该额外调用，实际 {} 次", r.calls());
}

/// 同一轮对话重复跑，**每次都重新调**。
///
/// 这里看着像反直觉：缓存听起来更省。但**缓存是错的**——
/// 用户在别处改了窗口大小、对方又发了消息，缓存的判断就是过期的。
/// 真要省，靠 [`chatservant_perceive::PaneWatcher`] 在**没有新消息时**
/// 根本不进到这里，而不是在推理层缓存。
#[test]
fn each_turn_calls_the_models_again_on_purpose() {
    let ctx = TurnContext::text(vec!["在吗".into()], "小号".into());
    let mut r = counted(GateReasoner::new());
    for _ in 0..3 {
        let _ = turn(&mut r, &ctx).expect("推理成功");
    }
    assert_eq!(r.calls(), 9, "三轮 × 三段 = 9 次");
}

/// 传输层能被替换——这是「测试不联网」的结构保证。
///
/// 断言的是**类型**：能把 `GateReasoner` 装进 `turn(&mut dyn Reasoner)`，
/// 就说明 seam 真的只依赖 trait，不依赖任何具体 HTTP 客户端。
#[test]
fn the_seam_only_depends_on_the_trait_not_on_any_client() {
    let mut r: Box<dyn Reasoner> = Box::new(GateReasoner::new());
    let ctx = TurnContext::text(vec!["在吗".into()], "小号".into());
    // `turn` 收 `&mut R` 且 `R: ?Sized`，所以直接传 `&mut dyn Reasoner`
    // （`r.as_mut()`）——**不用**传 `&mut r`。`Box<dyn T>` 本身不实现 `T`，
    // 传 `&mut Box<dyn T>` 会因为多了一层引用而找不到实现。
    let t = turn(r.as_mut(), &ctx).expect("推理成功");
    assert!(!t.draft.candidates.is_empty());
}
