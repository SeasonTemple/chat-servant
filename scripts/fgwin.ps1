# 当前前台窗口是谁（类名 / 标题 / 尺寸），用于判断能不能安全地发 ALT 前置键。
$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class FG {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
}
"@
$h = [FG]::GetForegroundWindow()
$c = New-Object System.Text.StringBuilder 256
[void][FG]::GetClassNameW($h, $c, 256)
$t = New-Object System.Text.StringBuilder 512
[void][FG]::GetWindowTextW($h, $t, 512)
"前台 = 0x{0:X}" -f $h.ToInt64()
"类名 = {0}" -f $c.ToString()
"标题 = {0}" -f $t.ToString()