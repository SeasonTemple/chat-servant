# 26: `contract.rs` 从来没进过编译 —— 契约层有三条定义是死的

**What to build:** 把 `ui/src-tauri/src/contract.rs` 里仍然「只有定义、没有消费者」的
三项接到生产路径上，或在它们真正失效时删掉。

**Blocked by:** None（但接线 `Action` 会碰 dispatch 的安全关键路径，
建议与 **20** 一起排期，别单独动）

**Status:** done（2026-10-13。四项全部处理，见下「做了什么」）

## 事实

`main.rs` 原本只声明了：

```rust
mod commands;
mod debug;
mod secret;
mod watch;
```

**没有 `mod contract;`。** 所以 `contract.rs` 从落盘到今天，
**一次都没被编译过**——它自己的那些测试也一次都没运行过。

这解释了票 21 的一个细节：票据写「`contract.rs` 的 `Phase` 枚举只在它自己的
测试里被构造过」。实际情况更糟：**连那些测试都没跑过**。

`git log` 显示它是 `d67487b fix(构建): 让云端会话那份没编译过的 Rust 编译通过
并进得了门禁` 引进来的——修的是「能编译」，没有人核对过它**有没有被引用**。

## 接进编译之后，一次性暴露了四件事

### 1. `Phase` 根本没被用（票 21 的真根因）

票 21 记的是「`Readout` 没有 `phase` 字段」。补了 `mod contract;` 之后确认：
**`Phase` 在生产代码里一处都没出现过**。所以「加个字段」这条修法方向没错，
但根因不止于此——契约层整个是死的。

### 2. `DraftCandidate` 缺 `PartialEq`

`Phase` 派生了 `PartialEq`，而 `Drafted` 变体带着 `DraftCandidate`。
于是 `mod contract;` 一加上就编译不过（E0369）。
**这个缺失潜伏了整个文件的存在期。**

### 3. 事件名散在五处，且有一处从没进过清单

`contract.rs` 自己有一份 `events`（4 个常量），`main.rs` 里**又有一份**
（3 个常量）——两份都是「事件名的集中处」，等于集中了两次。

而生产代码真正在用的是：

| 位置 | 写法 | 在清单里？ |
|---|---|---|
| `commands.rs` 装填后 | `app.emit("armed", …)` | 裸字面量 |
| `commands.rs` 打开设置 | `app.emit("menu:settings", ())` | **根本没有** |
| `commands.rs` 观察循环 | `app2.emit("readout", &out)` | 裸字面量 |
| `main.rs` 托盘重锁 | `events::MENU_RELOCK` | ✅ |
| `debug_pane` | —— | **Rust 侧一处 emit 都没有** |

⚠ 最后一行值得单独留意：前端 `routes/debug/+page.svelte:67` 有
`listen('debug_pane', …)`，而 Rust 侧我没搜到任何 emit。
**那可能是个死监听**（调试画布只靠轮询 `debug_pane` 命令刷新），
也可能是我没搜到——**这一条尚未核实**，别当结论。

`contract.rs` 的 `no_event_literal_outside_this_file` 这次**第一次运行**
就把上面三个字面量全抓出来了。测试是对的，纪律从来没被执行过。

### 4. `Action` / `Action::parse` / `Phase::status_hint` 是死代码

clippy 的 `dead_code` 报的。前两项的接线要动 `commands::dispatch` 的动作判定
（**硬约束第 5 条那条路径**）；`status_hint` 的消费者在
`(panel)/+page.svelte` 的 S3 那一格，那句「正在成形…／通常几秒」是写死的。

## 做了什么（2026-10-13）

### 1 + 4. `Action` 接进 `dispatch`

`dispatch` 的第一件事改成 `Action::parse(&action)`，解析不出来立刻回
`Failed` + 那句能直接给人看的解释，进不了业务逻辑。

⚠ 旧代码是 `action == "confirm"` 的字符串比较：**拼错的动作会静悄悄落进
最底下那条「只填入」的分支**——界面上写着一个看不懂的动作，程序却替你按了
最温和的那一下。不是「发出去」的风险，是**行为与界面说的不一致**。

