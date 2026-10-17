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

# ⚠⚠⚠ **2026-10-18 补的第二条不变量：纸必须画在「布局算法假定的那个框」里。**
#
# ⚠⚠⚠ **先记一次我自己犯过的错，因为它是这张票的由来**：
#   第一次补这条时，我断言的是「`<image>` 的尺寸必须 = 源 PNG ÷ 2」，
#   理由是「纸从来没铺满面板」。**那个前提是错的。**
#   `probe-sheet.svg` 从 `70fd0be` 引入起就是 **画布 380×560 + 纸 348×512
#   + `translate(16,0)`**，而 `PaperPanel.svelte` 里 `padding-bottom: 40`、
#   勾尾 ≤ 506、纸沿最高点 498.5 **全部按「纸底 = 512」推出来的**
#   （见该文件「参照物一直是错的 —— 『纸底』被当成了『元素底 560』」那段）。
#   底部那 48px 是**留给 `sheetDrop` 落影的透明带**，两侧 16px 是撕口留的余量。
#   我按「铺满」去改 `<image>`，结果纸变成 x=16..396：左边空 16px、
#   右边溢出 16px，**把一个设计常量改坏了**，而且门禁还绿着。
#
#   ⇒ 教训：**「看起来漏了」不等于漏了**。先去找设计依据（票 44 的那篇就在
#   `PaperPanel.svelte` 里），再决定动不动手。
#
# 所以这条不变量守的是**设计常量本身**：画布 380×560、纸 348×512、横移 16。
# 这三个数一改，`PaperPanel.svelte` 里那一整套 padding/落笔位置的推导
# 全部作废，而**任何门禁都不会报**——载荷照样同步，界面只是悄悄错位。
Add-Type -AssemblyName System.Drawing
$img = [System.Drawing.Image]::FromFile($png)
try { $pxW = $img.Width; $pxH = $img.Height }
finally { $img.Dispose() }

$EXPECT_CANVAS_W = 380
$EXPECT_CANVAS_H = 560
$EXPECT_PAPER_W  = 348
$EXPECT_PAPER_H  = 512
$EXPECT_SHIFT_X  = 16

$cm = [regex]::Match($text, '<svg\b[^>]*?\bwidth="(\d+)"[^>]*?\bheight="(\d+)"')
$mm = [regex]::Match($text, '<image\b[^>]*?\bwidth="(\d+)"[^>]*?\bheight="(\d+)"')
$gm = [regex]::Match($text, 'transform="translate\(\s*(\d+)\s*,\s*(\d+)\s*\)"')
if (-not ($cm.Success -and $mm.Success -and $gm.Success)) {
    throw 'probe-sheet.svg 的结构变了（<svg>/<image>/translate 有一个匹配不上）—— 脚本要跟着改'
}
$cw = [int]$cm.Groups[1].Value;  $ch = [int]$cm.Groups[2].Value
$pw = [int]$mm.Groups[1].Value;  $ph = [int]$mm.Groups[2].Value
$sx = [int]$gm.Groups[1].Value

Write-Output ("画布/纸框   : <svg> {0}x{1}   <image> {2}x{3} @({4},0)   源 PNG {5}x{6}" -f `
        $cw, $ch, $pw, $ph, $sx, $pxW, $pxH)
Write-Output ("纸实际覆盖   : x {0}..{1}   y 0..{2}   （底部 {3}px 是 sheetDrop 的透明落影带）" -f `
        $sx, ($sx + $pw - 1), $ph, ($ch - $ph))

$bad = @()
if ($cw -ne $EXPECT_CANVAS_W -or $ch -ne $EXPECT_CANVAS_H) { $bad += "<svg> 画布是 ${cw}x${ch}，设计值 ${EXPECT_CANVAS_W}x${EXPECT_CANVAS_H}" }
if ($pw -ne $EXPECT_PAPER_W  -or $ph -ne $EXPECT_PAPER_H)  { $bad += "<image> 纸是 ${pw}x${ph}，设计值 ${EXPECT_PAPER_W}x${EXPECT_PAPER_H}" }
if ($sx -ne $EXPECT_SHIFT_X) { $bad += "横移是 $sx，设计值 $EXPECT_SHIFT_X" }
if ($bad.Count -gt 0) {
    Write-Output '状态        : **纸不在设计框里**'
    foreach ($b in $bad) { Write-Output "  · $b" }
    throw ("纸框偏离设计常量。PaperPanel.svelte 里 padding-bottom: 40、勾尾<=506、" +
        '纸沿最高点 498.5 全是按「纸底=512、横移 16」推的；改了它那些数全部作废，' +
        '而载荷同步那道检查照样绿。' + ' 纸底那 ' + ($EXPECT_CANVAS_H - $EXPECT_PAPER_H) + 'px 是留给 sheetDrop 落影的透明带，' +
        '**不是漏画**——别为了让纸铺满而拉高它。')
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
