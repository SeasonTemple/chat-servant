param(
  # ⚠ 原先这里写死了 `'E:\devSpace\chat-servant-m31f\target\shots'`——
  #   **另一台机器的绝对路径**。本机（`E:\workspace\chat-servant`）上那个目录
  #   不存在，脚本会安静地扫到 0 张图然后抛「no st-*.png found」，
  #   读起来像是「夹具没渲染」，而真实原因是**量具指着别人的家目录**。
  # ⇒ 改成从脚本自身位置推出仓库根，与 `.gitignore` 里的 target 相对。
  [string]$Dir = (Join-Path $PSScriptRoot '..\..\target\shots')
)

# 量「正文区有多少墨」。用来证明**各状态真的各自渲染了**。
#
# ## 为什么需要它
#
# 逐张截图用眼睛比对有个具体的失效方式：状态如果全渲成同一张图，
# 「看起来都渲染了」和「全都渲染成一样」在截图上是同一件事。
# 而 PNG 的字节数也不够——纸面背景占满画面，改几个字可能一个字节都不差。
#
# 所以量一个**有区分度**的量：正文区（仿宋那几行）的墨色像素占比。
#
# ## ⚠ 2026-10-14：这套几何错过一次，而且**旧的量具自检没能发现**
#
# 旧采样窗是 2× 的 y 340..640（css 170..320）。票 25 把面板从
# `(panel)/panel/` 上移、归属行与正文行重排之后，正文实际落在
# 2× 的 y **224..444**（css 112..222），旧窗只吃到 5 行里的**最后 2 行**。
#
# 症状：`drafted` 从记录在案的 5.02% 掉到 2.03%，而
# `drafting`/`failed`/`noprep`/`silent` 全部变成 **0.00%**——
# 整张表看起来仍然「跑通了」，有格式、有采样数、没有一个错字。
#
# ⚠ 而旧自检（`采样` 必须等于硬编码的 57000）**照样通过**：
# 它只数「迭代了多少像素」，不看这些像素落在什么上。
# 一个整体偏掉的窗口，与一个正确的窗口，采样数完全一样。
# ⇒ 自检必须改成**能因为「偏了」而失败**的那种，见下面第 3 条。
#
# ## ⚠ 九个夹具 ≠ 九张不同的图（2026-10-14 实测）
#
# ⚠ 这里**不再抄字节数**：抄进来的输出必然过期，而过期之后没人会去改它
#   （`AGENTS.md` 的文档纪律明写「环境事实直接跑命令，不许抄进仓库」）。
#   ⇒ 要看各态差多少，**跑本脚本**，表里就是当前值。
#
# ⚠ 一条**结构性**事实仍然成立（它不随字节数变）：
#   `auto` 与 `armed` 想表达的两个界面**只差 `tier`**（2026-10-15 改档名前
#   这两个夹具叫 `confirm` 与 `armed`，且都写 `tier: Confirm`），
#   而动作行**根本不存在**
#   （`docs/前端设计进度.md` §3：动作行未做，碰发送路径）。
#   ⇒ 演示页本来就分不出这两态，不是夹具写错了。
#
# ⚠ `noprep` 与 `silent` 在这张表里占比相同（0.81%），**也是对的**：
#   两者 phase 都是 `read`，差别在**归属行**的 peer（空 vs 示例联系人），
#   而取景窗按设计不包含归属行。两者的整图字节数是不同的（580643 / 585051）。
#
# ⚠ `fresh`（2026-10-14 补的第八具，对方**还没说话**）占比 **0.00%**，
#   **同样是对的**，不是"这具没渲染出来"：
#   取景窗只取正文区，而 `fresh` 的正文是那句占位文案
#   「对方说完话，这里才落笔。」——它在归属行下方的位置，落在窗外。
#   这具的判据**不在这张表里**，靠看图（`?demo=1&state=fresh`）：归属行有名字、
#   **没有引文行**、正文是那一支空态文案。表看它的「0」只说明它正文没有多行正文。
#
# ⚠ `parked`（2026-10-14 补的第九具，**已停泊在别的会话上**）占比 **0.00%**，
#   **理由与 `fresh` 完全一样**，但它验的是**另一条**路径：
#   两者 `messages` 都是空数组，界面分不出「对方真没说话」与「已停泊」——
#   真机实测里，正文显示的是「对方说完话，这里才落笔。」，而那个会话
#   对方明明说过话，**那句话是假的**。⇒ 停泊时 Rust 改给
#   「锁着别的对话，切回来才继续。」，判据在 `watch.rs` 的
#   `a_closed_gate_replaces_the_waiting_line_with_an_honest_one`。
#   看图（`?demo=1&state=parked`）：正文那一句与 `fresh` **不同**。
#
# ## 取景框几何（⚠ 换任何一处尺寸都得同步改这三个数）
#
# 演示取景框 380×560 居中于 520×760 视口（`(panel)/+layout.svelte` 的
# `.cs-demo-frame`），出图 2× ⇒ 图像 1040×1520。
# 归属行在 css 44..82，正文行在 css **112..222**，档位行在 css 466..478。
# 取景窗选正文那一段：2× 的 224..450，**不碰归属行**——
# 归属行与正文是两回事，混成一个数就量不出「正文变没变」。
#
# ⚠ Chrome 会把窗口宽度钳到 500px 最小值，而「截图尺寸对」**不**证明
#   「布局宽度对」（`scripts/shoturl.ps1` 有同一条）。
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$Scale = 2
$X0 = 140
$X1 = 900
$Y0 = 224
$Y1 = 450

