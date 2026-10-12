param(
  [string]$Out = 'E:\workspace\chat-servant\target\probe\shot.png',
  [int]$X = 300,
  [int]$Y = 300,
  [int]$W = 460,
  [int]$H = 640,
  [int]$Zoom = 1,
  [int]$ZoomX = 0,
  [int]$ZoomY = 0,
  [int]$ZoomW = 0,
  [int]$ZoomH = 0
)

# Screen DC + BitBlt, NOT PrintWindow: PrintWindow flattens XAML islands and
# every icon into a vertical bar. Measured on this machine, don't switch back.

$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Drawing
Add-Type -Namespace Cap -Name G3 -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleDC(IntPtr hdc);
[DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleBitmap(IntPtr hdc, int w, int h);
[DllImport("gdi32.dll")] public static extern IntPtr SelectObject(IntPtr hdc, IntPtr o);
[DllImport("gdi32.dll")] public static extern bool BitBlt(IntPtr d,int x,int y,int w,int h,IntPtr s,int sx,int sy,int rop);
[DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr o);
[DllImport("gdi32.dll")] public static extern bool DeleteDC(IntPtr dc);
[DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
[DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h, IntPtr dc);
'@

[void][Cap.G3]::SetProcessDPIAware()

$sdc = [Cap.G3]::GetDC([IntPtr]::Zero)
$dc  = [Cap.G3]::CreateCompatibleDC($sdc)
$bmp = [Cap.G3]::CreateCompatibleBitmap($sdc, $W, $H)
[void][Cap.G3]::SelectObject($dc, $bmp)
[void][Cap.G3]::BitBlt($dc, 0, 0, $W, $H, $sdc, $X, $Y, 0x00CC0020)

$img = [System.Drawing.Image]::FromHbitmap($bmp)

if ($Zoom -gt 1) {
  if ($ZoomW -le 0) { $ZoomW = [Math]::Min(120, $W) }
  if ($ZoomH -le 0) { $ZoomH = [Math]::Min(120, $H) }
  # Draw straight from the source rect: Image.Clone on an Hbitmap-backed image
  # throws "lack of memory" on small crops, which is a GDI+ lie, not a real one.
  $big = New-Object System.Drawing.Bitmap(($ZoomW * $Zoom), ($ZoomH * $Zoom))
  $g = [System.Drawing.Graphics]::FromImage($big)
  $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
  $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::Half
  $dst = New-Object System.Drawing.Rectangle -ArgumentList 0, 0, ($ZoomW * $Zoom), ($ZoomH * $Zoom)
  $g.DrawImage($img, $dst, [int]$ZoomX, [int]$ZoomY, [int]$ZoomW, [int]$ZoomH,
    [System.Drawing.GraphicsUnit]::Pixel)
  $g.Dispose()
  $save = $big
} else {
  $save = $img
}

New-Item -ItemType Directory -Force -Path (Split-Path $Out) | Out-Null
$save.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$save.Dispose(); $img.Dispose()

[void][Cap.G3]::DeleteObject($bmp)
[void][Cap.G3]::DeleteDC($dc)
[void][Cap.G3]::ReleaseDC([IntPtr]::Zero, $sdc)

Write-Host ("saved: {0}  ({1}x{2} at {3},{4} zoom={5})" -f $Out, $W, $H, $X, $Y, $Zoom)