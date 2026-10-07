# 裁出消息气泡区域并放大，用于确认 OCR 到底看到了什么。
# 用法: crop.ps1 <in.png> <out.png> <x0> <y0> <x1> <y1> [scale]
param(
    [Parameter(Mandatory)][string]$In,
    [Parameter(Mandatory)][string]$Out,
    [Parameter(Mandatory)][int]$X0,
    [Parameter(Mandatory)][int]$Y0,
    [Parameter(Mandatory)][int]$X1,
    [Parameter(Mandatory)][int]$Y1,
    [int]$Scale = 4
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$src = [System.Drawing.Bitmap]::FromFile((Resolve-Path $In))
try {
    $rw = $X1 - $X0
    $rh = $Y1 - $Y0
    "源图 $($src.Width)x$($src.Height)  裁剪 ${rw}x${rh} @ ($X0,$Y0)  放大 ${Scale}x"

    # 先原尺寸裁剪，再用最近邻放大，放大后不引入插值模糊。
    $crop = New-Object System.Drawing.Bitmap($rw, $rh)
    $g = [System.Drawing.Graphics]::FromImage($crop)
    $g.DrawImage($src,
        (New-Object System.Drawing.Rectangle(0, 0, $rw, $rh)),
        (New-Object System.Drawing.Rectangle($X0, $Y0, $rw, $rh)),
        [System.Drawing.GraphicsUnit]::Pixel)
    $g.Dispose()

    $big = New-Object System.Drawing.Bitmap(($rw * $Scale), ($rh * $Scale))
    $g2 = [System.Drawing.Graphics]::FromImage($big)
    $g2.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
    $g2.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::Half
    $g2.DrawImage($crop, 0, 0, ($rw * $Scale), ($rh * $Scale))
    $g2.Dispose()

    $big.Save((Join-Path (Get-Location) $Out), [System.Drawing.Imaging.ImageFormat]::Png)
    "已写出 $Out  ($($big.Width)x$($big.Height))"
    $big.Dispose()
    $crop.Dispose()
}
finally {
    $src.Dispose()
}