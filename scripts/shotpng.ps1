<#
  截**渲染后真实像素**的 PNG。用来做像素级测量与 NEAREST 放大。

  为什么不用内置浏览器截图：那条路出的是 **JPEG**（有损），放大之后
  每一条边的阶梯都是压缩伪影，量出来的密度斜坡不可信。而「这版笔迹到
  底有没有渐变」恰恰只能靠像素数。

  ## ⚠ `--headless=new` 会把窗口宽度**钳到最小 500px**

  实测 2026-10-12：`--window-size=380,560` 时，`innerWidth` 实测是
  **500**，而 `--screenshot` 出来的图**只有 380 宽**——右边 120px 直接
  没了。也就是拿到的其实是「500px 布局被裁成 380px」的图。

  这个坑很贵：据此我以为面板第三档溢出了、以为 `background` 简写被
  Svelte 丢掉，还把这个**假结论写进了代码注释**。两个都不是 bug。

  两个替代方案都试过：
    - `--force-device-scale-factor=2`：**❌** 只放大**输出像素**，CSS
      视口仍是窗口宽，拿到的还是同一个宽的布局。
    - `--headless=old`：**❌ 也一样钳**。它输出 380×560 看着像成功了，
      其实那 120px 是**裁掉的**，布局仍是 500 宽。

  ⚠ 输出尺寸**证明不了**布局宽度——两种模式下输出都等于 `--window-size`。
    判据只能是下面那道 `LayoutW ≥ 500` 的硬拦：面板是 380 宽，**这个
    脚本量不了它**，要量就上真机窗口 `scripts/shotpanel.ps1`。

    这个坑的实际代价：我据此以为面板第三档溢出了、以为 `background`
    简写被 Svelte 丢掉，还把这个**假结论写进了代码注释**。两个都不是
    bug——真 bug 只有一条（CSS 的 `fill-opacity` 盖掉了每层的属性值）。

  用法: shotpng.ps1 <url> <out.png> [-LayoutW 500] [-LayoutH 560]
#>
param(
  [Parameter(Mandatory)][string]$Url,
  [Parameter(Mandatory)][string]$Out,
  [int]$LayoutW = 500,
  [int]$LayoutH = 560
)

$ErrorActionPreference = 'Stop'

#  ⚠ 实测下限，不是猜的。new 和 old 两种 headless **都**钳到这里。
#    低于它的布局宽度会被静默放大，然后从右边裁掉——输出尺寸看不出来。
if ($LayoutW -lt 500) {
  throw "LayoutW=$LayoutW 小于 Chrome 的实测下限 500：布局会被放大到 500 再从右边裁掉，`--screenshot` 看不出来。"
  throw '要量比 500 更窄的窗口（例如 380 宽的面板），用真机窗口截图：scripts/shotpanel.ps1。'
}

$exe = "$env:ProgramFiles\Google\Chrome\Application\chrome.exe"
if (-not (Test-Path $exe)) {
  $exe = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
}
if (-not (Test-Path $exe)) { throw 'no chromium binary found' }

$prof = Join-Path $env:TEMP ('shotpng-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
$outDir = Split-Path -Parent $Out
if ($outDir -and -not (Test-Path $outDir)) {
  New-Item -ItemType Directory -Path $outDir -Force | Out-Null
}

$args = @(
  '--headless=old'
  '--disable-gpu'
  '--hide-scrollbars'
  '--no-first-run'
  '--no-default-browser-check'
  "--user-data-dir=$prof"
  "--window-size=$LayoutW,$LayoutH"
  "--screenshot=$Out"
  '--virtual-time-budget=4000'
  $Url
)

& $exe @args | Out-Null
$code = $LASTEXITCODE

if (Test-Path $prof) { Remove-Item -Recurse -Force $prof -ErrorAction SilentlyContinue }

if (-not (Test-Path $Out)) {
  throw "chrome exit=$code，没出图。"
}

Add-Type -AssemblyName System.Drawing
$bmp = [System.Drawing.Image]::FromFile($Out)
$w = $bmp.Width
$h = $bmp.Height
$bmp.Dispose()

if ($w -ne $LayoutW -or $h -ne $LayoutH) {
  throw "输出尺寸 ${w}x${h} 与要求 ${LayoutW}x${LayoutH} 不符——命令没按预期跑，别拿这张图量东西。"
}

"$Out  ${w}x${h}  = 布局 CSS 像素，1:1  exit=$code"