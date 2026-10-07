# 打印 PNG 上若干点的真实 RGB。用法: pix.ps1 <png> <x,y> [<x,y> ...]
param(
    [Parameter(Mandatory)][string]$Png,
    [Parameter(Mandatory)][string[]]$At
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$bmp = [System.Drawing.Bitmap]::FromFile((Resolve-Path $Png))
try {
    "图 $($bmp.Width)x$($bmp.Height)  格式=$($bmp.PixelFormat)"
    foreach ($p in $At) {
        $xy = $p -split ','
        $x = [int]$xy[0]
        $y = [int]$xy[1]
        if ($x -ge $bmp.Width -or $y -ge $bmp.Height) { "($x,$y) 越界"; continue }
        $c = $bmp.GetPixel($x, $y)
        "({0,4},{1,4})  rgb({2,3},{3,3},{4,3})  a={5}" -f $x, $y, $c.R, $c.G, $c.B, $c.A
    }
}
finally {
    $bmp.Dispose()
}