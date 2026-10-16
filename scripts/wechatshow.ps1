# 把微信主窗口从托盘隐藏状态拉出来，供 WGC 抓帧。
#
# 为什么需要它
# ------------
# `winscan.ps1` 只报**可见**顶层窗口，而微信在托盘模式下主窗口
# `Visible=False`（实测 2026-10-14：`0x3D02FC Qt51514QWindowIcon 标题「微信」
# 1267x930 Visible=False Iconic=False`）—— 不是最小化，是**隐藏**。
# ⇒ 只按可见窗口找，永远找不到它，会误判成「微信没开」。
#
# 这会**动桌面**：把微信窗口显示出来并前置。做真机取证时需要它，
#   因为 WGC 抓帧要求窗口可见且在屏幕上。
param(
  [int]$Hwnd = 0x3D02FC
)

$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class WX {
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
'@

$h = [IntPtr]$Hwnd
if (-not [WX]::IsWindowVisible($h)) {
  # SW_RESTORE = 9：先恢复（应对最小化），再 SW_SHOW = 5（应对隐藏）
  [void][WX]::ShowWindow($h, 9)
  [void][WX]::ShowWindow($h, 5)
  Start-Sleep -Milliseconds 400
}
[void][WX]::SetForegroundWindow($h)
Start-Sleep -Milliseconds 600

$r = New-Object WX+RECT
[void][WX]::GetWindowRect($h, [ref]$r)
"hwnd      = 0x{0:X}" -f $Hwnd
"visible   = $([WX]::IsWindowVisible($h))"
"iconic    = $([WX]::IsIconic($h))"
"rect      = $($r.Left),$($r.Top) - $($r.Right),$($r.Bottom)"
"size      = $($r.Right - $r.Left)x$($r.Bottom - $r.Top)"

if (-not [WX]::IsWindowVisible($h)) {
  throw "ABORT: hwnd 0x{0:X} 仍然不可见 - WGC 抓不到帧。" -f $Hwnd
}