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
$ErrorActionPreference = 'Continue'

# ⚠ 必须显式指到 npmjs（默认源没有 audit 端点）。
$registry = 'https://registry.npmjs.org'

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
$unreachable = 'ERR_PNPM_AUDIT_ENDPOINT_NOT_EXISTS|ERR_PNPM_FETCH|ENOTFOUND|ETIMEDOUT|ECONNREFUSED|EAI_AGAIN|ECONNRESET|certificate'
if ($out -match $unreachable) {
    Write-Output '============================================================'
    Write-Output '!! npm audit **没跑到**（取不到 audit 端点 / 网络不通）'
    Write-Output '!! ⇒ 本次**没有**扫 npm 侧（这不是「扫过了没洞」）'
    Write-Output '============================================================'
    Write-Output $out.TrimEnd()
    exit 0
}

# 扫到了、且有洞 ⇒ 红。
Write-Output $out.TrimEnd()
exit $code