# 把生成的 JPEG 图标变成带 alpha 的 PNG，再交给 tauri icon 出全套尺寸。
#
# 为什么要这一步：JPEG **没有 alpha 通道**，圆角外是白底。深色任务栏上
# 一个白方块图标比圆角图标难看得多。生成服务给的又是 .jpg（请求里的
# .png 后缀被忽略了），所以透明只能在这一步补。
#
# 键白是安全的：图标本身只有近黑底 + 强调绿两种颜色，**没有近白像素**，
# 所以「近白 → 全透明」不会啃到图形本身。
param(
    [Parameter(Mandatory)][string]$Src,
    [Parameter(Mandatory)][string]$Dst
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$img = New-Object System.Drawing.Bitmap $Src
try {
    $w = $img.Width
    $h = $img.Height
    $out = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $cut = 240   # 近白阈值。图形里最亮的绿也远低于它。
        $made = 0
        for ($y = 0; $y -lt $h; $y++) {
            for ($x = 0; $x -lt $w; $x++) {
                $p = $img.GetPixel($x, $y)
                if ($p.R -ge $cut -and $p.G -ge $cut -and $p.B -ge $cut) {
                    $out.SetPixel($x, $y, [System.Drawing.Color]::FromArgb(0, $p.R, $p.G, $p.B))
                    $made++
                } else {
                    $out.SetPixel($x, $y, [System.Drawing.Color]::FromArgb(255, $p.R, $p.G, $p.B))
                }
            }
        }
        $out.Save($Dst, [System.Drawing.Imaging.ImageFormat]::Png)
        Write-Output ("$w x $h -> $Dst   透明像素 {0}（{1:P1}）" -f $made, ($made / ($w * $h)))
    } finally { $out.Dispose() }
} finally { $img.Dispose() }