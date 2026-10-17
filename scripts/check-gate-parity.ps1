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

# ─────────────────────────────────────────────────────────────────────────
# ④（另一条不变量，同属「CI 能不能跑本地能跑的东西」）
#
# `tauri.conf.json` 的 `bundle.resources` 里**只要有一条指向仓库之外**（`target/`），
# 干净 runner 上就不存在，tauri-build 的 build script 会在**每个 cargo 步骤**
# （clippy / test / build 全都编 chat-servant-app）抛 `GlobPathNotFound`。
#
# 2026-10-18 实测就是这个，而且**同一个缺陷在两个文件里各犯了一次**：
#   · `ci.yml` —— 从 `ab8831d`（加 resources 那次）起连红 28 次，
#     边界是实测的：`acf3d0f` 绿、`ab8831d` 红。
#   · `release.yml` 的 `verify` job —— dry run `39036210343` 当场抓到，
#     报错逐字相同。**修了 ci.yml 却没修隔壁文件，它就原样复发。**
#
# ⇒ 这条比「少跑一步」更阴：它不是让 CI 少做，是让 CI **根本编不起来**，
#   而本地全绿（本机有权重，glob 命中）。
#
# 所以这里扫的是**所有** workflow 文件，而且**逐 job**：
# 一个 workflow 里可以同时存在「需要权重的 build job」与「不需要的 verify job」，
# 文件级判断会把它们混成一句「这个文件取没取权重」。
#
# 规则（逐 job）：**要么自己在跑 cargo 之前把权重取回来，要么声明 null 覆盖。**
#
# ⚠ job 的切分是**行级文本扫描**（`jobs:` 之后两空格缩进的 `name:` 切块），
#   不是 YAML 解析——那会给一个守卫脚本拖进一个 YAML 依赖。
#   覆盖只认「workflow 顶格 env」或「该 job 自己的块内」两处；
#   写在第三处（比如 job 内某个 step 的 env）认不出来，属已知的漏检面。
$confPath = 'ui\src-tauri\tauri.conf.json'
if (Test-Path $confPath) {
  $confRaw = [IO.File]::ReadAllText((Resolve-Path $confPath))
  # 取 resources 那段里所有形如 "../../..." 的 key；只认仓库外的相对起点。
  $outside = [regex]::Matches($confRaw, '"(\.\./\.\./[^"]*)"\s*:') |
    ForEach-Object { $_.Groups[1].Value } |
    Where-Object { $_ -match '^\.\./\.\./(target|\.\.)/' -or $_ -match '\.\./\.\./target/' } |
    Sort-Object -Unique
  if ($outside) {
    $wfDir = '.github\workflows'
    $nJobs = 0
    foreach ($wf in (Get-ChildItem $wfDir -Filter '*.yml' | Sort-Object Name)) {
      $wfRaw = [IO.File]::ReadAllText($wf.FullName)
      $lines = $wfRaw -split "`r?`n"
      # 只认 **workflow 级**的覆盖（顶格 `env:` 块里的 TAURI_CONFIG）。
      # ⚠ 两个坑都踩过：
      #   ① 曾经写成「文件里任意地方出现过就算」——release.yml 里 verify job 的
      #      覆盖会替**整个文件**免检，build job 的取权重步骤删掉也不响（自证组 L）。
      #   ② 改成只认顶格的 `TAURI_CONFIG:` 又错位了——workflow 级 env 的**子项**
      #      是 2 空格缩进（job 级是 6 空格）。判据改成「顶格 env: 块的内容」。
      $fileOverride = $false
      for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^jobs:\s*$') { break }      # 顶格 env 只可能在 jobs 之前
        if ($lines[$i] -notmatch '^env:\s*$') { continue }
        for ($k = $i + 1; $k -lt $lines.Count; $k++) {
          if ($lines[$k] -notmatch '^\s+\S') { break }    # 出了这个块（回到顶格或空行）
          if ($lines[$k] -match 'TAURI_CONFIG:.*"resources"\s*:\s*null') { $fileOverride = $true }
        }
        break
      }

      # 切出 jobs 之后每个 job 的块
      $sawJobs = $false
      $jobStarts = @()
      $inJobs = $false
      for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^jobs:\s*$') { $inJobs = $true; $sawJobs = $true; continue }
        if ($inJobs -and $lines[$i] -match '^  ([A-Za-z0-9_-]+):\s*$') {
          $jobStarts += [pscustomobject]@{ Name = $Matches[1]; Line = $i }
        }
      }
      # ⚠ 有 `jobs:` 却一个 job 都切不出来 ⇒ **量具失效**，不能当成「没发现问题」。
      #   真实来源：自证组 M 的一次正则写错（末尾 `.*\n` 配 `(?s)` 贪婪）
      #   把整个 workflow 吃掉，只剩 27 行，守卫对着残文件报「合规」。
      #   「量具坏了」和「量具没找到东西」在屏幕上原本长得一模一样。
      if ($sawJobs -and $jobStarts.Count -eq 0) {
        $fail.Add(('{0}.yml 里有 jobs: 却一个 job 都切不出来 —— 本守卫此刻什么都没验' -f $wf.BaseName))
        $fail.Add('    （要么 job 的缩进写法变了，要么文件被改坏了。别把「没查到」当「没问题」。）')
        continue
      }
      if ($jobStarts.Count -eq 0) { continue }

      for ($j = 0; $j -lt $jobStarts.Count; $j++) {
        $from = $jobStarts[$j].Line
        $to = if ($j + 1 -lt $jobStarts.Count) { $jobStarts[$j + 1].Line - 1 } else { $lines.Count - 1 }
        $block = $lines[$from..$to]
        $nJobs++

        $cargoAt = -1; $fetchAt = -1
        for ($k = 0; $k -lt $block.Count; $k++) {
          $ln = $block[$k]
          if ($ln -match '^\s*#') { continue }            # 注释里的 cargo 不算
          # ⚠ `tauri build` / `pnpm tauri build` **本身就是 cargo 步骤**（CLI 去调
          #   cargo build）。只认字面 `cargo` 的话，release.yml 那个**唯一真正需要
          #   权重**的 build job 会被整段跳过——自证组 L 删掉它的取权重步骤也不响。
          if ($cargoAt -lt 0 -and
              ($ln -match '\bcargo\s+(clippy|test|build|check|run|deny|fmt|xtask)\b' -or
               $ln -match '\b(tauri|pnpm tauri)\s+build\b')) { $cargoAt = $k }
          if ($fetchAt -lt 0 -and $ln -match 'fetch_models\.py' -and $ln -notmatch '--check') { $fetchAt = $k }
        }
        if ($cargoAt -lt 0) { continue }                  # 这个 job 不跑 cargo

        $jobOverride = $fileOverride -or (($block -join "`n") -match '"resources"\s*:\s*null')
        $fetchedFirst = ($fetchAt -ge 0 -and $fetchAt -lt $cargoAt)
        if (-not ($jobOverride -or $fetchedFirst)) {
          $fail.Add(('{0}.yml 的 job `{1}`：跑 cargo 时权重还不在盘上，也没有 null 覆盖' -f $wf.BaseName, $jobStarts[$j].Name))
          $fail.Add('    ⇒ 干净 runner 上 tauri-build 会在那一步抛 GlobPathNotFound，')
          $fail.Add('      而本机有这些文件所以本地全绿。修法二选一：')
          $fail.Add('      · 在跑 cargo 之前加一步取权重（python scripts/fetch_models.py）；')
          $fail.Add('      · 或给该 job 加 env：TAURI_CONFIG: ''{"bundle":{"resources":null}}''')
          $fail.Add('        （必须是 null：TAURI_CONFIG 走 json_patch::merge，空对象是空操作。）')
        }
      }
    }
    $script:jobsChecked = $nJobs
  }
} else {
  Write-Output "找不到 $confPath —— resources 那条不变量本次没验。"
  exit 1
}

