# 生成「真人聊天原文」的**字面黑名单**。
#
# ## 这份黑名单是什么
#
# 本机真帧（`target/probe/*.png`）里 OCR 读到的**真人会话原文**，
# 一行一条，落在 `target/probe/real-chat-blacklist.txt`。
#
# ⚠⚠ **这份文件本身是真人聊天内容。**
#   ⇒ 它必须待在 gitignored 目录里（`/target`），**绝不能**进版本库。
#   ⇒ 本脚本、守卫脚本、票面里都**不许**抄它的内容 —— 只写代码路径。
#   ⇒ 验收时用 `git check-ignore -v` + `git ls-files` 确认它没被跟踪。
#
# ## 为什么是「字面黑名单」而不是启发式
#
# 「这段文字像不像真人聊天」不可靠判 ⇒ 硬做成守卫只会得到一条永远绿着的假判据
# （`AGENTS.md` 明写）。所以判据是**逐字比对**：
# 「这句话在真帧里逐字出现过没有」。
#
# ## 多久刷一次
#
# 抓了新帧、或换了会话之后刷一次。它是**快照**，不是长驻日志：
# 真帧被删 ⇒ 黑名单变短 ⇒ 守卫会变松（已知局限，见票 93）。
#
# ## 排除哪些帧
#
# 排除**演示态 / 合成产物**：`*-sheet*`（排版稿）、名字含 `probe-`、
# `fixture-`（夹具副本）、`demo`（演示态出图）。
# ⚠ 但真正把住关口的其实是另一件事：`perceive` 走的是**微信窗口**的生产接缝，
#   锚点推不出来的帧（非微信窗口的截图）会**跑失败**并被跳过并计数。
#   ⇒ 演示夹具与应用自身界面的截图进不来，不靠文件名猜。
#
# 退出码：0 = 黑名单已生成（哪怕是空的）；1 = 一帧都没跑成。

param(
  [string]$ProbeDir = 'target/probe',
  [string]$OutFile = 'target/probe/real-chat-blacklist.txt',
  # ⚠ 2026-10-24 父代理拍板：门槛从 8 降到 **6**。
  #   理由：票 93 泄漏的是**六个字**的原文 ⇒ 8 的门槛**抓不到它**；
  #   而黑名单里的句子来自真机自己的会话，那些句子几乎不会出现在
  #   代码/注释里 ⇒ 6 的门槛**误报风险低**，而漏报的代价是硬约束第 3 条被破。
  #   ⇒ 门槛**不该**再当"降噪旋钮"用；真遇到合理命中用**明写的豁免口**
  #     （`real-chat-ok:` + 非空理由），见 scripts/check-real-chat.ps1 的帮助文本。
  [int]$MinLen = 6,
  [switch]$UseCargo,
  [string]$Xtask = ''
)

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

# ── 挑一个能跑的 xtask ──────────────────────────────────────────────────────
# ⚠ 默认**优先用已经编好的 `target/debug/xtask.exe`**：本脚本要跑上百次
#   `perceive`，每次都走 `cargo run` 就要**上百次去抢 cargo 的构建锁**——
#   别人正在编译时它会一路等着。
#   只有当 exe 比最新的 Rust 源码**旧**时才退回 `cargo xtask`（那会重新编）。
$srcNewest = Get-ChildItem -Path 'crates', 'xtask' -Recurse -File -Filter '*.rs' -ErrorAction SilentlyContinue |
  Sort-Object LastWriteTime -Descending | Select-Object -First 1
$exe = 'target/debug/xtask.exe'
$useCargo = $UseCargo.IsPresent
if ($Xtask) {
  $exe = $Xtask
} elseif (-not $useCargo) {
  if (-not (Test-Path -LiteralPath $exe)) {
    $useCargo = $true
  } elseif ($srcNewest -and (Get-Item -LiteralPath $exe).LastWriteTime -lt $srcNewest.LastWriteTime) {
    Write-Output ("xtask.exe 比最新的源码旧（{0}）⇒ 退回 cargo xtask 重编。" -f $srcNewest.Name)
    $useCargo = $true
  }
}
if ($useCargo) { Write-Output '用的是：cargo xtask（每次都要过一遍 cargo 的构建锁，慢）' }
else { Write-Output ("用的是：{0}" -f $exe) }

# ── 收帧 ────────────────────────────────────────────────────────────────────
if (-not (Test-Path -LiteralPath $ProbeDir)) {
  Write-Output ("找不到 {0} —— 没有真帧就没有黑名单。" -f $ProbeDir)
  Write-Output '  ⇒ 先抓一帧：cargo xtask probe frame --scene-confirmed'
  exit 1
}
$frames = @(Get-ChildItem -LiteralPath $ProbeDir -File -Filter '*.png' | Sort-Object Name)
if ($frames.Count -eq 0) {
  Write-Output ("{0} 下一张 png 都没有 —— 没有真帧就没有黑名单。" -f $ProbeDir)
  exit 1
}

