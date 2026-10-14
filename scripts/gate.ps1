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

# 编码损坏（U+FFFD）。这道排在最前面，因为它最便宜——
# 受损的全是注释与文档，fmt / clippy / test / svelte-check 一律照绿，
# 不专门扫就永远没人发现。ticket 19。
Step "mojibake scan" { & pwsh -NoProfile -File "$PSScriptRoot\scan-mojibake.ps1" 2>&1 | Select-Object -Last 30 }

# 纸的源（probe-sheet.png）与生成物（probe-sheet.svg）必须同步。
# SVG-as-image 不能外链资源，所以 PNG 只能靠打包脚本内嵌进去；
# 两者一旦漂移，纸就会静默地变成「旧的那张」，而所有截图都照样出得来。
Step "sheet pack in sync" { & pwsh -NoProfile -File ui\scripts\sheetpack.ps1 -Check 2>&1 | Select-Object -Last 10 }

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
$ErrorActionPreference = "Continue"
Write-Output "---------- target gc ----------"
& uv run --no-project python scripts\targetgc.py 2>&1 | Select-Object -Last 6
# ⚠ 这一步不参与 $fail，**所以它坏了必须自己喊**。
#   不加这句的话：uv 没装 / 脚本语法错 / 路径写错，输出会和「一切正常」一模一样，
#   而机制已经三个月没干活了没人知道——本轮反复踩的就是这类静默失效。
if ($LASTEXITCODE -ne 0) {
  Write-Output ">>> target gc FAILED (exit=$LASTEXITCODE) - 回收机制没干活，磁盘仍会涨"
  Write-Output ">>> 这不是代码质量红，但请顺手修：uv 是否在 PATH、scripts\targetgc.py 是否存在"
}
$ErrorActionPreference = "Continue"

if ($fail -eq 0) { Write-Output "GATE: ALL GREEN" } else { Write-Output "GATE: FAILED" }
Write-Output "GATE_EXIT=$fail"

# ⚠ **必须真的 exit，而不是只把 GATE_EXIT 打印出来。**
#
# 2026-10-11 实测踩到：脚本原来只 `Write-Output "GATE_EXIT=$fail"` 而不 exit，
# 于是 `pwsh -File gate.ps1` 永远返回 0 —— clippy 红了、门禁打了
# `GATE: FAILED`，而外层看到的 `$LASTEXITCODE` 是 0。
# 结果是一个**带红门禁的 commit 被提交了**，还差点当成全绿汇报出去。
#
# 判断门禁成不成功**只能看它打印的那两行**，退出码是给自动化用的；
# 两者现在终于说同一件事了。
exit $fail