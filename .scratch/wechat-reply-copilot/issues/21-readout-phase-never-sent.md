# 21: `phase` 判别字段 Rust 运行时从不发送 —— S3/S4/S6 在真机上不可达

**What to build:** 真机上面板能显示「起草中」「已起草」「出错」三态，
起草好的回复**出现在正文里**，而不是只在 demo 夹具里出现。

**Blocked by:** None（可以立刻做）

**Status:** ready-for-agent · **P0 · 2026-10-12 实测，不是推断**

## 症状

`ui/src/routes/(panel)/+page.svelte` 的 `apply()` 里：

```ts
candidates = r.phase === 'drafted' ? r.candidates : [];
```

而 `crates/…/watch.rs` 的 `Readout` 结构体**根本没有 `phase` 字段**
（逐字核对过）。`contract.rs` 的 `Phase` 枚举只在它自己的测试里被构造过。

于是 `r.phase` 恒 `undefined` ⇒ `candidates` 恒空 ⇒
**S3（起草中）、S4（已起草）、S6（出错）在真机上永远不可达。**

## 为什么门禁全绿也发现不了

`ui/static/demo/*.json` 的夹具**手写了 `phase`**，所以 demo 模式八态全渲染得出来。
`cargo test` 测的是 Rust，测不到「Rust 有没有发这个字段」。
`svelte-check` 只管类型不管数据。

⇒ 演示页是绿的、真机是断的，**两边看着一样**。

## 修法二选一（不定，由做的人按下面判据选，并写下来）

- **A 补字段**：给 `Readout` 加 `phase`，在 `watch.rs` 每个产出点显式赋值。
  代价是所有构造点都得改，漏一个就又变成「某个态发不出来」。
- **B 换判据**：面板不用 `phase`，改成从已有的真值推
  （例如 `candidates.len() > 0` ⇒ 已起草）。
  代价是「起草中」和「已起草」的区分就得另找真值 —— **不能靠时间猜**。

**判据**： whichever 选了，都要有一个**真机**测试证明三态都能到达。
B 的风险是「起草中」可能退化成「还没候选」——那和「静默」就分不开了。

## 验收项

- [ ] 真机（不是 demo）上能看到 S3 / S4 / S6，截图存进 `target/probe/`
- [ ] 起草完成后，正文里**真的出现**那句回复与备选
- [ ] `Readout` 的每个构造点都赋了值。若选 A：新增一条测试
      枚举所有构造点，**漏一个就红**
- [ ] `?demo=1` 的八态仍然全部渲染（改判据不许改坏夹具路径）
- [ ] 修完之后把 `docs/设计规范.md` §10 第 12 条销账
