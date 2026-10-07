# 整屏抓一张 PNG，用途是「看看用户实际看到的是什么」。
# 用法: shotfull.ps1 <out.png>
param([Parameter(Mandatory)][string]$Out)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class SS {
    [DllImport("user32.dll")] public static extern IntPtr GetDesktopWindow();
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int L, T, R, B; }
}
"@

$r = New-Object SS+RECT
[void][SS]::GetWindowRect([SS]::GetDesktopWindow(), [ref]$r)
$w = $r.R - $r.L
$h = $r.B - $r.T
"整屏 ${w}x${h}"

$bmp = New-Object System.Drawing.Bitmap($w, $h)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.L, $r.T, 0, 0, (New-Object System.Drawing.Size($w, $h)))
$g.Dispose()
$bmp.Save((Join-Path (Get-Location) $Out), [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
"已写出 $Out"