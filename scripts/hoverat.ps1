# 把鼠标挪到某个坐标并停住（不点击），用来验证 hover 态到底有没有生效。
# 用途：区分「事件根本没进 webview」和「进了但处理函数没反应」——
#       只看点击后的画面分不出这两种情况。
# 用法: hoverat.ps1 <x> <y>
param(
    [Parameter(Mandatory)][int]$X,
    [Parameter(Mandatory)][int]$Y
)

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class HV {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
}
"@
[void][HV]::SetCursorPos($X, $Y)
"鼠标已在 ($X,$Y)"