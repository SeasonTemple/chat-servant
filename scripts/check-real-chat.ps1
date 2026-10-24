<#
.SYNOPSIS
    真人聊天**原文**闸：逐字比对暂存区新增行与提交信息。

.DESCRIPTION
    硬约束第 3 条有一半是关于**文字**的：「提交信息、测试夹具、注释、示例代码里
    都不写真实聊天内容」。图片那一半有 `.gitignore` + `pre-commit` 两道；
    **文字那一半曾经一道都没有** ⇒ 2026-10-24 票 93：真机截图里一条真人会话
    气泡的原文被抄进了票面，随提交进了**公开仓**（本仓在 GitHub 上）。

    判据是**字面黑名单**，不是启发式：黑名单 = 本机真帧的 OCR 文本
    （`target/probe/real-chat-blacklist.txt`，**本机私有、gitignored**）。
    「这段文字像不像真人聊天」不可靠判 ⇒ 硬做成守卫只会得到一条永远绿着的假判据
    （`AGENTS.md` 明写）⇒ 本闸只问「这句话在真帧里逐字出现过没有」。

    命中 ⇒ 报红，并打印**命中的那行**与**命中的条目**。

.PARAMETER Blacklist
    黑名单文件路径。默认 `target/probe/real-chat-blacklist.txt`。
    ⚠ 文件**不存在或为空** ⇒ 打横幅、**exit 0**：
    那是「**没有结论**」，不是「没问题」（票 67 的教训）。

.PARAMETER MessageFile
    要扫的提交信息文件。`.githooks/commit-msg` 把它收到的 `$1` 传进来。
    ⚠ 实测 2026-10-24：`pre-commit` 阶段 `.git/COMMIT_EDITMSG` **还不存在**
    （只有上一次的残留），本次提交信息要到编辑器关掉之后才定下来
    ⇒ 能看到**最终**提交信息的 hook 只有 `commit-msg`。

.PARAMETER SkipStaged
    只扫提交信息，不扫暂存区。`commit-msg` 那条路用它
    （暂存区那一半由 `.githooks/pre-commit` 扫）。

.EXAMPLE
    scripts\check-real-chat.ps1
    扫暂存区新增行 + 上一次提交的提交信息。

.EXAMPLE
    scripts\check-real-chat.ps1 -MessageFile .git\COMMIT_EDITMSG -SkipStaged
    只扫提交信息。

.NOTES
    ▸ 豁免口（**明写**的，2026-10-24 父代理拍板加的）

      一行里带 `real-chat-ok:` 且后面跟着**非空理由**时，这行**不拦**。

      用途：真遇到合理命中（某个词恰好也在私聊里出现过）时，
      **用一行注释说明白**，而不是把门槛调回去。

      - 形如：`real-chat-ok: 这个词在设计文档里指的是 UI 元素，不是聊天内容`
      - ⚠ **空理由不算豁免**：`real-chat-ok:` 后面什么都没有 ⇒ 照样拦。
      - ⚠ 豁免**不是静默放过**：输出里会**点出来**（文件、行号、理由）。
      - ⚠ 豁免只对**这一行**生效，且**必须由人明写**——它不是「关掉这道闸」。

    ▸ 退出码

      0 = 没命中（**或**没有结论：缺黑名单）
      1 = 命中
#>

param(
  [string]$Blacklist = '',
  [string]$MessageFile = '',
  [switch]$SkipStaged
)

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

# ⚠⚠ 这句横幅必须与 `.githooks/pre-commit` 与 `.githooks/commit-msg` 里的
#   那份**逐字相同** —— `scripts/check-hooks.ps1` 断言三处都含它。
#   少一处、或两处不一致 ⇒ 门禁红。
$BANNER = '!! 本次没有黑名单 ⇒ 没扫真人原文（缺 target/probe/real-chat-blacklist.txt）'

# ⚠ 豁免标记。**必须**带非空理由，见文件头的帮助文本。
$EXEMPT_MARK = 'real-chat-ok:'

if (-not $Blacklist) { $Blacklist = 'target/probe/real-chat-blacklist.txt' }

