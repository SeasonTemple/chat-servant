# 20: 发送档位从三档砍到两档（Auto 整档取消）

**What to build:** 面板与 Rust 只剩两档 —— `Manual`（只填入）与 `Confirm`（确认后发）。
`Auto`（填入即发、不打断）**整档取消**。

**Blocked by:** None（可以立刻做）

**Status:** ready-for-human · **决定日期 2026-10-13**
· ✅ **2026-10-13 实现完毕**（代码与门禁）
· ⚠ 标 `ready-for-human` 而不是 `ready-for-agent`：剩下的两项都要在
  **真实微信会话**上点一遍，agent 做不了（既要动手，又涉及别人的聊天窗口）

## 这是什么性质的改动

⚠ **不是排版改动，是安全模型变更。** 别当成「删个枚举变体顺手的事」做掉。

2026-10-12 之前这个产品是三档，并在根 `AGENTS.md` 硬约束第 5 条里写死：

```text
Manual（只填入）→ Confirm（二次确认后发送）→ Auto
```

砍成两档会把那条约束整个改写，所以**必须同一个提交里把约束改掉**，
否则仓库里会出现「规范说两档、代码说三档」这种反向骗人的状态。

## 取消 Auto 会一并删掉一条真实行为

`crates/core/src/dispatch.rs` 的 `Dispatcher::next` 里有一条分支：

```rust
if self.tier == Dispatch::Auto {
    return Next::NeedsConfirmation { proposed: Dispatch::Confirm, … }
}
```

它的语义是「**Auto 是『不打断』的承诺，失败的兑现方式是停下来问人，
而不是自己降档重试又发一次**」。两档之下这个场景会消失
（Confirm 失败本来就要问人）。**这条行为差异必须写进 ADR 或注释，
否则下一个读代码的人会以为 Auto 只是一档『更快的 Confirm』。**

## 需要一起改的地方（逐条核过）

- [x] `crates/core/src/dispatch.rs` 的 `enum Dispatch` 去掉 `Auto`；
      `Default` 手写落在 `Manual` 这条**保持不变**（注释重写：末位现在是 `Confirm`，
      派生默认会落到它）
- [x] 同文件 `Display` 的三支匹配收成两支
- [x] `sends_without_asking()` —— **删掉**。它恒为 `false`，留着就是骗人。
      替代物不是另一个恒定的函数，而是测试：
      `the_tier_set_is_exactly_two_and_named`（档位集合恰好两档且名字锁死）
      与 `no_tier_sends_without_asking`（遍历两档 × 五种回执 × 五次重试，
      断言 `next()` 只返回 `Retry`/`Halt`，没有任何「发出去」的变体）
- [x] `dispatch.rs` 里引用 `Auto` 的测试逐条处置 ——
      **边界在两档下还成不成立，逐条确认过**：
      - `the_program_never_raises_its_own_tier` **成不立** → 改成从两档各出发，
        并删掉 `NeedsConfirmation` 分支（`next` 是 `&self`，它没能力改档）
      - `auto_failure_needs_human_confirmation_instead_of_downgrading`
        **不成立**（没有降档可做）→ 整条删除，原文记进 `dispatch.rs` 模块文档
      - `no_retry_happens_before_human_confirmation` → 改成
        `no_retry_happens_after_a_halt`（「停下」不是「第 N 次可以再试」）
      - `ending_a_session_always_falls_back_to_manual` /
        `possibly_sent_is_never_replayed` → 枚举数组换成 `Dispatch::ALL`
      - **新增** `retry_is_allowed_exactly_when_nothing_was_sent`：
        现在的边界是「确定没发出去才可重试」，与档位无关
- [x] `Next::NeedsConfirmation` 与 `Dispatcher::confirm_downgrade` **一并删除** ——
      两档之下它们永远构造不出来，是死变体。死变体比缺功能更糟
- [x] `ui/src/routes/(panel)/+page.svelte`：`TIERS`、`TIER_NAME`、
      三条 `data-tier='…'` 的 CSS 选择器
- [x] `ui/static/demo/auto.json` —— **删除**。实测它与 `confirm.json`
      只差 3 行（`_note` / `reason` / `tier`），是纯重复
- [x] 根 `AGENTS.md` 硬约束第 5 条改写为两档，**同一提交内**
- [x] `docs/设计规范.md` §10 第 3b 条销账，§5 措辞复查
- [x] 全仓 grep `Auto` / `自动发送` / `auto.json`，确认没有漏掉的引用。
      `CONTEXT.md`（域文档）与 `FEATURE.md`（需求源）也一并改了 ——
      **不改就是「需求说三档、规范说两档、代码说两档」的三方矛盾**

## ⚠ 两处不是「删掉就完事」，需要单独说明

### 1. 托盘红点没有跟着取消，而是改挂了 `Confirm`

