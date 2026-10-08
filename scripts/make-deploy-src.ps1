$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$out = Join-Path $root 'target\deploy-src'

if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force -Path $out | Out-Null

# 只带**源码与文档**。不带 target/（35GB 编译产物）与 node_modules/
# （依赖能由 Cargo.lock / package.json 复原），部署服务对源码包有 200MB 上限。
$include = @(
    'crates', 'ui/src', 'ui/src-tauri/src', 'ui/src-tauri/capabilities',
    'ui/src-tauri/icons', 'ui/src-tauri/tauri.conf.json', 'ui/src-tauri/Cargo.toml',
    'ui/src-tauri/build.rs', 'ui/static', 'ui/package.json', 'ui/svelte.config.js',
    'ui/vite.config.ts', 'ui/tsconfig.json', 'ui/pnpm-workspace.yaml',
    'docs', 'scripts', 'xtask/src', 'xtask/Cargo.toml',
    'Cargo.toml', 'Cargo.lock', 'deny.toml', '.scratch', 'AGENTS.md', 'CONTEXT.md', 'README.md', '.gitignore'
)
foreach ($rel in $include) {
    $src = Join-Path $root $rel
    if (-not (Test-Path $src)) { continue }
    $dst = Join-Path $out $rel
    New-Item -ItemType Directory -Force -Path (Split-Path $dst) | Out-Null
    Copy-Item -Recurse -Force $src $dst
}

# 图标只留 Windows 桌面真正用得到的几个。
#
# `ui/src-tauri/icons/` 是 Tauri 默认生成的**全平台**套件：
# Android mipmap（5 档密度）+ iOS AppIcon + Windows Store 方块 + macOS icns。
# 本项目 `bundle.targets` 只有 `msi` / `nsis`，`tauri.conf.json` 里也只引用了
# `icons/icon.ico` —— 那些平台变体一份都用不上，却占了源码副本里最大的头。
#
# ⚠ **只裁这份副本，不动仓库里的图标。** 仓库保留全套是对的：
# 哪天要出安卓/iOS 包，`tauri icon` 的输入还在，不用重新生成。
$iconDir = Join-Path $out 'ui\src-tauri\icons'
if (Test-Path $iconDir) {
    Get-ChildItem $iconDir -Directory | Remove-Item -Recurse -Force
    Get-ChildItem $iconDir -File |
        Where-Object { $_.Name -notmatch '^(icon\.ico|icon\.png|32x32\.png|128x128\.png|128x128@2x\.png)$' } |
        Remove-Item -Force
}

$size = (Get-ChildItem $out -Recurse -File | Measure-Object -Property Length -Sum).Sum
"源码副本：$out"
"大小：$([math]::Round($size/1KB,1)) KB（上限 200MB）"
"文件数：$((Get-ChildItem $out -Recurse -File).Count)"