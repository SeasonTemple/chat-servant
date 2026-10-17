# 在屏幕绝对坐标上点一下。
# 用途：验证 Tauri 面板上的真实控件（读快照按钮、档位切换、候选「填入」）
#       是否真的把 invoke 打到了 Rust——截图只能证明「长得对」，
#       证明不了「点了有反应」。
# 用法: clickat.ps1 <x> <y>   （屏幕绝对坐标，主显示器）
param(
    [Parameter(Mandatory)][int]$X,
    [Parameter(Mandatory)][int]$Y
)

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class MC {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, IntPtr e);
    public const uint LEFTDOWN = 0x0002, LEFTUP = 0x0004;
}
"@
# ⚠ 必须调，且必须在任何窗口创建之前（2026-10-18 补）。
#   `AGENTS.md` 踩坑区写着「按坐标点东西必须先 SetProcessDPIAware()」——
#   UIA / GetWindowRect 报的矩形与 SetCursorPos 用的屏幕坐标**不是同一个空间**
#   （DPI 虚拟化）。原先这里没调，靠本机 DPI 缩放 = 1 侥幸对上；
#   一旦缩放不是 100%，它会安静地点在**别的位置**上。
#   ⇒ 「点了没反应」会读成「控件坏了」，而真因是坐标算错了。
[void][MC]::SetProcessDPIAware()
[void][MC]::SetCursorPos($X, $Y)
Start-Sleep -Milliseconds 250
[MC]::mouse_event([MC]::LEFTDOWN, 0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 80
[MC]::mouse_event([MC]::LEFTUP, 0, 0, 0, [IntPtr]::Zero)
"已点击 ($X,$Y)"