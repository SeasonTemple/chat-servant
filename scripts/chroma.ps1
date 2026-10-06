# 量一块区域的彩度分布，用来区分「照片」与「灰度卡片/引文块」。
#
# 为什么不直接看图：候选区域里可能含聊天正文。**只出数字，不出像素。**
# 判据：彩度 = max(r,g,b) - min(r,g,b)。灰度内容恒为 0；
# 照片必然有彩色像素。
#
# ⚠ 变量名教训：PowerShell 变量**大小写不敏感**，`$w` 和 `$W` 是同一个。
# 早先的版本同时用了 `$W`（区域宽）和 `$w`（图片宽），区域参数被图片
# 尺寸静默覆盖，扫的是整幅图——四条候选量出来几乎一模一样才发现。
# 现在全部用带前缀的名字，物理上不可能撞。
#
# 用法：pwsh -File scripts/chroma.ps1 -Image a.png -X 1028 -Y 103 -RectW 207 -RectH 120
param(
    [Parameter(Mandatory)][string]$Image,
    [Parameter(Mandatory)][int]$X,
    [Parameter(Mandatory)][int]$Y,
    [Parameter(Mandatory)][int]$RectW,
    [Parameter(Mandatory)][int]$RectH
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$bitmap = New-Object System.Drawing.Bitmap $Image
try {
    $imgW = $bitmap.Width
    $imgH = $bitmap.Height
    $full = New-Object System.Drawing.Rectangle 0, 0, $imgW, $imgH
    $data = $bitmap.LockBits($full, 'ReadOnly', 'Format32bppArgb')
    try {
        $stride = $data.Stride
        $buf = New-Object byte[] ($stride * $imgH)
        [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $buf, 0, $buf.Length)
    } finally { $bitmap.UnlockBits($data) }

    $total = 0; $chromaSum = 0; $chromaMax = 0
    $gray = 0; $low = 0; $mid = 0; $high = 0
    $colors = [System.Collections.Generic.HashSet[int]]::new()

    for ($row = $Y; $row -lt ($Y + $RectH); $row++) {
        if ($row -lt 0 -or $row -ge $imgH) { continue }
        $rowBase = $row * $stride
        for ($col = $X; $col -lt ($X + $RectW); $col++) {
            if ($col -lt 0 -or $col -ge $imgW) { continue }
            $idx = $rowBase + $col * 4
            $b = $buf[$idx]; $g = $buf[$idx + 1]; $r = $buf[$idx + 2]
            # ⚠ 移位前必须转 [int]：`[byte]200 -shl 16` 在 PowerShell 里是 **0**
            # （字节算术溢出），颜色键会塌成少数几个值，色种数少算一个数量级。
            $key = ([int]$r -shl 16) -bor ([int]$g -shl 8) -bor [int]$b
            [void]$colors.Add($key)
            $mx = [Math]::Max($r, [Math]::Max($g, $b))
            $mn = [Math]::Min($r, [Math]::Min($g, $b))
            $chroma = $mx - $mn
            $chromaSum += $chroma; $total++
            if ($chroma -gt $chromaMax) { $chromaMax = $chroma }
            if ($chroma -eq 0) { $gray++ }
            elseif ($chroma -le 8) { $low++ }
            elseif ($chroma -le 24) { $mid++ }
            else { $high++ }
        }
    }

    if ($total -eq 0) {
        Write-Output ("区域 [{0},{1}] {2}x{3}  **没采到任何像素**" -f $X, $Y, $RectW, $RectH)
        return
    }
    Write-Output ("区域 [{0},{1}] {2}x{3}  采样 {4} px（应为 {5}）" -f `
        $X, $Y, $RectW, $RectH, $total, ($RectW * $RectH))
    Write-Output ("  精确色种数 {0}   彩度 均值 {1:N1}   最大 {2}" -f `
        $colors.Count, ($chromaSum / $total), $chromaMax)
    Write-Output ("  彩度=0  (纯灰)   {0,6:P2}" -f ($gray / $total))
    Write-Output ("  彩度 1-8 (微彩)   {0,6:P2}" -f ($low / $total))
    Write-Output ("  彩度 9-24(中彩)   {0,6:P2}" -f ($mid / $total))
    Write-Output ("  彩度>=25(重彩)   {0,6:P2}" -f ($high / $total))
} finally { $bitmap.Dispose() }
