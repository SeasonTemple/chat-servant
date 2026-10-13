# 28: `debug_pane` 与 `telemetry` 两个事件名**从来没有 emit 过**

**What to fix:** 这两个事件名进了契约层的集中清单、前端也确实在 `listen`，
但 Rust 侧**一处 emit 都没有**。要么补上 emit，要么把两处死代码删干净。

**Blocked by:** None

**Status:** ready-for-agent · **2026-10-13 查票 26 的 `debug_pane` 时连根拔出来的**

## 事实（2026-10-13 实测，`rg -n "TELEMETRY|DEBUG_PANE"`）

| 事件名 | Rust 侧 emit | 前端 listen | 结论 |
|---|---|---|---|
| `readout` | ✅ `commands.rs` 观察循环 | ✅ | 活的 |
| `armed` | ✅ `dispatch` 装填后 | ✅ | 活的 |
| `menu:settings` / `menu:relock` | ✅ `main.rs` 托盘 | ✅ | 活的 |
| **`debug_pane`** | ❌ **一处都没有** | `routes/debug/+page.svelte:67` | **死监听** |
| **`telemetry`** | ❌ **一处都没有** | `(panel)/+layout.svelte`、`routes/debug/` | **死监听** |

后果，两条：

1. **调试画布只在挂载时拉一次。** `debug/+page.svelte` 里 `await pull()`
   之后没有定时器、没有刷新按钮，唯一的刷新来源就是这个不会触发的监听。
   想看下一帧只能重开调试窗口。
2. **一条注释是假的。** `(panel)/+layout.svelte` 写着订阅 `telemetry` 是为了
   「让 Rust 侧的 emit 有接收方，才算一条真的链路」——
   **emit 根本不存在**，这条链路从头到尾没有过。

## 为什么单独开票而不在票 26 里顺手做掉

补 emit 是**行为改动**：观察循环每轮要多推一份数据。
而这一轮已经在动 `dispatch` 的安全关键路径（硬约束第 5 条），
两件事混在一起，出问题时分不清是哪一件引起的。

## 两个方向，选一个

- **补 emit**：观察循环里在推 `readout` 的同一处再推 `telemetry`
  （帧计数，payload 已经现成），调试画布改听 `telemetry` → 变成活的。
  代价：每轮多一次序列化，`debug_pane` 的数据量要先量一下再决定要不要每轮推。
- **删干净**：两个常量从 `events::ALL` 里拿掉，两处 `listen` 删掉，
  `(panel)/+layout.svelte` 那条假注释一并删。调试画布明说「挂载时拉一次」。

⚠ 无论选哪个，`contract.rs` 里 `events::ALL` 那条「没有漏在外面的字面量」的
  纪律都要一起更新——它是**清单**，清单里躺着一个不存在的事件名，
  正是它让人以为这条链路是通的。

## 验收项

- [ ] `debug_pane` 或 `telemetry` 至少有一个变成**真链路**，
      或者两个连同前端监听一起删掉——**不许留着一个声明了却永远不触发的事件名**
- [ ] `(panel)/+layout.svelte` 里「让 emit 有接收方」那条注释与实际一致
- [ ] `contract.rs` 的 `event_names_are_stable` 跟着更新
