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

# ⚠ **调本地二进制，不要 `npx -y @sveltejs/mcp`**（2026-10-15 改）。
#
# 原来每个文件跑一次 `npx -y`，而 `-y` 的语义是「自动同意安装」——
# npm 缓存冷的时候每个文件都要付一次解析 + 拉取，而 `src` 下有 10 个
# .svelte。实测（2026-10-15 本机，已缓存的稳态）：**单文件 9 秒，一轮约 90 秒**，
# 且每次都查 registry，网络一慢就整步卡住且不报任何错。
#
# 现在 `@sveltejs/mcp` 进了 `devDependencies`（版本 `^0.1.26` 锁死），
# 于是：① 版本随 lockfile 走，CI 与本机一致，不会某天拉到一个新版本
# 突然多报一堆告警；② 不受 registry 波动影响；③ 离线也能跑。
#
# ⚠ 可执行文件叫 **`svelte-mcp`**，不叫 `svelte-autofixer`（2026-10-15 实测
# `node_modules/.bin/` 下的名字）。子命令才是 `svelte-autofixer`。
# 下面找不到就抛错——**不要静默回落去 npx 拉**，那正是本条要根治的事。
$localBin = Join-Path $PSScriptRoot '..\node_modules\.bin\svelte-mcp.CMD'
if (-not (Test-Path $localBin)) {
    throw "本地没有 $localBin —— 先在 ui/ 下跑 `pnpm install`（@sveltejs/mcp 是 devDependency）"
}

foreach ($f in $files) {
    $rel = Resolve-Path -Relative $f.FullName
    $out = & $localBin svelte-autofixer $rel 2>&1 | Out-String

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