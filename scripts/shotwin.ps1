# 截取某个窗口的像素存盘。用法: shotwin.ps1 <hwnd-hex> <out.png>
param(
    [Parameter(Mandatory)][string]$Hwnd,
    [Parameter(Mandatory)][string]$Out
)

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct RECT { public int L, T, R, B; }
public class SW {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
}
"@ -ErrorAction SilentlyContinue

Add-Type -AssemblyName System.Drawing

$h = [IntPtr]([Convert]::ToInt64($Hwnd.TrimStart('0x'), 16))
$r = New-Object RECT
if (-not [SW]::GetWindowRect($h, [ref]$r)) { throw "取不到窗口矩形" }
$w = $r.R - $r.L
$ht = $r.B - $r.T
"窗口 0x{0:X}  {1}x{2} @ ({3},{4})" -f $h.ToInt64(), $w, $ht, $r.L, $r.T

$bmp = New-Object System.Drawing.Bitmap($w, $ht)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
# flag 2 = PW_RENDERFULL_CONTENT，透明窗口必须用它才有画面
$ok = [SW]::PrintWindow($h, $hdc, 2)
$g.ReleaseHdc($hdc)
$g.Dispose()
"PrintWindow = $ok"
$bmp.Save((Join-Path (Get-Location) $Out), [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
"已写出 $Out"