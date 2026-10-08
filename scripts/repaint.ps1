# 强制整窗重绘（含所有子窗口）。
# 用途：面板上「有的区域更新了、有的区域还是旧的」时，要先分清
#       「状态没变」和「变了但没重绘」——前者是逻辑问题，
#       后者是 WebView2/DWM 合成问题，两者的修法完全不同。
# 用法: repaint.ps1 <hwnd>
param([Parameter(Mandatory)][string]$Hwnd)

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class RP {
    [DllImport("user32.dll")] public static extern bool RedrawWindow(IntPtr h, IntPtr r, IntPtr rgn, uint flags);
    [DllImport("user32.dll")] public static extern bool InvalidateRect(IntPtr h, IntPtr r, bool erase);
    [DllImport("user32.dll")] public static extern bool UpdateWindow(IntPtr h);
}
"@
$h = [IntPtr][Convert]::ToInt64($Hwnd, 16)
# RDW_INVALIDATE 1 | RDW_ERASE 4 | RDW_ALLCHILDREN 0x80 | RDW_UPDATENOW 0x100 | RDW_FRAME 0x400
$flags = 0x00000001 -bor 0x00000004 -bor 0x00000080 -bor 0x00000100 -bor 0x00000400
[void][RP]::InvalidateRect($h, [IntPtr]::Zero, $true)
$ok = [RP]::RedrawWindow($h, [IntPtr]::Zero, [IntPtr]::Zero, $flags)
[void][RP]::UpdateWindow($h)
"RedrawWindow($Hwnd) = $ok"