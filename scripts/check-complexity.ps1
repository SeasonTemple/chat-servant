# 复杂度与函数长度门禁（lizard：CCN <= 15, length <= 100, 0 告警）。
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

if (Get-Command uv -ErrorAction SilentlyContinue) {
  & uv run --no-project --with lizard==1.25.1 lizard crates ui/src-tauri/src -C 15 -L 100 -w
} else {
  & python -m lizard crates ui/src-tauri/src -C 15 -L 100 -w
}

if ($LASTEXITCODE -ne 0) {
  Write-Output "复杂度门禁未通过：crates/ 或 ui/src-tauri/src 存在超过阈值（CCN > 15 或 length > 100）的函数。"
  exit 1
}

Write-Output "复杂度门禁：crates/ 与 ui/src-tauri/src 全数通过（CCN <= 15, length <= 100，0 告警）"
exit 0
