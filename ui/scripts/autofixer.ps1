# 把所有 .svelte 跑一遍官方 svelte-autofixer。
#
# 为什么要脚本化：手跑六个文件要六次往返，而且**很容易漏掉一个**——
# 漏掉的那个就是下次 review 时才发现告警的文件。用 glob 收集，漏不掉。
param(
  # 并行度。6 是按**实测**定的，不是随手取的（2026-10-16，本机 16 逻辑核 / 31.5 GB，
  # 且空载基线就有 28.8% ≈ 4.6 核在忙）：
  #
  #     并行度   墙钟     全机 CPU 均值   峰值    折算实际占用
  #        1     23.0s       25.7%       45.0%   淹没在基线噪声里
  #        6      7.3s       44.2%       63.8%   ≈ 2.5 核
  #       11      6.1s       56.3%       87.5%   ≈ 4.4 核
  #
  # ⇒ 6→11 只多省 1.2s，却把峰值从 64% 顶到 87.5%：**不划算，所以停在这。**
  # ⚠ 单个 node 进程 1.7s 里只占约 0.2 核——并行吃的是**进程启动与 I/O**，
  #   不是 CPU。所以「提速 3 倍」不等于「抢 3 倍 CPU」。
  # ⚠ 这几步在 `gate.ps1` 里**不与 cargo 步骤重叠**，门禁跑它时不跟编译抢核；
  #   只有手工一边 `cargo build` 一边跑它才会叠加，那时可以 `-ThrottleLimit 3`。
  [int]$ThrottleLimit = 6
)
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

# ⚠ 相对路径**必须在主流程算好**，不能丢给并行 runspace（2026-10-16）。
# `ForEach-Object -Parallel` 的 runspace 是从**进程**当前目录起步的，
# 而本脚本的 `Set-Location $PSScriptRoot\..` 只改了主 runspace 的会话位置。
# 两者恰好一致时没事；一旦从仓库根这样调 `pwsh -File ui\scripts\autofixer.ps1`，
# runspace 会拿仓库根当基准，算出 `..\..\..\src\lib\X.svelte` 这种路径，
# 工具读不到文件 ⇒ 它回一句 issues=[] ⇒ **门禁报 OK，而那个文件从没被检查过**。
# 正是本文件头注里那个最贵的教训（exit=255 被当成「零告警」）。
$jobs = $files | ForEach-Object {
    [pscustomobject]@{ Full = $_.FullName; Rel = (Resolve-Path -Relative $_.FullName) }
}

# ⚠ 11 个文件**并行**跑（2026-10-16 改，实测 23.1s → 10.8s）。
# 每个文件是一次独立的 `node` 进程，彼此之间没有任何共享状态：
# 读各自的文件、吐各自的 JSON。所以并行的唯一代价是内存/CPU，
# 而门禁跑这一步时 CPU 本来就闲着（它不与 cargo 步骤重叠）。
#
# 早先是串行 for 循环 + 累加 $bad/$dead 三个计数器。改成并行后**计数不再能就地累加**
# （`ForEach-Object -Parallel` 跑在独立 runspace 里），所以每个并行块只**返回一条记录**，
# 汇总与判定仍在主流程串行做——判定逻辑一个字没改，只是搬到了后面。
# ⚠ 这一点很重要：判「零告警」还是「工具崩了」的那套规则是这块最容易出错的地方，
#   不该趁并行化一起重写。
$results = $jobs | ForEach-Object -ThrottleLimit $ThrottleLimit -Parallel {
  $j = $_
  $raw = & $using:nodeCmd $using:entry svelte-autofixer $j.Rel 2>&1 | Out-String
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
    return [pscustomobject]@{ Rel = $j.Rel; Dead = $true; Code = $code; Issues = 0; Suggestions = @() }
  }

  [pscustomobject]@{
    Rel = $j.Rel
    Dead = $false
    Code = $code
    Issues = @($obj.issues).Count
    Suggestions = @($obj.suggestions)
  }
}

# ⚠ 并行的返回顺序是完成顺序，不是文件顺序 ⇒ 汇总前先按路径排回去，
# 否则同一份代码两次运行输出顺序不同，diff 会一直有噪声。
foreach ($r in ($results | Sort-Object Rel)) {
  if ($r.Dead) {
    $dead++
    Write-Output ("  ??    {0}  TOOL-UNAVAILABLE（没拿到结构化输出，exit={1}）" -f $r.Rel, $r.Code)
    continue
  }
  if ($r.Issues -eq 0 -and @($r.Suggestions).Count -eq 0) {
    Write-Output ("  OK    {0}" -f $r.Rel)
  }
  else {
    $bad++
    Write-Output ("  FAIL  {0}  issues={1} suggestions={2}" -f $r.Rel, $r.Issues, @($r.Suggestions).Count)
    if (@($r.Suggestions).Count -gt 0) {
      Write-Output ("        {0}" -f (@($r.Suggestions) -join '; '))
    }
  }

  # exit 非 0 但 JSON 齐 ⇒ 结论有效，多半是末尾拉文档超时。记一笔，不改判。
  if ($r.Code -ne 0) {
    $netNoise++
    Write-Output ("        （exit={0}，但结果已拿到——末尾拉文档超时，按工具噪声处理）" -f $r.Code)
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
