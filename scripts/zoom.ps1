# 放大探针帧的局部，用来看清残留像素是什么。
# 只读 target/probe/ 下的帧，输出也写回那里（已 gitignore）。
param(
    [string]$Source = 'target/probe/xiaohao.png',
    [string]$Out    = 'target/probe/zoom.png',
    [int]$X = 0, [int]$Y = 0, [int]$W = 30, [int]$H = 30, [int]$Scale = 24
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$img = [System.Drawing.Image]::FromFile($Source)
try {
    $big = New-Object System.Drawing.Bitmap ($W * $Scale), ($H * $Scale)
    $g = [System.Drawing.Graphics]::FromImage($big)
    try {
        $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
        $g.PixelOffsetMode   = [System.Drawing.Drawing2D.PixelOffsetMode]::Half
        $dst = New-Object System.Drawing.Rectangle 0, 0, ($W * $Scale), ($H * $Scale)
        $src = New-Object System.Drawing.Rectangle $X, $Y, $W, $H
        $g.DrawImage($img, $dst, $src, [System.Drawing.GraphicsUnit]::Pixel)
        $big.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
    }
    finally { $g.Dispose(); $big.Dispose() }
}
finally { $img.Dispose() }

Write-Output "$Out  <-  $Source [$X,$Y ${W}x${H}] x$Scale"