# 采样数由这三个数**算出来**，不再硬编码。
# 硬编码过一次（57000），结果几何一改它就成了摆设——
# 而它偏偏是那个本该拦住这次错位的检查。
$ExpectW = 520 * $Scale
$ExpectH = 760 * $Scale
$ExpectSamples = ((($X1 - $X0) / 2) * (($Y1 - $Y0) / 2))

$rows = @()
foreach ($f in Get-ChildItem $Dir -Filter 'st-*.png' | Sort-Object Name) {
  $bmp = [System.Drawing.Bitmap]::FromFile($f.FullName)
  try {
    $dark = 0
    $sampled = 0
    for ($y = $Y0; $y -lt $Y1; $y += 2) {
      for ($x = $X0; $x -lt $X1; $x += 2) {
        $c = $bmp.GetPixel($x, $y)
        $sampled++
        # 墨 rgb(20,26,31) 与铅笔灰 rgb(76,90,99) 都落在这个亮度带内，
        # 而纸底是暖黄 rgb(243,230,192)、取景框外是纯白。
        if (($c.R + $c.G + $c.B) -lt 330) { $dark++ }
      }
    }
    $rows += [pscustomobject]@{
      态   = $f.BaseName -replace '^st-', ''
      图   = "$($bmp.Width)x$($bmp.Height)"
      采样 = $sampled
      墨点 = $dark
      占比 = [math]::Round(100 * $dark / $sampled, 2)
    }
  } finally { $bmp.Dispose() }
}
$rows | Format-Table -AutoSize

if ($rows.Count -eq 0) { throw 'no st-*.png found - render them first, the gauge has no input.' }

# ── self-check 1: image size ─────────────────────────────────────────
# 视口或缩放变了，逐像素循环的下标就会越界 / 漏采样。
$badSize = @($rows | Where-Object { $_.图 -ne "$ExpectW`x$ExpectH" })
if ($badSize.Count -gt 0) {
  throw "image is not ${ExpectW}x${ExpectH} (got $($badSize[0].图)): viewport or Scale changed, X0/X1/Y0/Y1 must follow."
}

# ── self-check 2: sample count ───────────────────────────────────────
$badSample = @($rows | Where-Object { $_.采样 -ne $ExpectSamples })
if ($badSample.Count -gt 0) {
  throw "sample count != $ExpectSamples (got $($badSample[0].采样)): X0/X1/Y0/Y1 arithmetic is wrong, ratios unusable."
}

# ── self-check 3: the window must actually land on body text (the key one)
#
# 判据用 `drafted`：它是唯一有整段正文的状态，取景窗一旦整体偏出正文，
# 它的墨点必然是 0。而 `drafting` / `failed` / `noprep` / `silent`
# 本来就接近 0，**不能用它们当判据**——那样「全都 0」反而看着自洽。
#
# 这条正是旧自检缺的那条：旧版只校验采样数，于是窗口整体偏掉也照样通过。
$drafted = $rows | Where-Object { $_.态 -eq 'drafted' } | Select-Object -First 1
if ($null -eq $drafted) { throw 'st-drafted.png missing - cannot tell whether the window still lands on body text.' }
if ($drafted.墨点 -le 0) {
  throw 'no dark pixel inside the window for st-drafted: the window drifted off the body text, so **the zeros above mean no ink was measured, not that there is no content**. Scan the dark-pixel y-bands first, then fix Y0/Y1.'
}
