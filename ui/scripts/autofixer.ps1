# 把所有 .svelte 跑一遍官方 svelte-autofixer。
#
# 为什么要脚本化：手跑六个文件要六次往返，而且**很容易漏掉一个**——
# 漏掉的那个就是下次 review 时才发现告警的文件。用 glob 收集，漏不掉。
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
$dead = 0
$netNoise = 0

# ⚠ **调本地二进制，不要 `npx -y @sveltejs/mcp`**（2026-10-15 改）。
#
# 原来每个文件跑一次 `npx -y`，而 `-y` 的语义是「自动同意安装」——
# npm 缓存冷的时候每个文件都要付一次解析 + 拉取，而 `src` 下有 10 个
# .svelte。实测（2026-10-15 本机，已缓存的稳态）：**单文件 9 秒，一轮约 90 秒**，
# 且每次都查 registry，网络一慢就整步卡住且不报任何错。
#
# 现在 `@sveltejs/mcp` 进了 `devDependencies`（版本 `^0.1.26` 锁死），
# 于是：① 版本随 lockfile 走，CI 与本机一致，不会某天拉到一个新版本
# 突然多报一堆告警；② 不受 registry 波动影响。
#
# ⚠ 可执行文件叫 **`svelte-mcp`**，不叫 `svelte-autofixer`（2026-10-15 实测
# `node_modules/.bin/` 下的名字）。子命令才是 `svelte-autofixer`。
# 下面找不到就抛错——**不要静默回落去 npx 拉**，那正是本条要根治的事。
#
# ⚠⚠ **「不查 registry」不等于「不联网」**（2026-10-15 实测，第二次踩）：
# 这个工具在**吐出结果之后**还会去 `get_sections` 拉 Svelte 文档站，
# 于是：① 网络不通时它在结果后面抛 `TimeoutError` / `ERR_CONNECTION_CLOSED`
# 并 exit 非 0；② 实测单文件 11 秒，那 11 秒大部分是等超时。
# 它**没有**离线开关（`--help` 只有 `--async` / `--svelte-version`）。
# ⇒ 超时是常态，判据必须分清「结论」与「工具自己有没有崩」——
#   这就是下面那段解析要干的事。
# ⚠⚠ **不调 `.CMD` 包装层，改用 `node <入口>` 直调**（2026-10-15，第三次踩）。
#
# `node_modules/.bin/svelte-mcp.CMD` 是给 cmd.exe 用的，**路径里的括号会
# 被它当语法吃掉**。本项目恰好有这类路径：
#     src\routes\(panel)\+layout.svelte
#     src\routes\(panel)\+page.svelte
#     src\routes\(panel)\panel\+page.svelte
# 这三个文件在改成直调之前**从来没有被真正检查过**（exit=255），
# 而旧脚本把「没拿到输出」当成「零告警」，于是门禁每轮都报
# `AUTOFIXER_ALL_CLEAN` —— **在骗人**。
#
# 入口取自 `@sveltejs/mcp` 的 package.json `bin.svelte-mcp`（`./dist/index.mjs`）。
# 版本号跟着 devDependencies 走，改依赖时这个路径要跟着核一次。
$pkgJson = Join-Path $PSScriptRoot '..\node_modules\@sveltejs\mcp\package.json'
if (-not (Test-Path $pkgJson)) {
    throw "本地没有 $pkgJson —— 先在 ui/ 下跑 `pnpm install`（@sveltejs/mcp 是 devDependency）"
}
$entry = Join-Path $PSScriptRoot ('..\node_modules\@sveltejs\mcp\' + (Get-Content $pkgJson -Raw | ConvertFrom-Json).bin.'svelte-mcp')
$entry = (Resolve-Path $entry).Path
if (-not (Test-Path $entry)) {
    throw "autofixer 入口解析不到：$entry —— bin 字段变了？核一下 package.json"
}
$nodeCmd = Get-Command node -ErrorAction SilentlyContinue
if (-not $nodeCmd) {
    throw '找不到 node。autofixer 是 node 工具，PATH 上必须有它。'
}

foreach ($f in $files) {
    $rel = Resolve-Path -Relative $f.FullName
    $raw = & $nodeCmd.Source $entry svelte-autofixer $rel 2>&1 | Out-String
    $code = $LASTEXITCODE

    # ── 取结构化输出 ──────────────────────────────────────────────
    # ⚠ 2026-10-15 改：原来用正则抓 `issues:\s*\[(.*?)\]` 再 `split ','`，
    # 而**一条 issue 的文本里含逗号**（比如 `"',' expected"`）会被切成三段
    # ⇒ 报成 issues=3，实际只有 1 个。改为整段 JSON 解析。
    $obj = $null
    $start = $raw.IndexOf('{')
    if ($start -ge 0) {
        $end = $raw.LastIndexOf('}')
        if ($end -gt $start) {
            try { $obj = $raw.Substring($start, $end - $start + 1) | ConvertFrom-Json } catch { $obj = $null }
        }
    }

    # ⚠⚠ **拿不到 JSON ⇒ 工具没跑完，不等于这个文件干净。**
    # 原来的写法是「issue 数 = 0 ⇒ OK」，于是工具崩在出结果之前时，
    # 空输出与「干净」完全同形，门禁**静默放行一个从没被检查过的文件**。
    # 那比误报严重得多：误报会被看见，静默放行不会。
    if ($null -eq $obj) {
        $dead++
        Write-Output ("  ??    {0}  TOOL-UNAVAILABLE（没拿到结构化输出，exit={1}）" -f $rel, $code)
        continue
    }

    $nIssue = @($obj.issues).Count
    $nSug = @($obj.suggestions).Count

    if ($nIssue -eq 0 -and $nSug -eq 0) {
        Write-Output ("  OK    {0}" -f $rel)
    }
    else {
        $bad++
        Write-Output ("  FAIL  {0}  issues={1} suggestions={2}" -f $rel, $nIssue, $nSug)
        if ($nSug -gt 0) { Write-Output ("        {0}" -f (@($obj.suggestions) -join '; ')) }
    }

    # exit 非 0 但 JSON 齐 ⇒ 结论有效，多半是末尾拉文档超时。记一笔，不改判。
    if ($code -ne 0) {
        $netNoise++
        Write-Output ("        （exit={0}，但结果已拿到——末尾拉文档超时，按工具噪声处理）" -f $code)
    }
}

Write-Output ""
if ($dead -gt 0) {
    Write-Output "AUTOFIXER_TOOL_UNAVAILABLE ($dead / $($files.Count) 个文件没被检查到)"
    Write-Output "  ⇒ 这一步**没有结论**，不是「有问题」。查网络/工具本身，别改代码。"
    exit 2
}
if ($netNoise -gt 0) {
    Write-Output "（$netNoise / $($files.Count) 个文件在末尾拉文档时超时，结论不受影响）"
}
if ($bad -eq 0) {
    Write-Output "AUTOFIXER_ALL_CLEAN ($($files.Count) files)"
    exit 0
}
Write-Output "AUTOFIXER_HAS_WARNINGS ($bad / $($files.Count))"
exit 1
