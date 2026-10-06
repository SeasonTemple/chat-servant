# 统计一块区域里出现的颜色与各自像素数。用来查「脱敏产物里还剩哪些色」。
# 用法：pwsh -File scripts/colors.ps1 -Image <png> -X 120 -Y 590 -W 240 -H 80
param(
    [Parameter(Mandatory)][string]$Image,
    [int]$X = 0,
    [int]$Y = 0,
    [int]$W = 40,
    [int]$H = 20
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$img = New-Object System.Drawing.Bitmap $Image
try {
    $hist = @{}
    for ($row = $Y; $row -lt ($Y + $H); $row++) {
        for ($col = $X; $col -lt ($X + $W); $col++) {
            $c = $img.GetPixel($col, $row)
            $key = 'rgb({0,3},{1,3},{2,3})' -f $c.R, $c.G, $c.B
            if ($hist.ContainsKey($key)) { $hist[$key] += 1 } else { $hist[$key] = 1 }
        }
    }
    $total = $W * $H
    Write-Output ("{0}  [{1},{2}] {3}x{4} = {5} px  bitmap={6}x{7}" -f $Image, $X, $Y, $W, $H, $total, $img.Width, $img.Height)
    $hist.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 20 | ForEach-Object {
        Write-Output ("  {0}  {1,6} px  {2,6:P2}" -f $_.Key, $_.Value, ($_.Value / $total))
    }
    Write-Output ("  共 {0} 种颜色" -f $hist.Count)
}
finally { $img.Dispose() }