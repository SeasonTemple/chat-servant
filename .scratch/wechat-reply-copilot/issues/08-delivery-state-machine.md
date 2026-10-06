# 08: 投递状态机——把"可能已发送"变成可测的不变量

**What to build:** 纯逻辑的一层：派发策略 + 上一次回执 → 下一步动作。
不碰 Win32。

`DeliveryReceipt` 不是布尔值，是状态机：`NotSent` / `Sent` / `PossiblySent` /
`Failed(reason)` / `TimedOut`。`PossiblySent` 存在的唯一理由是——
**重放一条已经发出的消息无法撤销**。

这条 ticket 可以和 01 并行开工，不依赖任何 Win32 或 OCR 工作。

**Blocked by:** None (can start immediately)

**Status:** done

- [x] `PossiblySent` 之后**永不**重放，且有测试锁住这条
- [x] `TimedOut` 按 `PossiblySent` 对待（有测试）
- [x] 会话结束一定回落到 `Manual`，有测试
- [x] `Auto` 档下投递失败时**不**自动降级重试成 `Auto`，必须显式重新确认
- [x] 重试有指数退避和次数上限，不无限重试
- [x] 三档的升降级只有显式操作能触发，程序自己不能升档
- [x] 状态机对每一步都产出可读的决策理由（界面上要能显示"为什么不重试了"）

## 落在哪

`crates/core/src/dispatch.rs`。放在 core 是因为它**纯逻辑、不碰 Win32**，
所以每条不变量都能在普通单元测试里锁死，不必开窗口。

## 三个关键决定

**`PossiblySent` 的检查放在 `next()` 的最前面**，Auto 档也不例外。
理由：Auto 是「不打断」的承诺，但「可能已经发出去」时再发一次会发两条，
而**发出去的东西无法撤销**。宁可停下来。

**Auto 档失败返回 `NeedsConfirmation` 而不是 `Retry`。**
`NeedsConfirmation` 只是一个**提议**——`Dispatcher::tier()` 在人点确认之前
**一个字都不变**。理由：Auto 的兑现方式是失败后停下来问人，
而不是自己改主意又发一次。用户以为在 Auto，程序偷偷降档重发，比直接失败更糟。

**`TimedOut` 与 `PossiblySent` 同等对待，但提示语不同。**
「可能已经发出去了」与「回读超时」对用户的下一步动作是一样的（去看一眼），
但**该去查的事情不一样**（前者怀疑已送达，后者怀疑回读失败）。
所以比对的是 `blocks_automatic_retry()` / `replay_safe()`，
不是 `explain()` 的字符串——测试里专门断言了两者提示语**不相等**。