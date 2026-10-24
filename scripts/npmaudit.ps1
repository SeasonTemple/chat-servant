# npm audit —— npm 侧的漏洞扫描（2026-10-23 加，票 81）。
#
# ## 为什么要有这道步
#
# Rust 侧有 `cargo deny check advisories`（门禁一步），**npm 侧一步都没有**
# ⇒ 2026-10-21 那批公告（1 high / 3 moderate / 2 low）能一直躺在锁文件里，
#   直到有人手扫才发现（票 81）。
#
# ## ⚠ 必须显式指到 npmjs
#
# `pnpm audit` 在本机**默认跑不起来**：默认 registry 是 npmmirror，
# 而它**没有实现 audit 端点** ⇒ `ERR_PNPM_AUDIT_ENDPOINT_NOT_EXISTS`。
# ⇒ 必须显式 `--registry=https://registry.npmjs.org`。
#
# ## 形状（仓库主人 2026-10-23 拍板：**取不到就跳过，但必须显眼**）
#
# · 扫得到、0 条洞 → 绿；
# · 扫得到、有洞 → **红**；
# · **扫不到**（端点不存在 / 网络不通）→ **跳过 + 打横幅**
#   「本次**没有**扫 npm 侧」。
#
# ## 为什么不是「取不到就红」（票 67 的先例是判红）
#
# 票 67 禁的是「**静默**变成跳过」。而这道步的「取不到」是**环境问题**
# （国内网络 / 镜像源），**不是漏洞** ⇒ 判红会让门禁在本机**天天红**
# ⇒ 门禁一红就没人看 ⇒ 覆盖率归零，比不扫还糟。
# ⇒ 折中：**跳过但必须显眼**（横幅 + 原文），不静默。
#
# ## 豁免表（2026-10-24 加）：**逐条带理由**，与 `deny.toml` 的 exceptions 同款
#
# Rust 侧的 `cargo deny` 有豁免机制（`deny.toml` 的 `[advisories.exceptions]`，
# 逐条写清为什么不适用）；npm 侧原先没有 ⇒ 公告一发布就**只能整包升级**才能变绿。
#
# ⚠ **豁免的门槛**（三条，缺一不可）：
# ① **逐条**列 GHSA 编号，**不许**用包名或范围豁免；
# ② 每条必须写清「**为什么这条路径在本产品里不存在**」——
#    「危害小」「暂时没空」**不算理由**；
# ③ **新公告照样红** ⇒ 豁免表是**白名单**，不是降级开关。
#
# ⚠ 豁免了也**必须打横幅**（票 67 的教训：不许静默）。
$ErrorActionPreference = 'Continue'

# ⚠ 必须显式指到 npmjs（默认源 npmmirror **没有实现 audit 端点**）。
#
# ⚠ `CHATSERVANT_AUDIT_REGISTRY` 这个覆盖口**只为让「扫不到」那条分支
#   **可被验证**：默认源可达 ⇒ 那条分支在本机永远走不到 ⇒ 它就只是纸面上的。
#   ⇒ 用它指到一个没有 audit 端点的源，就能真跑一次：
#     $env:CHATSERVANT_AUDIT_REGISTRY='https://registry.npmmirror.com'
#     pwsh -NoProfile -File scripts\npmaudit.ps1
#   ⇒ 期望：打横幅「本次没有扫 npm 侧」且 exit=0（不是红）。
#   ⚠ 默认值**不许**改：npmjs 才是有端点、真正扫得到的那一个。
$registry = if ($env:CHATSERVANT_AUDIT_REGISTRY) { $env:CHATSERVANT_AUDIT_REGISTRY }
else { 'https://registry.npmjs.org' }

# ── 豁免表：GHSA 编号 → 为什么不适用 ─────────────────────────────────
# ⚠ **每条都要能指着代码说出「这条路径不存在」**，否则不许进这张表。
$exempt = @{
    'GHSA-pfhg-q3qv-9q6r' = @'
SvelteKit「增强表单提交」可跳转到不可信站点（low）。
**本产品没有 form action**：面板是 SPA + Tauri IPC（`invoke`/`emit`），
`ui/src/` 里没有任何 `<form>`、`use:enhance` 或 `export const actions`
⇒ 这条代码路径在本产品里**不存在**。
'@
    'GHSA-27mw-82g8-phjj' = @'
SvelteKit「预渲染页读到不可信 cache-control 头 ⇒ XSS」（low）。
触发前提是**有一个 HTTP 服务器**把不可信头带进预渲染页。
**本产品没有 HTTP 服务器**：Tauri 走 `file://` 本地协议加载页面
⇒ 没有响应头、也就没有不可信的 `cache-control` ⇒ 这条路径不存在。
（`ui/src/routes/+layout.ts` 确实有 `prerender = true`，但预渲染产物是
本地文件，攻击者**没有任何渠道**往它的响应头里塞东西。）
'@
}

Push-Location (Join-Path $PSScriptRoot '..' 'ui')
try {
    $out = (& pnpm audit --registry=$registry 2>&1 | Out-String)
    $code = $LASTEXITCODE
} finally {
    Pop-Location
}

if ($code -eq 0) {
    Write-Output $out.TrimEnd()
    Write-Output '>>> npm audit: no known vulnerabilities found'
    exit 0
}

# 判「取不到」：端点不存在 / 网络不通 / TLS 失败。
$unreachable = 'ERR_PNPM_AUDIT_ENDPOINT_NOT_EXISTS|ERR_PNPM_FETCH|ENOTFOUND|ETIMEDOUT|ECONNRESUSED|ECONNREFUSED|EAI_AGAIN|ECONNRESET|certificate'
if ($out -match $unreachable) {
    Write-Output '============================================================'
    Write-Output '!! npm audit **没跑到**（取不到 audit 端点 / 网络不通）'
    Write-Output '!! ⇒ 本次**没有**扫 npm 侧（这不是「扫过了没洞」）'
    Write-Output '============================================================'
    Write-Output $out.TrimEnd()
    exit 0
}

# ── 扫到了、有洞 ⇒ **先看是不是全在豁免表里** ───────────────────────
# ⚠ 只豁免**白名单里逐条列出的** GHSA；只要有一条不在表里 ⇒ 照样红。
$found = [regex]::Matches($out, 'GHSA-[0-9a-z\-]+') | ForEach-Object { $_.Value } |
    Sort-Object -Unique
$unknown = @($found | Where-Object { -not $exempt.ContainsKey($_) })

if ($found.Count -gt 0 -and $unknown.Count -eq 0) {
    Write-Output '============================================================'
    Write-Output "!! npm audit: $($found.Count) 条公告**已逐条豁免**（理由如下）"
    Write-Output '!! ⇒ 这些公告的触发路径在本产品里**不存在**（不是「危害小」）'
    foreach ($id in $found) {
        Write-Output ''
        Write-Output "--- $id ---"
        Write-Output $exempt[$id].Trim()
    }
    Write-Output '============================================================'
    exit 0
}

# 有洞（且不在豁免表里）⇒ 红。
Write-Output $out.TrimEnd()
if ($unknown.Count -gt 0) {
    Write-Output '============================================================'
    Write-Output "!! 下列公告**不在豁免表**里 ⇒ 判红：$($unknown -join ', ')"
    Write-Output '============================================================'
}
exit $code