# ── 读黑名单 ────────────────────────────────────────────────────────────────
# ⚠ 这个文件里是**真人聊天原文**。它落在 `/target` 下（`.gitignore` 覆盖），
#   本脚本只读它、**从不把它写进别处**。
$entries = @()
if (Test-Path -LiteralPath $Blacklist) {
  # ⚠⚠ **不许**把「以 # 开头的行」当注释丢掉。
  #   实测 2026-10-24：本机黑名单里就有一条**以 # 开头**的真人原文
  #   （聊天里发个话题标签就会这样）⇒ 第一版守卫把它当注释丢了，
  #   147 条只扫了 146 条，**那道闸在漏**。
  #   ⇒ 这份文件是**机器生成**的（refresh 脚本只写条目），没有注释语法。
  $entries = @(Get-Content -LiteralPath $Blacklist -Encoding UTF8 |
    ForEach-Object { $_.Trim() } |
    Where-Object { $_ } |
    Sort-Object -Unique)
}
if ($entries.Count -eq 0) {
  Write-Output $BANNER
  Write-Output ("  （找的是：{0}）" -f $Blacklist)
  Write-Output '  ⇒ 这一步**没有结论**，不是「没问题」。'
  Write-Output '  ⇒ 生成它：pwsh -NoProfile -File scripts\refresh-real-chat-blacklist.ps1'
  exit 0
}

# 最小的黑名单条目长度。用来**先**剔掉明显不可能命中的候选行，
# 免得对每个候选都把整份黑名单扫一遍。
$minLen = ($entries | ForEach-Object { $_.Length } | Measure-Object -Minimum).Minimum

# ── 豁免判定 ────────────────────────────────────────────────────────────────
# 返回理由（非空 ⇒ 这是明写的豁免）；返回 $null ⇒ 不是豁免。
# ⚠ **空理由不算豁免** —— 否则 `real-chat-ok:` 后面漏写就等于白拿一张通行证。
function Get-ExemptReason([string]$line) {
  $m = [regex]::Match($line, [regex]::Escape($EXEMPT_MARK) + '(?<reason>.*)$')
  if (-not $m.Success) { return $null }
  $reason = $m.Groups['reason'].Value.Trim()
  if (-not $reason) { return $null }
  return $reason
}

# ── 收候选行 ────────────────────────────────────────────────────────────────
# 每条候选带一个「来源」标签，命中时能说清它是从哪来的。
$cands = [System.Collections.Generic.List[object]]::new()
# 明写的豁免：记下来，输出时**点出来**（不是静默放过）。
$exempted = [System.Collections.Generic.List[object]]::new()

# ① 暂存区新增行。
# ⚠ `+++ b/<path>` 是**文件头**，不是新增行 —— 但真新增的行也可能以 `++` 开头。
#   ⇒ 判据是「上一行是不是 `--- `」，不是「这一行是不是以 `+++` 开头」。
$diff = @()
if (-not $SkipStaged) {
  $diff = @(git diff --cached --unified=0 --no-color 2>$null)
}
$expectFileHeader = $false
$currentFile = ''
$newLine = 0
foreach ($l in $diff) {
  if ($l.StartsWith('--- ')) { $expectFileHeader = $true; continue }
  if ($expectFileHeader -and $l.StartsWith('+++ ')) {
    $expectFileHeader = $false
    $currentFile = $l.Substring(4).Trim()
    if ($currentFile -eq '/dev/null') { $currentFile = '' }
    continue
  }
  $expectFileHeader = $false
  # hunk 头给出新文件里这一段的起始行号 ⇒ 命中时报得出「第几行」。
  if ($l -match '^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@') {
    $newLine = [int]$Matches[1]
    continue
  }
  if (-not $l.StartsWith('+')) {
    # 上下文行会推进新文件的行号；删除行不推进。
    if ($l.StartsWith(' ')) { $newLine++ }
    continue
  }
  $text = $l.Substring(1)
  $at = $newLine
  $newLine++
  if ($text.Length -lt $minLen) { continue }
  $reason = Get-ExemptReason $text
  if ($reason) {
    $exempted.Add([pscustomobject]@{ Source = '暂存区新增行'; File = $currentFile; At = $at; Reason = $reason })
    continue
  }
  $cands.Add([pscustomobject]@{ Source = '暂存区新增行'; File = $currentFile; At = $at; Line = $text })
}

