# 扫源码与文档里的 U+FFFD（Unicode 替换字符）。
#
# 为什么值得单独立一步：**受损的全是注释与文档**，所以 fmt / clippy / test /
# svelte-check 一样绿，损坏能静静躺着一直没人发现。
#
# 成因见 ticket 19：PS 5.1 的 `(Get-Content -Raw) -replace … | Set-Content
# -Encoding UTF8` 做机械替换，UTF-8 被按 ANSI 读回再写——字符已经丢了，
# 事后无法从文件里还原。
#
# **只扫项目自己的目录，不扫 git ls-files 的全量。**
# `.agents/` `.claude/` `.minimax/` `.zcode/` 是 CLI 为别的运行时装的，四份
# 共 3150 个文件；`git ls-files` 全量逐个 ReadAllText 要几十秒，而那里由
# `skills-lock.json` 管版本，改内容不是本仓库该做的事。

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$ext = '\.(rs|svelte|ts|js|md|json|toml|ps1|py|css|html|yml|yaml)$'
# `ui\scripts` 必须在这里（2026-10-13 补）。原先只有 `ui\src`，
#   而 `ui/scripts/` 恰好是**中文注释最密的地方**——量具脚本
#   （`sheetprobe.ps1` / `pencilgen.py` / `icongen.py`）每一条都在解释
#   「为什么这么量」，全是长句中文，最经不起编码往返。
#   漏掉它的实测证据：那天 `stateink.ps1` 里一个 U+FFFD 就摆在注释里，
#   而这一步扫了 179 个文件报「干净」。
#   同理补了 `py` 扩展——`pencilgen.py` / `icongen.py` 也在这个目录里。
$roots = @('crates', 'xtask', 'ui\src', 'ui\src-tauri\src', 'ui\scripts', 'scripts', 'docs', '.scratch')
$files = @()
foreach ($r in $roots) {
  if (Test-Path $r) {
    $files += Get-ChildItem -Path $r -Recurse -File | Where-Object { $_.FullName -notmatch '\\target\\|node_modules|\.svelte-kit' }
  }
}
$files += Get-ChildItem -Path . -File -Include *.md,*.toml -ErrorAction SilentlyContinue
$files += Get-ChildItem -Path ui -File -Include *.json,*.ts,*.js -ErrorAction SilentlyContinue
$files = $files | Where-Object { $_.Name -match $ext } | Sort-Object FullName -Unique

$hits = @()
foreach ($f in $files) {
  try { $c = [IO.File]::ReadAllText($f.FullName) } catch { continue }
  if ($c -match "\uFFFD") {
    $n = ([regex]::Matches($c, "\uFFFD")).Count
    $idx = ([regex]::Match($c, "\uFFFD")).Index
    $ln = ($c.Substring(0, $idx) -split "`n").Count
    $hits += ('  {0}:{1}  {2} 个 U+FFFD' -f $f.FullName.Replace((Get-Location).Path + '\', ''), $ln, $n)
  }
}

Write-Output ("扫了 {0} 个文件。" -f $files.Count)
if ($hits.Count -gt 0) {
  Write-Output "发现已损坏字符（U+FFFD）："
  $hits | ForEach-Object { Write-Output $_ }
  Write-Output ''
  Write-Output '这些位置编译得过、测试也跑得过——受损的是注释与文档，所以没有任何'
  Write-Output '其他门禁会报它们。处理办法见 .scratch/…/19-mojibake-in-source-comments.md'
  exit 1
}

Write-Output 'U+FFFD 扫描：干净'
exit 0