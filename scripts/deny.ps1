# cargo deny check。
#
# RustSec 的 advisory-db 是 git 仓库，cargo-deny 每次都会去拉最新 main。
# 本机直连 github.com 会 schannel 握手失败——**那是网络问题，不是门禁红**。
#
# 两条对策：
#   ① 挂代理。**默认走局域网那个 10.46.35.24:10080**（2026-10-13 用户确认）；
#      已存在的环境变量优先，所以临时换端口不用改这个文件。
#   ② git 换成 openssl 后端——schannel 走 HTTP CONNECT 代理经常握手失败，
#      openssl 能通。用 GIT_CONFIG_* 环境变量传，**不污染全局 git config**。
#
# ⚠ 2026-10-13 踩过：这里原先硬编码 `127.0.0.1:7897`，而那台机器上的代理
#   是局域网地址，于是 `cargo deny check` 以 exit=1 结束，报的是
#   「拉不到 advisory-db」——**看起来像门禁红，其实是代理地址写错**。
#   换成正确地址后同一条命令输出 `advisories ok`。
$ErrorActionPreference = 'Continue'

$proxy = if ($env:CHATSERVANT_PROXY) { $env:CHATSERVANT_PROXY }
elseif ($env:HTTPS_PROXY) { $env:HTTPS_PROXY }
else { 'http://10.46.35.24:10080' }

$env:HTTP_PROXY  = $proxy
$env:HTTPS_PROXY = $proxy

$env:GIT_CONFIG_COUNT  = '3'
$env:GIT_CONFIG_KEY_0  = 'http.sslBackend'
$env:GIT_CONFIG_VALUE_0 = 'openssl'
$env:GIT_CONFIG_KEY_1  = 'http.proxy'
$env:GIT_CONFIG_VALUE_1 = $proxy
$env:GIT_CONFIG_KEY_2  = 'https.proxy'
$env:GIT_CONFIG_VALUE_2 = $proxy

cargo deny check
exit $LASTEXITCODE