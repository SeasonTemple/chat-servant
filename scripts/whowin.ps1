# 找出盖住目标窗口的是谁：按 HWND 报标题、类名、矩形。
# 用途：遮挡诊断——如果挡住的是**本程序自己的窗口**，那就是产品在挡自己的路。
param([Parameter(Mandatory)][string]$Target)

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class W32 {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumProc cb, IntPtr p);
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetWindowTextLengthW(IntPtr h);
    public delegate bool EnumProc(IntPtr h, IntPtr p);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int L, T, R, B; }
}
"@

function Get-Info([IntPtr]$h) {
    $t = New-Object System.Text.StringBuilder 512
    [void][W32]::GetWindowTextW($h, $t, 512)
    $c = New-Object System.Text.StringBuilder 256
    [void][W32]::GetClassNameW($h, $c, 256)
    # ⚠ 变量名不能叫 `$pid`——那是 PowerShell 的**只读**自动变量，
    # 而且大小写不敏感，`$pid` / `$PID` 是同一个。撞上会直接抛异常。
    $procId = 0
    [void][W32]::GetWindowThreadProcessId($h, [ref]$procId)
    $r = New-Object W32+RECT
    [void][W32]::GetWindowRect($h, [ref]$r)
    [pscustomobject]@{
        Hwnd = ('0x{0:X}' -f $h.ToInt64())
        Title = $t.ToString()
        Class = $c.ToString()
        Visible = [W32]::IsWindowVisible($h)
        ProcId = $procId
        Rect = "$($r.L),$($r.T)-$($r.R),$($r.B)"
    }
}

Write-Output "=== 所有可见且有标题的顶层窗口 ==="
$rows = New-Object System.Collections.Generic.List[object]
$cb = [W32+EnumProc]{
    param($h, $p)
    if ([W32]::IsWindowVisible($h) -and [W32]::GetWindowTextLengthW($h) -gt 0) {
        $rows.Add((Get-Info $h))
    }
    return $true
}
[void][W32]::EnumWindows($cb, [IntPtr]::Zero)
$rows | ForEach-Object {
    $proc = try { (Get-Process -Id $_.ProcId -ErrorAction Stop).ProcessName } catch { '?' }
    "{0,-12} {1,-26} {2,-24} {3}" -f $_.Hwnd, $proc, $_.Rect, $_.Title
}