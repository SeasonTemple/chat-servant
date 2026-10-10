# 列出全部**可见**顶层窗口：HWND / 类名 / 标题 / 矩形 / IsIconic / PID。
# 用途：判断某个 HWND 是否还活着、窗口是否最小化（WGC 对最小化窗口会立刻 Closed）。
#
# ⚠ **本脚本只报可见窗口。** 「窗口根本没被创建」和「窗口被创建了但被藏起来」
#   在这里长得一模一样——两者都不会出现在输出里。诊断「窗口不见了」时，
#   先用 `target/probe/pidwins.ps1 <pid>` 枚举**某个进程的**全部顶层窗口
#   （含不可见的）再下结论。2026-10-11 票 17 就是被这一点带偏过：
#   设置窗口停在 (-32000,-32000)、vis=False，本脚本里根本不出现，
#   一度被当成「没建」。（真相是本 crate 的 `park_settings` 主动藏的。）
$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class WS {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    public delegate bool EnumProc(IntPtr h, IntPtr p);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int L, T, R, B; }
}
"@

$rows = [System.Collections.Generic.List[object]]::new()
$cb = [WS+EnumProc]{
    param([IntPtr]$h, [IntPtr]$p)
    $t = New-Object System.Text.StringBuilder 512
    [void][WS]::GetWindowTextW($h, $t, 512)
    $c = New-Object System.Text.StringBuilder 256
    [void][WS]::GetClassNameW($h, $c, 256)
    $procId = 0
    [void][WS]::GetWindowThreadProcessId($h, [ref]$procId)
    $r = New-Object WS+RECT
    [void][WS]::GetWindowRect($h, [ref]$r)
    $w = $r.R - $r.L
    $ht = $r.B - $r.T
    # 只保留有标题的可见窗口，外加任何 WeChat 窗口（哪怕不可见）
    $vis = [WS]::IsWindowVisible($h)
    $isWx = $t.ToString() -match '微信|WeChat' -or $c.ToString() -match 'WeChat'
    if ($vis -and ($t.Length -gt 0 -or $isWx)) {
        $rows.Add([pscustomobject]@{
            Hwnd   = ('0x{0:X}' -f $h.ToInt64())
            Class  = $c.ToString()
            Title  = $t.ToString()
            Size   = "${w}x${ht}"
            Iconic = [WS]::IsIconic($h)
            ProcId = $procId
        })
    }
    return $true
}
[void][WS]::EnumWindows($cb, [IntPtr]::Zero)

"可见顶层窗口共 $($rows.Count) 个："
$rows | Sort-Object Title | Format-Table -AutoSize | Out-String -Width 220