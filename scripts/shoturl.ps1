param(
  [Parameter(Mandatory = $true)][string]$Url,
  [Parameter(Mandatory = $true)][string]$OutPng,
  [int]$Width = 520,
  [int]$Height = 760,
  [int]$Wait = 20000,
  [int]$Scale = 2
)

# ⚠ 量具自检 1：Chrome 会把窗口宽度**钳到最小 500px**，而 --screenshot 输出的
#   宽度仍等于 --window-size。所以「截图尺寸对」**不能**证明「布局宽度对」。
#   本面板内部被 cs-demo-frame 钉死 380px，与视口无关，故此处只当出图用，
#   任何尺寸结论都必须另找独立量法。
#
# ⚠⚠ 量具自检 2：**`--virtual-time-budget` 会截到补间的中间帧**（票 37 定位）。
#   本项目用 GSAP 做「滑过去」这类动画，而虚拟时间**不保证**在 rAF 补间
#   跑完之前就到期。实测：同一页面的分段控件指示块，
#     budget 4000 / 15000 / 40000 → 量到的宽 44 / 44 / 54 px（目标 61px）
#   ⇒ **预算越大越接近，但不是"够大就行"，没有保证**。
#   症状极具误导性：截出来的是一个**合法的、但只长了一半**的元素，
#   肉眼看像布局错位，于是被当成「窄视口下排版坏了」去查布局——
#   而布局根本没问题（真实 Chromium 里量 DOM，两边逐像素相等）。
#
#   ⇒ 默认预算已从 4000 提到 **20000**。**仍然不要**拿本脚本的像素
#   下几何结论：要么用真实浏览器量 DOM，要么先确认目标元素没有在补间。
$ErrorActionPreference = 'Stop'
$chrome = 'C:\Program Files\Google\Chrome\Application\chrome.exe'
if (-not (Test-Path $chrome)) { throw "chrome not found" }

$profileDir = Join-Path $env:TEMP ("shot-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $profileDir | Out-Null

$args = @(
  '--headless=new',
  "--user-data-dir=$profileDir",
  '--no-first-run',
  '--no-default-browser-check',
  '--disable-gpu',
  '--hide-scrollbars',
  "--force-device-scale-factor=$Scale",
  "--window-size=$Width,$Height",
  "--virtual-time-budget=$Wait",
  "--screenshot=$OutPng",
  $Url
)

& $chrome @args 2>&1 | Out-Null
$code = $LASTEXITCODE
if (-not (Test-Path $OutPng)) { throw "no screenshot produced (exit=$code)" }
$img = Get-Item $OutPng
"$($img.FullName) | $($img.Length) bytes | exit=$code"
