# 21: `phase` 判别字段 Rust 运行时从不发送 —— S3/S4/S6 在真机上不可达

**What to build:** 真机上面板能显示「起草中」「已起草」「出错」三态，
起草好的回复**出现在正文里**，而不是只在 demo 夹具里出现。

**Blocked by:** None（可以立刻做）

**Status:** ready-for-human · **P0 · 2026-10-12 实测，不是推断**
· ✅ **2026-10-13 修了（选 A + 补一帧 Drafting）**
· ⚠ 剩下的两项要开真实微信会话验，`ready-for-human` 而非 `ready-for-agent`

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

## ✅ 2026-10-13 实际怎么做的（选 A，且不是「加个字段」那么简单）

### 选了 A，理由

B 要让界面重新开始猜 Rust 的状态——而 `contract.rs` 的模块注释里写着
「**判断归 Rust，界面只渲染**」，那次收口就是为了拆掉界面里的猜测。
前端类型 `FullReadout = Phase & Extra` 本来就按「判别字段与数据平级」写着，
说明**契约的方向一直是 A**，只是没人把它接上。

### 但 A 光加字段不够，还差两处

1. **`phase` 得是 `#[serde(flatten)]` 摊平进 `Readout`**，
   而不是加一个独立字段。独立的 `phase` + 独立的 `messages`/`candidates`
   是**两份数据**，得靠人保证一致；摊平之后它们同属一个枚举变体，
   **不一致在类型上就构造不出来**。
   `messages` / `candidates` / `error` 因此从 `Readout` 上删掉了——
   前端的 `r.candidates` 读法一个字没改。
2. **「起草中」这一帧原先根本不存在。** 光有 `phase` 字段，
   `Drafting` 仍然出不来——循环只在轮末 emit，而轮末模型已经回来了。
   要让 S3 可达，得在 **OCR/归类之后、调模型之前**多推一帧。
   顺带就是 `contract.rs` 里写过、但从没实现过的**首屏提速**
   （6~9s → 1.1s）。

### 连带的第三处：push 去重

一轮现在推**两帧**（`Drafting` → `Drafted`），两帧 `ticks` 相同。
`cs-pump` 原来只比 `ticks`，会把**真正有候选的那一帧当重复吃掉**，
界面永远停在「正在起草」。指纹改成 `(ticks, phase.key())`——
`contract.rs` 里 `Phase::key()` 就是为这件事写的，之前也没被用过。

## 验收项

- [ ] 真机（不是 demo）上能看到 S3 / S4 / S6，截图存进 `target/probe/`
      ⛔ **未验**：云端会话碰不到微信实机，留给本机
- [ ] 起草完成后，正文里**真的出现**那句回复与备选
      ⛔ **未验**：同上，同一次实机里一起验
- [x] `Readout` 的每个构造点都赋了值。新增 `every_readout_goes_through_the_snapshot_constructor`
      —— 它 grep 源码，断言 `Readout {` 字面构造**全仓只有一处**
      （`snapshot()`）。多一处少一处都红。
- [x] 补了 `readout_carries_the_phase_discriminant_at_the_top_level`
      与 `every_phase_shape_carries_its_own_discriminant`，
      锁死「判别字段与 messages/candidates 平级」这个形状
- [x] `?demo=1` 的八态仍然全部渲染 —— 前端**一个字没改**
      （`types.ts` 的 `ReadoutPayload = Phase` 本来就对）
- [x] 修完之后把 `docs/设计规范.md` §10 第 12 条销账
- [x] `cargo test --workspace` 绿、`cargo clippy -D warnings` 绿

## ⚠ 做这件事时连根拔出来的（票 26）

`main.rs` 里**从来没声明过 `mod contract;`** —— `contract.rs` 从落盘到今天
**一次都没被编译过**，它自己的测试也一次都没跑过。

所以票 21 记的「`Phase` 只在自己的测试里被构造过」实际情况更糟。
接进编译之后一次暴露四件事：`DraftCandidate` 缺 `PartialEq`、
事件名散在五处（其中 `menu:settings` 根本没进过任何清单）、
`Action`/`Action::parse`/`status_hint` 是死代码。
→ 开 **票 26**，本轮没顺手做。


## Update 2026-10-15：S5 真机上不存在；S6 只能靠采集层故障

机器预验的结果直接影响「怎么点出 S3/S4/S6」，故先记：

| 阶段 | 真机可达性 |
|---|---|
| **S3 起草中** | ✅ 帧在推理发起**之前**就推（`watch.rs:1121`），只要 OCR 读到对方说话就出现 |
| **S4 已起草** | ⚠ **必须有真模型成功返回** ⇒ 需要凭据管理器里有 key（`secret.rs:5-8`） |
| **S5 待发射** | ❌ **不存在，而且比原先更彻底**。`Auto` 档于 2026-10-15 回来后**没有第二次点击**，于是「点下去→装填→等确认发」这个中间态**在产品模型里就没有了**；加上 `PaperPanel.svelte:661-663` 的动作行本来就是空的（见票 20 的 Update），界面与模型**两头都没有它** |
| **S6 出错** | ⚠ **不需要模型，但也不能靠缺 key / 401 / 网络失败造出来** |

⚠ **S6 那条最容易踩**：模型层失败走的是 `s.reason = "推理失败：{e}"`
（`watch.rs:1205-1209`），`phase` 仍是 `read`，界面显示「对方说完了，
这轮没起草。」——**不是 S6**。要进 S6 只能制造**采集层**故障：
抓不到窗口（`watch.rs:347`）、全黑帧（`:357`）、裁不出消息区（`:921`）、
学不到配色（`:1027`）。

⚠ **前置条件里有一处硬编码**：闸锁定的会话名**写死为 `"小号"`**
（`commands.rs:1586`）。名字不匹配 ⇒ 闸关 ⇒ 只有停泊态，**永远到不了 S3/S4**。

**人工点测的最短路径**：配好 key → 起 app → 微信切到名字含「小号」的会话
→ 让**对方**发一条新消息（水位线只认 `Speaker::Them`）
→ 期待 `read` 首屏 → S3「正在起草，通常几秒」→ 数秒后 S4 正文出现候选。
截图：`pwsh -NoProfile -File scripts\fgwin.ps1` 取 hwnd（十六进制），
再 `pwsh -NoProfile -File scripts\shotwin.ps1 <hwnd> target\probe\S4.png`。
⚠ 截图**不入 git**（取证目录纪律）。