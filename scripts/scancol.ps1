# 沿一条竖线逐段统计色种数，用来定位媒体的真实上下边界。
# 用法：pwsh -File scripts/scancol.ps1 -Image target/probe/text-chat.png -X0 1010 -X1 1150 -Step 10
param(
    [Parameter(Mandatory)][string]$Image,
    [int]$X0 = 1010,
    [int]$X1 = 1150,
    [int]$Step = 10
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$img = New-Object System.Drawing.Bitmap $Image
try {
    $w = $img.Width; $h = $img.Height
    $rect = New-Object System.Drawing.Rectangle 0, 0, $w, $h
    $data = $img.LockBits($rect, 'ReadOnly', 'Format32bppArgb')
    try {
        $stride = $data.Stride
        $buf = New-Object byte[] ($stride * $h)
        [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $buf, 0, $buf.Length)
    } finally { $img.UnlockBits($data) }

    Write-Output ("帧 {0}  {1}x{2}  列区间 X {3}..{4}" -f $Image, $w, $h, $X0, $X1)
    for ($y = 0; $y + $Step -le $h; $y += $Step) {
        $set = [System.Collections.Generic.HashSet[int]]::new()
        for ($row = $y; $row -lt ($y + $Step); $row++) {
            $base = $row * $stride
            for ($col = $X0; $col -lt $X1; $col++) {
                $i = $base + $col * 4
                [void]$set.Add((($buf[$i + 2] -shl 16) -bor ($buf[$i + 1] -shl 8) -bor $buf[$i]))
            }
        }
        $bar = '#' * [Math]::Min(60, [int]($set.Count / 10))
        Write-Output ("  Y {0,4}  {1,4} 种  {2}" -f $y, $set.Count, $bar)
    }
} finally { $img.Dispose() }
