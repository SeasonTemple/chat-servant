# 参数只在**开头几行**之内、不带副作用：CI 与 AGENTS.md 里那句
# `pwsh -NoProfile -File scripts/gate.ps1` 必须继续表示「每一步都跑」。
#
# -Skip / -Only 是给**开发内循环**用的（2026-10-16 加）。加它们的理由是实测：
# 一轮里 `cargo deny check` 要联网拉 advisory-db、`pnpm build` 要全量打包，
# 改一个 .svelte 时这两步与改动完全无关，却各要几十秒。
# 默认路径一个字节都没变，所以 CI / release 与文档里那句话仍然成立。
#
# -Child 是**内部用**的（2026-10-19 加）：把一条轨交给子进程，见下面「分轨并行」。
# 手工不要传——父进程自己会按轨把它开起来。
# ⚠⚠⚠ **过滤参数必须写成「一个字符串 + 按名传」**（2026-10-20 实测后改）。
#
# 原先是 `[string[]]$Skip` / `[string[]]$Only`，而 **`pwsh -File` 对数组参数的
# 绑定有一条极隐蔽的规矩**（实测，`target\argtest.ps1`）：
#
#   pwsh -File x.ps1 -Skip deny 'ui build'
#     ⇒ $Skip = @('deny')     ← 只拿到紧跟其后的**一个**值
#     ⇒ $Only = @('ui build')  ← **第二个值被按位置绑给了下一个参数！**
#
# ⇒ 传两个片段时，`-Skip` 的第二个片段**静默地变成了 `-Only`**，
#   过滤语义整个反过来：实测「跳过 deny 与 ui build」变成了
#   「**只有** ui build 跑，其余 15 步全被跳」。
#   ⚠⚠ 这是本仓最熟的那个形状：**参数与实际行为不符，而报告读起来完全正常**。
#
# ⚠ 而重复参数名是**报错**的（实测）：`-Skip a -Skip b` ⇒
#   「无法绑定参数，因为多次指定了参数 'Skip'」⇒ 那条路至少是响的。
#   危险的是**位置式**的第二个值（上面那个），它一声不响。
#
# ⇒ 两手都要：
#   ① 参数声明成 `[string]`（逗号分隔的多个片段写在一个值里），
#      并关掉位置绑定（`PositionalBinding = $false`）⇒ 位置式的第二个值
#      **无处可绑 ⇒ 直接报错**，不再静默错位；
#   ② 内部自己按逗号拆（脚本头部那两行），不指望调用方拆。
#
# ⚠ 改动的兼容性：`-Skip 'deny,ui build'`（逗号形式，AGENTS.md 里的写法）
#   **完全不受影响**；`-Skip 'deny','ui build'` 现在会**报错**（响的，好过静默）。
[CmdletBinding(PositionalBinding = $false)]
param(
  [string]$Skip = '',
  [string]$Only = '',
  [string]$Child = ''
)

# ⚠⚠ **必须拆逗号**（2026-10-19 补，2026-10-20 改写措辞）。
# 参数现在是 `[string]`（一个值里逗号分隔的多个片段），拆的是**这个值**：
# `pwsh -File gate.ps1 -Skip 'deny,ui build,secret'` ⇒ 拆成三个片段，
# 逐片段 `-like "*片段*"` ⇒ 三步都跳得掉。
#
# 后果不是「跳多了」而是「跳少了」，而那正是本项目反复付过学费的形状：
#   `pwsh -File gate.ps1 -Skip 'deny,ui build,secret'` → 16 步全跑，
#   脚本照样打 `GATE_SCOPE: PARTIAL —— 跳过了部分步骤（-Skip deny,ui build,secret）`，
#   **那行提示读起来像是跳过了三步，实际一步都没跳**。而外层 `$LASTEXITCODE` 是 0。
#
# ⇒ 两手都要：① 这里把逗号拆开，让文档里的写法真的生效；
#   ② 下面 `GATE_SCOPE` 那行按**实际命中的步数**说话，不再照抄用户给的参数。
$script:skipArgs = @($Skip  | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$script:onlyArgs = @($Only  | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })

$ErrorActionPreference = "Continue"
Set-Location $PSScriptRoot\..
$repoRoot = $PWD.Path
$uiDir = Join-Path $repoRoot 'ui'
$fail = 0
$stepTime = [ordered]@{}
# 实际被跳过的**步名**（不是用户给的参数）。-Skip 'a,b' 没按逗号切分时，
# 参数看着像跳了三步、实际一步没跳，那行提示就会说谎（2026-10-19 实测）。
$script:skippedNames = @()
# 全部步名。用来核对 -Skip/-Only 的每个片段是否真的匹配到了什么。
$script:allStepNames = @()
# 声明出来的步（名字 + 动作）。父进程只收集不执行，子进程按轨执行。
$script:steps = @()

# 本机 target/ 在 E: 盘上，cargo 的增量编译目录偶尔会被另一个进程锁住
# （os error 5 拒绝访问），clippy 会因此以 101 退出——那不是代码红。
# 关掉增量编译换一个确定的缓存位置，代价是慢一点。
$env:CARGO_INCREMENTAL = "0"

# 与 `ci.yml:42` / `release.yml:44` 同一份覆盖，**不是**本机的特殊需要——
# 而是本地门禁从来没设过它，于是「本机能编」取决于磁盘上碰巧有没有权重：
# 取过权重的机器绿，没取过的机器在 cargo 步骤上 exit 101。
# 而 `target/` 是 gitignored 的，**两种机器都在正常使用**
# ⇒ 门禁的结论跟着机器走，不跟代码走，而绿的那一侧证明不了任何东西。
#
# 2026-10-19 实测（无权重那支，`Test-Path target\models` = False）：
# `cargo clippy` / `cargo test` 直接挂在 build script 上，exit 101——
# tauri-build 在 `cargo check/clippy/test` 时就遍历 `bundle.resources`，
# glob 匹配为空**无条件**抛 `GlobPathNotFound`（tauri-utils `resources.rs:289`）。
# 措辞要紧：不是「文件缺失才抛」，是「匹配为空就抛」，
# 所以目录在、内容不在也一样炸。
#
# 为什么是 `null` 而不是 `{}`：`TAURI_CONFIG` 走 `json_patch::merge`
# （tauri-build `lib.rs:684`，即 RFC 7396 merge patch），**空对象是空操作**——
# 写 `{"bundle":{"resources":{}}}` 什么都不会删。必须给 `null` 才真的删掉那个键，
# 删掉后 `config.bundle.resources` 是 `None`，`lib.rs:745-749` 的
# `unwrap_or(List(vec![]))` 就不会去迭代 glob。
#
# 为什么本机可以这么干：`gate.ps1` 从不跑 `tauri build`，那个 resources 映射
# 在这里本来就不会被用到（与 CI 同一个理由）。**真要打包的那条路**在
# `release.yml` 里，靠 `fetch_models.py` 取权重，不能靠这个覆盖糊过去。
#
# ⚠ 并行化之后这条更重要：它是**父进程**设的，子进程靠继承拿到。
#   删掉它 ⇒ 四条轨里凡是 cargo 步骤的那条（rust 轨）在无权重机器上 exit 101。
$env:TAURI_CONFIG = '{"bundle":{"resources":null}}'

