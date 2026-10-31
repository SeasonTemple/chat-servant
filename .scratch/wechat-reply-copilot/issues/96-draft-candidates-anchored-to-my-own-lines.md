# 96: 用户报「我的回复也被当成对方回复生成了候选」——起草锚错了人；附带一条单字误识

**Status:** ready-for-human · 缺陷 A 已修（真机 + 单测双证）· 缺陷 B 停在「四条预处理路全证伪」，等下轮拍板

## 一、用户原话

> 我的回复也被当成对方回复生成了候选？而且对方的「……」识别多了个日文？

（第二条引用处按硬约束 3 脱敏：那是一句真帧读到的两字短语，具体字形与码位都不进仓库。）

## 二、缺陷 A：候选在接**我自己**的话

### 现象（真机 2026-11-01，一张夹具帧复现）

对方 1 条、我方 8 条，我方那 8 条在**后面**。面板给出的三条候选，
内容全在顺着我方最后几条往下写——也就是模型把「接下来该说什么」理解成了
「接着用户自己刚说的话说」。

### 根因（读代码，不是猜）

- `ui/src-tauri/src/watch.rs` 起草判据原来是
  `let newest = novel.iter().filter(|m| m.speaker == Speaker::Them).map(|m| m.rect.y1).max();`
  ⇒ `if newest.is_some() { …起草… }`。判据是「本轮**有没有**对方新消息」，
  不是「**最后一条**是不是对方」。我方把住话头时它也开闸。
- 同一文件 `s.history.extend(novel.iter().map(|m| m.text.clone()));`
  ⇒ **只存正文，丢了 speaker**。
- `crates/reason/src/context.rs` 的 `Trimmed::render()` 只把那些行原样拼进
  `[最近的对话]` 段落。
- `crates/reason/src/transport.rs` 的 `SYS_DRAFT` 里**没有**「回复对方最后一句」
  这条锚。

⇒ 模型拿到的是 9 行分不出谁说的、末尾 8 行全是我方 ⇒ 顺着我方的话写。

### 修法（用户选的是「两段一起」）

1. **闸**：`ui/src-tauri/src/watch.rs:1317` 新增
   `fn should_draft_now(peer_spoke_now: bool, visible: &[Message]) -> bool`，
   函数体唯一一行：
   `peer_spoke_now && matches!(visible.last().map(|m| m.speaker), Some(Speaker::Them))`；
   调用点 `:1659` 由 `if newest.is_some()` 改成 `if should_draft_now(newest.is_some(), &messages)`。
2. **标注**：`:1293` 新增 `fn history_line(m: &Message) -> String`
   （`我：` / `对方：` / `系统：` 前缀），写入口 `:1698`
   由 `.map(|m| m.text.clone())` 改成 `.map(history_line)`。
3. **提示词**：`crates/reason/src/transport.rs:833-834` 补两句——
   记录里每行标着谁说的；**要写的是替用户回复对方最后那一句，不要接着用户自己说过的话往下写**。
   禁钱那行（`:832`）逐字未动，守卫 `the_draft_prompt_still_forbids_money_topics` 仍绿。
4. **面板文案归 Rust**：`ui/src-tauri/src/watch.rs:1064` 新增
   `const I_HAVE_THE_FLOOR_HINT: &str = "你说了最后一句，等对方回话。";`，
   `snapshot()`（`:1080-1097`）在闸开且最后一条是我方时给出它。
   根因在前端：`ui/src/lib/PaperPanel.svelte:249` 的 `lastFromThem` 判据是
   「最后一条**对方**消息存在与否」⇒ 我方把住话头时它照样非空 ⇒ 面板说假话
   （「对方说完了，这轮没起草。」）。Rust 是单一事实来源，所以补在 Rust 侧。
5. `crates/reason/src/model.rs:202-212` 的过期注释改准
   （原文说 `TurnContext.messages` 每行不带说话人标签，已被第 2 条推翻）。

### 守卫

