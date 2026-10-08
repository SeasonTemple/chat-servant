# 把前台切到某个窗口（用于验证「别的程序能不能在不抢前台的前提下操作它」）。
# 用法: focuswin.ps1 <hwnd>
param([Parameter(Mandatory)][string]$Hwnd)

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class FW {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool c);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
}
"@
$h = [IntPtr][Convert]::ToInt64($Hwnd, 16)
$me = [FW]::GetCurrentThreadId()
# 借前台线程的输入队列，否则 SetForegroundWindow 会被 Windows 前台锁拒绝
$proc = 0
$tid = [FW]::GetWindowThreadProcessId($h, [ref]$proc)
[void][FW]::AttachThreadInput($me, $tid, $true)
[void][FW]::ShowWindow($h, 9)
$ok = [FW]::SetForegroundWindow($h)
[void][FW]::AttachThreadInput($me, $tid, $false)
$fg = [FW]::GetForegroundWindow()
"SetForegroundWindow=$ok  现在前台=0x$('{0:X}' -f $fg.ToInt64())（目标 0x$('{0:X}' -f $h.ToInt64())）"