# ═══════════════════════════════════════════════════════════════════════
# 分轨并行（2026-10-19 加）
# ═══════════════════════════════════════════════════════════════════════
#
# ## 为什么要并行
#
# 实测（2026-10-19，本机 16 逻辑核，完整门禁热缓存 **229.3s**）：
#
#     cargo test 48.3   ui build 31.0   cargo deny 25.7   clippy 23.4
#     ui lint 22.4     ui svelte-check 21.4   ui autofixer 17.3   fmt 3.6
#
# 那 229.3s 里有**将近一半是纯等待**：cargo 轨在等 rustc、ui 轨在等 node、
# 便宜守卫在等磁盘，而这四条轨**互不相干**——不共享 `target/` 的 cargo 构建锁、
# 不共享 `ui/.svelte-kit`、不共享端口。串行跑等于让四条互不相干的队列
# 轮流独占同一台机器，而它们各自都在等别人。按轨并行是这里唯一的大头。
#
# ## 为什么是**子进程**而不是 runspace / job
#
# 下面四条都是 2026-10-19 实测出来的，每一条都会让「丢进 runspace」这条路直接死掉：
#
# ① `Start-ThreadJob` / `ForEach-Object -Parallel` 里 **`$PSScriptRoot` 是空的**，
#    `Set-Variable -Scope Global -Force` 也注入不进去。
#    而本文件里 cargo 那几步之外的大多数步骤都靠相对路径找脚本 ⇒ 一进 runspace 就找不到文件。
# ② `ForEach-Object -Parallel` **拒绝接收脚本块变量**（报「管道输入对象不能是脚本块」）。
# ③ runspace 里读不到父进程 `script:` 作用域的变量，父进程定义的函数也不可见。
# ④ `Start-ThreadJob` 里 `& cmd /c exit 7` 之后，**job 的 `State` 仍然是 `Completed`**
#    ——「跑完了」与「跑的东西成功了」在 job 对象上**长得一模一样**。
#
# ⇒ 每条轨一个**子 pwsh 进程**（`ProcessStartInfo.ArgumentList`；
#   **不要**用 `Start-Process -ArgumentList`，实测它会把参数里的引号吃掉）。
#   子进程里 `$PSScriptRoot`、函数、作用域全都正常。
#
# ⚠ ④ 是**假绿**那一族：只看 `State` 就等于把「这条轨失败了」读成「跑完了」。
#   所以每个并行步都显式回传退出码，见 Invoke-StepParallel。
#
# ## 轨的划分：为什么不是「每步一轨」
#
# · `cargo fmt` / `clippy` / `test` **必须串行**——三者共用 `target/` 的 cargo 构建锁，
#   并行只会互相 `Blocking waiting for file lock on build directory`，既不快也不安全。
# · `license consistency` 里有 `cargo metadata`，吃同一把锁 ⇒ 归 rust 轨，
#   不能因为「它看起来是纯读文件」就塞进 cheap 轨。
# · `ui build`（vite）会写 `.svelte-kit`，而 `pnpm check` 的 `svelte-kit sync`
#   也写同一个目录 ⇒ 两者**不能重叠**，所以 build 排在 ui 那组并行步**之后**。
# · ui 那三步（check / lint / autofixer）都是只读的，实测串行 35.0s → 并行 **17.9s**。
#   三者各是一个独立 node 进程，不共享任何状态（autofixer 逐文件起进程）。
#
# ⚠ 并行化之后 `ui/scripts/autofixer.ps1` 头注里「这几步在 gate.ps1 里
#   **不与 cargo 步骤重叠**」那句话**已经不成立**，该处注释已同步改写。
#
# ## 可诊断性
#
# 每步**仍然**打自己的 `========== name ==========` 头与
# `>>> name OK|FAILED [Ns] (exit=N)` 判定行，父进程边收边转发。
# 唯一的差别是**步与步的输出不再保证按声明顺序出现**——并行的必然结果。
# 防退化断言见下面 Assert-Lanes：「新增步忘了登记轨」会**直接红**而不是安静地退回串行，
# 因为那种情况下该步在每条轨的匹配里都不中，没有任何轨会跑它，而门禁照样报 ALL GREEN。
$script:laneOf = @{
  # ── 便宜守卫：纯读文件，彼此无共享状态 ──
  'mojibake scan'          = 'cheap'
  'sheet pack in sync'     = 'cheap'
  'gate/CI parity'         = 'cheap'
  'version consistency'    = 'cheap'
  'opaque backgrounds'     = 'cheap'
  'forensics hook guard'   = 'cheap'
  # ── cargo 轨：共用 target/ 构建锁，只能串行 ──
  'cargo fmt --all --check'  = 'rust'
  'cargo clippy -D warnings' = 'rust'
  'cargo test --workspace'   = 'rust'
  'secret scan'              = 'rust'
  'license consistency'    = 'rust'
  # ── 网络轨：拉 advisory-db / 查 npm 漏洞，与本机编译零共享 ──
  'cargo deny check'       = 'net'
  'npm audit'              = 'net'
  # ── ui 轨 ──
  'ui svelte-check'        = 'ui'
  'ui lint'                = 'ui'
  'ui svelte-autofixer'    = 'ui'
  'ui build'               = 'ui'
}
$script:lanes = @('cheap', 'rust', 'net', 'ui')

# ui 轨里那三步**并行**跑。写成「哈希表」而不是「数组的数组」，是因为
# PowerShell 会在参数绑定与 `foreach` 两处**摊平嵌套数组**：
#   `$cand[0]` 对**字符串**取的是**首字符**（实测拿到 `'u'` 而不是 `'ui svelte-check'`），
#   于是查表失效、并行组退化成「跑三次单步」，而门禁照样全绿。
# 哈希表的值仍然可以是数组（`$h.Values` 取值时才组装，不经参数绑定），
# 键是字符串，**不可摊平**，所以拿键当句柄最稳。
#
# 为什么不给 Step 加 -Parallel 开关：`check-gate-parity.ps1` 用
# `^\s*Step\s+"([^"]+)"` 解析步名 —— `Step "名" {...}` 这个调用形状不能变。
$script:parallelSets = [ordered]@{
  'ui-static' = @('ui svelte-check', 'ui lint', 'ui svelte-autofixer')
}

# 子进程 ⇄ 父进程之间的机器可读行。选一个源码里绝不会出现的词头。
$script:mark = '##GATE##'

function Should-Run([string]$name) {
  # 过滤：子串匹配、大小写不敏感（-Skip deny / -Skip 'ui build' 都认）。
  if ($script:onlyArgs.Count -gt 0 -and -not ($script:onlyArgs | Where-Object { $name -like "*$_*" })) { return $false }
  if ($script:skipArgs.Count -gt 0 -and ($script:skipArgs | Where-Object { $name -like "*$_*" })) { return $false }
  return $true
}

