# 检查 `.github/workflows/ci.yml` 有没有漏掉 `scripts/gate.ps1` 里的某一步。
#
# ## 为什么要有这一步
#
# `CONTRIBUTING.md` / `README.md` / `README.zh-CN.md` 三处都写着「CI 与本地门禁
# 逐条对应」，而这句话**从来没有被任何东西验证过**。2026-10-18 第一次去核，发现
# `sheet pack in sync`（`ui/scripts/sheetpack.ps1 -Check`，纯 pwsh，CI 完全能跑）
# **本地每次都跑、CI 里根本没有**。后果很具体：一份改了 SVG 内嵌 base64 的 PR，
# **本地门禁红、CI 绿**——而人往往更信绿灯。
#
# 「三处文档都说对应」并没有让这句话更真一点，反而是最容易骗人的东西：
# 副本之间的一致性看起来像互相印证。
#
# ## 判据：名单从 gate.ps1 **反推**，不是从 ci.yml 数出来
#
# 方向很关键。若从 ci.yml 数「一共几步」再和本地比，改哪边都能自洽。
# 这里改成：`gate.ps1` 里每个 `Step "…"` 都必须在 CI 里有一个**见证子串**，
# 且**见证子串取自命令本身**（`cargo test --workspace`、`sheetpack.ps1 -Check`），
# 不是取自步的名字——名字两边本来就故意不一样（本地 `ui build` / CI `vite build`）。
#
# 三类红，各自对应一种真实漂移：
#
# | 红 | 现实含义 |
# |---|---|
# | 本地步没有见证条目 | 有人给本地加了步，**还没决定 CI 要不要跟** —— 必须人来判 |
# | 见证子串在 ci.yml 里找不到 | 本地跑了、CI 没跑（就是 sheetpack 那次） |
# | 表里有条目但本地已无此步 | 本地删了步，表没清 —— 会让前两条变宽松 |
#
# ## 为什么这份表是手写的
#
# 命令完全自动抽取看着更美，但有三处**正当**差异挡着：
#
# · `cargo deny check` 本地走 `scripts/deny.ps1`（代理回退逻辑在那一层），
#   CI 直接跑 `cargo deny check`——GitHub runner 直连，没有那个代理问题。
#   ⇒ 见证取**内层**命令，不是包装脚本。
# · `secret scan` 本地没设 `CHATSERVANT_PROBE_KEY` 时**跳过**，CI 用运行时生成的
#   占位 key 必跑。⇒ 两边步数天生不等，这正是 ci.yml 刻意不写 `/总数` 的原因。
# · 其余步都是同一条命令，只是 `Push-Location ui` vs `working-directory`。
#
# 手写表的代价是「本地加了步要顺手加一行」，而那正是本脚本会主动喊出来的事。
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$gatePath = 'scripts\gate.ps1'
$ciPath   = '.github\workflows\ci.yml'

# key = gate.ps1 里的 Step 名（逐字）；value = 必须在 ci.yml 里出现的见证子串。
# 见证取**命令**。改命令时这里要一起改——那正是它该响的时候。
$witness = [ordered]@{
  'mojibake scan'         = 'scan-mojibake.ps1'
  'sheet pack in sync'    = 'sheetpack.ps1 -Check'
  'cargo fmt --all --check' = 'cargo fmt --all --check'
  'cargo clippy -D warnings' = 'cargo clippy --workspace --all-targets -- -D warnings'
  'cargo test --workspace' = 'cargo test --workspace'
  'secret scan'           = 'secret-scan'
  'cargo deny check'      = 'cargo deny check'
  'ui svelte-check'       = 'pnpm check'
  'ui lint'               = 'pnpm lint'
  'ui svelte-autofixer'   = 'scripts/autofixer.ps1'
  'ui build'              = 'pnpm build'
  # 本脚本自己也在名单里：谁把这一步从 CI 删掉，本守卫会喊。
  'gate/CI parity'        = 'check-gate-parity.ps1'
}

foreach ($p in @($gatePath, $ciPath)) {
  if (-not (Test-Path $p)) { Write-Output "找不到 $p"; exit 1 }
}

$gate = [IO.File]::ReadAllText((Resolve-Path $gatePath))
$ciRaw = [IO.File]::ReadAllText((Resolve-Path $ciPath))

