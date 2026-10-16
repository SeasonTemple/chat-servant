# 一条命令重跑整套 OCR 对照评测。
#
#   pwsh -NoProfile -File scripts\ocrbench\run.ps1
#
# 为什么是 .ps1 而不是直接 `uv run`：
# uv 的默认 cache 在 %LOCALAPPDATA%\uv，而硬约束要求所有产物与权重
# 都留在项目所在盘。这里把 cache 与 Python 安装目录都指到项目下的
# target/，于是 C 盘不会因为跑一次评测而多出任何东西。
#
# 传入参数会原样转给 ocrbench.py（`a` / `b`，默认两条都跑）。
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$uv = 'G:\tools\uv\uv.exe'
if (-not (Test-Path $uv)) { throw "找不到 $uv —— 本机没有系统 Python，必须用这个 uv 可执行文件" }

$env:UV_CACHE_DIR = Join-Path $root 'target\uv-cache'
$env:UV_PYTHON_INSTALL_DIR = Join-Path $root 'target\uv-python'

# 权重在项目盘，跑之前先确认它确实在那儿。
# 拿不到就报错，不要静默回落成让 rapidocr 自己下载到别处。
foreach ($f in 'PP-OCRv6_det_small.onnx', 'PP-OCRv6_rec_small.onnx', 'ppocrv6_dict.txt') {
    $p = Join-Path $root "target\models\ppocrv6-small\$f"
    if (-not (Test-Path $p)) {
        throw "缺 $p。先跑 scripts\ocrbench\fetch_models.py 拿权重。"
    }
}

# 开发期复用同一个 venv，避免每次重解依赖。
$venvPy = Join-Path $root 'target\bench-venv\Scripts\python.exe'
if (Test-Path $venvPy) {
    & $venvPy (Join-Path $root 'scripts\ocrbench\ocrbench.py') @args
    exit $LASTEXITCODE
}

# 没有 venv 就让 uv 按脚本内联的 PEP 723 依赖现建一个。
& $uv run --python 3.11 (Join-Path $root 'scripts\ocrbench\ocrbench.py') @args
exit $LASTEXITCODE
