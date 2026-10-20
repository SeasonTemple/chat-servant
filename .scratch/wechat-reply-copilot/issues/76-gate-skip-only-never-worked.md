# 76: 门禁的 `-Skip` / `-Only` 从未生效 ⇒ 内循环一直在跑完整门禁

**What to build:** 让「只跑一部分」真的只跑那部分；让门禁的耗时报告与
「实际跳过了什么」都如实说话。

**Status:** done · **2026-10-20 定位并修完，四组自证逐条验过**

## 起因：确认进展时看到耗时表里并行组三步是 `0.0s`

本轮开工是「同步 + 确认进展」。跑完整门禁得到 `GATE: ALL GREEN`，
但自打耗时表里 `ui svelte-check` / `ui lint` / `ui svelte-autofixer`
三行都显示 **0.0s**，而同一轮那三步的 `>>> name [17.7s]` 行打的是组墙钟
——**同一个门禁里两个地方报同一件事，一个 0.0、一个 17.7**。

⚠ 追下去才发现：**这不是一个 bug，是四个**。

## 缺陷 ①：`-Skip` / `-Only` **根本没传进 lane 子进程**

过滤逻辑（`Should-Run`）跑在 **lane 子进程**里，父进程只负责起进程、
转发输出，**它自己不跑步**。而起子进程时只传了 `-Child $lane`
⇒ 子进程的 `$script:skipArgs` / `$script:onlyArgs` 是**空的**
⇒ `Should-Run` 恒为真 ⇒ **每条轨都跑自己轨上的全部步**。

实测（修前）：`pwsh -File scripts\gate.ps1 -Only 'ui svelte-check'`
跑出了 cheap / rust / net 三轨的**全部**步，外加 ui 轨的整个并行组
⇒ 「只跑一步」实际跑的是**完整门禁**。

⚠⚠ 假绿的方向不止是慢：父进程收到的 `##GATE##SKIP` 标记也是空的
⇒ 连 `GATE_SCOPE: PARTIAL` 那行**根本不会出现**
⇒ 用户以为自己跳过了几步，报告却显示「没跳过任何步」。
本仓的 `dev.ps1` 转发是对的，断点一直在 `gate.ps1` 内部。

**已修**：`gate.ps1` 起 lane 时把过滤参数一起传下去。

## 缺陷 ②：计时回传把秒数**硬写成 0**

并行组回传 `##GATE##STEP` 标记时写的是 `..., $code, 0`
⇒ 父进程回填 `$script:stepTime` 时三步的秒数是 0
⇒ 耗时表里那三行永远 `0.0s`。

**已修**：改回整组墙钟（`[math]::Round($sw.Elapsed.TotalSeconds, 1)`），
与同一轮 `>>> name [Ns]` 那行**同源**。

## 缺陷 ③：跳过名单用 `List.Add(数组)` ⇒ 混进一条**假步名**

`$sawSkipped.Add((($line -split "`t")[1] -split '\|'))`
—— `Add` 不展开 `IEnumerable`，数组被 `ToString()` 成
「所有步名用空格连起来」的一条长串，混进「实际跳过 N 步」的**计数与列表**。

⚠⚠ **这条路径在 `-Skip`/`-Only` 真正生效之前从来没被激活过**
（`skippedNames` 恒空 ⇒ `GATE_SCOPE` 那行从不打印）
⇒ 它自己的 bug 藏了整整四天。
⇒ 教训同族：**一个从未被执行过的分支，不等于它是对的。**

**已修**：改成逐个 `Add`。

## 缺陷 ④：步秒之和虚高 ⇒ 撞上「并行化前的旧基线」

修完 ② 之后，「步秒之和」从 **96.2s 跳到 230.8s**
（并行组三步各记一次组墙钟 ⇒ 多算 2×组墙钟），
而 `AGENTS.md` 里「并行化**之前**的串行基线」正好是 **229.3s**
⇒ 下一个读表的人会直接读成「并行化白做了」。

