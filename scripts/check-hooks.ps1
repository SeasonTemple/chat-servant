# 检查 `.githooks/pre-commit` 这个凭据闸本身还在不在、判据还是不是那样。
#
# ## 为什么要有这一步
#
# 凭据闸是硬约束第 7 条的**最后一道防线**（2026-10-18 实测：
# `grep -n 'githooks|pre-commit' scripts/gate.ps1` **零命中**）。
# 它整个消失、或判据被改坏，门禁一声不吭——
# 而「守卫本身没有守卫」正是本仓反复付过学费的同一族问题。
#
# ## 三条判据，各自对应一种真实失效
#
# | 红 | 现实含义 |
# |---|---|
# | 闸门文件不在 / 判据被抹掉 | 硬约束第 7 条的最后一道防线没了 |
# | 闸门里出现**完整的**豁免标记 | 它开始给自己开免检（2026-10-18 真的发生过） |
# | 图片白名单与仓库实际图片不一致 | 白名单过期了，或漏了一张图没登记 |
#
# ⚠ 判据用**退出码**，不要用输出文本匹配（PS 5.1 的 `-match` 默认不区分
# 大小写，见 ADR-0006 第（四）节那串「拿两把尺子比」的教训）。
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$hookPath = '.githooks/pre-commit'
$fail = [System.Collections.Generic.List[string]]::new()

# ── ① 闸门在不在、判据还在不在 ──────────────────────────────────────
if (-not (Test-Path $hookPath)) {
  Write-Output "找不到 $hookPath —— 硬约束第 7 条的最后一道防线没了。"
  Write-Output '恢复它见 .githooks/README 或 CONTRIBUTING §维护陷阱。'
  exit 1
}
$hook = [IO.File]::ReadAllText((Resolve-Path $hookPath))

# 判据取**判据本身**，不取步的名字（本地与 CI 的步名本来就故意不一样）。
foreach ($needle in @('sk-[', 'ghp_', 'BEGIN', 'marker=', 'git show ":$f"')) {
  if (-not $hook.Contains($needle)) {
    $fail.Add("闸门里找不到判据片段 '$needle' —— 判据被改坏了或被删了")
  }
}
# 两段式 marker：`marker=` 必须出现**两次**（拆分 + 拼接）。
$nMarker = ([regex]::Matches($hook, '(?m)^\s*marker=')).Count
if ($nMarker -lt 2) {
  $fail.Add("闸门里 `marker=` 只出现 $nMarker 次 —— 两段式拼接被合并回去了")
  $fail.Add('    ⇒ 一旦完整标记出现在本文件里，闸门就给自己开免检（commit 9cf53f9 修的就是这个）')
}

# ── ② 闸门是否开始给自己开免检 ──────────────────────────────────────
# ⚠ 判据是「去掉注释之后，闸门正文里有没有完整标记」。
#   不剥注释的话，**讲这件事的注释本身**就会让这条判据永远红——
#   而它红着又会被 --no-verify 习惯掉，等于没有。
$hookCode = (($hook -split "`r?`n") | Where-Object { $_ -notmatch '^\s*#' }) -join "`n"
if ($hookCode -match 'secret-scan\s*:\s*allow') {
  $fail.Add('闸门正文里出现了完整的豁免标记 —— 它给自己开免检了')
  $fail.Add('    ⇒ 往本文件里粘真 key 将不会被告警拦下。marker 必须保持两段拼接。')
}

# ── ③ 图片白名单与仓库实际图片是否一致 ──────────────────────────────
# 「白名单过期」是这一条要抓的东西：白名单里列着、仓库里却没有 ⇒
# 说明某张图被删/改名了，白名单该跟着清理。
#
# ⚠ 反过来（仓库里有、白名单没列）**不由这条抓**：那种情况下
# pre-commit 自己就会拦下提交，人必须显式改白名单才会放行——
# 那个摩擦是**有意**的，不该由本守卫代劳。
$tracked = @(git ls-files | Where-Object { $_ -match '(?i)\.(png|jpe?g|bmp|webp|gif)$' })
if ($tracked.Count -eq 0) {
  # 量具自检：一张都列不出来，说明扫描面本身失效了，不许当合规放过。
  $fail.Add('一条图片都没列出来 —— 扫描面失效了，本守卫此刻什么都没验。')
}

# 白名单里点名的条目。⚠ **一条白名单行里可能有多个模式**
#   （hook 里写的是 `ui/static/probe-sheet.png|ui/static/strip.png)`），
#   第一版正则只匹配行末那一个 ⇒ 两个合法图片全被报成「不在白名单」，
#   报了个**假红**。⚠ 那是量具坏了，不是事实坏了——
#   「量具失效」与「事实真的不对」在屏幕上原本长得一模一样。
$entry = 'ui/[\w./-]+(?:/\*|\.(?:png|jpg|jpeg|bmp|webp|gif))'
$entries = [regex]::Matches($hook, "(?m)^\s+($entry(?:\|$entry)*)\s*\)") |
  ForEach-Object { $_.Groups[1].Value -split '\|' } |
  Sort-Object -Unique

$imgExt = '\.(png|jpg|jpeg|bmp|webp|gif)$'
# 具体文件（逐个点名的那几条）
$listed = @($entries | Where-Object { $_ -match "(?i)$imgExt" })
# 目录通配（整目录放行的那几条）
$dirs = @($entries | Where-Object { $_ -notmatch "(?i)$imgExt" })

if ($entries.Count -eq 0) {
  # 量具自检：一条白名单都没解析出来 ⇒ 本守卫此刻什么都没验，不许当合规放过。
  $fail.Add('闸门里一条白名单模式都没解析出来 —— 本守卫此刻什么都没验。')
  Write-Output '凭据闸这道防线本身出了问题：'
  $fail | ForEach-Object { Write-Output $_ }
  exit 1
}

$covered = $tracked | Where-Object { $f = $_
  ($dirs | Where-Object { $f.StartsWith(($_ -replace '\*$', '')) }) -or
  ($listed -contains $f) }
foreach ($f in $tracked) {
  if ($covered -notcontains $f) {
    $fail.Add("仓库跟踪着图片 $f，但闸门的白名单没有它")
    $fail.Add('    ⇒ 改它时 pre-commit 会拦（那是**有意的摩擦**）；把它显式加进白名单即可。')
  }
}
foreach ($l in $listed) {
  if ($tracked -notcontains $l) {
    $fail.Add("白名单里点名的 $l 在仓库里已不存在 —— 白名单过期了，该清理")
  }
}

if ($fail.Count) {
  Write-Output '凭据闸这道防线本身出了问题：'
  Write-Output ''
  $fail | ForEach-Object { Write-Output $_ }
  exit 1
}

Write-Output ("凭据闸：判据片段齐全、marker 两段式（未自豁免）、" +
               "图片白名单与仓库实际 {0} 张一致。" -f $tracked.Count)
exit 0
