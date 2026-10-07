# 把一张 PNG 贴到更宽的画布上（用指定颜色填充余量），用于检验
# 「Windows OCR 漏读是否与图像总宽度有关」——贴到左边还是右边会给出不同答案。
# 用法: pad.ps1 <in.png> <out.png> <padLeft> <padRight> [fill]
param(
    [Parameter(Mandatory)][string]$In,
    [Parameter(Mandatory)][string]$Out,
    [Parameter(Mandatory)][int]$PadLeft,
    [Parameter(Mandatory)][int]$PadRight,
    [int]$Fill = 0x1E1E1F
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$src = [System.Drawing.Bitmap]::FromFile((Resolve-Path $In))
try {
    $w = $src.Width + $PadLeft + $PadRight
    $h = $src.Height
    "源 $($src.Width)x$($src.Height) -> 画布 ${w}x${h}  左补 $PadLeft 右补 $PadRight"

    $dst = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($dst)
    $g.Clear([System.Drawing.Color]::FromArgb(255, ($Fill -band 0xFF), (($Fill -shr 8) -band 0xFF), (($Fill -shr 16) -band 0xFF)))
    $g.DrawImage($src, $PadLeft, 0)
    $g.Dispose()

    $dst.Save((Join-Path (Get-Location) $Out), [System.Drawing.Imaging.ImageFormat]::Png)
    "已写出 $Out"
    $dst.Dispose()
}
finally {
    $src.Dispose()
}