**已修**：耗时表里**并行组折叠成一行**、只计一次
（`ui-static（并行组 3 步：…）`）。
⚠ **折叠的判据是「模式」不是「数值相同」**：组里只要有一步被过滤掉，
整组就**退回逐个跑**（各步是自己的耗时）⇒ 那时**不折叠**。

## ⚠⚠ 顺带挖出来的根因：`pwsh -File` 对数组参数的绑定会**静默错位**

修完 ① 之后 `-Skip 'deny,ui build'` 仍然不对：实测**只有 `ui build` 跑了**，
其余 15 步全被跳。最小实验（`target/argtest.ps1`）给出答案：

```text
pwsh -File x.ps1 -Skip deny 'ui build'
  ⇒ $Skip = @('deny')      ← 只拿到紧跟其后的**一个**值
  ⇒ $Only = @('ui build')   ← **第二个值被按位置绑给了下一个参数！**
```

⇒ 传两个片段时，`-Skip` 的第二个片段**静默地变成了 `-Only`**，
过滤语义整个反过来。而**重复参数名是报错的**（`-Skip a -Skip b` 直接失败）
⇒ 危险的是**位置式**的第二个值，它一声不响。

**已治本**（两处）：
- `gate.ps1`：`[CmdletBinding(PositionalBinding = $false)]` +
  `$Skip` / `$Only` 声明成 `[string]`（逗号分隔）⇒
  位置式的第二个值**无处可绑 ⇒ 直接报错**，不再静默错位；
- `dev.ps1`：同样改成 `[string]`，转发时**合并成一个逗号分隔的值**。

⚠ 兼容性：`-Skip 'deny,ui build'`（逗号形式，`AGENTS.md` 里的写法）
**完全不受影响**；`-Skip 'deny','ui build'` 现在会**报错**（响的，好过静默）。

## 顺带修的（同一根因族）

- 本仓自己的 Python 命令一律写 `uv run --no-project python …`：
  本机 PATH 上**没有裸 `python`**（`python scripts/ticketstatus.py` 直接 exit 9009），
  Python 由 uv 管理并装在**项目盘**（`G:/tools/uv/python`，不在 C 盘）。
  文档与 `ticketstatus.py` / `targetgc.py` 的头部曾各写着一份裸 `python`
  ⇒ 照着文档敲的人当场 exit 9009，而文档本身不会红。
- ⚠ 写这条的时候踩了个 Python 的坑：docstring 里写 Windows 路径
  `G:\tools\uv\python` ⇒ `\u` 被当 Unicode 转义 ⇒ **SyntaxError**，
  且报错位置（`position 2708`）与真正的原因（docstring 第 22 行）相隔很远。
  ⇒ Python 源码里的路径**一律用正斜杠**。

## 验收项

- [x] `-Only 'ui svelte-check'` 只跑那一步（其余 15 步 SKIPPED），
      `GATE_SCOPE: PARTIAL` 如实报「实际跳过 15 步 … 结论只覆盖跑过的那 1 步」
- [x] `-Skip 'deny,ui build'` 跳掉**那两步**，其余照跑（修前是「只有 ui build 跑」）
- [x] 位置式错位（`-Skip deny 'ui build'`）现在**报错**，不再静默反转语义
- [x] 耗时表里并行组三步不再显示 `0.0s`（修前恒为 0，修后是整组墙钟）
- [x] 耗时表里并行组**折叠成一行**、步秒之和回到「每份墙钟只算一次」的口径
      （修前 96.2s → 修 ② 后虚高到 230.8s → 折叠后不再撞 229.3s 旧基线）
- [x] 完整门禁仍 `GATE: ALL GREEN`，且**不带** `GATE_SCOPE: PARTIAL`
      （条件跳过与过滤跳过是两条路径：条件跳过不发机器可读标记）
- [x] `dev.ps1` 的 `gate` 动词转发同一个逗号合并值，行为与直调一致
- [x] 文档同步：`AGENTS.md` 记下这四个坑与 `pwsh -File` 的绑定规矩；
      `ticketstatus.py` / `targetgc.py` 头部的 Python 命令改成 uv 形式