两条 `#[allow(dead_code)]` 都删了（枚举与 `parse`），clippy 绿**不是**靠它们。

另加一条纯函数 `sends_for_real(tier, act, armed)`，把「什么时候能按下回车」
从分支顺序里拿出来，于是能**穷举全部 8 种组合**：
`only_armed_confirm_clicks_send` 断言能发出去的组合恰好一个。
`dispatch` 的二次确认分支上有一道 `debug_assert` 调它——
哪天有人改了上面的分支条件，当场炸，而不是等发错一条真消息才发现。

### 2. `Phase::status_hint` 被界面消费

`Readout` 多了 `status_hint` 字段，由 `snapshot()` 从 `phase.status_hint()` 取。
新面板 `panel/+page.svelte` 的起草态渲染它，**不再写死「正在写……」**，
并保留一句短句只作夹具缺字段时的退路。

📐 证据 `target/probe/panel-drafting-zoom.png`：显示的是 Rust 那句
「正在起草，通常几秒」。

⚠ 字段名是 **snake_case**：`Readout` 结构体上没有 `rename_all`，
  而 `Action` 有 `camelCase`。两处都是刻意的，改一边要看另一边。

⚠ **旧面板 `(panel)/+page.svelte` 那一格还留着硬编码的「正在成形…」/
  「通常几秒」**——它已被冻结，等票 25 选 C 时随旧面板一起删。
  在那之前仓库里确实有两份文案，这一点**如实记着**，不装作已经只有一份。

### 3. `debug_pane` 的结论：**是死监听，而且不止它一个**

`rg -n "TELEMETRY|DEBUG_PANE"` 全仓的结果（2026-10-13 实测）：

| 事件名 | Rust 侧 emit | 前端 listen | 结论 |
|---|---|---|---|
| `readout` | ✅ `commands.rs` 循环里 | ✅ | 活的 |
| `armed` | ✅ `dispatch` 里 | ✅ | 活的 |
| `menu:settings` / `menu:relock` | ✅ | ✅ | 活的 |
| **`debug_pane`** | ❌ **一处都没有** | `routes/debug/+page.svelte:67` | **死监听** |
| **`telemetry`** | ❌ **一处都没有** | `(panel)/+layout.svelte` 与 `routes/debug/` | **死监听** |

所以调试画布**只在挂载时拉一次**（`debug/+page.svelte` 没有定时器、没有刷新按钮），
`telemetry` 那条「为了让 emit 有接收方」的注释是**假的**。

**本轮不动它们**：补 emit 是行为改动（每轮多推一帧数据），该单独一票——
已开 **28**。这里只把结论钉死，不靠「看起来像」判。

### 4. 孤儿模块守门测试

`no_orphan_module_file_in_src`：遍历 `src/*.rs`，每个文件名必须在别的文件里
以 `mod <名>;` 出现过。例外是 crate 的两个根 `main.rs` 与 `lib.rs`
（Tauri 2 要求有 `[lib]`，两个都由 Cargo 声明）。

Rust 自己不报这件事——`contract.rs` 躺在 `src/` 里一次都没被编译过，
而 `clippy` 与 `cargo test` 全绿。加这条之后，下一个同类的文件落盘即报红。

## 验收项

- [x] `Action::parse` 在 `dispatch` 的入口处真的被调用，
      `action: "删库"` 进不了业务逻辑（有测试）
- [x] `Phase::status_hint` 被界面消费，
      S3 的文案只有一处来源（Rust 或前端，二选一，另一处删掉）
      —— ⚠ **新面板已达成；旧面板那份等票 25 选 C 时随文件删除**
- [x] `debug_pane` 到底是死监听还是我漏搜了——**给出结论**，
      不要靠「看起来像」判（结论：死监听，且 `telemetry` 同样是，另开票 28）
- [x] 加一条测试守住「孤儿模块」这类问题：列出的模块文件若没被 `mod` 声明，
      就红（Rust 本身不会报，只有 `dead_code` 会间接暴露）
- [x] `cargo clippy --workspace --all-targets -- -D warnings` 绿，
      且**不是靠给整块加 `#[allow(dead_code)]` 变绿的**
