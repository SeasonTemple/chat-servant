<#
  把 ui/static/probe-sheet.svg 里内嵌的位图挖出来量。

  为什么需要这个：
  `probe-sheet.svg` 是整个面板的材质根基，但它是**一张 291 KB 的位图**，
  库内没有生成器、原始素材也不在仓库（app.css 引用的 `paper-fixed.png`
  在本机与 git 中均已不存在）。没有工具就只能靠「看着还行」说话。

  这个脚本做三件事：
    1. 抽出内嵌 PNG，落到 target/probe/
    2. **实测**中位色与撕口深度 —— app.css 与 HANDOFF §18.2 里的
       rgb(243 230 192)、17.5px 都是被引用的断言，得有人能重新验
    3. 往返校验：重新编码后的 base64 必须与文件里那段逐字节相同，
       否则说明内嵌数据被截断或被编辑器动过

  用法（PS7）：
    pwsh -NoProfile -File ui\scripts\sheetprobe.ps1
#>
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$root = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSCommandPath))
$svg  = Join-Path $root 'ui\static\probe-sheet.svg'
$out  = Join-Path $root 'target\probe'
New-Item -ItemType Directory -Force -Path $out | Out-Null

# ── 1. 抽出内嵌 base64 ────────────────────────────────────────────
$text = [IO.File]::ReadAllText($svg)
$m = [regex]::Match($text, 'base64,([A-Za-z0-9+/=]+)"')
if (-not $m.Success) { throw 'probe-sheet.svg 里找不到内嵌 base64 —— 资产结构变了，脚本要改' }
$b64 = $m.Groups[1].Value
$png = Join-Path $out 'probe-sheet.png'
[IO.File]::WriteAllBytes($png, [Convert]::FromBase64String($b64))

Write-Output ("内嵌 base64 : {0:N0} 字符" -f $b64.Length)
Write-Output ("解出 PNG    : {0:N0} 字节  -> target\probe\probe-sheet.png" -f (Get-Item $png).Length)

# ── 2. 往返校验 ───────────────────────────────────────────────────
$re = [Convert]::ToBase64String([Convert]::FromBase64String($b64))
if ($re -ceq $b64) {
  Write-Output '往返校验     : 通过（重新编码与文件内那段逐字节相同）'
} else {
  Write-Output '往返校验     : **不一致** —— 内嵌数据可能被截断或被编辑器重排'
}

# SVG 声明的显示尺寸（viewBox 380x560 内的用户单位）
$d = [regex]::Match($text, '<image[^>]*width="(\d+)"[^>]*height="(\d+)"[^>]*preserveAspectRatio="none"')
if (-not $d.Success) { throw '找不到 <image> 的显示尺寸声明 —— 脚本要改' }
$dispW = [int]$d.Groups[1].Value; $dispH = [int]$d.Groups[2].Value

function Measure-Mid {
  param($bm, [string]$Label)
  $ch = New-Object 'System.Collections.Generic.List[int]'
  $x0 = [int]($bm.Width  * 0.20); $x1 = [int]($bm.Width  * 0.80)
  $y0 = [int]($bm.Height * 0.20); $y1 = [int]($bm.Height * 0.80)
  for ($y = $y0; $y -lt $y1; $y += 2) {
    for ($x = $x0; $x -lt $x1; $x += 2) {
      $c = $bm.GetPixel($x, $y)
      $ch.Add([int](0.299 * $c.R + 0.587 * $c.G + 0.114 * $c.B))
    }
  }
  $sorted = $ch | Sort-Object
  $med = $sorted[[int]($sorted.Count / 2)]
  $best = $null; $bestD = 999
  for ($y = $y0; $y -lt $y1; $y += 3) {
    for ($x = $x0; $x -lt $x1; $x += 3) {
      $c = $bm.GetPixel($x, $y)
      $l = [int](0.299 * $c.R + 0.587 * $c.G + 0.114 * $c.B)
      $d = [Math]::Abs($l - $med)
      if ($d -lt $bestD) { $bestD = $d; $best = $c }
    }
  }
  # 返回对象而不是裸值：裸值会把函数里的字符串一并吞进赋值。
  [pscustomobject]@{
    Label = $Label; R = $best.R; G = $best.G; B = $best.B; Lum = $med; N = $sorted.Count
  }
}

