param(
  [Parameter(Mandatory = $true)][string]$Url,
  [Parameter(Mandatory = $true)][string]$OutPng,
  [int]$Width = 520,
  [int]$Height = 760,
  [int]$Wait = 20000,
  [int]$Scale = 2,
  [switch]$ShowScrollbars
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

$chromeArgs = @(
  '--headless=new',
  "--user-data-dir=$profileDir",
  '--no-first-run',
  '--no-default-browser-check',
  '--disable-gpu',
  "--force-device-scale-factor=$Scale",
  "--window-size=$Width,$Height",
  "--virtual-time-budget=$Wait",
  "--screenshot=$OutPng",
  $Url
)

# ⚠ 量具自检 3：`--hide-scrollbars` 原本**写死**在这里，于是本脚本
#   结构上就**截不出任何滚动条**——而「滚动条有没有按设计渲染」
#   恰恰是要量的东西（票 42 的验收项就写着「实测确实渲染出来」）。
#   ⇒ 改成开关：默认仍隐藏（面板出图要的就是画面本身），
#     要量滚动条时显式 `-ShowScrollbars`。
#   ⚠ 漏掉这个开关时的症状极具误导性：图出得来、布局全对，
#     只是右边永远没有滚动条，于是被读成「CSS 写了但没生效」。
if (-not $ShowScrollbars) { $chromeArgs = @('--hide-scrollbars') + $chromeArgs }

& $chrome @chromeArgs 2>&1 | Out-Null
$code = $LASTEXITCODE
if (-not (Test-Path $OutPng)) { throw "no screenshot produced (exit=$code)" }
$img = Get-Item $OutPng
"$($img.FullName) | $($img.Length) bytes | exit=$code"
