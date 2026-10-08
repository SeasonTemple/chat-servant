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
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, IntPtr e);
    public const uint LEFTDOWN = 0x0002, LEFTUP = 0x0004;
}
"@
[void][MC]::SetCursorPos($X, $Y)
Start-Sleep -Milliseconds 250
[MC]::mouse_event([MC]::LEFTDOWN, 0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 80
[MC]::mouse_event([MC]::LEFTUP, 0, 0, 0, [IntPtr]::Zero)
"已点击 ($X,$Y)"