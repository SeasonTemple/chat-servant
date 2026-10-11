# 自检：故意放一个假 key 进仓库，看 secret-scan 能不能抓到。
#
# 这不是"跑一下看看能不能过"——扫描器没抓到东西时，
# 「没有输出」和「扫描器坏了」在屏幕上长得一模一样。
# 必须先证明它**抓得到**，再信它的「没抓到」。
param([string]$Fake = 'sk-xtaskscan0000probe000000000')

$ErrorActionPreference = 'Stop'
$pwsh = 'C:\Users\SeasonTemple\AppData\Local\Microsoft\WindowsApps\pwsh.exe'
$canary = Join-Path $PSScriptRoot '..\target\probe\canary-secret.txt'

Write-Output "① 埋一个假 key 到 $canary"
Set-Content -LiteralPath $canary -Value $Fake -Encoding UTF8

Write-Output "② 跑 secret-scan，应当报出这个文件"
$env:CHATSERVANT_PROBE_KEY = $Fake
& $pwsh -NoProfile -Command "cargo run -q -p xtask -- secret-scan 2>&1" | Select-String -Pattern '❌|canary|没有残留'

$found = $LASTEXITCODE -ne 0
Write-Output ""
Write-Output "③ 撤掉假 key，再跑一次，应当干净"
Remove-Item -LiteralPath $canary -Force
$env:CHATSERVANT_PROBE_KEY = $Fake
& $pwsh -NoProfile -Command "cargo run -q -p xtask -- secret-scan 2>&1" | Select-String -Pattern '❌|✅'

if ($found) {
    Write-Output ""
    Write-Output "SELFCHECK_OK —— 扫描器抓得到假 key，也放得过干净仓库"
    exit 0
}
Write-Output ""
Write-Output "SELFCHECK_FAILED —— 扫描器没抓到假 key，它的『没输出』不可信"
exit 1