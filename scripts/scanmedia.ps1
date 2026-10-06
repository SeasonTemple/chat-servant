# 在真实帧上扫描「色种数异常高」的瓦片，用来定位图片区域并拿到实测数字。
# 用法：pwsh -File scripts/scanmedia.ps1 -Image target/probe/text-chat.png
param(
    [Parameter(Mandatory)][string]$Image,
    [int]$Tile = 40,
    [int]$Step = 20,
    [int]$MinColors = 40,
    [int]$X0 = 0,
    [int]$Y0 = 0,
    [int]$X1 = -1,
    [int]$Y1 = -1
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$img = New-Object System.Drawing.Bitmap $Image
try {
    $w = $img.Width
    $h = $img.Height
    if ($X1 -lt 0) { $X1 = $w }
    if ($Y1 -lt 0) { $Y1 = $h }
    Write-Output ("帧 {0}  {1}x{2}  扫描范围 [{3},{4}]-[{5},{6}]" -f $Image, $w, $h, $X0, $Y0, $X1, $Y1)

    # LockBits 直读，比 GetPixel 快一个数量级
    $rect = New-Object System.Drawing.Rectangle 0, 0, $w, $h
    $data = $img.LockBits($rect, 'ReadOnly', 'Format32bppArgb')
    try {
        $stride = $data.Stride
        $buf = New-Object byte[] ($stride * $h)
        [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $buf, 0, $buf.Length)
    } finally { $img.UnlockBits($data) }

    $hits = @()
    for ($ty = $Y0; $ty + $Tile -le $Y1; $ty += $Step) {
        for ($tx = $X0; $tx + $Tile -le $X1; $tx += $Step) {
            $set = [System.Collections.Generic.HashSet[int]]::new()
            for ($row = $ty; $row -lt ($ty + $Tile); $row++) {
                $base = $row * $stride
                for ($col = $tx; $col -lt ($tx + $Tile); $col++) {
                    $i = $base + $col * 4
                    $b = $buf[$i]; $g = $buf[$i + 1]; $r = $buf[$i + 2]
                    [void]$set.Add(($r -shl 16) -bor ($g -shl 8) -bor $b)
                }
            }
            if ($set.Count -ge $MinColors) {
                $hits += [pscustomobject]@{ X = $tx; Y = $ty; Colors = $set.Count }
            }
        }
    }

    Write-Output ("色种数 >= {0} 的瓦片：{1} 个" -f $MinColors, $hits.Count)
    if ($hits.Count -eq 0) {
        Write-Output '  没有检出图片类区域'
    } else {
        $hits | Sort-Object -Property @{ Expression = 'X' }, @{ Expression = 'Y' } |
            ForEach-Object { "  [{0},{1}] {2}x{3}  {4} 种色" -f $_.X, $_.Y, $Tile, $Tile, $_.Colors }
        $xs = ($hits | Measure-Object -Property X -Minimum -Maximum)
        $ys = ($hits | Measure-Object -Property Y -Minimum -Maximum)
        Write-Output ("  覆盖范围 X {0}..{1}  Y {2}..{3}" -f $xs.Minimum, $xs.Maximum, $ys.Minimum, $ys.Maximum)
    }
} finally { $img.Dispose() }
