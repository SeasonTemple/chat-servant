# 抓整屏，再按某个窗口的实际位置裁出来。
# 用途：面板这类「不知道自己跑到哪儿去了」的窗口，只能靠整屏截图 + 实际矩形
#       去找——凭印象写死坐标是上一次裁偏的原因。
# 用法: shotwin2.ps1 <hwnd> <out.png>
param(
    [Parameter(Mandatory)][string]$Hwnd,
    [Parameter(Mandatory)][string]$Out,
    [int]$Scale = 2
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class W2 {
    [DllImport("user32.dll")] public static extern IntPtr GetDesktopWindow();
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int L, T, R, B; }
}
"@

$h = [IntPtr][Convert]::ToInt64($Hwnd, 16)
$r = New-Object W2+RECT
if (-not [W2]::GetWindowRect($h, [ref]$r)) { throw "GetWindowRect($Hwnd) 失败" }
$w = $r.R - $r.L
$ht = $r.B - $r.T
"窗口 $Hwnd  屏幕矩形 = ($($r.L),$($r.T))-($($r.R),$($r.B))  ${w}x${ht}"

$dw = New-Object W2+RECT
[void][W2]::GetWindowRect([W2]::GetDesktopWindow(), [ref]$dw)
$sw = $dw.R - $dw.L
$sh = $dw.B - $dw.T
$full = New-Object System.Drawing.Bitmap($sw, $sh)
$g = [System.Drawing.Graphics]::FromImage($full)
$g.CopyFromScreen($dw.L, $dw.T, 0, 0, (New-Object System.Drawing.Size($sw, $sh)))
$g.Dispose()

# **用整屏截图，不用 PrintWindow。**
# WebView2 走 DirectComposition，PrintWindow 回来的是黑帧/空帧——
# 上一轮就是被它骗过一次。整屏永远是真相。
$crop = $full.Clone(
    (New-Object System.Drawing.Rectangle($r.L, $r.T, $w, $ht)),
    $full.PixelFormat)
$full.Dispose()

if ($Scale -gt 1) {
    $big = New-Object System.Drawing.Bitmap(($w * $Scale), ($ht * $Scale))
    $g2 = [System.Drawing.Graphics]::FromImage($big)
    # InterpolationMode.NearestNeighbor：调试截图要看得见真实像素边界，不能糊
    $g2.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
    $g2.DrawImage($crop, 0, 0, ($w * $Scale), ($ht * $Scale))
    $g2.Dispose()
    $crop.Dispose()
    $crop = $big
}
$crop.Save((Join-Path (Get-Location) $Out), [System.Drawing.Imaging.ImageFormat]::Png)
$crop.Dispose()
"已写出 $Out"