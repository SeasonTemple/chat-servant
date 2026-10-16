$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$out = Join-Path $root 'target\deploy-src'

if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force -Path $out | Out-Null

# **按排除来，不用白名单。**
#
# 白名单漏一个目录，漏掉的东西就永远传不上去了——而「把所有源码传上去」
# 本身就是要防这种漏。第一次做的是白名单，结果漏了 `.git`（76 个提交的历史）
# 和 `examples/`（净室参考项目），用户直接问「所有源码都上传了吗」。
# 排除法才是这个场景该用的形式。
# 只给**目录名**，不给绝对路径。robocopy 的 /XD 拿绝对路径时**只匹配顶层**，
# 于是 `ui\node_modules`、`ui\.svelte-kit`、`ui\build` 全都漏了过去——
# 第一次跑出来 840MB / 57014 个文件，其中绝大部分是依赖。
$excludeDirs = @(
    'target',        # 编译产物，可再生；仓库里曾经有 33GB
    'node_modules',  # 依赖由 package.json / Cargo.lock 复原
    '.svelte-kit',   # SvelteKit 构建中间产物
    'build',         # ui build 产物，由 pnpm build 重新生成
    '.pnpm-store',
    '__pycache__',
    '.venv'
)

$robocopyArgs = @($root, $out, '/E', '/R:1', '/W:1', '/NFL', '/NDL', '/NJH', '/NJS', '/NP', '/XD')
$robocopyArgs += $excludeDirs
$null = & robocopy @robocopyArgs

# 嵌套仓库的 .git 与二进制包：`examples/jev-chat-jarvis` 是**别人的**项目
# （AGENTS.md 明确说只读它的实测结论、不要移植代码）。它自带 69.6MB 的
# .git 历史和一个 24.6MB 的 APK——那是第三方的历史和二进制，
# 跟交接本项目的工作没关系，带上去只是把上传撑大。
$null = & robocopy $out $out '*.apk' /S /MOV
foreach ($g in Get-ChildItem (Join-Path $out 'examples') -Recurse -Directory -Force -Filter '.git' -ErrorAction SilentlyContinue) {
    Remove-Item -Recurse -Force $g.FullName
}

# robocopy 的退出码：0~7 都算成功（1=有文件被复制，3=有额外文件…）。
# >=8 才是真失败。
if ($LASTEXITCODE -ge 8) { throw "robocopy 失败，退出码 $LASTEXITCODE" }

$size = (Get-ChildItem $out -Recurse -File -Force | Measure-Object Length -Sum).Sum
$files = (Get-ChildItem $out -Recurse -File -Force).Count
"源码副本：$out"
"大小：$([math]::Round($size/1KB,1)) KB（部署上限 200 MB）"
"文件数：$files"
"含 .git = $(Test-Path (Join-Path $out '.git'))"
"含 examples = $(Test-Path (Join-Path $out 'examples'))"