# ─────────────────────────────────────────────────────────────────────────
# ⑤ Windows 上 Python 打印中文会崩：stdout 默认 cp1252
#
# 实测（dry run `39037571268`，2026-10-18）：
#   UnicodeEncodeError: 'charmap' codec can't encode characters in position 0-1
#     File "...\Lib\encodings\cp1252.py", line 19, in encode
#       print(f"下载 {name} …")
# `actions/setup-python` 装的 Python 3.12 在 Windows 上 stdout 用传统代码页，
# 而本仓的脚本**输出全是中文**。
#
# 规则：**gate.ps1 用 python 调起来的脚本，若含非 ASCII，就必须自己 reconfigure。**
# 为什么要机器查：这一类错在**调用方**才显形（CI 的 runner、本机 bench 台架），
# 本地「记得设 PYTHONIOENCODING=utf-8」就能糊过去——本会话里我糊了好几次。
# `scripts/targetgc.py` 早就有 reconfigure（说明这个坑在别处已咬过一次），
# 而 `fetch_models.py` / `targetaudit.py` / `ticketstatus.py` 三家都没有。
$pyInvoked = [regex]::Matches($gate, '(?m)python[\s\\]+([\w\\./-]+\.py)') |
  ForEach-Object { $_.Groups[1].Value -replace '\\', '/' } |
  Sort-Object -Unique
$pyNonAscii = 0
foreach ($py in $pyInvoked) {
  if (-not (Test-Path $py)) {
    $fail.Add("gate.ps1 要跑 $py，但仓库里没有这个文件")
    continue
  }
  $pyRaw = [IO.File]::ReadAllText((Resolve-Path $py))
  if ($pyRaw -notmatch '[^\x00-\x7F]') { continue }   # 全 ASCII，cp1252 编得动，不管
  $pyNonAscii++
  if ($pyRaw -notmatch '\.reconfigure\s*\(') {
    $fail.Add("$py 含非 ASCII 输出，但没自己 reconfigure stdout/stderr")
    $fail.Add('    ⇒ Windows 上（CI 的 runner 尤其）stdout 默认 cp1252，打印中文会 UnicodeEncodeError。')
    $fail.Add('      修：在模块顶层加')
    $fail.Add('        for _s in (sys.stdout, sys.stderr):')
    $fail.Add('            if hasattr(_s, "reconfigure"): _s.reconfigure(encoding="utf-8", errors="replace")')
    $fail.Add('      别指望调用方记得设 PYTHONIOENCODING。样板：scripts/targetgc.py')
  }
}

if ($fail.Count) {
  Write-Output 'CI 与本机门禁没有对齐：'
  Write-Output ''
  $fail | ForEach-Object { Write-Output $_ }
  Write-Output ''
  Write-Output '「本地绿、CI 红」和它的反面倒过来都最坏：人更信另一侧，'
  Write-Output '而另一侧那份结论少跑了东西、或根本编不起来。改哪边都行，但要两边一起改。'
  exit 1
}

$j = if ($script:jobsChecked) { $script:jobsChecked } else { 0 }
Write-Output ("gate/CI 对齐：gate.ps1 的 {0} 步在 ci.yml 里都有见证命令；" -f $steps.Count)
Write-Output ("bundle.resources 那条：扫了 {0} 个 workflow 的 job，逐个都有覆盖或先取权重；" -f $j)
Write-Output ("Python 编码那条：gate.ps1 调起 {0} 个脚本，其中 {1} 个含非 ASCII，都已自己 reconfigure。" -f $pyInvoked.Count, $pyNonAscii)
exit 0
