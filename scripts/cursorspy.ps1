# 读当前光标位置 + 前景窗口 + 指定点上的窗口，用来确认
# 「鼠标到底能不能被程序移动」以及「那个点上到底是谁」。
# 用途：上一次「点「新」按钮没反应」的排查里，必须先分清
#       「光标没动过去」和「动了但事件没进 webview」——两者的修法完全不同。
# 用法: cursorspy.ps1 [x y]
param([int]$X = 0, [int]$Y = 0)

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class CS {
    [StructLayout(LayoutKind.Sequential)] public struct PT { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out PT p);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(PT p);
    [DllImport("user32.dll")] public static extern int GetWindowThreadProcessId(IntPtr h, out int pid);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
}
"@
$p = New-Object CS+PT
[void][CS]::GetCursorPos([ref]$p)
"光标现在在 ($($p.X),$($p.Y))"

$fg = [CS]::GetForegroundWindow()
$fgPid = 0
[void][CS]::GetWindowThreadProcessId($fg, [ref]$fgPid)
"前台窗口 = 0x$('{0:X}' -f $fg.ToInt64())  pid=$fgPid"

if ($X -ne 0 -or $Y -ne 0) {
    $q = New-Object CS+PT
    $q.X = $X; $q.Y = $Y
    $under = [CS]::WindowFromPoint($q)
    $uPid = 0
    [void][CS]::GetWindowThreadProcessId($under, [ref]$uPid)
    "($X,$Y) 命中窗口 = 0x$('{0:X}' -f $under.ToInt64())  pid=$uPid"
}