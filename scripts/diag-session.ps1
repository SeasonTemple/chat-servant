$ErrorActionPreference = "Continue"
Set-Location "E:\devSpace\chat-servant-m31f"

Write-Output "=== 会话类型 ==="
Write-Output ("SESSIONNAME = " + $env:SESSIONNAME)
Write-Output ("USERNAME    = " + $env:USERNAME)
try { qwinsta 2>&1 | Out-String | Write-Output } catch { Write-Output "qwinsta n/a" }

Write-Output ""
Write-Output "=== GPU / 图形驱动 ==="
Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue |
  Select-Object Name, DriverVersion, VideoProcessor | Format-List

Write-Output "=== 当前进程是否在交互式桌面 ==="
Write-Output ("ProcessId = " + $PID)
Write-Output ("[环境变量 SESSIONNAME 为空 = 非 RDP 交互会话的常见特征]")