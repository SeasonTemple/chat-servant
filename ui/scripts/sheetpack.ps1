<#
  把 probe-sheet.png 重新打包进 probe-sheet.svg。

  ## 为什么要有这个脚本

  `probe-sheet.svg` 是**生成物**，源是同目录下的 `probe-sheet.png`。
  两者都在库里，SVG 只是把 PNG 的 base64 内嵌进去，好让消费方能用单文件
  `url('/probe-sheet.svg')` 取到纸（见下面「为什么不能改成外链」）。

  这套「源 + 生成物都入库 + 一个打包脚本」是本仓既定的做法，
  红铅笔笔迹就是这么管的（`ui/scripts/pencilgen.py` → `ui/src/lib/marks.js`）。

  在此之前这张纸是**产物入库、来源没入库**：`paper-fixed.png` 在本机与
  git 中都不存在，生成方式也没有任何记录，于是这张纸改不动。
  现在源文件在库里，`sheetprobe.ps1` 能直接量它。

  ## 为什么**不能**让 SVG 直接外链 PNG（2026-10-12 实测）

  想过更省事的做法：把 SVG 里的 base64 换成 `href="probe-sheet.png"`，
  SVG 就缩到 439 字节。**实测不行**——SVG 一旦被当作
  `background-image` / `<img>` 载入，就处于「安全静态模式」，浏览器
  **不加载任何外部资源**，纸整个画没。

  实测数据：改外链后 `/panel` 的截图从 454,974 B 塌到 12,690 B，
  851,200 个像素里 670,001 个变了（78.7%）。同时确认 PNG 本身
  `GET /probe-sheet.png` 返回 200 / 217,945 bytes——所以不是文件没伺服，
  是策略拦的。

  ## 用法（PS7）

    # 打包（改过 PNG 之后）
    pwsh -NoProfile -File ui\scripts\sheetpack.ps1

    # 只校验是否同步，不写文件。不同步则退出码非 0 —— 给门禁用
    pwsh -NoProfile -File ui\scripts\sheetpack.ps1 -Check
#>
param([switch]$Check)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSCommandPath))
$svg  = Join-Path $root 'ui\static\probe-sheet.svg'
$png  = Join-Path $root 'ui\static\probe-sheet.png'

if (-not (Test-Path $png)) {
  throw "找不到源图 $png —— 纸的源必须入库，否则又回到「产物入库、来源没入库」"
}

$raw = [IO.File]::ReadAllBytes($png)
$b64 = [Convert]::ToBase64String($raw)

$text = [IO.File]::ReadAllText($svg)
$m = [regex]::Match($text, 'base64,([A-Za-z0-9+/=]+)"')
if (-not $m.Success) {
  throw 'probe-sheet.svg 里找不到内嵌 base64 —— 资产结构变了，脚本要改'
}

$g   = $m.Groups[1]
$cur = $g.Value

Write-Output ("源图        : {0:N0} 字节  ->  base64 {1:N0} 字符" -f $raw.Length, $b64.Length)

# ⚠⚠⚠ **2026-10-18 补的第二条不变量：`<image>` 的 width/height 必须与 PNG 对得上。**
#
# 为什么必须有这条：下面那个 `exit 0` 之前的判断只比 **base64 载荷**，
# 而 `<image x="0" y="0" width="348" height="512">` 这几个数字是**手写的**
# ——打包脚本第 75 行的注释自己写着「只换载荷，不碰任何其他标记」。
# ⇒ 载荷可以完全同步，而纸被画在一个**错的框**里。
#
# 真机上就是这样暴露的（2026-10-18 验收）：
#   · 源 PNG 是 760×1120 = 380×560 @2x，**正好是面板尺寸**
#   · 而 `<image>` 写的是 348×512 ⇒ 右边 32px、下边 48px 根本没有纸
#   · 量具实测：纸可见底在面板本地 y≈500、右缘 x≈348，往下/往右全是
#     底下微信的界面透过来，能直接读出对话列表
#   · 而 `sheet pack in sync` 那一步**是绿的**
#
# ⚠ 守的是「纸铺满面板」这个**真不变量**，不是「载荷同步」那个代理。
#   代理恰好成立而真不变量已经坏了，正是这道检查原先漏掉的那类。
Add-Type -AssemblyName System.Drawing
$img = [System.Drawing.Image]::FromFile($png)
try {
    $pxW  = $img.Width
    $pxH  = $img.Height
    $cssW = [int]($pxW / 2)
    $cssH = [int]($pxH / 2)
}
finally { $img.Dispose() }

$mm = [regex]::Match($text, '<image\b[^>]*?\bwidth="(\d+)"[^>]*?\bheight="(\d+)"')
if (-not $mm.Success) {
    throw 'probe-sheet.svg 里找不到 <image width= height= —— 资产结构变了，脚本要改'
}
$imgW = [int]$mm.Groups[1].Value
$imgH = [int]$mm.Groups[2].Value
Write-Output ("纸框        : <image> {0}x{1}   源 PNG {2}x{3} = {4}x{5} CSS px @2x" -f `
        $imgW, $imgH, $pxW, $pxH, $cssW, $cssH)
if ($imgW -ne $cssW -or $imgH -ne $cssH) {
    Write-Output '状态        : **纸框与源图对不上** —— 纸不会铺满面板，边缘会透出底下窗口'
    throw ("<image> 是 {0}x{1}，而源 PNG 是 {2}x{3}（= {4}x{5} CSS px @2x）。" -f `
            $imgW, $imgH, $pxW, $pxH, $cssW, $cssH) +
        ' 面板窗口是 380x560 ⇒ 差出来的部分完全没有纸，底下窗口会直接透出来。' +
        ' 改 <image> 的 width/height 与源图对上（别去缩放 PNG，那会把撕口一起缩小）。'
}

if ($cur -ceq $b64) {
  Write-Output '状态        : 已同步，SVG 内嵌段与 probe-sheet.png 逐字节相同'
  exit 0
}

Write-Output ("状态        : **不同步** —— SVG 里是 {0:N0} 字符的旧载荷" -f $cur.Length)

if ($Check) {
  Write-Output '            重跑：pwsh -NoProfile -File ui\scripts\sheetpack.ps1'
  exit 1
}

# 只换载荷，不碰任何其他标记 —— 纸的 viewBox、落影、撕边参数一个都不动
$new = $text.Substring(0, $g.Index) + $b64 + $text.Substring($g.Index + $g.Length)
[IO.File]::WriteAllText($svg, $new, (New-Object Text.UTF8Encoding $false))
Write-Output ("已写入      : {0}  ({1:N0} 字符)" -f $svg, $new.Length)
