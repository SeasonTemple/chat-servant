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
  'version consistency'   = 'check-version.ps1'
  'license consistency'   = 'check-license.ps1'
  'opaque backgrounds'    = 'check-opaque-bg.ps1'
  'forensics hook guard'  = 'check-hooks.ps1'
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
  # ⚠⚠ 下面这段扫描**不再被 `$outside` 门住**（原来是的）。
  # 它同时供两条不变量用：④ bundle.resources 与 ⑥「跑测试前备夹具」，
  # 而 ⑥ 与 resources 无关。⇒ 门住的代价是「哪天 resources 被清干净，
  # 整段扫描连同 ⑥ 一起不再执行」，⑥ 就会**静默地不再验**——
  # 正是本文件量具自检（`有 jobs: 却切不出 job ⇒ 红`）要防的那同一件事。
  # 只有 ④ 那一条判定自己还留在 `if ($outside)` 里面。
  #
  # ⚠⚠ **别把它写成一个裸的 `{ … }` 块**：PowerShell 在语句位置会把裸块
  # 当成要**输出**的脚本块，于是整段源码被打印出来，而里面赋的
  # `$script:wfCount` / `$script:testJobsChecked` 全是空 ⇒ 结论行变成
  # 「扫了  个 workflow 的 0 个 job」还 exit=0。第一次就踩了。
  $wfDir = '.github\workflows'
    $nJobs = 0
    $script:wfCount = 0
    $script:testJobsChecked = 0
    foreach ($wf in (Get-ChildItem $wfDir -Filter '*.yml' | Sort-Object Name)) {
      $script:wfCount++
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

        $cargoAt = -1; $fetchAt = -1; $testAt = -1; $corpusAt = -1
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
          # ⚠ `cargo nextest` 也是跑测试，只认字面 `cargo test` 会给它开一个后门。
          if ($testAt -lt 0 -and $ln -match '\bcargo\s+(test\b|nextest\b)') { $testAt = $k }
          if ($corpusAt -lt 0 -and $ln -match 'ocrbench[\\/]gen_corpus\.py') { $corpusAt = $k }
        }

        # ── ⑥ 跑测试的 job 必须先造出夹具，否则「跳过」与「通过」同形 ──
        #
        # 不变量：**每个会跑测试的 workflow job，都要在它之前把权重与合成语料备齐。**
        #
        # ⚠⚠ 这条是被现实教出来的，而且**同一个不变量已经复发三次**：
        #   ① `ci.yml` 从不取权重 ⇒ 字准守卫在 CI 上是死代码；
        #   ② 取了权重却没造语料 ⇒ run `39142436305` 全绿，而那条测试
        #      `finished in 0.00s`（本机真推理 20.61 s）——**验的是一次跳过**；
        #   ③ `release.yml` 的 verify job 同款（`cargo test` 在第 69 行，
        #      取权重在另一个 job 的第 225 行）。
        # ⇒ 三次都是**逐个文件**修的，而范围本该从「这条不变量在哪些地方成立」推。
        #
        # ⚠ 为什么这条不能用上面那个 null 覆盖豁免：TAURI_CONFIG 只解决
        #   **编得过**（tauri-build 的 glob），而字准测试要的是**文件在盘上**。
        #   现在缺件会 panic（`CI` 存在时），所以后果从「静默跳过」变成
        #   **整个 workflow 红、build job 被 needs 连坐跳过**——同 ① 的形状。
        if ($testAt -ge 0) {
          $script:testJobsChecked++
          if ($fetchAt -lt 0 -or $fetchAt -gt $testAt) {
            $fail.Add(('{0}.yml 的 job `{1}`：跑测试之前没有取权重（fetch_models.py）' -f $wf.BaseName, $jobStarts[$j].Name))
          }
          if ($corpusAt -lt 0 -or $corpusAt -gt $testAt) {
            $fail.Add(('{0}.yml 的 job `{1}`：跑测试之前没有造合成语料（ocrbench/gen_corpus.py）' -f $wf.BaseName, $jobStarts[$j].Name))
            $fail.Add('    ⇒ 字准守卫会走「缺件」那一支。⚠ 在 cargo test 的输出里，')
            $fail.Add('      跳过与通过**逐字同形**（都是 `... ok`），只有一个 `finished in`')
            $fail.Add('      能区分：0.00s 是跳过，十几秒才是真跑。')
          }
        }

        if ($cargoAt -lt 0) { continue }                  # 这个 job 不跑 cargo

        $jobOverride = $fileOverride -or (($block -join "`n") -match '"resources"\s*:\s*null')
        $fetchedFirst = ($fetchAt -ge 0 -and $fetchAt -lt $cargoAt)
        if ($outside -and -not ($jobOverride -or $fetchedFirst)) {
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
# 规则：**本项目自有的 Python 脚本，若含非 ASCII，就必须自己 reconfigure。**
# 为什么要机器查：这一类错在**调用方**才显形（CI 的 runner、本机 bench 台架），
# 本地「记得设 PYTHONIOENCODING=utf-8」就能糊过去——本会话里我糊了好几次。
# `scripts/targetgc.py` 早就有 reconfigure（说明这个坑在别处已咬过一次），
# 而 `fetch_models.py` / `targetaudit.py` / `ticketstatus.py` 三家都没有。
#
# ⚠⚠ **2026-10-18 第二次收范围。** 原先这条写的是「**gate.ps1 用 python 调起的**
#   脚本」——范围是从「当时出问题的三个文件在哪」推出来的，不是从
#   「这条不变量在哪些地方成立」推出来的。代价当场兑现：
#   `scripts/ocrbench/` 整族 **17 个脚本全是中文输出、一个都没加**，
#   而它们恰好是排查 OCR 时天天要跑的那几个。
#   ⇒ 扫描面改成**仓库跟踪的本项目 Python 脚本**，
#     用 `git ls-files` 一次列全（枚举范围要能被一条命令列完，不能靠记忆）。
#
#   ⚠ **刻意排除 `.agents/skills/`**：那 16 个是第三方 skill 的 vendored 内容，
#     本仓没有理由去改别人的脚本。判据不是「文件在哪」，是「这是不是我们自己的代码」。
#
# ⚠ 这条是**保守**的：判据只看「源码含非 ASCII」，而中文可能只出现在注释里、
#   根本不进输出。宁可多要求那 5 行，也不给「看起来只在注释里所以放过」留口子。
$pyGateInvoked = [regex]::Matches($gate, '(?m)python[\s\\]+([\w\\./-]+\.py)') |
  ForEach-Object { $_.Groups[1].Value -replace '\\', '/' } |
  Sort-Object -Unique

# gate.ps1 点名要跑的文件必须存在——这是另一条不变量，范围窄是**故意的**。
foreach ($py in $pyGateInvoked) {
  if (-not (Test-Path $py)) {
    $fail.Add("gate.ps1 要跑 $py，但仓库里没有这个文件")
  }
}

$pyScope = @(
  git ls-files 'scripts/*.py' 'scripts/**/*.py' 'ui/scripts/*.py' 'ui/scripts/**/*.py'
) | Where-Object { $_ -and $_ -notmatch '^\.agents/' } | Sort-Object -Unique

if (-not $pyScope) {
  $fail.Add('一条 .py 都没列出来 —— 扫描面本身失效了，不许当合规放过。')
}

$pyNonAscii = 0
foreach ($py in $pyScope) {
  if (-not (Test-Path $py)) { continue }
  $pyRaw = [IO.File]::ReadAllText((Resolve-Path $py))
  if ($pyRaw -notmatch '[^\x00-\x7F]') { continue }   # 全 ASCII，cp1252 编得动，不管
  $pyNonAscii++
  if ($pyRaw -notmatch '\.reconfigure\s*\(') {
    $fail.Add("$py 含非 ASCII，但没自己 reconfigure stdout/stderr")
    $fail.Add('    ⇒ Windows 上（CI 的 runner 尤其）stdout 默认 cp1252，打印中文会 UnicodeEncodeError。')
    $fail.Add('      修：在模块顶层加')
    $fail.Add('        for _s in (sys.stdout, sys.stderr):')
    $fail.Add('            if hasattr(_s, "reconfigure"): _s.reconfigure(encoding="utf-8", errors="replace")')
    $fail.Add('      别指望调用方记得设 PYTHONIOENCODING。样板：scripts/targetgc.py')
    $fail.Add('      ⚠ 插在**完整的 import 语句之后**：多行括号 import 的顶格那行是开头不是结尾。')
  }
}

# ⚠ 量具自检：⑥ 一个跑测试的 job 都没找到 ⇒ 它此刻**什么都没验**。
#   「量具坏了」与「量具没找到东西」在屏幕上原本长得一模一样，
#   而这一条守卫的整个存在意义就是抓「静默跳过」——
#   它自己静默起来是最坏的一种。⇒ 直接红。
# ⚠ 这一段**必须在下面的判决之前**：放到判决之后，$fail 添了也没人看。
if ($script:testJobsChecked -eq 0) {
  $fail.Add('⑥ 一个「跑测试的 job」都没扫到 —— 这条守卫此刻什么都没验。')
  $fail.Add('   （要么 workflow 里真的没有 cargo test，要么 job 的切分/判据坏了。别把「没查到」当「没问题」。')
}

# ─────────────────────────────────────────────────────────────────────────
# ⑦ 依赖 gitignored 真帧的测试，必须是「本机专属」（`#[ignore]`）
#
# ## 这条守卫的由来（2026-10-19）
#
# `d67918f` 把「缺夹具」从「跳过」改成**硬失败**，理由是「`ok` 与跳过逐字同形」
# ——那个判断没错。但它同时写下「CI 要在 Gate 8 之前把
# `crates/capture/fixtures/` 备齐」，而夹具是**真实会话**的脱敏帧、
# 硬约束第 3 条禁止它们进公开仓库 ⇒ **那一步在 CI 上永远备不出来**。
# ⇒ 「硬失败」+「夹具永远不在」= **CI 永远红**。
# 实测：main 上连续三个 run 全红（`39236999113` / `39246826894` / `39249556236`），
# 红的都是 `a_real_bubble_never_has_a_busy_ring`。
#
# ## 现在的判据（三条，缺一不可）
#
# 1. 依赖真帧的 `#[test]` 一律标 `#[ignore]`（本机专属）：
#    `cargo test --workspace` 不跑它们，但它们出现在**汇总行**里
#    （`... 0 failed; 2 ignored;`）⇒「没跑」是**机器可读**的，与「ok」不同形。
# 2. 那条不变量**在 CI 上有替身**：合成帧版必须还在、且**不能**被 ignore
#    （否则等于把守卫整个删掉）。
# 3. `ci.yml` 必须写明本机怎么真跑它们（`--include-ignored`）。
#
# ⚠ 这条守卫抓的是**复发**：同族的四个测试当时有**三种**策略
#   （硬失败 / 静默跳过 / 硬失败），其中 `perceive/src/media.rs` 那个
#   至今还是「eprintln + return」——**它才是 `d67918f` 要根除的那个坑**，
#   而那一轮漏掉了它。⇒ 策略必须由机器统一，不能靠人记得。
#
# ## 枚举范围：按**函数体**推，不靠名单
#
# 名单会漂（上一轮就漏了一个）。这里改成：凡是**函数体**（或它调用的、
# 同文件的辅助函数）碰到 `capture/fixtures` 的 `#[test]`，都必须是 `#[ignore]`。
# 范围能被 `git ls-files` 一次列全，不靠记忆。
$fixtureScope = @(
  git ls-files 'crates/*/src/*.rs' 'crates/*/src/**/*.rs' 'crates/*/tests/*.rs' 'crates/*/tests/**/*.rs'
) | Where-Object { $_ -and $_ -notmatch '^\.agents/' } | Sort-Object -Unique

if (-not $fixtureScope) {
  $fail.Add('⑦ 一条 crates/*.rs 都没列出来 —— 扫描面本身失效了，不许当合规放过。')
}

# 解析一个 Rust 文件里的 fn：名字、缩进、body 范围、父子关系。
# ⚠ 用**缩进**配对花括号：这些文件里的 fn 都在 `mod` 里，缩进是唯一可靠的层级信号。
function Get-FnTree([string[]]$lines) {
  $fns = @()
  $stack = [System.Collections.Generic.List[object]]::new()
  for ($i = 0; $i -lt $lines.Count; $i++) {
    $ln = $lines[$i]
    if ($ln -match '^(\s*)fn\s+([A-Za-z_][A-Za-z0-9_]*)') {
      $indent = $Matches[1].Length
      $name = $Matches[2]
      while ($stack.Count -gt 0 -and $stack[$stack.Count - 1].indent -ge $indent) {
        $stack.RemoveAt($stack.Count - 1)
      }
      $fn = [pscustomobject]@{
        name = $name; indent = $indent; start = $i; end = ($lines.Count - 1)
        fixture = $false; isTest = $false; ignored = $false
      }
      $fns += $fn
      $stack.Add($fn)
      continue
    }
    if ($ln -match '^(\s*)\}') {
      $indent = $Matches[1].Length
      while ($stack.Count -gt 0 -and $stack[$stack.Count - 1].indent -ge $indent) {
        $stack.RemoveAt($stack.Count - 1)
      }
    }
  }
  foreach ($f in $fns) {
    for ($j = $f.start + 1; $j -lt $lines.Count; $j++) {
      if ($lines[$j] -match '^(\s*)\}' -and $Matches[1].Length -eq $f.indent) { $f.end = $j; break }
    }
  }
  return $fns
}

$fixtureTests = [System.Collections.Generic.List[string]]::new()
$fixtureTestCount = 0
$syntheticTwinFound = $false

foreach ($file in $fixtureScope) {
  if (-not (Test-Path $file)) { continue }
  $lines = [IO.File]::ReadAllLines((Resolve-Path $file))
  $fns = Get-FnTree $lines

  # 「依赖真帧」= body 碰到夹具路径，**或**调用了同文件里**已判定为依赖真帧**的
  # 辅助函数。⇒ 迭代到不动点（`fixture_dir` → `load` → `check` → 测试函数，
  # 这条链要三步才收敛）。
  #
  # ⚠⚠ 传播**只能来自已判定为真的函数**：写成「调用同文件任何函数就算依赖」时，
  #   `crates/reason/src/voice.rs` 的每个测试都会被误判（它们只是互相调用）。
  #   判据是「碰没碰真帧」，不是「有没有调用关系」。
  do {
    $changed = $false
    foreach ($f in $fns) {
      if ($f.fixture) { continue }
      $body = ($lines[$f.start..$f.end] -join "`n")
      $hit = ($body -match 'capture[\\/]fixtures|fixture_dir')
      if (-not $hit) {
        foreach ($c in $fns) {
          if ($c -eq $f -or -not $c.fixture) { continue }
          if ($body -match ('\b' + [regex]::Escape($c.name) + '\s*\(')) { $hit = $true; break }
        }
      }
      if ($hit) { $f.fixture = $true; $changed = $true }
    }
  } while ($changed)

  foreach ($f in $fns) {
    # 属性行：紧挨在 fn 上方的 `#[...]` 与注释行
    $attrs = @()
    for ($k = $f.start - 1; $k -ge 0; $k--) {
      $t = $lines[$k].Trim()
      if ($t -match '^#\[' -or $t -match '^//') { $attrs += $t; continue }
      break
    }
    $f.isTest = [bool]($attrs -match '^#\[test')
    $f.ignored = [bool]($attrs -match '^#\[ignore')

    if (-not $f.isTest) { continue }
    if ($f.name -eq 'real_text_lines_stay_below_both_icon_thresholds') {
      # 判据 ②：合成替身必须还在，且**不能**被 ignore
      $syntheticTwinFound = $true
      if ($f.ignored) {
        $fail.Add("⑦ 合成替身 `real_text_lines_stay_below_both_icon_thresholds` 被标了 #[ignore] —— 那等于把守卫整个删掉")
      }
      if ($f.fixture) {
        $fail.Add("⑦ 合成替身 `real_text_lines_stay_below_both_icon_thresholds` 竟然依赖真帧 —— 它该是合成的")
      }
      continue
    }
    if ($f.fixture) {
      $fixtureTestCount++
      $fixtureTests.Add("$file :: $($f.name)")
      if (-not $f.ignored) {
        $fail.Add("⑦ $file :: $($f.name) 依赖 gitignored 真帧，却没标 #[ignore]")
        $fail.Add('   ⇒ 它的缺件路径要么 panic（CI 永远红），要么静默 return（CI 绿着但没验）。')
        $fail.Add('     两种都不对：标 #[ignore]（本机专属），CI 上用合成替身守同一条不变量。')
      }
    }
  }
}

# ⚠ 量具自检：⑦ 一个依赖真帧的测试都没扫到 ⇒ 它此刻**什么都没验**。
#   「量具坏了」与「量具没找到东西」在屏幕上原本长得一模一样，
#   而这一条守卫的全部意义就是抓「静默失效」——它自己静默起来最坏。⇒ 直接红。
if ($fixtureTestCount -eq 0) {
  $fail.Add('⑦ 一个「依赖真帧的测试」都没扫到 —— 这条守卫此刻什么都没验。')
  $fail.Add('   （要么真帧依赖已经全被改名/挪走，要么解析器坏了。别把「没查到」当「没问题」。')
}

# 判据 ③：ci.yml 必须写明本机怎么真跑它们。
if (-not (Test-Path $ciPath)) {
  $fail.Add("⑦ 找不到 $ciPath —— 判据 ③ 本次没验。")
} elseif (-not ([IO.File]::ReadAllText((Resolve-Path $ciPath)) -match '--include-ignored')) {
  $fail.Add('⑦ ci.yml 里没有 `--include-ignored` —— 本机专属测试的跑法没写下来。')
  $fail.Add('   ⇒ 「这些测试在 CI 上没验」这件事必须**写出来**，不能只靠一个 ignored 计数让人自己猜。')
}

# ─────────────────────────────────────────────────────────────────────────
# ⑧ 本机跑 cargo 的入口，必须都带**同一份** TAURI_CONFIG 覆盖
#
# ## 这条守卫的由来（2026-10-20）
#
# `tauri.conf.json` 的 `bundle.resources` 指向 `../../target/models/ppocrv6-small/*`，
# 而 `target/` 是 gitignored 的 ⇒ **干净检出上那个目录不存在**，tauri-build 的
# build script 在每个 cargo 步骤抛 `GlobPathNotFound`（tauri-utils `resources.rs:289`）。
#
# 门禁早就知道这件事（`gate.ps1:75` 设了 `TAURI_CONFIG` 覆盖），
# 而**本机还有两个入口会直接跑 cargo**：
#   · `scripts/targetgc.py` —— 列举活产物时跑 `cargo test --no-run`；
#   · `scripts/dev.ps1` —— `check` / `test` 两个动词直接跑 cargo。
#
# 2026-10-20 实测（`Test-Path target\models` = False）：这两个入口 exit 101，
# 后果是**两个方向都坏**：
#   · targetgc 拿不到活产物清单 ⇒ **清理脚本永远不删任何东西**，
#     而它不报错，只打一行「本轮不删任何文件」——看着像「没什么可清」；
#   · `dev.ps1 check` / `test` 恒红，而红的原因与代码无关。
#
# ⇒ 与 ⑥ 同族：**同一个不变量散在多个文件里，逐个修就会复发**
#   （票 51 修完 `ci.yml` 忘了 `release.yml`，同一个形状）。
#   这次改成机器逐字比对：三处必须**一模一样**，改一处就要改三处。
#
# ⚠ 匹配前必须剥掉**整行注释**（与本文件上方那条同理由）：
#   注释里写着那串 JSON，不等于代码里真的设了它。
$localCargoEntries = @(
  @{ Path = 'scripts\gate.ps1';    Label = 'scripts/gate.ps1' },
  @{ Path = 'scripts\dev.ps1';     Label = 'scripts/dev.ps1' },
  @{ Path = 'scripts\targetgc.py'; Label = 'scripts/targetgc.py' }
)
$canonicalOverride = '{"bundle":{"resources":null}}'
$script:overrideEntriesChecked = 0
foreach ($e in $localCargoEntries) {
  if (-not (Test-Path $e.Path)) {
    $fail.Add(("⑧ 本机 cargo 入口 {0} 不存在 —— 这条守卫此刻什么都没验" -f $e.Label))
    continue
  }
  $raw = [IO.File]::ReadAllText((Resolve-Path $e.Path))
  $codeLines = ($raw -split "`r?`n") |
    Where-Object { $_ -notmatch '^\s*#' -and $_ -match 'TAURI_CONFIG' }
  $script:overrideEntriesChecked++
  if (-not $codeLines) {
    $fail.Add(("⑧ {0} 里没有任何（非注释的）TAURI_CONFIG 设置" -f $e.Label))
    $fail.Add('   ⇒ 这台机器没装 OCR 权重时，cargo 会挂在 tauri-build 的 resources glob 上（exit 101）。')
    continue
  }
  # 取出那串 JSON（嵌套一层大括号），去掉空白后逐字比对
  $vals = [System.Collections.Generic.List[string]]::new()
  foreach ($ln in $codeLines) {
    foreach ($m in [regex]::Matches($ln, '\{[^{}]*\{[^{}]*\}[^{}]*\}')) {
      $vals.Add(($m.Value -replace '\s', ''))
    }
  }
  if (-not $vals.Count) {
    $fail.Add(("⑧ {0} 提到了 TAURI_CONFIG，但取不到那串覆盖 JSON" -f $e.Label))
    continue
  }
  foreach ($v in $vals) {
    if ($v -ne ($canonicalOverride -replace '\s', '')) {
      $fail.Add(("⑧ {0} 的 TAURI_CONFIG 覆盖与门禁那份不一致" -f $e.Label))
      $fail.Add(("   它是：{0}" -f $v))
      $fail.Add(("   门禁那份必须是：{0}" -f $canonicalOverride))
      $fail.Add('   ⇒ 必须是 null：TAURI_CONFIG 走 json_patch::merge，空对象是空操作。')
    }
  }
}

# ⚠ 量具自检：一个入口都没扫到 ⇒ 它此刻**什么都没验**（同上那条理由）。
if ($script:overrideEntriesChecked -eq 0) {
  $fail.Add('⑧ 一个本机 cargo 入口都没扫到 —— 这条守卫此刻什么都没验。')
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
Write-Output ("bundle.resources 那条：扫了 {0} 个 workflow 的 {1} 个 job，逐个都有覆盖或先取权重；" -f $script:wfCount, $j)
Write-Output ("跑测试前备夹具那条：{0} 个跑测试的 job，逐个都在跑之前取权重并造语料。" -f $script:testJobsChecked)
Write-Output ("Python 编码那条：扫本项目自有 .py {0} 个（不含 .agents/ 第三方），其中 {1} 个含非 ASCII，都已自己 reconfigure。" -f $pyScope.Count, $pyNonAscii)
Write-Output ("真帧测试那条：扫到 {0} 条依赖 gitignored 真帧的 #[test]，全部标了 #[ignore]（本机专属）；合成替身在场。" -f $fixtureTestCount)
Write-Output ("本机 cargo 入口那条：{0} 个入口（gate.ps1 / dev.ps1 / targetgc.py），逐个都带着同一份 TAURI_CONFIG 覆盖。" -f $script:overrideEntriesChecked)
exit 0
