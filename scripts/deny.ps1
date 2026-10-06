# cargo deny check。
#
# RustSec 的 advisory-db 是 git 仓库，cargo-deny 每次都会去拉最新 main。
# 本机直连 github.com 会 schannel 握手失败——**那是网络问题，不是门禁红**。
#
# 两条对策：
#   ① 挂本机代理 127.0.0.1:7897
#   ② git 换成 openssl 后端——schannel 走 HTTP CONNECT 代理经常握手失败，
#      openssl 能通。用 GIT_CONFIG_* 环境变量传，**不污染全局 git config**。
$ErrorActionPreference = 'Continue'

$env:HTTP_PROXY  = 'http://127.0.0.1:7897'
$env:HTTPS_PROXY = 'http://127.0.0.1:7897'

$env:GIT_CONFIG_COUNT  = '3'
$env:GIT_CONFIG_KEY_0  = 'http.sslBackend'
$env:GIT_CONFIG_VALUE_0 = 'openssl'
$env:GIT_CONFIG_KEY_1  = 'http.proxy'
$env:GIT_CONFIG_VALUE_1 = 'http://127.0.0.1:7897'
$env:GIT_CONFIG_KEY_2  = 'https.proxy'
$env:GIT_CONFIG_VALUE_2 = 'http://127.0.0.1:7897'

cargo deny check
exit $LASTEXITCODE