$ErrorActionPreference = "Continue"
Set-Location "E:\devSpace\chat-servant-m31f"

Write-Output "=== 1) 启动记事本作对照 ==="
$p = Start-Process notepad -PassThru
Start-Sleep -Seconds 2
$p.Refresh()
$npHwnd = $p.MainWindowHandle
Write-Output ("notepad pid={0} hwnd=0x{1:X}" -f $p.Id, $npHwnd.ToInt64())

Write-Output ""
Write-Output "=== 2) 抓记事本 ==="
cargo xtask probe frame --hwnd ("0x" + $npHwnd.ToInt64().ToString("X")) 2>&1 | Select-Object -Last 20
Write-Output "NOTEPAD_EXIT=$LASTEXITCODE"

Write-Output ""
Write-Output "=== 3) 抓微信（设置已改为 Default）==="
cargo xtask probe frame 2>&1 | Select-Object -Last 20
Write-Output "WECHAT_EXIT=$LASTEXITCODE"

Write-Output ""
Write-Output "=== 4) 微信版本 ==="
Get-Item "C:\Program Files\Tencent\Weixin\Weixin.exe" -ErrorAction SilentlyContinue |
  Select-Object -ExpandProperty VersionInfo | Select-Object ProductVersion, FileVersion | Format-List
Get-Item "${env:ProgramFiles}\Tencent\Weixin\Weixin.exe" -ErrorAction SilentlyContinue |
  Select-Object -ExpandProperty VersionInfo | Select-Object ProductVersion, FileVersion | Format-List
Get-Process weixin -ErrorAction SilentlyContinue | Select-Object -First 1 Path | Format-List

Stop-Process -Name notepad -Force -ErrorAction SilentlyContinue