# ② 提交信息。
# ⚠ 这次提交的信息（`-MessageFile`，`commit-msg` hook 传进来）与
#   **上一次**的提交信息（`git log -1`）是两回事，分开标来源。
if ($MessageFile) {
  if (Test-Path -LiteralPath $MessageFile) {
    $i = 0
    foreach ($l in @(Get-Content -LiteralPath $MessageFile -Encoding UTF8)) {
      $i++
      if ($l.Length -lt $minLen) { continue }
      $reason = Get-ExemptReason $l
      if ($reason) {
        $exempted.Add([pscustomobject]@{ Source = '本次提交信息'; File = ''; At = $i; Reason = $reason })
        continue
      }
      $cands.Add([pscustomobject]@{ Source = '本次提交信息'; File = ''; At = $i; Line = $l })
    }
  }
}
$prev = @(git log -1 --pretty=%B 2>$null)
$i = 0
foreach ($l in $prev) {
  $i++
  if ($l.Length -lt $minLen) { continue }
  $reason = Get-ExemptReason $l
  if ($reason) {
    $exempted.Add([pscustomobject]@{ Source = '上一次提交的提交信息'; File = ''; At = $i; Reason = $reason })
    continue
  }
  $cands.Add([pscustomobject]@{ Source = '上一次提交的提交信息'; File = ''; At = $i; Line = $l })
}

# ── 逐字比对 ────────────────────────────────────────────────────────────────
# 判据是**子串**匹配：抄来的那句话哪怕前后还带着别的字，也得拦下。
# ⚠ 大小写敏感（Ordinal）。这是有意的：黑名单是「这一句话」，
#   放宽成不分大小写会多出一批「同一个词但不同句」的误报。
# ⚠ 用 **hashtable** 而不是 List：判重要用「来源+文件+行+条目」四元组
#   （同一行可能命中多个条目，都要报出来）。
$hits = @{}
foreach ($c in $cands) {
  foreach ($e in $entries) {
    if ($c.Line.Length -lt $e.Length) { continue }
    if ($c.Line.Contains($e, [StringComparison]::Ordinal)) {
      # ⚠⚠ **不要 `break`**（2026-10-24 改）。
      #   原来一行只报**第一条**命中的条目（理由是「报全了只是噪音」）
      #   ⇒ 同一行上的**其它**条目被掩盖 ⇒ 实测严格扫描比守卫多报 13 处，
      #   并多浮出 2 个条目（都是本产品自己的 UI 文案）。
      #   ⇒ 现在**报全**：一行命中几条就报几条（同一条目重复命中去重）。
      $key = "$($c.Source)|$($c.File)|$($c.At)|$e"
      if (-not $hits.ContainsKey($key)) {
        $hits[$key] = [pscustomobject]@{ Source = $c.Source; File = $c.File; At = $c.At; Line = $c.Line; Entry = $e }
      }
    }
  }
}

if ($hits.Count -gt 0) {
  Write-Output '✗ 真人聊天原文闸：命中了本机真帧里的**真人聊天原文**。'
  Write-Output '  （硬约束第 3 条：提交信息、夹具、注释、示例代码里都不写真实聊天内容。）'
  Write-Output ''
  foreach ($h in $hits) {
    Write-Output ("  来源：{0}{1}" -f $h.Source, $(if ($h.File) { "（{0}:{1}）" -f $h.File, $h.At } else { "（第 {0} 行）" -f $h.At }))
    Write-Output ("  命中的那行：{0}" -f $h.Line)
    Write-Output ("  命中的条目：{0}" -f $h.Entry)
    Write-Output ''
  }
  Write-Output '  ⇒ 把这句从提交里去掉。引用真帧时引用它的**位置/形状/颜色**'
  Write-Output '    （右对齐的绿色气泡、那条时间戳），**别引用它的字**。'
  Write-Output '  ⇒ 这不是误报判定，是**逐字命中**：那句话在本机真帧里出现过。'
  Write-Output ("  ⇒ 确实要提交它？在本行末尾写 `{0} <非空理由>`，这一行就不拦。" -f $EXEMPT_MARK)
  Write-Output '    （豁免是**明写**的，理由不能空；输出里会把它点出来。）'
  exit 1
}

if ($exempted.Count -gt 0) {
  Write-Output ("注意：有 {0} 行带了**明写的豁免**（{1} + 非空理由）⇒ 这几行没拦。" -f $exempted.Count, $EXEMPT_MARK)
  foreach ($e in $exempted) {
    $where = if ($e.File) { "{0}:{1}" -f $e.File, $e.At } else { "{0} 第 {1} 行" -f $e.Source, $e.At }
    Write-Output ("  (豁免) {0} —— 理由：{1}" -f $where, $e.Reason)
  }
  Write-Output ''
}

Write-Output ("真人聊天原文闸：{0} 条候选行 × {1} 条黑名单条目，逐字比对 —— 没有命中。" -f $cands.Count, $entries.Count)
exit 0
