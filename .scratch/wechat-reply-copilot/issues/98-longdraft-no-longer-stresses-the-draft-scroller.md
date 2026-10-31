# 98: `longdraft` 夹具已经不再压测正文位滚动 —— 票 88 的现场可能失守

**Status:** done · 2026-11-01 技术判定完成：票 88 现场已由票 92 与票 97 结构性消灭（结构证明 + 13 态与极端注入实测），`longdraft` / `auto-longdraft` 已改记为候选区（`.cs-actions`）长文本压测夹具（3/3 验收项通过）

## 一、事实

票 97 把正文位（`.cs-draft`）退成「只放一句状态句」之后，实测它的
`scrollTop / scrollHeight / clientHeight` = **0 / 43 / 43**（2026-11-01 状态句升 18px 后由 39 升为 43）⇒ **不可滚**。
而 `longdraft` / `auto-longdraft` 两张夹具的设计目的原本是「草稿太长」那一路。

⇒ 用它们去验「长草稿会不会把档位行顶出纸」（票 88 的现场）时，
**`.cs-draft` 测的已经不是滚动，而是一个 43px 矮槽位**；长文本实际上落进了 `.cs-actions`。

## 二、要回答的问题与逐条判定（2026-11-01 实测）

1. **票 88 的现场还有没有一条能复现的路径？（若正文位结构上永远不滚动，那条缺陷是否已被票 97 的结构性改动顺带消灭？）**
   - **答：没有可复现路径——结构上不可能再由长草稿或任意正文把档位行（`.cs-pick`）顶出纸外。该缺陷已被票 92（候选全部移入 `.cs-actions`，正文位退为状态句）与票 97（引文 3 行夹取 + 状态句单行 43px 下限）的结构性改动彻底消灭。**
   - **结构性证明（代码行号与 Flex 预算闭环）**：
     - 外层纸 `.cs-sheet`（`ui/src/lib/PaperPanel.svelte:1792-1825`）：高 `560px`，`padding: 40px 32px 20px`，`overflow: hidden`；实心纸底 `y = 496px`（标称纸底 `512px`）。
     - 主列 `.cs-body-col`（`ui/src/lib/PaperPanel.svelte:3253-3395`）：`flex: 1; min-height: 0; padding: 0 0 40px; gap: 14px;`。它的高度被锁死为 `560 - 40 - 20 = 500px`（`top: 40, bottom: 540`），内部可用内容高恒为 `500 - 40 = 460px`（`y: 40..500`）。
     - 列内四个流内子项的 Flex 收缩与上限关系：
       1. 归属行 `.cs-where`（`PaperPanel.svelte:3397-3455`）：引文 `.cs-where-quote` 被 `-webkit-line-clamp: 3; line-clamp: 3; overflow: hidden;`（`PaperPanel.svelte:3426-3438`）硬夹死在 3 行以内，1 行时高 `49.4px`，200 字长引文（`longquote`）满 3 行时高 `101.6px`。
       2. 正文位 `.cs-draft`（`PaperPanel.svelte:1255-1333`、`3519-3587`）：
          - 在有候选的 `drafted` 态下，模板（`PaperPanel.svelte:1259-1285`）只渲染常量短句 `<p class="cs-draft-text cs-draft-status">候选如下</p>`，文字高 `27px` + 上下 `padding: 8px 0` = **恒定 `43px`**（`scrollTop=0, scrollHeight=43, clientHeight=43`）。长草稿（`candidates[0].text`）根本不进 `.cs-draft`。
          - 在无候选的其余分支（`drafting` / `failed` / `read`）下，`.cs-draft` 具有 `flex: 0 1 auto; min-height: 43px; overflow-y: auto;`（`PaperPanel.svelte:3520, 3577-3578`）。显式的 `min-height: 43px` 覆盖了 Flexbox 默认的 `min-height: auto`，哪怕注入 20 倍超长报错文本（实测 `scrollHeight = 1042px`），它也只收缩到列内剩余高度 `334.3px` 并在内部滚动，绝不外推兄弟节点。
       3. 候选区 `.cs-actions`（`PaperPanel.svelte:1354-1720`、`2138-2252`）：在 `.cs-has-acts` 下为 `flex: 1 1 auto; min-height: 0; overflow-y: auto;`（`PaperPanel.svelte:2138-2140, 2249-2252`）。**正是这一个 `flex: 1 1 auto; min-height: 0` 的滚动容器吸收了全部候选溢出与上方归属行（1~3 行）的高度变化**：当归属行 1 行（`49.4px`）时，`.cs-actions` 分配到 `460 - 49.4 - 43 - 48.3 - 3×14 = 277.3px`；当归属行满 3 行（`101.6px`）时，`.cs-actions` 自动收缩到 `225.1px`，多余候选高度全部转为内部 `scrollHeight`。
       4. 档位行 `.cs-pick`（`PaperPanel.svelte:2963-2992`）：`flex: none; margin-top: auto; padding-bottom: 20px;`（有候选时 `.cs-has-acts .cs-pick { margin-top: 0 }`，`PaperPanel.svelte:2293-2295`），高 `48.3px`。
     - 因此，无论哪一态，`.cs-pick` 的底边恒为 `40 + 460 = 500.0px`（顶边恒为 `451.7px`），档位格内最低可见像素 `slotBottom = 487px`、红铅笔勾底 `487.2px`，恒定在实心纸底 `496px` 之上（余量 `9px`），`.cs-sheet` 的 `overflow: hidden` 结构上不可能裁到档位行。
   - **实测证据**：
     - 命令 1：`cd ui && node scripts/run-budget.mjs`
       - 输出：`ALL GREEN（49 条判据）`；`drafted / auto / sent / many / longdraft / auto-longdraft` 六态的 `slotBottom` 全为 `487` vs `纸底 496`（`clipped=false, over=-9`），红勾底全为 `487.2 <= 496`。
     - 命令 2：`cd ui && node scripts/probe-header.mjs --states=longdraft,auto-longdraft,many,failed,drafted,auto,longquote,noprep,drafting,silent,parked,fresh,armed --brief=1`
       - 输出：全部 **13 张夹具**中，`.cs-sheet` 恒为 `{top:0, h:560, bottom:560}`，`.cs-body-col` 恒为 `{top:40, h:500, bottom:540}`，`.cs-pick` **一字不差全为 `{top:451.7, h:48.3, bottom:500}`**。
       - 其中 `longdraft` 与 `auto-longdraft` 的 `.cs-draft` 均为 `{top:103.4, h:43, bottom:146.4}`、`draftScroll: {clientH:43, scrollH:43, scrollTop:0, moreBelow:"false"}`。
     - 命令 3（极端注入测试，向 `failed` 态注入 20 倍合成长错因使 `.cs-draft` 的 `scrollHeight=1042`）：
       - 输出：`.cs-draft` 为 `{top:103.4, h:334.3, bottom:437.7, clientH:334, scrollH:1042}`（内部滚动），`.cs-pick` 仍为 `{top:451.7, h:48.3, bottom:500}`。

