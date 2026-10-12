# 20: 发送档位从三档砍到两档（Auto 整档取消）

**What to build:** 面板与 Rust 只剩两档 —— `Manual`（只填入）与 `Confirm`（确认后发）。
`Auto`（填入即发、不打断）**整档取消**。

**Blocked by:** None（可以立刻做）

**Status:** ready-for-agent · **决定日期 2026-10-13 · 代码里目前还是三档，一条未动**

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

- [ ] `crates/core/src/dispatch.rs` 的 `enum Dispatch` 去掉 `Auto`；
      `Default` 手写落在 `Manual` 这条**保持不变**
- [ ] 同文件 `Display` 的三支匹配收成两支
- [ ] `sends_without_asking()` 的语义要重新表述 —— 两档之下它恒为 `false`，
      **要么删掉，要么改成别的判据**，不许留一个恒假的函数骗人
- [ ] `dispatch.rs` 里引用 `Auto` 的测试逐条处置：
      `the_program_never_raises_its_own_tier`、`auto_failure_needs_human_confirmation_instead_of_downgrading`、
      `ending_a_session_always_falls_back_to_manual`、`possibly_sent_is_never_replayed`
      里的 `[Manual, Confirm, Auto]` 枚举。**测试守的是边界不是实现，
      改断言前先确认那条边界在两档下还成不成立**
- [ ] `ui/src/routes/(panel)/+page.svelte`：`TIERS`、`TIER_NAME`、
      以及三条 `data-tier='…'` 的 CSS 选择器
- [ ] `ui/static/demo/auto.json` —— 删除，或改成 `confirm.json` 的重复
- [ ] 根 `AGENTS.md` 硬约束第 5 条改写为两档
- [ ] `docs/设计规范.md` §10 第 3b 条销账，§5 配色里「红只给不可逆」
      的措辞复查（Auto 没了，「不打断」这个承诺也要跟着消失）
- [ ] 全仓 grep `Auto` / `自动发送` / `auto.json`，确认没有漏掉的引用

## 验收项

- [ ] 真机上**没有任何路径能不发确认就发出去**。逐条走：`Manual` 只填入、
      `Confirm` 弹二次确认。掐掉任一处守卫，实机验证必须失败
- [ ] `cargo test --workspace` 全绿，且**没有为了让测试过而删掉断言**
- [ ] `?demo=1&state=` 的八态仍然全部渲染，零 pageerror
- [ ] 托盘 tooltip 的档位名不再出现「自动发送」
- [ ] 根 `AGENTS.md` 硬约束第 5 条与代码**逐字一致**