- `ui/src-tauri/src/watch.rs:3505` `the_drafting_gate_shuts_while_i_have_the_floor`
- `ui/src-tauri/src/watch.rs:3533` `every_line_the_model_sees_names_its_speaker`
- `ui/src-tauri/src/watch.rs:2647` `when_i_have_the_floor_the_panel_says_so`（6 例）

### 验收

- [x] 单测：`pwsh -NoProfile -File scripts\dev.ps1 test`（**不带 `-Filter`**）整工作区 0 failed
      （app 129 / capture 104 / core 50 / perceive 102(+2 ignored) / reason 105 / xtask 10）
- [x] `pwsh -NoProfile -File scripts\dev.ps1 check` 两步 OK（fmt --check + clippy `-D warnings`）
- [x] 真机（同一张夹具帧，重建后起包）：日志 `emit readout tick=1 phase=read … candidates=0`，
      面板正文位显示 `你说了最后一句，等对方回话。`，**无候选卡**；
      修前同一帧是 `phase=drafted … candidates=3`
- [x] 活读数：对方真来新消息那轮（`本轮新到=3`，最后一条是对方）⇒ `phase=drafting` 正常打开
      ⇒ 闸不是「关死」，两个方向都验过
- [ ] 真机复验（人眼）：点一条候选（`只填入`）之后**不再**起草新一轮 —— 本机禁输入注入，
      `SetCursorPos` 空操作 ⇒ 只能人点

## 三、缺陷 B：单字被模型按语义合并（**已证伪四条路，等下轮拍板**）

对方发来的那条**两字短语**（真帧，脱敏）被读成 **1 个字**，而且读出的那个字
与原文在字形上也对不上。取证与反证（日志都在 `target/probe/`，已 gitignore）：

- **不是词典缺字**：两个字都在 `target/models/ppocrv6-small/ppocrv6_dict.txt` 里（18708 行）。
- **不是漏框**：det 的概率带证明框覆盖那一行；框内灰度剖面实测墨迹占列 4..29，
  第 12..15 列是 4 列纯背景 ⇒ 两个字之间的缝**完整地在框内**。
- **不是分辨率**：`--scale 2 / 3` 反而让全帧从 62 行掉到 48~55 行、净字符 −8~−30。
- **不是 rec 容量**：换同族 `PP-OCRv6_rec_medium`（73.1 MiB，Apache-2.0）后该行
  **一个码位都没变**，而单帧稳态从 1293 ms 涨到 13435 ms（10.39×）⇒ 已回退。
- **不是裁剪插值**：rec 裁剪由手搓最近邻换成双线性，该行不变、4 条对照行 ED 全 0、
  但全帧有 21 行文本改变 ⇒ 改动确实进了通路，方向无效 ⇒ 已回退。
- **不是框内超采样**：4 轮 4 种（双线性铺开 / 相位对齐 / 硬边复制+复刻 scale-2 采样格 /
  二值化），该行**每次都还是 1 个字**，4 条对照行每轮逐字未变 ⇒ 已回退。

⇒ 结论落在「模型把两个字形按语义合并成一个字」（那种合并关系在字形上是可解释的），
而唯一读出过两个字的那次靠的是**整帧先放大 2 倍**——那条路 S2 已量过，
代价是整帧劣化（为一行牺牲二十行）。

- [ ] 下一步：换 rec 模型家族（不同词典，成本最高）还是记成已知边界 —— 用户 2026-11-01 定
      「下轮再说」，本轮**不动**
- ⚠ 复现要用的那帧真机截图仍在 `target/probe/frame-1793475836.png`（gitignored，
  显式调试命令的产物）：**不许**进提交、注释、夹具；缺陷 B 结案时删掉

## 四、纪律教训（本轮踩到的）

- `scripts/dev.ps1` 的 `-Filter` 是**测试名子串**，不是包名 ⇒ 写成
  `-Filter chatservant_reason` 会 0 测试空跑**假绿**。要么不带 `-Filter` 跑整工作区，
  要么传测试名。
- 只跑 `test` + `fmt` 就报完成 ⇒ 漏掉 clippy（本轮一条 `useless use of vec!`
  在第一轮就写进来了，第三轮跑 `check` 才照出来）⇒ `dev.ps1 check` 才是 Rust 内循环入口。
