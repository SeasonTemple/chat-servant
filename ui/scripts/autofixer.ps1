# 把所有 .svelte 跑一遍官方 svelte-autofixer。
#
# 为什么要脚本化：手跑六个文件要六次 npx 往返，而且**很容易漏掉一个**——
# 漏掉的那个就是下次 review 时才发现告警的文件。
# 用 glob 收集，漏不掉。
$ErrorActionPreference = 'Stop'
# 找 pwsh 而不是写死绝对路径：写死的路径既是本机用户名（会跟着进公开仓库），
# 在别的机器和 CI 上也必然不存在。Get-Command 认的就是本机 PATH。
$pwshCmd = Get-Command pwsh -ErrorAction SilentlyContinue
if (-not $pwshCmd) {
  throw '找不到 pwsh（PowerShell 7+）。本脚本与 gate.ps1 都依赖它。'
}
$pwsh = $pwshCmd.Source
Set-Location $PSScriptRoot\..

$files = Get-ChildItem -Path src -Recurse -Filter *.svelte | Sort-Object FullName
$bad = 0

foreach ($f in $files) {
    $rel = Resolve-Path -Relative $f.FullName
    $out = & npx -y @sveltejs/mcp svelte-autofixer $rel 2>&1 | Out-String

    # 只认结构化输出。autofixer 返回 JSON，形如
    #   { issues: [...], suggestions: [...], require_another_tool_call_after_fixing: bool }
    $issues = ([regex]::Match($out, 'issues:\s*\[(.*?)\]', 'Singleline')).Groups[1].Value.Trim()
    $sug = ([regex]::Match($out, 'suggestions:\s*\[(.*?)\]', 'Singleline')).Groups[1].Value.Trim()

    $nIssue = if ($issues -eq '') { 0 } else { ($issues -split ',').Count }
    $nSug = if ($sug -eq '') { 0 } else { ($sug -split ',\s*(?=$)').Count }

    if ($nIssue -eq 0 -and $nSug -eq 0) {
        Write-Output ("  OK    {0}" -f $rel)
    }
    else {
        $bad++
        Write-Output ("  FAIL  {0}  issues={1} suggestions={2}" -f $rel, $nIssue, $nSug)
        if ($nSug -gt 0) { Write-Output ("        {0}" -f $sug) }
    }
}

Write-Output ""
if ($bad -eq 0) {
    Write-Output "AUTOFIXER_ALL_CLEAN ($($files.Count) files)"
    exit 0
}
Write-Output "AUTOFIXER_HAS_WARNINGS ($bad / $($files.Count))"
exit 1