# 两张图逐像素求差，输出「有多少像素不一样」+ 差值最大的几处坐标。
# 用途：验证「鼠标事件到底有没有进到 webview」——
#       靠肉眼对比两张近乎全黑的截图是不可靠的，hover 态只是 4% 的白，
#       肉眼根本看不出来，但像素差是确定的。
# 用法: imgdiff.ps1 <a.png> <b.png> [容忍阈值]
param(
    [Parameter(Mandatory)][string]$A,
    [Parameter(Mandatory)][string]$B,
    [int]$Tol = 6
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$ia = [System.Drawing.Bitmap]::FromFile((Join-Path (Get-Location) $A))
$ib = [System.Drawing.Bitmap]::FromFile((Join-Path (Get-Location) $B))

if ($ia.Width -ne $ib.Width -or $ia.Height -ne $ib.Height) {
    "尺寸不同：$($ia.Width)x$($ia.Height) vs $($ib.Width)x$($ib.Height)"
    $ia.Dispose(); $ib.Dispose(); exit 2
}

$diff = 0
$maxd = 0
$maxAt = ''
$w = $ia.Width; $h = $ia.Height
$ra = $ia.LockBits((New-Object System.Drawing.Rectangle(0, 0, $w, $h)), 'ReadOnly', 'Format32bppArgb')
$rb = $ib.LockBits((New-Object System.Drawing.Rectangle(0, 0, $w, $h)), 'ReadOnly', 'Format32bppArgb')
$stride = $ra.Stride
$ba = New-Object byte[] ($stride * $h)
$bb = New-Object byte[] ($stride * $h)
[System.Runtime.InteropServices.Marshal]::Copy($ra.Scan0, $ba, 0, $ba.Length)
[System.Runtime.InteropServices.Marshal]::Copy($rb.Scan0, $bb, 0, $bb.Length)
$ia.UnlockBits($ra); $ib.UnlockBits($rb)

for ($y = 0; $y -lt $h; $y++) {
    $row = $y * $stride
    for ($x = 0; $x -lt $w; $x++) {
        $i = $row + $x * 4
        $d = [Math]::Abs([int]$ba[$i] - [int]$bb[$i])
        if ($d -gt $maxd) { $maxd = $d; $maxAt = "($x,$y)" }
        if ($d -gt $Tol) { $diff++ }
    }
}
$total = $w * $h
"尺寸 ${w}x${h}  共 $total px"
"差异 >$Tol 的像素：$diff  （占比 $([Math]::Round(100.0 * $diff / $total, 4))%）"
"最大单通道差：$maxd @ $maxAt"

$ia.Dispose(); $ib.Dispose()
if ($diff -eq 0) { '判定：两张图实质相同 —— 鼠标事件没进到界面' } else { '判定：两张图有可见差异 —— 事件进去了' }