2. **若仍可复现，`longdraft` 应该改成压测候选区（`.cs-actions`）还是重新造一个让正文位真的溢出的状态？**
   - **答：票 88 的现场已不可复现；同时不应当为了「让正文位溢出」而违背票 92/97 的产品设计硬造长状态句。`longdraft` 与 `auto-longdraft` 实际上已经在压测候选区（`.cs-actions`）的「单条超长候选折行溢出」。**
   - **实测证据（`probe-header.mjs` 与 `run-budget.mjs`）**：
     - `drafted`（普通 3 条候选）：`.cs-actions` 的 `scrollTop/scrollH/clientH = 0 / 277 / 277`（`below = 0`，不滚）。
     - `many`（5 条短/中候选）：`.cs-actions` 的 `scrollTop/scrollH/clientH = 0 / 357 / 277`（`below = 80px`，压测「条数多」导致的溢出）。
     - `longdraft`（3 条候选，但首条 `c0` 含 122 字合成长句、在 231px 正文栏宽里折成 6 行）：`.cs-actions` 的 `scrollTop/scrollH/clientH = 0 / 355 / 277`（`below = 78px`，`moreBelow="true"`，`fadeBottom="32px"`，`markActs.h = 216.1`）——即便只有 3 条候选，单条长草稿也会把 `.cs-actions` 撑出 `78px` 溢出。
     - `auto-longdraft`（同上 3 条候选 + `Auto` 档红字不可逆警告行 `.cs-act-warn`）：`.cs-actions` 的 `scrollTop/scrollH/clientH = 0 / 374 / 277`（`below = 97px`，`moreBelow="true"`，`fadeBottom="32px"`，`markActs.h = 205.2`）——比 `longdraft` 再多切 `19px`。
     - 因此 `many`（条数多：5 条）与 `longdraft` / `auto-longdraft`（单条长：3 条里首条 122 字 + Manual/Auto 两档）在 `.cs-actions` 上恰好构成正交的两组压测。

3. **若已不可复现，票 88 应当在票面里写明「由票 97 的结构消灭」，并把 `longdraft` 的用途改成候选区压测夹具（否则它继续挂着「草稿太长」这个名字骗人）。**
   - **答：由于任务边界规定只动本票（票 98）不改其它票，本票将「票 88 现场已由票 92/97 的结构性改动消灭」完整记账在本票中；保留 `longdraft` / `auto-longdraft` 的文件名（遵循第四节「不要动 `longdraft` 的名字去掩盖问题：名字是判据，不是装饰」，且 `ui/static/budget.html:88,93` 与 `ui/scripts/probe-header.mjs:27,28` 均以该名引用），同时更新 `ui/static/demo/longdraft.json`、`ui/static/demo/auto-longdraft.json` 的 `_note` 与 `ui/src/lib/PaperPanel.svelte:3570-3576` 处的注释，明确它们现在压测的是「候选区 `.cs-actions` 在单条长草稿（及 Auto 警告行）下的折行滚动、渐隐与档位行不越界」，不再声称压测 `.cs-draft` 正文位滚动。**

## 三、判据与验收

- [x] 给出「结构上不可能」的完整证明与 13 态 + 极端注入实测数字（见第二节第 1 条）
- [x] 更新 `ui/static/demo/longdraft.json` 与 `ui/static/demo/auto-longdraft.json` 的 `_note` 用途说明，并同步修正 `ui/src/lib/PaperPanel.svelte` 中关于票 88 的注释
- [x] `pwsh -NoProfile -File scripts\dev.ps1 ui-check` 通过（含 `AUTOFIXER_ALL_CLEAN`），且留存 `deviceScaleFactor: 2` 面板复核截图于 `target/probe/issue-98/panel-issue98-longdraft-scroll0.5@2x.png`

## 四、不要做的事

- 不要让「测试是绿的」当成「现场还在」——那次 `finished in 0.00s` 的教训同上。
- 不要动 `longdraft` 的**名字**去掩盖问题：名字是判据，不是装饰。
