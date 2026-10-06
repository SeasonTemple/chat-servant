# 08: 投递状态机——把"可能已发送"变成可测的不变量

**What to build:** 纯逻辑的一层：派发策略 + 上一次回执 → 下一步动作。
不碰 Win32。

`DeliveryReceipt` 不是布尔值，是状态机：`NotSent` / `Sent` / `PossiblySent` /
`Failed(reason)` / `TimedOut`。`PossiblySent` 存在的唯一理由是——
**重放一条已经发出的消息无法撤销**。

这条 ticket 可以和 01 并行开工，不依赖任何 Win32 或 OCR 工作。

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] `PossiblySent` 之后**永不**重放，且有测试锁住这条
- [ ] `TimedOut` 按 `PossiblySent` 对待（有测试）
- [ ] 会话结束一定回落到 `Manual`，有测试
- [ ] `Auto` 档下投递失败时**不**自动降级重试成 `Auto`，必须显式重新确认
- [ ] 重试有指数退避和次数上限，不无限重试
- [ ] 三档的升降级只有显式操作能触发，程序自己不能升档
- [ ] 状态机对每一步都产出可读的决策理由（界面上要能显示"为什么不重试了"）