# 票据勾选状态审计。
#
# **为什么不能用 Select-String**（2026-10-13 实测翻车）：
#   同一份命令，早先报「票 20 = 0 已勾 / 14 未勾」，而文件正文里明明有 16 个
#   `[x]`。用 PS 的 `Select-String -Encoding UTF8` 走一遍仍不可靠——它的
#   路径/编码处理在中文文件名 + 连字符混合时给过假数字。
#
#   这里改用 `[IO.File]::ReadAllText`（自动按 BOM/UTF-8 解码，不过 PS 的
#   ANSI 那一层）+ `[regex]::Matches` 计数，绕开 `Get-Content` 那条路。
#
# 判据：**造出来的指标要先在一个已知样本上肉眼验一次**。
# 本脚本跑完会打印一张表，对照票正文抽一行就能看出对不对。
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$dir = '.scratch\wechat-reply-copilot\issues'
$rows = @()
foreach ($f in Get-ChildItem $dir -Filter *.md | Sort-Object Name) {
    $t = [IO.File]::ReadAllText($f.FullName)
    $done = ([regex]::Matches($t, '(?m)^\s*- \[x\]')).Count
    $todo = ([regex]::Matches($t, '(?m)^\s*- \[ \]')).Count
    # 「未验」是本仓约定的「做不了，只能留给本机」的标记。
    $blocked = ([regex]::Matches($t, '⛔')).Count
    $rows += [pscustomobject]@{
        票      = $f.BaseName
        已勾    = $done
        未勾    = $todo
        完成率  = if (($done + $todo) -gt 0) { "$([math]::Round(100 * $done / ($done + $todo)))%" } else { '-' }
        阻断标记 = $blocked
    }
}
$rows | Format-Table -AutoSize

$total = $rows | Measure-Object -Property 已勾 -Sum
Write-Output "合计已勾 $($total.Sum)"
$zero = $rows | Where-Object { $_.已勾 -eq 0 -and $_.未勾 -gt 0 }
Write-Output "一张都没勾的票：$(if ($zero) { ($zero.票 -join ', ') } else { '无' })"