$ErrorActionPreference = 'Continue'
$root = Split-Path $PSScriptRoot -Parent
Set-Location (Join-Path $root 'ui')
$log = Join-Path $root 'target\app-fixture.log'
Remove-Item $log -ErrorAction SilentlyContinue

# 夹具模式：一帧冻结的**真实**微信画面，循环每轮都读它。
# 于是 OCR、归类、会话锁定、起草、事件推送、界面渲染一条都不少，
# 只有「这一帧从哪来」变了——不需要去点微信。
$fixture = $args[0]
if (-not $fixture) { throw "用法: run-app-fixture.ps1 <png 路径>" }
$env:CHATSERVANT_FIXTURE = (Resolve-Path $fixture).Path
$env:RUST_LOG = 'info,chat_servant=debug'

$p = Start-Process -FilePath 'pnpm.cmd' -ArgumentList 'tauri', 'dev' `
    -WorkingDirectory (Join-Path $root 'ui') `
    -RedirectStandardOutput $log -RedirectStandardError "$log.err" `
    -WindowStyle Hidden -PassThru
"tauri dev pid=$($p.Id)"
"夹具 = $env:CHATSERVANT_FIXTURE"
"日志 = $log"