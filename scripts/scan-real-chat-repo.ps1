<#
.SYNOPSIS
    真人聊天原文的**全仓 / 单提交**扫描器（票 93 的配套）。

.DESCRIPTION
    `scripts/check-real-chat.ps1` 是**提交时**的闸：它扫暂存区新增行与提交信息。
    这一个脚本是**事后**的扫描器：它能把**已经提交的**内容、乃至**整个仓库**
    逐行比对一遍黑名单。

    ⚠ 为什么需要它（2026-10-25 实测）：
      ① `check-real-chat.ps1` 只看**暂存区** ⇒ 提交之后就够不着那一笔了
         ⇒ 提交后想复验「这一笔到底抄没抄」没有工具。
      ② 票 93 §「分诊」里那句「靠这一条**发现**了残留，而不是靠它**拦住**残留」
         —— 全仓扫描才是**发现**残留的那一步。
      ③ 2026-10-25 重建黑名单后，正是本脚本扫出**黑名单被自家 demo 合成语料
         与产品文案污染**（19 条里只有 5 条是真人原文）
         ⇒ 修法见票 93 §「2026-10-25：黑名单的输入被污染了」。

.PARAMETER Blacklist
    黑名单文件路径。默认 `target/probe/real-chat-blacklist.txt`。
    ⚠ 不存在或为空 ⇒ 打横幅、**exit 0**：那是「**没有结论**」，不是「没问题」
      （与 `check-real-chat.ps1` 同一口径：票 67 的教训）。

.PARAMETER Mode
    · `repo`   —— 扫**全部跟踪文件**（默认）。
    · `commit` —— 扫**某个提交**的新增行 + 提交信息。

.PARAMETER Ref
    `-Mode commit` 时要扫的提交。默认 `HEAD`。

.PARAMETER MinLen
    黑名单条目的最短长度。默认 6（与刷新脚本一致，票 93 §两处决定）。

.EXAMPLE
    pwsh -NoProfile -File scripts\scan-real-chat-repo.ps1
    全仓扫描。

.EXAMPLE
    pwsh -NoProfile -File scripts\scan-real-chat-repo.ps1 -Mode commit -Ref HEAD
    扫 HEAD 这一笔的新增行与提交信息。

.NOTES
    ▸ 退出码
      0 = 没命中（或**没有结论**：缺黑名单）
      1 = 命中（会打印命中的文件/行号与命中的条目）

    ▸ ⚠ **黑名单脏了，这道扫描就白扫**
      条目里混进自家合成语料或产品自己的文案 ⇒ 满屏误报 ⇒ 没人再看它。
      ⇒ 重建黑名单后先跑 `-Mode repo`，若命中数**比预期大很多**，
        先查是不是**输入**脏了（票 93 §2026-10-25）。
#>
param(
    [string]$Blacklist = 'target/probe/real-chat-blacklist.txt',
    [ValidateSet('repo', 'commit')]
    [string]$Mode = 'repo',
    [string]$Ref = 'HEAD',
    [int]$MinLen = 6
)

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

if (-not (Test-Path -LiteralPath $Blacklist)) {
    Write-Host "!! 没有黑名单 ⇒ 没有结论（不是『没问题』）：$Blacklist"
    Write-Host '   ⇒ 先跑：pwsh -NoProfile -File scripts/refresh-real-chat-blacklist.ps1'
    exit 0
}
$entries = @(Get-Content -LiteralPath $Blacklist -Encoding UTF8 |
    Where-Object { $_.Trim().Length -ge $MinLen })
if ($entries.Count -eq 0) {
    Write-Host "!! 黑名单是空的 ⇒ 没有结论（不是『没问题』）：$Blacklist"
    exit 0
}

$hits = [System.Collections.Generic.List[object]]::new()

function Test-Line {
    param([string]$Where, [int]$LineNo, [string]$Text)
    foreach ($e in $entries) {
        $n = $e.Trim()
        if ($n.Length -ge $MinLen -and $Text.Contains($n)) {
            $hits.Add([pscustomobject]@{ Where = $Where; Line = $LineNo; Entry = $n })
        }
    }
}

if ($Mode -eq 'commit') {
    $added = @(& git show --format='' --unified=0 $Ref | Where-Object { $_ -like '+*' -and $_ -notlike '+++*' })
    $i = 0
    foreach ($l in $added) {
        $i++
        Test-Line -Where "$Ref 新增行 #$i" -LineNo $i -Text $l
    }
    $msg = @(& git log -1 --format=%B $Ref)
    $i = 0
    foreach ($l in $msg) {
        $i++
        Test-Line -Where "$Ref 提交信息 行 $i" -LineNo $i -Text $l
    }
    Write-Host ("扫：{0} 的新增行 {1} 行 + 提交信息 {2} 行；黑名单 {3} 条。" -f $Ref, $added.Count, $msg.Count, $entries.Count)
} else {
    $files = @(& git ls-files)
    $scanned = 0
    foreach ($f in $files) {
        if (-not (Test-Path -LiteralPath $f)) { continue }
        try { $lines = [IO.File]::ReadAllLines((Resolve-Path -LiteralPath $f)) } catch { continue }
        $scanned++
        for ($n = 0; $n -lt $lines.Count; $n++) {
            Test-Line -Where $f -LineNo ($n + 1) -Text $lines[$n]
        }
    }
    Write-Host ("扫：{0} 个跟踪文件；黑名单 {1} 条。" -f $scanned, $entries.Count)
}

if ($hits.Count -eq 0) {
    Write-Host 'CLEAN: 没有任何黑名单条目出现在被扫内容里'
    exit 0
}
$hits | Format-Table -AutoSize | Out-String -Width 220 | Write-Host
Write-Host ("HITS={0}" -f $hits.Count)
exit 1
