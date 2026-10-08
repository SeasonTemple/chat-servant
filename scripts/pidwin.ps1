# 列出某个 pid 的**所有**顶层窗口（含不可见、空标题），带类名/样式/位置。
# 用途：面板上的鼠标事件全被某个看不见的窗口吃掉时，
#       winscan（只列可见有标题的）根本看不到它，必须按 pid 全量枚举。
# 用法: pidwin.ps1 <pid>
param([Parameter(Mandatory)][int]$ProcId)

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public class EW {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtr(IntPtr h, int i);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }

    public static List<IntPtr> ForPid(uint want) {
        var list = new List<IntPtr>();
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p == want) list.Add(h);
            return true;
        }, IntPtr.Zero);
        return list;
    }
}
"@

$hs = [EW]::ForPid([uint32]$ProcId)
"pid $ProcId 顶层窗口共 $($hs.Count) 个（含不可见）"
foreach ($h in $hs) {
    $t = New-Object System.Text.StringBuilder 512
    [void][EW]::GetWindowText($h, $t, 512)
    $c = New-Object System.Text.StringBuilder 256
    [void][EW]::GetClassName($h, $c, 256)
    $r = New-Object EW+RECT
    [void][EW]::GetWindowRect($h, [ref]$r)
    $style = [EW]::GetWindowLongPtr($h, -16).ToInt64()
    $ex = [EW]::GetWindowLongPtr($h, -20).ToInt64()
    $flags = @()
    if (([Math]::Abs($style -band 0x80000000) -ne 0)) { }
    if (($ex -band 0x00000020) -ne 0) { $flags += 'TRANSPARENT(点击穿透)' }
    if (($ex -band 0x00000080) -ne 0) { $flags += 'TOOLWINDOW' }
    if (($ex -band 0x00040000) -ne 0) { $flags += 'NOACTIVATE' }
    if (($style -band 0x80000000) -ne 0) { $flags += 'TOPMOST(置顶)' }
    "{0,-10} vis={1,-5} en={2,-5} rect=({3},{4})-({5},{6}) {7}x{8}  class={9,-28} title='{10}' {11}" -f `
        ('0x{0:X}' -f $h.ToInt64()), [EW]::IsWindowVisible($h), [EW]::IsWindowEnabled($h),
        $r.L, $r.T, $r.R, $r.B, ($r.R - $r.L), ($r.B - $r.T),
        $c.ToString(), $t.ToString(), ($flags -join ',')
}