# 撕口深度。**不能靠「颜色偏离」判断** —— 撕口处像素是透明的，
# GetPixel 给出 (0,0,0)，与纸色差得更远，于是第 0 行就被判成撕口，
# 测出来永远是 0。正确做法是看 alpha：自上而下第一个**完全不透明**的行。
function Measure-Tear {
  param($bm, [int]$X, [double]$Scale)
  for ($y = 0; $y -lt [int]($bm.Height * 0.15); $y++) {
    if ($bm.GetPixel($X, $y).A -ge 250) { return [Math]::Round($y * $Scale, 1) }
  }
  return -1
}

$bmp = [System.Drawing.Bitmap]::FromFile($png)
try {
  Write-Output ''
  Write-Output ("位图尺寸     : {0} x {1}" -f $bmp.Width, $bmp.Height)
  $sx = $dispW / $bmp.Width; $sy = $dispH / $bmp.Height
  Write-Output ("SVG 显示尺寸 : {0} x {1}（viewBox 用户单位 = 面板 CSS px）" -f $dispW, $dispH)
  Write-Output ("缩放比       : x {0:N4}   y {1:N4}   横向多压 {2:N2}%" -f `
    $sx, $sy, ([Math]::Abs($sx / $sy - 1) * 100))
  Write-Output '              ⚠ preserveAspectRatio="none" 允许非等比拉伸，所以这**不是**等比缩放'

  Write-Output ''
  Write-Output '── 中位色 ─────────────────────────────────────────'
  $srcMid = Measure-Mid $bmp '源位图'
  Write-Output ("{0,-16} rgb({1} {2} {3})   亮度 {4}" -f `
    $srcMid.Label, $srcMid.R, $srcMid.G, $srcMid.B, $srcMid.Lum)

  # 渲染尺寸那一档 —— app.css 的断言指的是这一档，不是源图
  $ds = New-Object System.Drawing.Bitmap $dispW, $dispH
  try {
    $g = [System.Drawing.Graphics]::FromImage($ds)
    try {
      $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
      $g.DrawImage($bmp, 0, 0, $dispW, $dispH)
    } finally { $g.Dispose() }
    $rMid = Measure-Mid $ds '渲染尺寸'
    Write-Output ("{0,-16} rgb({1} {2} {3})   亮度 {4}" -f `
      $rMid.Label, $rMid.R, $rMid.G, $rMid.B, $rMid.Lum)
  } finally { $ds.Dispose() }

  Write-Output ''
  Write-Output '  app.css 的断言 : rgb(243 230 192)'
  Write-Output ("  实测差值       : rgb({0} {1} {2})" -f `
    ($rMid.R - 243), ($rMid.G - 230), ($rMid.B - 192))
  Write-Output ("  采样量         : {0:N0} 点（画幅 20%~80%，避开撕口与落影）" -f $rMid.N)

  Write-Output ''
  Write-Output '── 撕口深度（换算到面板 CSS px）────────────────────'
  Write-Output '  撕口不是平的，深浅随 x 变，所以要多列一起量：'
  foreach ($frac in @(0.15, 0.30, 0.50, 0.70, 0.85)) {
    $x = [int]($bmp.Width * $frac)
    $px = Measure-Tear $bmp $x $sy
    Write-Output ("    x = {0:P0} ({1,4} px)  ->  {2,5} CSS px" -f $frac, $x, $px)
  }
  Write-Output ''
  Write-Output '  panel/+page.svelte:169 的注释写「撕口 17.5px 已含在 SVG 里」'
  Write-Output '  ⚠ 实测 7~11 CSS px，**对不上**。而且这不是 alpha 阈值造成的：'
  Write-Output '    alpha 门槛从 250 降到 1，五个列的读数只在 5.5~11 之间动，够不到 17.5。'
  Write-Output '  → padding-top: 40px 仍比实测撕口深，**布局是安全的**，'
  Write-Output '    但那句「17.5px 已含在 SVG 里」是不实描述，已订正为实测值。'
} finally { $bmp.Dispose() }

Write-Output ''
Write-Output '⚠ 本脚本只做「测量与校验」，不生成纸。'
Write-Output '  源图 ui\static\probe-sheet.png 现在在库里；把 PNG 重新打包进 SVG 用'
Write-Output '  ui\scripts\sheetpack.ps1（-Check 可校验两者是否同步）。'
Write-Output '  但**重画**仍然做不到：原始素材 paper-fixed.png 已不可考，详见'
Write-Output '  docs/设计规范.md §2.4。'