# ⚠ 排除项是**点名**的，不是「像不像」的启发式。
$excludeRe = '(?i)(sheet|probe-|fixture-|demo)'
$skipped = [System.Collections.Generic.List[string]]::new()
$todo = @()
foreach ($f in $frames) {
  if ($f.Name -match $excludeRe) { $skipped.Add($f.Name); continue }
  $todo += $f
}
Write-Output ("帧：共 {0} 张，排除演示态/合成产物 {1} 张，待扫 {2} 张。" -f $frames.Count, $skipped.Count, $todo.Count)

# ── 解析 perceive 的输出 ────────────────────────────────────────────────────
# perceive 打的是一张固定列的表：# / y 范围 / 说话人 / 归类 / 依据 / 文字。
#
# ⚠⚠ **不能按列偏移切**：依据那一列会溢出（Rust 的 `{:<30}` 是**最小**宽度，
#   不是最大宽度）⇒ 按偏移切会把依据尾巴当成文字。
#   ⇒ 改用**依据自己的结尾**当锚点：依据的格式固定以「高=N」收尾
#   （见 `crates/perceive/src/attribute.rs` 里的 Display 实现），
#   它后面那个空格之后就是**文字**。
$rowRe = [regex]'^\s*\d+\s+\d+-\d+\s+\(\d+\.\.\d+\)\s+\S+\s+\S+\s+.*?高=\d+\s+(?<txt>\S.*)$'
# 「归并后的消息」那一段也收一遍（形如 `  [我] (x,y)-(x,y)  文字`）：
# 那是**正文**（真人说的），而表格那一列还混着标题、灰字、图内字。
$msgRe = [regex]'^\s*\[(?:我|对方)\]\s+\(\d+,\d+\)-\(\d+,\d+\)\s+(?<txt>\S.*)$'

$collected = [System.Collections.Generic.List[string]]::new()
$ok = 0
$failed = 0
$unparsed = 0
$withText = 0

foreach ($f in $todo) {
  $rel = $f.FullName
  if ($useCargo) {
    $raw = & cargo xtask perceive --file $rel 2>&1
  } else {
    $raw = & $exe perceive --file $rel 2>&1
  }
  $code = $LASTEXITCODE
  if ($code -ne 0) {
    # ⚠ 跑失败**跳过并计数**，不要整体失败：不是每一张 png 都是微信窗口。
    $failed++
    continue
  }
  $ok++
  $lines = @($raw | ForEach-Object { if ($_ -is [string]) { $_ } else { $_.ToString() } })
  $hit = 0
  foreach ($l in $lines) {
    $m = $rowRe.Match($l)
    if (-not $m.Success) {
      $m = $msgRe.Match($l)
      if (-not $m.Success) {
        # 表格里的一行没能解析 ⇒ 记数（量具自己坏掉时要看得见）。
        if ($l -match '^\s*\d+\s+\d+-\d+\s+\(\d+\.\.\d+\)') { $unparsed++ }
        continue
      }
    }
    $t = $m.Groups['txt'].Value.Trim()
    if ($t.Length -lt $MinLen) { continue }
    $collected.Add($t)
    $hit++
  }
  if ($hit -gt 0) { $withText++ }
}

$entries = @($collected | Sort-Object -Unique)

# ── 写盘 ────────────────────────────────────────────────────────────────────
# ⚠ 用 WriteAllText（不是 WriteAllLines）：后者按 Windows 默认写 **CRLF**。
$outDir = Split-Path -Parent $OutFile
if ($outDir -and -not (Test-Path -LiteralPath $outDir)) {
  New-Item -ItemType Directory -Path $outDir -Force | Out-Null
}
$text = if ($entries.Count -gt 0) { ($entries -join "`n") + "`n" } else { '' }
[IO.File]::WriteAllText((Join-Path (Get-Location).Path $OutFile), $text, [Text.UTF8Encoding]::new($false))

Write-Output ''
Write-Output ("跑成 {0} 张，跑失败（跳过）{1} 张，其中有字 {2} 张，表格行没解析出来 {3} 行。" -f $ok, $failed, $withText, $unparsed)
Write-Output ("黑名单：{0} 条（长度 ≥ {1}），写入 {2}" -f $entries.Count, $MinLen, $OutFile)
if ($skipped.Count -gt 0) {
  Write-Output ("排除的演示态/合成产物：{0}" -f ($skipped -join '、'))
}
Write-Output ''
Write-Output '⚠ 这份文件里是**真人聊天原文**，它只该待在 gitignored 的 target/ 下。'
Write-Output '  ⇒ 确认它没被跟踪：git ls-files -- target/probe/real-chat-blacklist.txt（应为空）'
Write-Output '  ⇒ 确认它被忽略：git check-ignore -v target/probe/real-chat-blacklist.txt（应有输出）'
exit 0
