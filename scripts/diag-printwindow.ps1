$ErrorActionPreference = "Continue"
Set-Location "E:\devSpace\chat-servant-m31f"

Write-Output "=== 微信 + PrintWindow ==="
cargo xtask probe frame --method printwindow 2>&1 | Select-Object -Last 22
Write-Output "WECHAT_PW_EXIT=$LASTEXITCODE"

Write-Output ""
Write-Output "=== 记事本 + PrintWindow（对照）==="
$p = Start-Process notepad -PassThru
Start-Sleep -Seconds 2
$p.Refresh()
$np = $p.MainWindowHandle.ToInt64()
Write-Output ("notepad hwnd=0x{0:X}" -f $np)
cargo xtask probe frame --method printwindow --hwnd ("0x" + $np.ToString("X")) 2>&1 | Select-Object -Last 18
Write-Output "NOTEPAD_PW_EXIT=$LASTEXITCODE"
Stop-Process -Name notepad -Force -ErrorAction SilentlyContinue

Write-Output ""
Write-Output "=== 产物 ==="
Get-ChildItem target\probe\*.png -ErrorAction SilentlyContinue | Select-Object Name, Length | Format-Table -AutoSize