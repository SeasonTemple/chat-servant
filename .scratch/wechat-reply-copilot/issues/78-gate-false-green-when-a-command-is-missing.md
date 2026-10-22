# 78: 工具不在 PATH 时门禁报假绿——「命令没找到」被算成「检查通过」

**What to build:** 让门禁在**工具缺失**时判红；让「拿不到退出码」不再等价于「通过」。

**Status:** 修完并自证两向 · **2026-10-22 实测触发并验通**

## 起因

2026-10-22 本轮开工是「同步远程 + 核实进展是否达到交付」。
同步完跑完整门禁，得到 `GATE: ALL GREEN` —— 但**同一份输出里**
`>>> ui build FAILED [0.4s] (exit=)` 与 `##GATE##STEP\tui build\t\t0.4`
（**退出码字段是空的**）并排出现。

⇒ 「全绿」与「有一步失败」在屏幕上同形。
⇒ ⚠ 而本轮要回答的正是「是否达到交付」——**「全绿」这两个字本身成了待证物**。

⚠ 追下去发现：**这不是一个 bug，是三个**，每一处都单独就够假绿。

## 量具先验对（PowerShell 语义实测）

| 情形 | `$LASTEXITCODE` | `$Error.Count` |
|---|---|---|
| 原生命令写 stderr（`2>&1` 合并） | 0 | **0** |
| **命令不存在**（pnpm 不在 PATH） | **0（不变）** | **1**（CommandNotFoundException） |

⇒ **PowerShell 级错误不改 `$LASTEXITCODE`** ⇒ 只盯退出码就必然假绿；
⇒ `$Error` 可以当判据：原生命令的 stderr **不进** `$Error`（不会误伤）。

## 缺陷 ①：串行步红了，轨子照样 `exit 0`

`Invoke-StepNow` 只**打印** `FAILED`，**不置** `$script:fail`；
而 `Invoke-Lane` 判本轨红绿靠的正是「`$script:fail` 有没有变」
⇒ 串行步红了，`$laneFail` 仍是 0 ⇒ 轨子 `exit 0` ⇒ 父进程看轨子退出码也是 0。

**实测（修前）**：`ui build` 打 `FAILED (exit=)`，整轮仍 `GATE: ALL GREEN` / `GATE_EXIT=0`。

**已修**：`Invoke-StepNow` 判红时同时置 `$script:fail = 1`。

## 缺陷 ②：父进程把**空的**退出码当成 0

`if ([int]$f[2] -ne 0) { $script:fail = 1 }` —— 字段是**空串**时
`[int]""` 抛转换异常，**那句 if 根本不执行** ⇒ `$script:fail` 保持 0。
⇒ 「拿不到退出码」被当成了「通过」。

**已修**：改用 `[int]::TryParse`；解析不出来就**判红**并打 `GATE_FATAL`。

## 缺陷 ③：并行组的子脚本在命令不存在时 `exit 0`

子脚本模板是 `$global:LASTEXITCODE = 0` + 动作体 + `exit $LASTEXITCODE`
⇒ 命令不存在 ⇒ `$LASTEXITCODE` 不变 ⇒ 仍是那个 0 ⇒ 子进程 exit 0
⇒ 父进程把 `ui svelte-check` / `ui lint` 记成 **OK**。
（实测那两步的 stderr 里明明打着「术语 'pnpm' 不会被识别」。）

⚠ **这是同一族的第三次**：① Start-ThreadJob 那次（runspace 里连
`Select-Object` 都没有 ⇒ 三步全打 OK）；② 本次。
⇒ 判据不能只盯退出码。

**已修**：子脚本在动作体前后看 `$Error`；有 CommandNotFoundException ⇒ `exit 127`。

## 三处修法汇总

