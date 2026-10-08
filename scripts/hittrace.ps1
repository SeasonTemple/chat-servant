# 从 z 序由上往下遍历窗口，报告**覆盖指定点的所有窗口**。
# 用途：`WindowFromPoint` 只给一个结果，看不出「是谁压在谁上面」。
#       面板明明画在最上层却收不到鼠标时，必须把整条 z 序摊开看，
#       才知道是上面有个透明窗口，还是下面的窗口把点击吃了。
# 用法: hittrace.ps1 <x> <y>
param(
    [Parameter(Mandatory)][int]$X,
    [Parameter(Mandatory)][int]$Y
)

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public class HT {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtr(IntPtr h, int i);
    public static List<IntPtr> All() {
        var l = new List<IntPtr>();
        EnumWindows((h, x) => { l.Add(h); return true; }, IntPtr.Zero);
        return l;
    }
}
"@

"覆盖 ($X,$Y) 的窗口，按 z 序从上到下："
$n = 0
foreach ($h in [HT]::All()) {
    $r = New-Object HT+RECT
    if (-not [HT]::GetWindowRect($h, [ref]$r)) { continue }
    if ($X -lt $r.L -or $X -ge $r.R -or $Y -lt $r.T -or $Y -ge $r.B) { continue }
    $vis = [HT]::IsWindowVisible($h)
    $proc = 0
    [void][HT]::GetWindowThreadProcessId($h, [ref]$proc)
    $t = New-Object System.Text.StringBuilder 512
    [void][HT]::GetWindowText($h, $t, 512)
    $c = New-Object System.Text.StringBuilder 256
    [void][HT]::GetClassName($h, $c, 256)
    $ex = [HT]::GetWindowLongPtr($h, -20).ToInt64()
    $st = [HT]::GetWindowLongPtr($h, -16).ToInt64()
    $flags = @()
    if (($ex -band 0x00000020) -ne 0) { $flags += '点击穿透' }
    if (($ex -band 0x00000080) -ne 0) { $flags += 'TOOLWINDOW' }
    if (($ex -band 0x00040000) -ne 0) { $flags += 'NOACTIVATE' }
    if (($st -band 0x80000000) -ne 0) { $flags += '置顶' }
    if (($st -band 0x00800000) -ne 0) { $flags += '透明' }
    $mark = if ($vis) { '可见' } else { '不可见' }
    "  [{0}] 0x{1:X} pid={2,-6} {3} class={4,-26} title='{5}' {6}" -f `
        $n, $h.ToInt64(), $proc, $mark, $c.ToString(), $t.ToString(), ($flags -join ',')
    $n++
}
if ($n -eq 0) { '  （没有窗口覆盖这个点）' }