`commands::tray_icon(auto)` 的红点要回答的问题是
**「面板看不见的时候，程序到底会不会把话发出去」**。
`Auto` 取消之后这个问题**没消失**——`Confirm` 照样会发出去。

所以参数从 `auto` 改成 `live`，判断函数从 `current_tier_is_auto()`
改成 `tier_can_send()`（`matches!(current_tier(), Dispatch::Confirm)`）。

⚠ 命名是刻意的：`tier_can_send()` **不等于**「不问就发」。
硬约束第 5 条守的是后者，两件事别混。

顺带修掉一个旧歧义：三档时代红点是有歧义的——`Auto` 与 `Confirm`
都带点，看图分不出「要人确认」和「不问就发」。两档之下这个歧义反而没了。
`stamp_auto_dot` → `stamp_live_dot`，测试同名改。

### 2. `TierSwitch` 的警示闪从 `Auto` 改挂到 `Confirm`

同样不能删：删掉它等于取消升档提示，那是另一回事。
警示该跟着**最高档**走，而不是跟着某个具体档名走。

### 3. `commands.rs:331` 的 `send` 参数恒为 `false`，而且恒为 false 是对的

先确认过路径：`Confirm` 档在 `dispatch` 里走的是 254 行那个分支
（装填 → 二次点击 `send_only` 按回车），**根本到不了 331 行**；
331 行只服务 `Manual`。

⇒ 两档之下，**整个代码库里按下回车的地方只剩一处**：`inject::send_only`，
它的前置条件是 `Arming::arm` 成功（人已装填过一次）。
比三档时更容易审。代码里写了警告，别把它「简化」掉。

## 2026-10-14 补记：「改档就解除装填」是注释里的规则，代码里没有

复核时发现 `Arming` 的文档写着「三种情况都必须失效：候选换了、**档位变了**、
真的发出去了」，`dispatch` 里那句注释又写着「改档就解除装填：上一档的确认
不能带到下一档」——**两处都声称有，`set_tier` 从头到尾没碰过 `armed`**。

**漏出来的路**：Confirm 档点候选 A（只装填，不发）→ 切到 Manual → 在微信里
改成自己想发的内容 → 切回 Confirm → 再点 A。`arm("A", 同一 ticks)` 返回
true ⇒ **两段式被跳过，直接发送**，把用户自己写的内容覆盖掉。

已修，方向严格保守：**只让发送更严**（多要求一次装填），不打开任何新路径。
硬约束第 5 条的档位集合没动，动的只是「装填在什么时候作废」。

配套两件事，都属于本仓的老毛病：

- 规则抽成 `apply_tier(tier, armed, d)` 才测得着——`AppState` 得靠
  `watch::spawn()` 才造得出来（会真的起观察循环线程）。
  ⚠ 第一版测试是在测试里**复刻**判断的，把 `disarm` 短路成 `&& false`
  测试照样全绿。**复印件不是守卫。**
- 「档位没真变就不该解除装填」的反面同样有断言，否则面板每次改档都抹掉
  用户的装填，`Confirm` 永远发不完。

自证会响：`disarm` 短路后 `changing_tier_disarms` FAILED。

## 验收项

- [ ] 真机上**没有任何路径能不发确认就发出去**。逐条走：`Manual` 只填入、
      `Confirm` 弹二次确认。掐掉任一处守卫，实机验证必须失败
      ⛔ **未验**（云端会话碰不到微信实机）。代码侧已把「按回车的调用点
      只有一处」写成注释，但那一句注释不等于实测
- [x] `cargo test --workspace` 全绿，且**没有为了让测试过而删掉断言**
      —— 删掉的两个测试（`auto_failure_…`、`no_retry_happens_before_…`）
      是因为**它们守的边界在两档下不存在**，原文都搬进了注释
- [x] `?demo=1&state=` 各态仍然渲染，零 pageerror
      ⚠ **七态**不是八态：`auto.json` 删除后就是七态，
      `?state=auto` 落到 fallback 仍能渲染但不再是独立一态
- [x] 托盘 tooltip 的档位名不再出现「自动发送」（`tier_name` 收成两支）
- [x] 根 `AGENTS.md` 硬约束第 5 条与代码**一致**，且指向守卫测试
- [x] `cargo clippy --workspace --all-targets -- -D warnings` 绿
- [x] `pnpm check` / `pnpm lint` 绿
- [ ] 真机走一遍「改档 ⇒ 装填作废」：`Confirm` 档点候选（只装填）→ 切
      `Manual` → 改微信里的话 → 切回 `Confirm` → 再点同一条候选，**必须
      只填入、不得直接发送**
      ⛔ **未验**。代码侧已由 `changing_tier_disarms` 覆盖，但它只到
      「`apply_tier` 清了 `Arming`」这一层；`STATE` 与 `inject::send_only`
      那一段仍只在真机上成立