| 位置 | 修法 |
|---|---|
| `Invoke-StepNow` | `$Error.Clear()` → 跑动作体 → 有 PowerShell 级错误 ⇒ 127/1；退出码为 `$null` ⇒ 1；判红时置 `$script:fail = 1` |
| 并行组子脚本模板 | 同上（`$Error.Clear()` + 跑完判 `$Error`） |
| 父进程解析 `##GATE##STEP` | `[int]::TryParse`；解析不出来 ⇒ `GATE_FATAL` + 判红 |

⚠ **不新增门禁步** ⇒ 不触发「四处一起改」（`gate.ps1` + `ci.yml` +
`release.yml` + `AGENTS.md` + `CONTRIBUTING.md`）。
⚠ `check-gate-parity.ps1` 的 ⑨ 守卫与 `Get-JobSpans` **未动**（属用户所有）。

## 验收项

- [x] **方向一（该红）**：pnpm 不在 PATH 时跑完整门禁 ⇒ `GATE: FAILED`、`GATE_EXIT=1`
      ⇒ 且 `ui svelte-check` / `ui lint` / `ui build` 三步都是 `exit=127`
      （修前：前两步报 OK、第三步报 FAILED 但整轮 ALL GREEN）
- [x] **方向二（该绿）**：把 pnpm 放回 PATH 后跑完整门禁 ⇒ `GATE: ALL GREEN`、
      无 `GATE_FATAL` / `GATE_STEP_ERROR`
- [x] 修完跑完整门禁仍全绿，且**不带** `GATE_SCOPE: PARTIAL`
- [x] 票据与 `STATUS.md` 同步（`scripts/ticketstatus.py` 重跑）

## ⚠⚠ 方向二顺手挖出**第四种假绿形态**：退出码 0 ≠ 检查跑过

方向二里门禁是绿的，但 `ui svelte-check` / `ui lint` / `ui build` 三步
**一行输出都没有**，`ui build` 只花 **0.8s**（vite build 正常要十几秒）。

**根因不在门禁，在本机的 pnpm**：

```
E:\DevCache\npm-global\node_modules\pnpm\pnpm
  ├─ 内容仍是那句占位注释：「pnpm's native binary replaces this file
  │   during installation (see ./install.js)」
  └─ ⇒ 原生二进制**从没被装上**
```

⇒ `pnpm.cmd` 直接报 `is not recognized`（exit 1）；
⇒ 而 PowerShell 优先命中 `pnpm.ps1`，它执行那个**无扩展名的占位文本文件**
   ⇒ **静默空转：exit 0、零输出、0.5s**。

⇒ 门禁看到的是「退出码 0、没有错误」⇒ 报 OK。
⇒ ⚠⚠ **这是本仓最熟的形状**：输出正常、结论正常、什么都没发生。
   前三次分别在 Start-ThreadJob、`-Skip/-Only`、本票前两处。

⇒ **本票修的三处堵的是「工具缺失 / 退出码取不到」，堵不住这一形态**：
   工具**在**、退出码**是** 0，只是它什么也没做。
⇒ ⇒ **UI 轨缺的不是退出码，是「见证」**：这一步必须留下**产物或输出证据**
   （svelte-check 的 `found 0 errors`、`ui build` 的产物），否则退出码 0
   永远可以是「什么都没跑」。

⚠ **CI 侧不受影响**：`ci.yml` 用 `pnpm/action-setup` 在 runner 上装 pnpm，
且每次 run 都真跑 ⇒ 最近 8 次 run 全绿（含 tip `39651119209`）。
⇒ **本机 UI 侧的「全绿」不可作数，CI 的才是。**

⚠ **本机 pnpm 待修**（不在本票范围，是另一件事）：
两条修法都可行 ——
① 跑该包自带的 `node node_modules/pnpm/install.js`（补装原生二进制，
   从 npmmirror 拉；属**装软件**，要先问过仓库主人）；
② 把两个 shim 指向现成可用的 JS 入口
   `node node_modules/pnpm/bin/pnpm.mjs`（实测 `12.14.0`，1.4s 可用）——
   但那是改**全局 npm 目录**里的文件，npm 升级会覆盖。
