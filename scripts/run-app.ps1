$ErrorActionPreference = 'Continue'
$root = Split-Path $PSScriptRoot -Parent
Set-Location (Join-Path $root 'ui')
$log = Join-Path $root 'target\app.log'
New-Item -ItemType Directory -Force -Path (Split-Path $log) | Out-Null
Remove-Item $log -ErrorAction SilentlyContinue

# ⚠ 用 `Start-Process -RedirectStandardOutput` 而不是管道 + Tee。
# 管道那头一断（后台任务被取消、终端关闭），Tee 就跟着死，
# 应用还在跑却再也写不出日志——上一轮排查被这个坑误导过。
$env:RUST_LOG = 'info,chat_servant=debug'
$p = Start-Process -FilePath 'pnpm.cmd' -ArgumentList 'tauri', 'dev' `
    -WorkingDirectory (Join-Path $root 'ui') `
    -RedirectStandardOutput $log -RedirectStandardError "$log.err" `
    -WindowStyle Hidden -PassThru
"tauri dev pid=$($p.Id)"
"日志：$log"