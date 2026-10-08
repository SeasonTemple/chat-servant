# 反复测「光标能不能被程序放到指定位置并停住」。
# 用途：诊断「鼠标事件进不了面板」到底是**根本放不进去**
#       （有别的进程在抢鼠标，典型是游戏调 ClipCursor/SetCursorPos），
#       还是能放进去但窗口不收。
# 用法: cursortest.ps1 <x> <y> [次数]
param(
    [Parameter(Mandatory)][int]$X,
    [Parameter(Mandatory)][int]$Y,
    [int]$N = 5
)

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class CT {
    [StructLayout(LayoutKind.Sequential)] public struct PT { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out PT p);
}
"@
$ok = 0
for ($i = 0; $i -lt $N; $i++) {
    $set = [CT]::SetCursorPos($X, $Y)
    Start-Sleep -Milliseconds 700
    $p = New-Object CT+PT
    [void][CT]::GetCursorPos([ref]$p)
    $stay = ($p.X -eq $X -and $p.Y -eq $Y)
    if ($stay) { $ok++ }
    "第 $i 次：SetCursorPos 返回 $set，光标现在 ($($p.X),$($p.Y))  $(if ($stay) { '停住了' } else { '**被别的东西挪走了**' })"
}
"停住 $ok / $N"