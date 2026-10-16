# 给 agent 与人用的**一个**开发入口。
#
# ## 为什么要有它
#
# 不是为了「把命令包起来」，而是为了消灭两类反复出现的摩擦：
#
# 1. **拼错顺序 / 漏一步**。门禁九步的顺序在 `gate.ps1` 里是唯一的真相来源，
#    本脚本**只做转发**，不复制任何一步的命令行——复制就会漂（本仓在
#    命令文案上已经吃过两次「两处各抄一份」的亏）。
# 2. **shell 层的编码与引号事故**。会话默认 shell 是 Windows PowerShell 5.1：
#    `>` 默认写 UTF-16、`Format-Hex -Count` 不存在、`@{u}` 会被当成自己的语法。
#    这个脚本一律以 pwsh 7 启动自己，调用者只需要
#    `pwsh -NoProfile -File scripts\dev.ps1 <verb>`。

# 配色/排版不是本脚本的事；设计准则见 `docs/设计规范.md`。
[CmdletBinding()]
param(
  [Parameter(Position = 0)]
  [ValidateSet('check', 'test', 'ui', 'ui-check', 'app', 'gate', 'clean', 'size', 'help')]
  [string]$Verb = 'help',

  # test 的过滤串，直接透传给 `cargo test`（`cargo test <filter>`）。
  [string]$Filter,

  # gate 的步骤过滤，子串匹配，例如 -Skip 'deny,ui build'。
  [string[]]$Skip = @(),
  [string[]]$Only = @(),

  [switch]$DryRun
)

$ErrorActionPreference = 'Continue'
$repo = Resolve-Path (Join-Path $PSScriptRoot '..')
Set-Location $repo

# 本脚本自己可能就是被 PS 5.1 启用的，所以**在这之内**一律用 pwsh 7 干活。
$pwshCmd = Get-Command pwsh -ErrorAction SilentlyContinue
if (-not $pwshCmd) {
  Write-Output 'dev.ps1: 找不到 pwsh（PowerShell 7+）。请用它运行本脚本。'
  exit 127
}
$pwsh = $pwshCmd.Source

function Run([string]$label, [scriptblock]$body) {
  Write-Output ''
  Write-Output "========== $label =========="
  $sw = [Diagnostics.Stopwatch]::StartNew()
  & $body
  $code = $LASTEXITCODE
  $sw.Stop()
  Write-Output ('>>> {0} [{1:N1}s] (exit={2})' -f
    $(if ($code -ne 0) { "$label FAILED" } else { "$label OK" }),
    $sw.Elapsed.TotalSeconds, $code)
  return $code
}

function Say([string]$text) { Write-Output $text; Write-Output '' }

switch ($Verb) {

  'help' {
    Say @'
chat-servant 开发入口（用 pwsh 7 运行）

  dev.ps1 check        Rust 快速门禁：fmt --check + clippy -D warnings
  dev.ps1 test [串]    cargo test --workspace [过滤串]
  dev.ps1 ui-check     前端三项：svelte-check / lint / autofixer（不含 build）
  dev.ps1 ui           前端全查（含 vite build）
  dev.ps1 app          启动 Tauri dev（内部会自己起 vite，别再单起 pnpm dev）
  dev.ps1 gate         完整九步门禁（= scripts/gate.ps1，可加 -Skip / -Only）
  dev.ps1 size         target/ 体积报告 + 多哈希占比
  dev.ps1 clean        回收 target/ 的旧哈希产物（-DryRun 只报告）

⚠ 判门禁红绿只看 gate.ps1 打的 GATE: ALL GREEN / GATE: FAILED 那两行。
⚠ check / test / ui-check 都不含 cargo deny 与 secret-scan，**不是完整门禁**。
'@
  }

  'check' {
    # 这两条是改 Rust 之后最常跑的两条，也是最便宜的两条。
    # `cargo check` 不是其中之一：clippy 已经要编译，改用它不会更快。
    $a = Run 'cargo fmt --all --check' { & cargo fmt --all --check }
    $b = Run 'cargo clippy -D warnings' { & cargo clippy --workspace --all-targets -- -D warnings }
    if ($a -ne 0 -or $b -ne 0) { exit 1 } else { exit 0 }
  }

  'test' {
    $args = @('test', '--workspace')
    if ($Filter) { $args += $Filter }
    $code = Run "cargo test $($args -join ' ')" { & cargo @args }
    exit $code
  }

  'ui-check' {
    Push-Location ui
    $a = Run 'svelte-check' { & pnpm check }
    $b = Run 'prettier + eslint' { & pnpm lint }
    $c = Run 'svelte-autofixer' { & $pwsh -NoProfile -File scripts/autofixer.ps1 }
    Pop-Location
    if ($a -ne 0 -or $b -ne 0 -or $c -ne 0) { exit 1 } else { exit 0 }
  }

  'ui' {
    $code = Run 'ui 全查（含 build）' { & $pwsh -NoProfile -File scripts\dev.ps1 ui-check }
    if ($code -ne 0) { exit $code }
    Push-Location ui
    $d = Run 'vite build' { & pnpm build }
    Pop-Location
    exit $d
  }

  'app' {
    # 别先单独跑 `pnpm dev`：tauri dev 自己会起 vite（BeforeDevCommand），
    #   占着 1420 之后 beforeDevCommand 非零退出，应用起不来。
    Push-Location ui
    Run 'tauri dev' { & pnpm tauri dev }
    Pop-Location
  }

  'gate' {
    $gargs = @('-NoProfile', '-File', (Join-Path $PSScriptRoot 'gate.ps1'))
    if ($Skip.Count) { $gargs += @('-Skip') + $Skip }
    if ($Only.Count) { $gargs += @('-Only') + $Only }
    & $pwsh @gargs
    exit $LASTEXITCODE
  }

  'size' {
    # 体积是本机事实（实测会变），按项目纪律**直接跑命令**查，不抄进文档。
    Run 'target 体积' { & uv run --no-project python scripts\targetgc.py --report }
    Run '多哈希占比' { & uv run --no-project python ui\scripts\targetaudit.py }
  }

  'clean' {
    # 不删 .fingerprint —— Cargo 靠它判断要不要重编，删了会让整棵依赖树重编。
    #   保留策略写在 scripts/targetgc.py 的 docstring 里，这里只转发，不复制。
    if ($DryRun) {
      Run 'targetgc（只报告）' { & uv run --no-project python scripts\targetgc.py --dry-run }
    } else {
      Run 'targetgc' { & uv run --no-project python scripts\targetgc.py }
    }
  }
}

exit 0
