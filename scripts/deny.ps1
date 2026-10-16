# cargo deny check。
#
# RustSec 的 advisory-db 是 git 仓库，cargo-deny 每次都会去拉最新 main。
#
#**2026-10-15 纠正：本机直连 github.com 完全可用，代理是多余的。**
# 实测：`git -c http.proxy= -c https.proxy= ls-remote .../advisory-db HEAD` ⇒ exit=0。
# 而本机并没有 `127.0.0.1:7897` 那个代理（Test-NetConnection ⇒ False）。
# ⇒ 下面默认**不挂代理**；代理改成可选（设了环境变量才用）。
#
# 保留 openssl 后端：schannel 走 HTTP CONNECT 代理经常握手失败，openssl 能通。
# 用 GIT_CONFIG_* 传，**不污染全局 git config**。
#
# 2026-10-13 踩过：这里原先硬编码 `127.0.0.1:7897`，而当时那台机器上的代理
#   是局域网地址 `10.46.35.24:10080`，于是 `cargo deny check` 以 exit=1 结束，
#   报的是「拉不到 advisory-db」——**看起来像门禁红，其实是代理地址写错**。
#   换成正确地址后同一条命令输出 `advisories ok`。
#
# 2026-10-14 又踩了同一个坑的**镜像版**：默认值被改成了 `10.46.35.24:10080`，
#   而本机代理回到 `127.0.0.1:7897`。实测两端口：
#       127.0.0.1:7897   通
#       10.46.35.24:10080  不通（连接超时 21s）
#   ⇒ 症状与上面完全一样：`fatal: unable to access ... advisory-db`，
#     报的是网络，实际是**默认地址写的是另一台机器的**。
#   ⇒ 判据：**先确认端口通不通，再看 deny 报什么**。连不上就不是代码红。
$ErrorActionPreference = 'Continue'

$proxy = if ($env:CHATSERVANT_PROXY) { $env:CHATSERVANT_PROXY }
elseif ($env:HTTPS_PROXY) { $env:HTTPS_PROXY }
else { '' }

if ($proxy) {
    $env:HTTP_PROXY  = $proxy
    $env:HTTPS_PROXY = $proxy
    Write-Output ">>> deny via proxy: $proxy"
} else {
    # Must clear inherited values too, or git still dials a dead port.
    $env:HTTP_PROXY  = ''
    $env:HTTPS_PROXY = ''
    $env:ALL_PROXY   = ''
    Write-Output ">>> deny direct (github.com reachable on this machine)"
}

$env:GIT_CONFIG_COUNT  = '1'
$env:GIT_CONFIG_KEY_0  = 'http.sslBackend'
$env:GIT_CONFIG_VALUE_0 = 'openssl'
if ($proxy) {
    $env:GIT_CONFIG_COUNT  = '3'
    $env:GIT_CONFIG_KEY_1  = 'http.proxy'
    $env:GIT_CONFIG_VALUE_1 = $proxy
    $env:GIT_CONFIG_KEY_2  = 'https.proxy'
    $env:GIT_CONFIG_VALUE_2 = $proxy
}

cargo deny check
exit $LASTEXITCODE