# ⚠⚠ **匹配前必须先剥掉注释**，否则这个守卫是假的（2026-10-18 自证当场抓到）。
#
# 第一版直接在 `ci.yml` 全文里找见证子串，结果「从 ci.yml 删掉 sheet pack in sync」
# 这一组**没红**——因为本文件上方那段讲这件事的注释里就写着
# `ui/scripts/sheetpack.ps1 -Check`，散文替那一步交了账。
# ⇒ 一个断言「CI 跑过这条命令」的守卫，被**描述这条命令的注释**满足了。
# 这是「守卫断言了中间量、没断言产物」的同一族：查的是文本，不是可执行内容。
#
# 所以只对**去掉注释之后**的正文做匹配。附带好处：以后再往 ci.yml 里补订正注释，
# 不会不小心把守卫的嘴堵上。
function Remove-YamlComment($line) {
  if ($line -match '^\s*#') { return '' }          # 整行注释（含块标量里的 # 行）
  $q = ([regex]::Matches($line, '(?<!\\)"')).Count
  if ($q % 2 -eq 0) {
    # 引号成对 ⇒ 行尾 `#` 一定是真注释（YAML 里 ` #` 才起注释）
    $cut = $line -replace '\s+#.*$', ''
    return $cut
  }
  return $line                                    # 引号不成对（多半是跨行的 run: | 块），原样留
}
$ci = (($ciRaw -split "`r?`n") | ForEach-Object { Remove-YamlComment $_ }) -join "`n"

# 步名按出现顺序取，判据是**有向覆盖**，不比较条数。
$steps = [regex]::Matches($gate, '(?m)^\s*Step\s+"([^"]+)"') |
  ForEach-Object { $_.Groups[1].Value }

if ($steps.Count -eq 0) {
  # 量具先自检：一个 Step 都没解析出来，多半是 gate.ps1 的写法变了，
  # 而「解析器失效」和「门禁真的空了」在屏幕上长得一模一样。
  Write-Output "解析不到任何 Step —— gate.ps1 的写法变了？本守卫此刻什么都没验。"
  exit 1
}

$fail = [System.Collections.Generic.List[string]]::new()

# ① 本地步没有见证条目
$uncovered = $steps | Where-Object { -not $witness.Contains($_) }
if ($uncovered) {
  $fail.Add('本地门禁新增了步，但本表没有它 —— 必须决定 CI 要不要跟：')
  $uncovered | ForEach-Object { $fail.Add(('    gate.ps1: Step "{0}"' -f $_)) }
}

# ② 见证子串在 ci.yml 里找不到（= 本地跑了、CI 没跑）
$missing = [System.Collections.Generic.List[string]]::new()
foreach ($k in $witness.Keys) {
  if ($steps -notcontains $k) { continue }
  if (-not $ci.Contains($witness[$k])) {
    $missing.Add(('    gate.ps1 Step "{0}" -> ci.yml 里找不到 {1}' -f $k, $witness[$k]))
  }
}
if ($missing.Count) {
  $fail.Add('本地跑了、CI 没跑：')
  $missing | ForEach-Object { $fail.Add($_) }
}

# ③ 表里有条目但本地已无此步（本地删了步，表没清 → 前两条会变宽松）
$orphan = $witness.Keys | Where-Object { $steps -notcontains $_ }
if ($orphan) {
  $fail.Add('本表里的条目在 gate.ps1 里已经不存在了（本地删了步，表没清）：')
  $orphan | ForEach-Object { $fail.Add(('    Step "{0}"' -f $_)) }
}

if ($fail.Count) {
  Write-Output 'gate.ps1 与 ci.yml 没有对齐：'
  Write-Output ''
  $fail | ForEach-Object { Write-Output $_ }
  Write-Output ''
  Write-Output '本地红、CI 绿是最坏的组合：人更信绿灯，而这份绿灯少跑了一步。'
  Write-Output '改哪边都行，但要两边一起改，并在这里如实记账（exceptions 就写明理由）。'
  exit 1
}

Write-Output ("gate/CI 对齐：gate.ps1 的 {0} 步，在 ci.yml 里都能找到见证命令。" -f $steps.Count)
exit 0
