$ErrorActionPreference = "Continue"
Set-Location $PSScriptRoot\..
$fail = 0

# 本机 target/ 在 E: 盘上，cargo 的增量编译目录偶尔会被另一个进程锁住
# （os error 5 拒绝访问），clippy 会因此以 101 退出——那不是代码红。
# 关掉增量编译换一个确定的缓存位置，代价是慢一点。
$env:CARGO_INCREMENTAL = "0"

function Step($name, $scriptblock) {
  Write-Output ""
  Write-Output "========== $name =========="
  & $scriptblock
  if ($LASTEXITCODE -ne 0) { Write-Output ">>> $name FAILED (exit=$LASTEXITCODE)"; $script:fail = 1 }
  else { Write-Output ">>> $name OK" }
}

Step "cargo fmt --all --check" { cargo fmt --all --check 2>&1 | Select-Object -Last 40 }
Step "cargo clippy -D warnings" { cargo clippy --workspace --all-targets -- -D warnings 2>&1 | Select-Object -Last 60 }
Step "cargo test --workspace"  { cargo test --workspace 2>&1 | Select-Object -Last 60 }

# 仓库里不能有 API key（硬约束第 7 条）。
# key 从环境变量 CHATSERVANT_PROBE_KEY 读——**绝不能进命令行**，
# 命令行会留在 shell 历史和进程列表里，比日志泄露更隐蔽也更难清。
if ($env:CHATSERVANT_PROBE_KEY) {
  Step "secret scan" { cargo run -q -p xtask -- secret-scan 2>&1 | Select-Object -Last 20 }
} else {
  Write-Output ""
  Write-Output ">>> secret scan SKIPPED（CHATSERVANT_PROBE_KEY 未设置——机器上本来就不该有 key）"
}
if (Get-Command cargo-deny -ErrorAction SilentlyContinue) {
  # 走 scripts/deny.ps1：advisory-db 要从 github.com 拉，直连会 schannel
  # 握手失败，那是网络问题不是门禁红。代理与 openssl 后端都在那个脚本里。
  Step "cargo deny check" { & pwsh -NoProfile -File "$PSScriptRoot\deny.ps1" 2>&1 | Select-Object -Last 60 }
} else {
  Write-Output ""
  Write-Output ">>> cargo deny check SKIPPED（cargo-deny 未安装：cargo install cargo-deny）"
}

# ---- 前端 ----
# node_modules 不在就跳过，不算红：那只是还没装依赖。
if (Test-Path "ui\node_modules") {
  Step "ui svelte-check" { Push-Location ui; pnpm check 2>&1 | Select-Object -Last 40; Pop-Location }
  Step "ui lint"         { Push-Location ui; pnpm lint 2>&1 | Select-Object -Last 40; Pop-Location }
  Step "ui svelte-autofixer" { Push-Location ui; & pwsh -NoProfile -File scripts/autofixer.ps1 2>&1 | Select-Object -Last 30; Pop-Location }
  Step "ui build"        { Push-Location ui; pnpm build 2>&1 | Select-Object -Last 25; Pop-Location }
} else {
  Write-Output ""
  Write-Output ">>> ui SKIPPED（ui/node_modules 不存在，先 cd ui && pnpm install）"
}

Write-Output ""
Write-Output "=========================="
if ($fail -eq 0) { Write-Output "GATE: ALL GREEN" } else { Write-Output "GATE: FAILED" }
Write-Output "GATE_EXIT=$fail"