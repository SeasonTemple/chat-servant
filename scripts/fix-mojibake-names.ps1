$ErrorActionPreference = 'Stop'
$dir = $args[0]
# 原名由云端会话的交接说明给出；这里按**内容首行标题**对号入座，不靠猜——
# 猜文件名就是本项目反复在犯的那类错。
# ⚠ 标题两边都要去空白再比：Markdown 里 `# 本机验收清单` 的空格会让等值判断落空。
$want = @{
    '#本次改动清单' = '改动清单.md'
    '#本机验收清单' = '本机验收清单.md'
    '#面板设计·已落地' = '面板设计.md'
}
foreach ($f in Get-ChildItem $dir -Filter *.md) {
    $head = ((Get-Content $f.FullName -TotalCount 1 -Encoding UTF8) -replace '\s', '')
    $target = $want[$head]
    if ($target -and $f.Name -ne $target) {
        $dst = Join-Path $dir $target
        if (-not (Test-Path $dst)) {
            Rename-Item $f.FullName $dst
            "恢复：$($f.Name) → $target"
            continue
        }
    }
    "保持：$($f.Name)"
}