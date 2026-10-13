# 26: `contract.rs` 从来没进过编译 —— 契约层有三条定义是死的

**What to build:** 把 `ui/src-tauri/src/contract.rs` 里仍然「只有定义、没有消费者」的
三项接到生产路径上，或在它们真正失效时删掉。

**Blocked by:** None（但接线 `Action` 会碰 dispatch 的安全关键路径，
建议与 **20** 一起排期，别单独动）

**Status:** ready-for-agent · **2026-10-13 做票 21 时连根拔出来的**

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

## 验收项

- [ ] `Action::parse` 在 `dispatch` 的入口处真的被调用，
      `action: "删库"` 进不了业务逻辑（有测试）
- [ ] `Phase::status_hint` 被界面消费，
      S3 的文案只有一处来源（Rust 或前端，二选一，另一处删掉）
- [ ] `debug_pane` 到底是死监听还是我漏搜了——**给出结论**，
      不要靠「看起来像」判
- [ ] 加一条测试守住「孤儿模块」这类问题：列出的模块文件若没被 `mod` 声明，
      就红（Rust 本身不会报，只有 `dead_code` 会间接暴露）
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 绿，
      且**不是靠给整块加 `#[allow(dead_code)]` 变绿的**
