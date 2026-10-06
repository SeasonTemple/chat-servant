# 打印指定像素点的颜色。用来看脱敏产物里残留的形状到底是什么色。
# 用法：pwsh -File scripts/pixel.ps1 -Image <png> -Points 88,300 96,300 80,300
param(
    [Parameter(Mandatory)][string]$Image,
    [Parameter(Mandatory)][string[]]$Points
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$img = New-Object System.Drawing.Bitmap $Image
try {
    Write-Output ("{0}  {1}x{2}" -f $Image, $img.Width, $img.Height)
    foreach ($p in $Points) {
        $xy = $p.Split(',')
        $x = [int]$xy[0]; $y = [int]$xy[1]
        $c = $img.GetPixel($x, $y)
        Write-Output ("  ({0,4},{1,4}) = rgb({2,3},{3,3},{4,3})" -f $x, $y, $c.R, $c.G, $c.B)
    }
}
finally { $img.Dispose() }