# 声明一步。父进程只收集；子进程按轨执行。
#
# ⚠⚠ **第四个参数是 `-Action`，必须命名传**（2026-10-19 实测踩到，且是**假绿**）。
#   第一版写成 `function Step($name, $If, $Unless, $Action)` 四个位置参数，
#   而调用处是 `Step "mojibake scan" { ... }` —— 那个裸脚本块被绑到**第二个**参数
#   （`$If`）上，`$Action` 始终是 $null。
#   于是执行时 `& $null $uiDir` 抛 InvalidOperation，而 `$LASTEXITCODE`
#   **保持上一次的值 0** ⇒ 每步都打 `>>> xxx OK (exit=0)`，门禁 exit 0，
#   而实际上**一步都没跑**。
#   ⚠ 这正是本仓最熟的那个形状：静默失效 + 满屏 OK + 结论绿。
#     它能溜过去是因为「输出格式完全正常」——只有拿 stderr 逐行看、
#     或注入一次真失败才抓得到。静态检查、完整跑一遍、甚至读代码都看不出来。
#   ⇒ 所以动作体一律用 `-Action {...}` 命名传，位置参数只留给前三个。
function Step([string]$name, [scriptblock]$If = $null, [string]$Unless = '', [string]$Dir = '', [scriptblock]$Action) {
  # ⚠ 量具自检：动作体没绑上就直接崩在这里，而不是让 `& $null` 在后面静默失败。
  if (-not $Action) {
    throw ("Step `"{0}`" 没有动作体 —— 调用处必须写 `-Action {{ ... }}`。" -f $name)
  }
  $script:allStepNames += $name
  $script:steps += [pscustomobject]@{
    Name = $name; If = $If; Unless = $Unless; Dir = $Dir; Action = $Action
  }
}

# 单步执行。输出**直接流到控制台**，不先收进变量——
# 用户要看到进行中的输出（本仓为此反复要求：曾经因为 `| Out-String`
# 缓冲导致十几分钟零输出）。
function Invoke-StepNow($step) {
  $name = $step.Name
  if (-not (Should-Run $name)) {
    Write-Output ""
    Write-Output "========== $name =========="
    Write-Output "$($script:mark)SKIP`t$name"
    Write-Output ">>> $name SKIPPED（-Skip / -Only 过滤）"
    $script:skippedNames += $name
    return
  }
  if ($step.If -and -not (& $step.If)) {
    Write-Output ""
    Write-Output "========== $name =========="
    Write-Output ">>> $name SKIPPED（$($step.Unless)）"
    return
  }
  $sw = [Diagnostics.Stopwatch]::StartNew()
  Write-Output ""
  Write-Output "========== $name =========="
  # ⚠ `-Dir` 是**声明式**的工作目录：执行器负责切，动作体只管跑自己的命令。
  #   动作体里写 `Push-Location` / `Pop-Location` 配对是被否掉的形状（2026-10-19 实测）：
  #   `pnpm lint` 内部自己会切目录，于是末尾那句 Pop 找不到配对的 Push，
  #   报错并把退出码带成 1 ⇒ 明明通过却判红。
  #   同理也让并行组的子脚本不必再自己处理位置。
  if ($step.Dir) { Push-Location -LiteralPath $step.Dir }
  # ⚠⚠ 清空 `$Error` 再跑动作体（2026-10-22 实测加的）。
  #   动作体里的 **PowerShell 级错误**（命令不存在、脚本抛异常）**不改
  #   `$LASTEXITCODE`**：实测 pnpm 不在 PATH 时，`$LASTEXITCODE` 仍是 0，
  #   而 `$Error` 里多了一条 CommandNotFoundException。
  #   ⇒ 不看 `$Error`，就只剩「工具没装 ⇒ 这一步报 OK」这条假绿。
  #   ⚠ 原生命令写 stderr（2>&1 合并的那种）**不进 `$Error`**（实测
  #   ErrorCount=0）⇒ 这条判据不会把正常输出误判成失败。
  $Error.Clear()
  & $step.Action
  $code = $LASTEXITCODE
  if ($step.Dir) { Pop-Location }
  # 顺序有讲究：先存退出码再停表。反过来的话 Stop 之后再读
  # $LASTEXITCODE 仍是对的，但一旦以后有人在这里插一条会改 $LASTEXITCODE
  # 的命令，红绿就会读到别人的退出码。判据取自 2026-10-11 那次「假绿」。
  $code = $LASTEXITCODE
  # ⚠⚠ 三条判据，缺一条就假绿（2026-10-22 实测）：
  #   ① 有 PowerShell 级错误（命令不存在 / 脚本抛异常）⇒ 退出码取不到真值 ⇒ 判红；
  #   ② 退出码是 $null（这一步压根没回传退出码）⇒ 判红；
  #   ③ 退出码非 0 ⇒ 判红。
  # ⚠⚠ **判红必须同时置 `$script:fail`**：这一步以前只打印、不置标志，
  #   而 Invoke-Lane 判本轨红绿靠的是「`$script:fail` 有没有变」
  #   ⇒ 串行步红了，轨子照样 `exit 0`，父进程照样报 ALL GREEN。
  #   （2026-10-22 实测：`ui build` 打了 FAILED，整轮门禁却是 ALL GREEN。）
  if ($Error.Count -gt 0) {
    $notFound = @($Error | Where-Object {
      $_.Exception -is [System.Management.Automation.CommandNotFoundException] -or
      $_.FullyQualifiedErrorId -like 'CommandNotFoundException*'
    })
    $code = if ($notFound.Count -gt 0) { 127 } else { 1 }
    Write-Output ("GATE_STEP_ERROR: {0} —— {1}" -f $name, ($Error | Select-Object -First 1))
  } elseif ($null -eq $code) { $code = 1 }
  if ($code -ne 0) { $script:fail = 1 }
  $sw.Stop()
  $script:stepTime[$name] = [math]::Round($sw.Elapsed.TotalSeconds, 1)
  Write-Output ("{0}STEP`t{1}`t{2}`t{3}" -f $script:mark, $name, $code, $script:stepTime[$name])
  Write-Output (">>> {0} [{1:N1}s] (exit={2})" -f
    $(if ($code -ne 0) { "$name FAILED" } else { "$name OK" }),
    $script:stepTime[$name], $code)
}

# 一组只读步并行跑（当前只有 ui 那三步）。
#
# ⚠⚠ 每个 job 都**显式回传退出码**：job 的 `State` 只反映「跑完了没有」，
#   原生命令的非零退出在它那儿与成功**完全同形**（2026-10-19 实测
#   `& cmd /c exit 7` 之后 State 仍是 Completed）。只看 State 就是假绿。
# ⚠⚠ 进 job 第一件事是把 `$LASTEXITCODE` **归零**：它是进程级的，
#   带着上一个命令的残留进来；而脚本块若在碰到任何原生命令之前就失败，
#   回传上去的就是那个陈值。
# ⚠ 动作体不用父作用域变量（runspace 读不到，实测 ③），
#   所以它们都写成 `param($d)`，由这里把绝对路径传进去。
#
# ⚠⚠ **不要用 Start-ThreadJob 跑并行组**（2026-10-19 实测，这是第三次栽在 job 上）。
#   `Start-ThreadJob` 的 runspace 是**高度受限**的：连 `Select-Object` 这种
#   基础 cmdlet 都不可用（实测报「术语 'Select-Object' 不会被识别」）。
#   ⇒ 三步动作体里的 `| Select-Object -Last 40` 全部失败，
#   而 `pnpm` 在找不到 package.json 时**给的是 exit 0**
#   ⇒ 三步都打 `OK`、门禁全绿，而实际上**一步都没跑**。
#   这是本仓最熟的那个形状：输出正常、结论正常、什么都没发生。
#   （前两次分别是 `$PSScriptRoot` 为空、`State` 不反映退出码。）
#
# ⇒ 改用**子 pwsh 进程**：与外层四条轨同一种机制，`$PSScriptRoot`、全部 cmdlet、
#   退出码都正常。输出重定向到临时文件，跑完再读回来（不进内存缓冲）。
#   `Select-Object` 留在动作体里，在子进程内执行——那里它可用。
function Invoke-StepParallel([string]$SetKey) {
  if (-not $script:parallelSets.Contains($SetKey)) {
    Write-Output "GATE_FATAL: 找不到名为 `"$SetKey`" 的并行组（现有：$($script:parallelSets.Keys -join ', ')）。"
    $script:fail = 2
    return
  }
  $set = @($script:parallelSets[$SetKey])
  Write-Output ""
  Write-Output ("---------- 并行组 {0}：{1} ----------" -f $SetKey, ($set -join ' / '))

  $byName = @{}
  foreach ($s in $script:steps) { $byName[$s.Name] = $s }

  # 每步一个临时子脚本。⚠ 必须落到**绝对路径**：子进程的 cwd 由
  #   `WorkingDirectory` 设成 ui 目录，而 `-File` 的相对路径是相对 cwd 解析的。
  $logDir = Join-Path $repoRoot 'target\gate-parallel'
  New-Item -ItemType Directory -Force -Path $logDir | Out-Null
  $pwshExe = (Get-Command pwsh -ErrorAction SilentlyContinue)
  if (-not $pwshExe) {
    Write-Output "GATE_FATAL: PATH 上没有 pwsh，无法并行组。"
    $script:fail = 2
    return
  }

  $sw = [Diagnostics.Stopwatch]::StartNew()
  $procs = @()
  foreach ($n in $set) {
    $act = $byName[$n].Action
    $slug = ($n -replace '[^A-Za-z0-9]', '_')
    $runPs1 = Join-Path $logDir "$slug.run.ps1"
    $outFile = Join-Path $logDir "$slug.out"
    $errFile = Join-Path $logDir "$slug.err"
    # 子脚本 = 动作体 + 显式回传退出码。
    # ⚠ 退出码**必须显式写出**（`exit $LASTEXITCODE`），父进程只看进程退出码；
    #   这与 Invoke-Lanes 里「只看 ##GATE## 标记」是同一条纪律。
    $body = $act.ToString()
    [IO.File]::WriteAllText($runPs1, @"
`$ErrorActionPreference = 'Continue'
`$global:LASTEXITCODE = 0
`$Error.Clear()
Set-Location -LiteralPath '$($uiDir -replace "'", "''")'
$body
`$code = `$LASTEXITCODE
# ⚠⚠ **命令不存在时 `$LASTEXITCODE` 不会变**（PowerShell 级错误不碰它），
#   它还是上面显式置的 0 ⇒ 子进程 exit 0 ⇒ 父进程把这一步记成 OK。
#   ⇒ 用 `$Error` 兜底：原生命令的 stderr（2>&1 合并的那种）**不进** `$Error`
#     （实测 ErrorCount=0），而「命令找不到」会进（实测 ErrorCount=1）。
#   ⚠ 这已是同一族问题的第三次：Start-ThreadJob 那次（连 Select-Object 都没有）、
#     这次（工具不在 PATH）。所以判据不能只盯退出码。
if (`$Error.Count -gt 0) {
  `$notFound = @(`$Error | Where-Object {
    `$_.Exception -is [System.Management.Automation.CommandNotFoundException] -or
    `$_.FullyQualifiedErrorId -like 'CommandNotFoundException*'
  })
  `$code = if (`$notFound.Count -gt 0) { 127 } else { 1 }
  Write-Output ("GATE_STEP_ERROR: " + (`$Error | Select-Object -First 1))
}
if (`$null -eq `$code) { `$code = 1 }
exit `$code
"@, (New-Object Text.UTF8Encoding $false))

    $psi = [Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $pwshExe.Source
    foreach ($a in @('-NoProfile', '-File', $runPs1)) { [void]$psi.ArgumentList.Add($a) }
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.WorkingDirectory = $uiDir
    $p = [Diagnostics.Process]::Start($psi)
    $procs += [pscustomobject]@{
      Name = $n; Proc = $p; OutFile = $outFile; ErrFile = $errFile
      Out = $p.StandardOutput.ReadLineAsync(); Err = $p.StandardError.ReadLineAsync()
      Buf = [System.Collections.Generic.List[string]]::new()
    }
  }

  # 收割：一次读空所有已完成的行（否则每行 40ms，上万行要好几分钟）。
  $idle = 0
  $open = @($procs)
  while ($open.Count -gt 0) {
    $progressed = $false
    foreach ($h in @($open)) {
      foreach ($which in @('Out', 'Err')) {
        while ($true) {
          $t = $h.$which
          if ($null -eq $t -or -not $t.IsCompleted) { break }
          $line = $t.Result
          if ($which -eq 'Out') { $h.Out = $h.Proc.StandardOutput.ReadLineAsync() }
          else { $h.Err = $h.Proc.StandardError.ReadLineAsync() }
          if ($null -eq $line) { $h.$which = $null; break }
          $progressed = $true
          if ($which -eq 'Err') { $h.Buf.Add("[err] $line") } else { $h.Buf.Add($line) }
        }
      }
      if ($null -eq $h.Out -and $null -eq $h.Err) {
        $h.Proc.WaitForExit()
        $open = @($open | Where-Object { $_ -ne $h })
      }
    }
    if ($open.Count -gt 0) {
      if ($progressed) { $idle = 0 } else { Start-Sleep -Milliseconds 25; $idle++ }
    }
  }
  $sw.Stop()

  foreach ($h in $procs) {
    Write-Output ""
    Write-Output "========== $($h.Name) =========="
    # 尾部截断在这里做（`Select-Object -Last 40` 那种语义）——
    # ⚠ 动作体里那个 Select-Object 在子进程里已经跑过一次了，
    #   这里再截一次是为了吸收 stderr 的 `[err]` 行。
    $h.Buf | Select-Object -Last 40 | ForEach-Object { Write-Output $_ }
    $code = $h.Proc.ExitCode
    # ⚠⚠ 秒数**不能写 0**（2026-10-20 实测）：父进程靠这个标记回填
    #   `$script:stepTime`（`##GATE##STEP\t<名>\t<code>\t<秒>`），
    #   写 0 ⇒ 完整门禁自打的耗时表里这三步显示 `0.0s`，
    #   而**同一轮**下面那行 `>>> <名> [24.6s]` 打的是整组墙钟
    #   ⇒ 同一个门禁里两个地方报同一件事，一个 0.0、一个 24.6。
    #   而 AGENTS.md 明说「以 gate.ps1 每轮自打的那份为准」——
    #   那份对这三步当时**不可信**（不是"它们没跑"，是"没记上"）。
    #   ⚠ 语义是「三步共用整组墙钟」，不是各自耗时（注释见计时表那段）。
    #   ⇒ 步秒之和会因此多算 2×组墙钟，那是有意的：
    #     墙钟下界看「最慢的一步」那行，别拿步秒之和当墙钟。
    $groupWall = [math]::Round($sw.Elapsed.TotalSeconds, 1)
    Write-Output ("{0}STEP`t{1}`t{2}`t{3}" -f $script:mark, $h.Name, $code, $groupWall)
    Write-Output (">>> {0} [{1:N1}s] (exit={2})" -f
      $(if ($code -ne 0) { "$($h.Name) FAILED" } else { "$($h.Name) OK" }),
      $groupWall, $code)
    if ($code -ne 0) { $script:fail = 1 }
  }
}

# ── 防退化断言（2026-10-19）────────────────────────────────────────────
#
# ⚠⚠ 这两条不是「顺手加的好看」，它们守着并行化引入的**新**失败模式：
#
# ① 新增 Step 忘了在 `laneOf` 里登记 ⇒ 该步在每条轨的 `laneOf[$name] -eq $Child`
#    判断里都不匹配 ⇒ **没有任何轨会跑它** ⇒ 而门禁照样打 `GATE: ALL GREEN`。
#    「少跑一步」与「全跑过且都过」在屏幕上**逐字同形**——
#    这正是本仓反复栽过的形状（`sheet pack in sync` 本地每次都跑、CI 里根本没有；
#    `check-gate-parity.ps1` 头注里「守卫断言了中间量、没断言产物」那一族）。
#    串行版本里「忘了写一行」是显然的，**并行版本里它变成静默的**，所以必须机器拦。
# ② `laneOf` 里留了已删除的步名 ⇒ 与 ① 互为镜像（会让断言 ① 变宽松）。
#
# ⚠ 消息走 `$script:assertMsg`，**不靠返回值带出去**（2026-10-19）：
#   PowerShell 里 `Write-Output` 与 `return` 共用同一条输出流，
#   函数体内的提示会和返回码一起变成数组 —— `-ne 0` 虽仍成立，
#   但调用处拿到的东西已经分不清是消息还是退出码了。
$script:assertMsg = @()
function Assert-Lanes() {
  $script:assertMsg = @()
  $declared = @($script:allStepNames | Select-Object -Unique)
  if ($declared.Count -eq 0) {
    $script:assertMsg = @("GATE_FATAL: 一个 Step 都没声明 —— 量具/写法坏了，不许当合规放过。")
    return 2
  }
  $unmapped = @($declared | Where-Object { -not $script:laneOf.ContainsKey($_) })
  if ($unmapped.Count -gt 0) {
    $script:assertMsg = @("GATE_FATAL: 这些 Step 没在 `$script:laneOf` 里登记轨，不会有任何轨跑到它们：")
    $script:assertMsg += @($unmapped | ForEach-Object { ("    Step `"{0}`"" -f $_) })
    $script:assertMsg += @(
      "    ⇒ 不登记的后果是**静默**的：门禁会报 ALL GREEN，而这几步从没跑过。",
      "      修法：在 laneOf 里给每一步指定一条轨（cargo 相关进 rust，纯读文件进 cheap）。"
    )
    return 2
  }
  $orphan = @($script:laneOf.Keys | Where-Object { $declared -notcontains $_ })
  if ($orphan.Count -gt 0) {
    $script:assertMsg = @("GATE_FATAL: laneOf 里登记了这些步名，但文件里已没有对应的 Step 调用：")
    $script:assertMsg += @($orphan | Sort-Object | ForEach-Object { "    $_" })
    $script:assertMsg += @("    ⇒ 少跑一步。删掉 laneOf 里那行，或把 Step 加回来。")
    return 2
  }
  foreach ($setKey in $script:parallelSets.Keys) {
    foreach ($n in @($script:parallelSets[$setKey])) {
      if ($script:laneOf[$n] -ne 'ui') {
        $script:assertMsg = @(("GATE_FATAL: 并行组 {0} 里的 `"{1}`" 不在 ui 轨（现在是 {2}）——并行组只对 ui 轨成立。" -f $setKey, $n, $script:laneOf[$n]))
        return 2
      }
    }
  }
  return 0
}

# ── 编码损坏（U+FFFD）。ticket 19 ──
#
# 它跟其它守卫不同类：受损的全是注释与文档，fmt / clippy / test / svelte-check
# 一律照绿，不专门扫就永远没人发现。放到 cheap 轨（纯读文件，与 cargo 零共享）。
Step "mojibake scan" -Action { & pwsh -NoProfile -File "$PSScriptRoot\scan-mojibake.ps1" 2>&1 | Select-Object -Last 30 }

# 纸的源（probe-sheet.png）与生成物（probe-sheet.svg）必须同步。
# SVG-as-image 不能外链资源，所以 PNG 只能靠打包脚本内嵌进去；
# 两者一旦漂移，纸就会静默地变成「旧的那张」，而所有截图都照样出得来。
Step "sheet pack in sync" -Action { & pwsh -NoProfile -File ui\scripts\sheetpack.ps1 -Check 2>&1 | Select-Object -Last 10 }

# 本门禁自己的组成有没有漂到 CI 外面去。
# ⚠ 并行化之后它与其它守卫**同时**跑，不再排在编译前——而它红掉仍然让
#   下面每一步的「CI 也会跑」都成空话，所以它红就是门禁红，不因为跑得早而被忽略。
Step "gate/CI parity" -Action { & pwsh -NoProfile -File "$PSScriptRoot\check-gate-parity.ps1" 2>&1 | Select-Object -Last 30 }

# 版本号在**四个文件**里各声明一份，而且 JSON 那两处没有继承可借。
# 纯读文件；发版闸只比 tag ↔ tauri.conf.json，另外三处对不上它**不会拦**。
Step "version consistency" -Action { & pwsh -NoProfile -File "$PSScriptRoot\check-version.ps1" 2>&1 | Select-Object -Last 15 }

# license 在**每个**工作区成员里各声明一份，唯一出处是根 `[workspace.package]`。
# ⚠ 这一步不能指望 `cargo deny check` 顺带抓：MIT / Apache-2.0 都在白名单里，
#   写错成 MIT 只会让 deny.toml 那条 BUSL-1.1 例外变成 `warning[...]`
#   并**仍然 exit 0**（2026-10-18 实测）。
# ⚠⚠ 但它里面有一次 `cargo metadata --no-deps`，**吃 cargo 的构建锁**
#   ⇒ 必须跟着 rust 轨，不能因为「看起来是纯读文件」就塞进 cheap 轨。
Step "license consistency" -Action { & pwsh -NoProfile -File "$PSScriptRoot\check-license.ps1" 2>&1 | Select-Object -Last 15 }

# 底色不透明度：`app.css` 顶头那条硬约束（Tauri 透明窗口 + WebView2
# 分层合成，半透明底色整层不绘制）**浏览器里查不出来**——
# `?demo=1` 的出图评审、svelte-check、pnpm build 全在 Chromium 里跑。
# 2026-10-18 实测扫到 7 处，已全部改成「混背后那张纸」。
Step "opaque backgrounds" -Action { & pwsh -NoProfile -File "$PSScriptRoot\check-opaque-bg.ps1" 2>&1 | Select-Object -Last 15 }

# 取证闸（`.githooks/pre-commit`）本身还在不在、判据还是不是那样。
# 凭据闸是硬约束第 7 条的最后一道防线，而**它自己没有任何守卫**——
# 2026-10-18 实测 `grep -n 'githooks' scripts/gate.ps1` 零命中，
# 它整个消失或被改坏，门禁一声不吭（同「守卫本身没有守卫」那一族）。
Step "forensics hook guard" -Action { & pwsh -NoProfile -File "$PSScriptRoot\check-hooks.ps1" 2>&1 | Select-Object -Last 15 }

# ---- 以下按轨分派 ----

Step "cargo fmt --all --check" -Action { cargo fmt --all --check 2>&1 | Select-Object -Last 40 }
Step "cargo clippy -D warnings" -Action { cargo clippy --workspace --all-targets -- -D warnings 2>&1 | Select-Object -Last 60 }
Step "cargo test --workspace" -Action { cargo test --workspace 2>&1 | Select-Object -Last 60 }

# 仓库里不能有 API key（硬约束第 7 条）。
# key 从环境变量 CHATSERVANT_PROBE_KEY 读——**绝不能进命令行**，
# 命令行会留在 shell 历史和进程列表里，比日志泄露更隐蔽也更难清。
Step "secret scan" `
  -If { [bool]$env:CHATSERVANT_PROBE_KEY } `
  -Unless 'CHATSERVANT_PROBE_KEY 未设置——机器上本来就不该有 key' `
  -Action { cargo run -q -p xtask -- secret-scan 2>&1 | Select-Object -Last 20 }

# 走 scripts/deny.ps1：advisory-db 要从 github.com 拉，直连会 schannel
# 握手失败，那是网络问题不是门禁红。代理与 openssl 后端都在那个脚本里。
Step "cargo deny check" `
  -If { [bool](Get-Command cargo-deny -ErrorAction SilentlyContinue) } `
  -Unless 'cargo-deny 未安装：cargo install cargo-deny' `
  -Action { & pwsh -NoProfile -File "$PSScriptRoot\deny.ps1" 2>&1 | Select-Object -Last 60 }

# npm 侧漏洞（2026-10-23 加，票 81）。Rust 侧有 cargo deny advisories，
# **npm 侧原先一步都没有** ⇒ 2026-10-21 那批公告能一直躺在锁文件里。
#
# ⚠ 形状：**取不到就跳过，但必须显眼**（仓库主人拍板）。
# 走 scripts/npmaudit.ps1：默认源 npmmirror **没有 audit 端点**，
# 而「跳过要打横幅」这件事也在那一层。
Step "npm audit" `
  -If { Test-Path "ui\pnpm-lock.yaml" } `
  -Unless 'ui/pnpm-lock.yaml 不存在（先 cd ui && pnpm install）' `
  -Action { & pwsh -NoProfile -File "$PSScriptRoot\npmaudit.ps1" 2>&1 | Select-Object -Last 40 }

# ---- 前端 ----
# node_modules 不在就跳过，不算红：那只是还没装依赖。
$uiReady = { Test-Path "ui\node_modules" }
$uiUnless = 'ui/node_modules 不存在，先 cd ui && pnpm install'
# ⚠ 三个只读步**声明顺序在前、并行执行在后**：Run-Lanes 走到第一个并行组成员时
#   一次性把整组并行掉，所以 ui build（第四个声明）天然排在组之后。
#   顺序不能倒：vite 与 `svelte-kit sync` 都写 `.svelte-kit`，重叠会互相踩。
Step "ui svelte-check" -If $uiReady -Unless $uiUnless -Dir $uiDir -Action { pnpm check 2>&1 | Select-Object -Last 40 }
Step "ui lint"         -If $uiReady -Unless $uiUnless -Dir $uiDir -Action { pnpm lint 2>&1 | Select-Object -Last 40 }
Step "ui svelte-autofixer" -If $uiReady -Unless $uiUnless -Dir $uiDir -Action { & pwsh -NoProfile -File scripts\autofixer.ps1 2>&1 | Select-Object -Last 30 }
Step "ui build"        -If $uiReady -Unless $uiUnless -Dir $uiDir -Action { pnpm build 2>&1 | Select-Object -Last 25 }

# ═══════════════════════════════════════════════════════════════════════
# 执行：父进程开子进程，子进程跑自己那条轨
# ═══════════════════════════════════════════════════════════════════════

# 子进程：按声明顺序跑本轨的步。撞到并行组的第一个成员时整组并行掉。
function Invoke-Lane {
  $laneFail = 0
  $byName = @{}
  foreach ($s in $script:steps) { $byName[$s.Name] = $s }
  $done = @{}
  foreach ($s in $script:steps) {
    if ($script:laneOf[$s.Name] -ne $Child) { continue }
    if ($done.ContainsKey($s.Name)) { continue }

    # 判「这个步在不在某个并行组里」，并记下**组的键**（不是组本身——
    # 数组一经函数边界就会被摊平，见 Invoke-StepParallel 那条注释）。
    $setKey = $null
    foreach ($k in $script:parallelSets.Keys) {
      if (@($script:parallelSets[$k]) -contains $s.Name) { $setKey = $k; break }
    }
    if ($setKey) {
      $set = @($script:parallelSets[$setKey])
      foreach ($n in $set) { $done[$n] = $true }
      # 整组里只要有一个被 -Skip 掉，整组就不并行（否则跳过的步照样跑）
      $anySkipped = @($set | Where-Object { -not (Should-Run $_) }).Count -gt 0
      if ($anySkipped) {
        foreach ($n in $set) { Invoke-StepNow $byName[$n] }
      } else {
        Invoke-StepParallel $setKey
      }
      continue
    }
    $before = $script:fail
    Invoke-StepNow $s
    if ($script:fail -ne $before) { $laneFail = 1 }
  }
  # ⚠ 这里汇总发一次，是因为父进程要跨四条轨收集跳过的步名。
  #   而每一步在跳过时**已经**发过 `##GATE##SKIP` 单步标记（给判定行用）。
  #   父进程两处都会记 ⇒ 不去重的话 `GATE_SCOPE` 会把同一步数两遍
  #   （「实际跳过 4 步」而列出来只有 2 个名字）。所以结论区按唯一名计数。
  if ($script:skippedNames.Count -gt 0) {
    Write-Output ""
    Write-Output "$($script:mark)SKIPPED`t$(@($script:skippedNames | Select-Object -Unique) -join '|')"
  }
  exit $laneFail
}

# 父进程：起四条轨，收它们的机器可读行，边收边转发，最后汇总结论。
#
# ⚠ 转发是**流式**的（ReadLineAsync + 轮询），不是等一条轨跑完才打印：
#   本仓明确要求长任务期间能看到进展（曾因 `| Out-String` 缓冲被批评）。
# ⚠ 结论**只看机器可读行**（`##GATE##STEP`），不靠解析人类可读文本：
#   人类可读格式一改，解析就静默失配，而静默失配的方向恰好是「红读成绿」。
function Invoke-Lanes {
  $pwshExe = (Get-Command pwsh -ErrorAction SilentlyContinue)
  if (-not $pwshExe) {
    Write-Output "GATE_FATAL: PATH 上没有 pwsh，无法起子进程。"
    $script:fail = 2
    return
  }
  $gatePath = Join-Path $PSScriptRoot 'gate.ps1'

  Write-Output ""
  Write-Output "========== 分轨并行 =========="
  Write-Output ("轨：{0}（cargo 那几步共用 target/ 构建锁，故同属 rust 轨串行）" -f ($script:lanes -join ' / '))

  $procs = @()
  foreach ($lane in $script:lanes) {
    $psi = [Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $pwshExe.Source
    $laneArgs = @('-NoProfile', '-File', $gatePath, '-Child', $lane)
    # ⚠⚠⚠ **必须把 -Skip / -Only 传进子进程**（2026-10-20 实测后补）：
    #   过滤（`Should-Run`）跑在**子进程**里——父进程只负责起进程、转发输出，
    #   **它自己不跑步**。而原来这里只传了 `-Child $lane`
    #   ⇒ 子进程的 `$script:skipArgs` / `$script:onlyArgs` 是**空的**
    #   ⇒ `Should-Run` 恒为真 ⇒ **每条轨都跑自己轨上的全部步**。
    #
    #   实测（2026-10-20）：`pwsh -File scripts\gate.ps1 -Only 'ui svelte-check'`
    #   跑出了 cheap / rust / net 三条轨的**全部**步，外加 ui 轨的整个并行组。
    #   ⇒ 「只跑一步」实际跑的是**完整门禁**。
    #   ⚠⚠ 假绿的方向不止是慢：父进程收到的 `##GATE##SKIP` 标记也是空的
    #   ⇒ 连 `GATE_SCOPE: PARTIAL` 那行**根本不会出现**
    #   ⇒ 用户以为自己跳过了几步，报告却显示「没跳过任何步」。
    #   这与本仓最熟的那个形状同形：**没跑**与**跑了但报 0** 在观测上一样，
    #   而这次更甚——连「我跳过了」这个提示本身也没了。
    #   ⇒ 转发的是**一个逗号合并的字符串**：`-Skip` / `-Only` 现在都是 `[string]`，
    #     而 `pwsh -File` 的数组绑定会把第二个值**静默绑给下一个参数**
    #     （实测 `-Skip deny 'ui build'` ⇒ $Only 拿到了 'ui build'）⇒
    #     必须合并成一个值再传，子进程头部再拆一次（幂等）。
    if ($script:skipArgs.Count -gt 0) { $laneArgs += @('-Skip', ($script:skipArgs -join ',')) }
    if ($script:onlyArgs.Count -gt 0) { $laneArgs += @('-Only', ($script:onlyArgs -join ',')) }
    foreach ($a in $laneArgs) { [void]$psi.ArgumentList.Add($a) }
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.WorkingDirectory = $repoRoot
    $p = [Diagnostics.Process]::Start($psi)
    $procs += [pscustomobject]@{
      Lane = $lane; Proc = $p
      Out = $p.StandardOutput.ReadLineAsync(); Err = $p.StandardError.ReadLineAsync()
    }
  }

  $sawSkipped = [System.Collections.Generic.List[string]]::new()
  $open = @($procs)
  $idle = 0
  while ($open.Count -gt 0) {
    # ⚠⚠ **一轮收割完所有已完成的行**，而不是一轮只取一行（2026-10-19 实测）。
    #   第一版内层 `foreach ('Out','Err')` 每轮只收割**各一条**就
    #   `Start-Sleep 40ms`，于是吞吐上限被钉在「40ms 一行」。
    #   门禁一轮要转发上万行 ⇒ 光 sleep 就要好几分钟——
    #   那不是卡死，是**慢到看起来像卡死**（我为此误判了三次，
    #   把正常的长任务当成死锁去排查，浪费了好几轮）。
    #   改成「drain 到流暂时取不出为止」，实测 45 行从 20.4s 降到亚秒级。
    $progressed = $false
    foreach ($h in @($open)) {
      foreach ($which in @('Out', 'Err')) {
        # ⚠ Out 与 Err **各自独立续读**，不能合在一个 `foreach ('Out','Err')` 里共用续读代码。
        #   第一版在 stderr 分支里也去读 `StandardOutput`，
        #   于是**同一个 stdout 流上同时挂着两个 `ReadLineAsync`**：
        #   后一个把前一个的 Task 顶掉，那条被顶掉的 Task 永远不会被收割，
        #   `.Result` 也永远等不到 —— 实测表现为门禁**完全卡死、一行都不打**。
        #   这类「结构对了但流控错了」的 bug 静态检查看不出来，只能靠真跑一次。
        while ($true) {
          $t = $h.$which
          if ($null -eq $t -or -not $t.IsCompleted) { break }
          $line = $t.Result
          # 续读**必须读回同一条流**：读错流就等于凭空多起一个并发读取（见上）。
          if ($which -eq 'Out') { $h.Out = $h.Proc.StandardOutput.ReadLineAsync() }
          else { $h.Err = $h.Proc.StandardError.ReadLineAsync() }
          if ($null -eq $line) { $h.$which = $null; break }   # EOF
          $progressed = $true
          if ($which -eq 'Err') { $line = "[$($h.Lane)][err] $line" }
          elseif (-not $line.StartsWith($script:mark)) { $line = "[$($h.Lane)] $line" }
          Write-Output $line
          if ($line -like "$($script:mark)STEP*") {
            $f = $line -split "`t"
            # ⚠⚠ 退出码字段必须能解析成整数（2026-10-22 实测踩到）：
            #   字段是**空串**时 `[int]""` 抛转换异常，那句 if 根本不执行
            #   ⇒ 这一步明明打了 FAILED，整轮门禁却照样 ALL GREEN。
            #   ⇒ 解析不出来就**判红**：拿不到退出码 ≠ 通过。
            $stepCode = 0
            if (-not [int]::TryParse($f[2], [ref]$stepCode)) {
              Write-Output ("GATE_FATAL: 步 `"{0}`" 的退出码不是整数（`"{1}`"）—— 拿不到退出码不等于通过，判红。" -f $f[1], $f[2])
              $script:fail = 1
            } elseif ($stepCode -ne 0) { $script:fail = 1 }
            $secs = 0.0
            if ([double]::TryParse($f[3], [ref]$secs)) { $script:stepTime[$f[1]] = $secs }
          } elseif ($line -like "$($script:mark)SKIP`t*") {
            $script:skippedNames += ($line -split "`t")[1]
          } elseif ($line -like "$($script:mark)SKIPPED`t*") {
            # ⚠⚠ **必须展开数组**（2026-10-20 修）：
            #   `-split '\|'` 得到的是**数组**，而 `List[string].Add(数组)`
            #   会把整个数组当成**一个元素**加进去（`Add` 不展开 `IEnumerable`）
            #   ⇒ 元素被 `ToString()` 成一条「所有步名用空格连起来」的**假步名**
            #   ⇒ 后面 `GATE_SCOPE: PARTIAL —— 实际跳过 N 步` 的 N 虚高，
            #     而且那行列出的名字里混着一长串没人看得懂的东西。
            #   ⚠⚠ 这条路径**在 -Skip/-Only 真正生效之前从来没被激活过**
            #   （skippedNames 恒空 ⇒ 这行从不打印）⇒ 它自己的 bug 藏了整整四天。
            #   ⇒ 教训同族：**一个从未被执行过的分支，不等于它是对的**。
            foreach ($nm in (($line -split "`t")[1] -split '\|')) {
              if ($nm) { $sawSkipped.Add($nm) }
            }
          }
        }
      }
      if ($null -eq $h.Out -and $null -eq $h.Err) {
        $h.Proc.WaitForExit()
        # ⚠ 子进程整体退非 0 也算红：它可能是在加载阶段就崩了，
        #   那样一条 STEP 行都不会有，只看标记就会漏。
        if ($h.Proc.ExitCode -ne 0) { $script:fail = 1 }
        $open = @($open | Where-Object { $_ -ne $h })
      }
    }
    # ⚠ 只在**这一整轮没有任何进展**时才 sleep。有进展就立刻再转一圈，
    #   那样突发输出会被连续收割，而空闲期仍有 25ms 的轮询兜底
    #   （不能让 CPU 空转 100%）。
    if ($open.Count -gt 0) {
      if ($progressed) { $idle = 0 } else { Start-Sleep -Milliseconds 25; $idle++ }
    }
  }
  foreach ($n in $sawSkipped) { $script:skippedNames += $n }
  # ⚠⚠ **不能用 `return` 把退出码交出去**（2026-10-19 实测踩到）。
  #   PowerShell 的函数返回值是**输出流上的一切**：`Invoke-Lanes` 里那些
  #   `Write-Output $line`（转发子进程输出用的）会全部被收进返回值，
  #   于是 `$fail = Invoke-Lanes` 拿到的是「几万字输出 + 退出码」拼成的巨型对象。
  #   它的真值仍然非 0（所以红绿没判错），但结论区会把那堆文本**再打一遍**，
  #   而且 `exit $fail` 拿到非整数 ⇒ pwsh 退出码被截断成随机值。
  # ⇒ 退出码**就地留在 `$script:fail`**，函数末尾不回传任何值。
  #   ⚠ 这里**绝不能再写 `$script:fail = 0`**：那会把上面循环里
  #   累积的失败标记清掉 —— 正是「刚判红又自己抹掉」的假绿。
  #   同族：Assert-Lanes 里的 Write-Output 同样会混进 `$assertFail`，
  #   所以它只在**出错路径**输出，正常路径 return 0 且不输出。
}

$assertFail = Assert-Lanes
if ($assertFail -ne 0) {
  $script:assertMsg | ForEach-Object { Write-Output $_ }
  exit $assertFail
}

# ⚠⚠ **子进程必须在自己的轨跑完后立刻退出**（2026-10-19）。
#   下面那整段（target gc / 分步耗时 / GATE_SCOPE / 结论 / exit）是**父进程的事**。
#   忘了在这里 exit 的话，四条轨会各自跑一遍 target gc（四个 uv 进程抢同一把
#   cargo 锁）、各自打一份 `GATE: ALL GREEN`，而父进程还会再打一份 ⇒
#   同一轮门禁出现 5 份结论，其中 4 份只知道**自己那条轨**的红绿。
#   那不是冗余，那是**四个局部结论伪装成全局结论**——本仓最熟的那个坑。
if ($Child) { Invoke-Lane; exit 2 }   # 2 = 走到这里说明 Invoke-Lane 没 exit，是 bug
# ⚠ `Invoke-Lanes` **不返回值**（见函数末尾那条注释：它转发了几万字输出，
#   用 return 交退出码会把那些文本一起收进来）。红绿就地留在 `$script:fail`。
Invoke-Lanes

Write-Output ""
Write-Output "=========================="

# ---- 构建产物回收（维护，不参与 fail）--------------------------------
#
# 为什么要它：**Cargo 没有 target/ 的自动 GC**。Rust 1.88 加的自动清理官方原文
# 是 "garbage collection on the cache in its home directory" —— 管的是 `~/.cargo`
# 里下载的 .crate / registry 索引 / git 检出，**target/ 一个都不管**
# （cargo#13060 的 `gc` 子命令至今 unstable；本机 1.94.1 的 `cargo --list`
# 里也确实只有 `clean`）。
#
# 实测（2026-10-14，target 清空后全新构建对比）：
#     deps 1.90→10.55 GB (5.5×)  incremental 0.18→3.88 GB (21×)  合计 2.82→16.2 GB
# 而 `ui/scripts/targetaudit.py` 查过多哈希残留只占 deps 的 1%
# ⇒ 这是**逐轮叠加**，不是残留 ⇒ `cargo clean` 只能救急，得有自动机制兜底。
#
# 它刻意**不参与 $fail**：磁盘占用不是代码质量，红了会让人以为代码有问题。
# 超过 8 GiB 才动手，只回收 7 天没被动过的 `incremental/` 与 `deps/` 产物，
# 且**保留 `.fingerprint`**（Cargo 判断要不要重编全靠它）。
#
# ⚠ 必须排在四条轨**全部结束之后**：它自己会跑
#   `cargo test --workspace --all-targets --no-run --message-format=json`
#   去问构建工具「哪些产物还在用」，而那需要 cargo 的构建锁——
#   与任何一条 cargo 轨重叠都只会互相等锁。
$ErrorActionPreference = "Continue"
Write-Output "---------- target gc ----------"
& uv run --no-project python scripts\targetgc.py 2>&1 | Select-Object -Last 6
# 这一步不参与 $fail，**所以它坏了必须自己喊**。
#   不加这句的话：uv 没装 / 脚本语法错 / 路径写错，输出会和「一切正常」一模一样，
#   而机制已经三个月没干活了没人知道——本轮反复踩的就是这类静默失效。
if ($LASTEXITCODE -ne 0) {
  Write-Output ">>> target gc FAILED (exit=$LASTEXITCODE) - 回收机制没干活，磁盘仍会涨"
  Write-Output ">>> 这不是代码质量红，但请顺手修：uv 是否在 PATH、scripts\targetgc.py 是否存在"
}
$ErrorActionPreference = "Continue"

# ---- 分步耗时（2026-10-16 加）---------------------------------
#
# 为什么加：**在没有数字之前，「哪一步慢」全是猜的**，而猜错的方向就是白花的时间。
# 这个仓库已经因此白花过一轮：autofixer 被判定成瓶颈并改写成调本地二进制，
# 改完实测才发现它只占 19s（11 个文件 × 1.7s），真正的耗时大头在 cargo 那三步。
#
# ⚠⚠ **并行化之后这里的数字要换一种读法**（2026-10-19）：
#   每步的秒数是它**自己那条轨上**的耗时，而四条轨是同时跑的，
#   所以「各步秒数之和」**不再等于**门禁墙钟。
#   并行组那三步的秒数是**整组的墙钟**（三步共用），不是各自耗时。
#   ⇒ 想知道 ui 那三步**各自**的真实耗时，用 `-Only 'ui svelte-check'`
#     单步跑：组里另两步会被过滤掉 ⇒ 整组退回逐个跑 ⇒ 那一行才是它自己的耗时。
#     （`-Only ui` 仍会命中三步 ⇒ 仍是并行组 ⇒ 拿到的还是组墙钟。）
#   ⇒ 真正该看的对比是「MAX（各步秒数）」与本行 TOTAL 之间的差。
#
# ⚠⚠⚠ **并行组在表里折叠成一行，只计一次**（2026-10-20 加）：
#   三步共用同一份组墙钟，若各记一次，「步秒之和」会**多算 2×组墙钟**。
#   那不是小数点的事：修完计时回传（`##GATE##STEP` 原来把秒数硬写成 0，
#   于是这三步在表里显示 0.0s）之后，步秒之和实测从 **96.2s 跳到 230.8s**，
#   而 AGENTS.md 里「并行化**之前**的串行基线」正好是 **229.3s**
#   ⇒ 下一个读表的人会直接读成「并行化白做了」。
#   ⇒ 折叠成一行后，步秒之和回到「每份墙钟只算一次」的口径。
#
# ⚠⚠ **折叠的判据是「模式」，不是「数值相同」**（同一条纪律的第二轴）：
#   组里只要有一步被 `-Skip` / `-Only` 过滤掉，整组就**退回逐个跑**
#   （见 Invoke-Lane 里 `$anySkipped` 那条）⇒ 每步的秒数是**各自真实耗时**，
#   此时折叠成一行就是**错的**（会把三个不同的耗时说成一个）。
#   ⇒ 判据用「组内成员有没有被跳过」这个**行为事实**（父进程收到
#     `##GATE##SKIP` 标记才知道），不是「三行秒数恰好一样」。
$total = 0.0
$maxStep = 0.0
foreach ($k in $script:stepTime.Keys) {
  $total += $script:stepTime[$k]
  if ($script:stepTime[$k] -gt $maxStep) { $maxStep = $script:stepTime[$k] }
}
if ($script:stepTime.Count -gt 0) {
  Write-Output "---------- timing (steps, 四轨并行) ----------"

  # 判哪些组该折叠：组内**没有任何一步**被跳过 ⇒ 它是并行跑的。
  $skippedUniq = @($script:skippedNames | Select-Object -Unique)
  $foldRows = [ordered]@{}   # 显示名 -> 秒数
  $foldedMembers = @{}       # 被折叠掉的成员步名
  foreach ($setKey in $script:parallelSets.Keys) {
    $members = @($script:parallelSets[$setKey])
    $anySkipped = @($members | Where-Object { $skippedUniq -contains $_ }).Count -gt 0
    if ($anySkipped) { continue }
    $first = $null
    $allPresent = $true
    foreach ($n in $members) {
      if (-not $script:stepTime.Contains($n)) { $allPresent = $false; break }
      if ($null -eq $first) { $first = $n }
    }
    if (-not $allPresent) { continue }
    $label = "{0}（并行组 {1} 步：{2}）" -f $setKey, $members.Count, ($members -join ' / ')
    $foldRows[$label] = $script:stepTime[$first]
    foreach ($n in $members) { $foldedMembers[$n] = $true }
  }
  # 折叠掉的成员不再单独计一次（否则步秒之和会多算 2×组墙钟）。
  foreach ($n in $foldedMembers.Keys) {
    $total -= $script:stepTime[$n]
  }

  $rows = @()
  foreach ($k in $script:stepTime.Keys) {
    if ($foldedMembers.ContainsKey($k)) { continue }
    $rows += [pscustomobject]@{ Sec = $script:stepTime[$k]; Name = $k }
  }
  foreach ($label in $foldRows.Keys) {
    $rows += [pscustomobject]@{ Sec = $foldRows[$label]; Name = $label }
  }
  foreach ($r in ($rows | Sort-Object Sec -Descending)) {
    Write-Output ("  {0,7:N1}s  {1}" -f $r.Sec, $r.Name)
  }
  Write-Output ("  {0,7:N1}s  最慢的一步（并行下的下界）" -f $maxStep)
  Write-Output ("  {0,7:N1}s  步秒之和（并行下**不等于**墙钟，只看它会高估）" -f $total)
}

# 带 -Skip / -Only 的运行**不是完整门禁**。这行必须紧挨着结论，
# 否则「跳过 deny 之后全绿」会被读成「全绿」——那正是 2026-10-11
# 踩过的同一个形状（局部结论被当成全局结论，还据此提交了）。
#
# ⚠⚠ **报实际跳过的步名，不报用户给的参数**（2026-10-19 实测后改）。
# 原来的写法是 `跳过了部分步骤（-Skip $($Skip -join ',')）`，
# 而 `-Skip 'deny,ui build'` 在 `-File` 下没按逗号切分 ⇒ 一步都没跳，
# 可那行**照抄用户给的参数**，读起来像是跳过了两步。
# ⇒ 用户传的参数与实际行为不符时，这行就成了**假凭据** ——
#   比没有提示更糟：没有提示时人会怀疑，假的提示会让人相信跳过了。
if ($script:skippedNames.Count -gt 0) {
  # ⚠ 按**唯一**步名计数：每个跳过的步会发两次标记（单步 SKIP + 本轨汇总 SKIPPED），
  #   不去重的话「实际跳过 N 步」与列出来的名字数对不上。
  $uniqSkipped = @($script:skippedNames | Select-Object -Unique)
  Write-Output "GATE_SCOPE: PARTIAL —— 实际跳过 $($uniqSkipped.Count) 步（$($uniqSkipped -join ' / ')），结论只覆盖跑过的那 $($script:stepTime.Count) 步"
}

# ⚠⚠ **传了却匹配不到任何一步**也要说（2026-10-19 补）。
# `-Skip` / `-Only` 是子串匹配，于是拼错一个字母就静默不命中，
# 而「我没跳过」与「我跳过了」在结论里长得**一模一样**（都是 ALL GREEN）。
# 用户以为自己跳过了三步、实际全跑了一遍 —— 于是按内循环的假设读结论，
# 而那个假设根本不成立。
$asked = @($script:skipArgs + $script:onlyArgs)
if ($asked.Count -gt 0 -and $script:skippedNames.Count -eq 0) {
  Write-Output "GATE_SCOPE_WARN: 传了 -Skip/-Only（$($asked -join ',')）却**一步都没跳过** —— 检查写法（子串匹配，见脚本头部「必须拆逗号」那条）"
}
elseif ($asked.Count -gt 0) {
  # 部分命中也要点出来：`-Skip 'deny,ui build,dne'` 里 `dne` 是拼错的，
  # 而提示若只说「跳过了 2 步」用户不会知道第三个没生效。
  $allSteps = @($script:stepTime.Keys) + @($script:allStepNames)
  $missed = @()
  foreach ($pat in $script:skipArgs + $script:onlyArgs) {
    if (-not ($allSteps | Where-Object { $_ -like "*$pat*" })) { $missed += $pat }
  }
  if ($missed.Count -gt 0) {
    Write-Output "GATE_SCOPE_WARN: 这些 -Skip/-Only 片段**没匹配到任何一步**（子串匹配）：$($missed -join ', ')"
  }
}

if ($fail -eq 0) { Write-Output "GATE: ALL GREEN" } else { Write-Output "GATE: FAILED" }
Write-Output "GATE_EXIT=$fail"

# **必须真的 exit，而不是只把 GATE_EXIT 打印出来。**
#
# 2026-10-11 实测踩到：脚本原来只 `Write-Output "GATE_EXIT=$fail"` 而不 exit，
# 于是 `pwsh -File gate.ps1` 永远返回 0 —— clippy 红了、门禁打了
# `GATE: FAILED`，而外层看到的 `$LASTEXITCODE` 是 0。
# 结果是一个**带红门禁的 commit 被提交了**，还差点当成全绿汇报出去。
#
# 判断门禁成不成功**只能看它打印的那两行**，退出码是给自动化用的；
# 两者现在终于说同一件事了。
exit $fail
