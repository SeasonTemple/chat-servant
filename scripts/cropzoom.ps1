param(
  [Parameter(Mandatory = $true)][string]$In,
  [Parameter(Mandatory = $true)][string]$OutPng,
  [Parameter(Mandatory = $true)][int]$Scale,
  [Parameter(Mandatory = $true)][string]$Rects
)
# 一段 "x,y,w,h;x,y,w,h;..."。`pwsh -File` 传数组参数只吃第一个元素，
# 所以这里收单字符串自己拆。
$rectList = @()
foreach ($spec in $Rects.Split(';')) {
  $v = @($spec.Split(',') | ForEach-Object { [int]$_.Trim() })
  if ($v.Count -ne 4) { throw "bad rect: $spec" }
  $rectList += , $v
}
Add-Type -AssemblyName System.Drawing
$src = [System.Drawing.Bitmap]::new($In)
try {
  $crops = foreach ($r in $rectList) {
    $rect = [System.Drawing.Rectangle]::new($r[0], $r[1], $r[2], $r[3])
    $bmp = $src.Clone($rect, $src.PixelFormat)
    $scaled = [System.Drawing.Bitmap]::new(($r[2] * $Scale), ($r[3] * $Scale))
    $g = [System.Drawing.Graphics]::FromImage($scaled)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::Half
    $g.DrawImage($bmp, 0, 0, ($r[2] * $Scale), ($r[3] * $Scale))
    $g.Dispose()
    $bmp.Dispose()
    $scaled
  }
  # 尺寸直接由输入算，不从 $crops 反推——foreach 输出的 Bitmap 会被
  # Measure-Object 当成非预期形状，算出 0 宽然后 ctor 报 "Parameter is not valid"。
  $gap = 8 * $Scale
  $cw = @($rectList | ForEach-Object { $_[2] * $Scale })
  $ch = @($rectList | ForEach-Object { $_[3] * $Scale })
  $w = [int](($cw | Measure-Object -Sum).Sum) + $gap * ($rectList.Count - 1)
  $h = [int](($ch | Measure-Object -Maximum).Maximum)
  $sheet = [System.Drawing.Bitmap]::new($w, $h)
  $sg = [System.Drawing.Graphics]::FromImage($sheet)
  $sg.Clear([System.Drawing.Color]::FromArgb(107, 114, 120))
  $x = 0
  foreach ($c in $crops) {
    $sg.DrawImage($c, $x, 0)
    $x += $c.Width + $gap
    $c.Dispose()
  }
  $sg.Dispose()
  $sheet.Save($OutPng, [System.Drawing.Imaging.ImageFormat]::Png)
  $sheet.Dispose()
  Write-Output "SAVED $OutPng ${w}x${h}"
} finally {
  $src.Dispose()
}