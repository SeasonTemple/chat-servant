$ErrorActionPreference = 'Continue'
$root = Split-Path $PSScriptRoot -Parent
Set-Location (Join-Path $root 'ui')
$log = Join-Path $root 'target\app.log'
New-Item -ItemType Directory -Force -Path (Split-Path $log) | Out-Null
Remove-Item $log -ErrorAction SilentlyContinue

# 用 `Start-Process -RedirectStandardOutput` 而不是管道 + Tee。
# 管道那头一断（后台任务被取消、终端关闭），Tee 就跟着死，
# 应用还在跑却再也写不出日志——上一轮排查被这个坑误导过。
#
# ⚠⚠ 2026-10-18 订正：原来的 `info,chat_servant=debug` **一档都没命中**。
# `tracing` 的 `EnvFilter` 按 `::` 分段做**前缀**匹配，而实际的 target 名是：
#
# | target | 出处 | 跑验收时看什么 |
# |---|---|---|
# | `chat_servant::watch` | `watch.rs` **显式 `target:`** | `tick=… peer=… candidates=`（info）、`一轮 Nms`、`roi=… 归类=`（debug） |
# | `chat_servant::pump` | `commands.rs` **显式 `target:`** | `emit readout tick=… phase=… peer=… messages=… candidates=`（**票 21 的证据行**） |
# | `chat_servant_app` | `src-tauri` 的 bin（包名 `chat-servant-app`，连字符转下划线） | 托盘、命令层 |
# | `chatservant_capture` / `chatservant_perceive` | 工作区 crate | OCR 与归类细节 |
#
# ⚠ 上面两行显式 target **必须逐个点名**。守卫
# `xtask` 的 `every_explicit_tracing_target_is_named_in_the_run_app_filter`
# 会扫源码里每一个显式 `target:` 并要求这里逐个命中 ⇒ 谁新增一条忘了同步，这边就红。
# ⚠ 判据不是读源码猜的：从当前编译产物
# （`target/release/chat-servant-app.exe`）里 grep 这些字符串确认过；
# 而 `target/app.log` 里那份带连字符的样本是 **2026-10-10 的旧构建**，照抄会写错。
# ⚠ 那两处显式 target 名字**不统一**：`chat_servant::watch` 与 `chat_servant::pump`
# 一致（都下划线），但历史上 `watch.rs` 里**有一条**曾写成 `chat-servant::watch`
# （连字符），2026-10-18 已统一。target 字符串对 `EnvFilter` 不透明，
# **一条拼错就等于那一路的日志永远收不到**，而且它不报错，只是安静。
$env:RUST_LOG = 'info,chat_servant::watch=debug,chat_servant::pump=debug,chat_servant_app=debug,chatservant_capture=debug,chatservant_perceive=debug'

# ⚠ 起法用 `Start-Process`，**不是**因为「这样能脱离作业对象」——
# 2026-10-18 实测（`target/probe/detach-test.ps1`，无头、判据是存活）：
# 调用方进程退出后，`Start-Process` 起的子进程**仍然存活**，
# 而 `Win32_Process.Create`（WMI provider）那条**返回 8 起不来**
# （`pwsh` 在 WindowsApps 下，WMI 的 PATH 不同）。
# ⇒ 票 45 记的「应用被后台任务连带回收」**在这个环境下复现不出来**，
# 别据此改机制；真复现时先查是不是整棵树被 kill，而不是 `Start-Process` 本身的问题。
$p = Start-Process -FilePath 'pnpm.cmd' -ArgumentList 'tauri', 'dev' `
    -WorkingDirectory (Join-Path $root 'ui') `
    -RedirectStandardOutput $log -RedirectStandardError "$log.err" `
    -WindowStyle Hidden -PassThru
"tauri dev pid=$($p.Id)"
"日志：$log"