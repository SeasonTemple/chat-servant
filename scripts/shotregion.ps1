param(
  [Parameter(Mandatory = $true)][int]$Hwnd,
  [Parameter(Mandatory = $true)][string]$OutPng,
  [int]$X = 0, [int]$Y = 0, [int]$W = 0, [int]$H = 0
)

# 用**屏幕 DC 的 BitBlt**抓指定屏幕区域，不是 PrintWindow。
# 理由（实测 2026-10-14）：PrintWindow 对自绘/合成窗口会回放陈旧帧
# （连续两次抓同一窗口 MD5 完全相同），据此断言「重绘没发生」会得出假结论。
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Cap {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("gdi32.dll")] public static extern bool BitBlt(IntPtr dst, int x, int y, int w, int h, IntPtr src, int sx, int sy, int rop);
  [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleDC(IntPtr dc);
  [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleBitmap(IntPtr dc, int w, int h);
  [DllImport("gdi32.dll")] public static extern IntPtr SelectObject(IntPtr dc, IntPtr o);
  [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr o);
  [DllImport("gdi32.dll")] public static extern bool DeleteDC(IntPtr dc);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

# 按坐标抓必须先 DPI 感知，否则拿到的矩形与屏幕坐标不是一个空间
[void][Cap]::SetProcessDPIAware()

$r = New-Object Cap+RECT
if (-not [Cap]::GetWindowRect([IntPtr]$Hwnd, [ref]$r)) { throw "GetWindowRect 失败" }
if ($W -eq 0) { $W = $r.Right - $r.Left }
if ($H -eq 0) { $H = $r.Bottom - $r.Top }
$rx = $r.Left + $X; $ry = $r.Top + $Y
"窗口矩形 = $($r.Left),$($r.Top)..$($r.Right),$($r.Bottom)   抓取区域 = $rx,$ry ${W}x${H}"

$bmp = New-Object System.Drawing.Bitmap($W, $H, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$dst = $g.GetHdc()
$src = [Cap]::CreateCompatibleDC([IntPtr]::Zero)
$hbmp = [Cap]::CreateCompatibleBitmap([IntPtr]::Zero, $W, $H)
[void][Cap]::SelectObject($src, $hbmp)
$sdc = [System.Drawing.Graphics]::FromHwnd([IntPtr]$Hwnd)  # 占位，实际用屏幕 DC
$g.ReleaseHdc($dst)
[System.Windows.Forms.Screen] | Out-Null

# 屏幕 DC：Graphics.FromHwnd(IntPtr.Zero) 即整个屏幕
$screenG = [System.Drawing.Graphics]::FromHwnd([IntPtr]::Zero)
$sdcHdc = $screenG.GetHdc()
$dstHdc = $g.GetHdc()
$ok = [Cap]::BitBlt($dstHdc, 0, 0, $W, $H, $sdcHdc, $rx, $ry, 0x00CC0020)
$g.ReleaseHdc($dstHdc); $screenG.ReleaseHdc($sdcHdc)
$g.Dispose()

"BitBlt = $ok"
$bmp.Save($OutPng, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
"已写入 $OutPng"