# 把一小块区域打成字符图，看像素到底长什么样。
# 每个字符代表一个像素，按亮度分档；便于判断「残留是整块还是零星散点」。
# 用法：pwsh -File scripts/ascii.ps1 -Image <png> -X 130 -Y 630 -W 40 -H 20
param(
    [Parameter(Mandatory)][string]$Image,
    [int]$X = 0,
    [int]$Y = 0,
    [int]$W = 40,
    [int]$H = 20
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

# 亮度分档：越靠后越亮
$ramp = @('.', ':', '-', '=', '+', '*', '#', '%', '@')

$img = New-Object System.Drawing.Bitmap $Image
try {
    Write-Output ("{0}  [{1},{2}] {3}x{4}  bitmap={5}x{6}" -f $Image, $X, $Y, $W, $H, $img.Width, $img.Height)
    for ($row = $Y; $row -lt ($Y + $H); $row++) {
        $line = New-Object System.Text.StringBuilder
        for ($col = $X; $col -lt ($X + $W); $col++) {
            $c = $img.GetPixel($col, $row)
            $luma = 0.299 * $c.R + 0.587 * $c.G + 0.114 * $c.B
            $idx = [Math]::Min($ramp.Length - 1, [Math]::Max(0, [int]($luma / 256 * $ramp.Length)))
            [void]$line.Append($ramp[$idx])
        }
        Write-Output ("{0,4}| {1}" -f $row, $line.ToString())
    }
}
finally { $img.Dispose() }