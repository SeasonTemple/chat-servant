param([string]$Dir = 'E:\devSpace\chat-servant-m31f\target\shots')

# 量「正文区有多少墨」。用来证明**各状态真的各自渲染了**。
#
# ## 为什么需要它
#
# 逐张截图用眼睛比对有个具体的失效方式：八态如果全渲成同一张图，
# 「看起来都渲染了」和「全都渲染成一样」在截图上是同一件事。
# 而 PNG 的字节数也不够——纸面背景占满画面，改几个字可能一个字节都不差。
#
# 所以量一个**有区分度**的量：正文区（楷体那几行）的墨色像素占比。
# 实测同一批夹具：
#     drafted / armed / confirm  5.02%   ← 整段正文
#     failed                     1.71%   ← 一行「没写成：…」
#     noprep / silent            0.81%   ← 空态一行
#     drafting                   0.67%   ← 一行「正在写…」
#
# ⚠ **采样区不含归属行**（2× 下 y 258~310，在本脚本的 340 之下）。
#   所以 `noprep` 与 `silent` 的数字相同是**对的**：两者 phase 都是 `read`，
#   差别在归属行的 `peer`（空 vs 示例联系人），那一段的差异由出图的
#   字节数佐证（578093 / 581311），不在这张表里。
#   把采样区往上扩就能把它也量进来，但那会把「正文」和「归属」混成一个数。
#
# ## 取景框几何
#
# 演示取景框 380×560 居中于 520×760 视口（`+layout.svelte` 的 `.cs-demo-frame`），
# 出图 2× ⇒ 面板 CSS x 70..450。正文区取 CSS y 170..320 ⇒ 2× 的 340..640。
# ⚠ **换任何一处尺寸都得同步改这三个数**：Chrome 会把窗口宽度钳到 500px
#   最小值，而「截图尺寸对」不证明「布局宽度对」（`scripts/shoturl.ps1` 有同一条）。
#   自检手段：跑一次，`采样` 列必须恒等于 57000，对不上就是几何写错了。
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$Scale = 2
$X0 = 140
$X1 = 900
$Y0 = 340
$Y1 = 640

$rows = @()
foreach ($f in Get-ChildItem $Dir -Filter 'st-*.png' | Sort-Object Name) {
    $bmp = [System.Drawing.Bitmap]::FromFile($f.FullName)
    try {
        $dark = 0
        $sampled = 0
        for ($y = $Y0; $y -lt $Y1; $y += 2) {
            for ($x = $X0; $x -lt $X1; $x += 2) {
                $c = $bmp.GetPixel($x, $y)
                $sampled++
                # 墨 rgb(20,26,31) 与铅笔灰 rgb(76,90,99) 都落在这个亮度带内，
                # 而纸底是暖黄 rgb(243,230,192)、取景框外是纯白。
                if (($c.R + $c.G + $c.B) -lt 330) { $dark++ }
            }
        }
        $rows += [pscustomobject]@{
            态   = $f.BaseName -replace '^st-', ''
            采样 = $sampled
            墨点 = $dark
            占比 = [math]::Round(100 * $dark / $sampled, 2)
        }
    } finally { $bmp.Dispose() }
}
$rows | Format-Table -AutoSize

# 量具自检：采样数对不上说明取景框几何写错了，**这时上面那些数字全是假的**。
# 量具自检：采样数对不上说明取景框几何写错了，这时上面的数字不可信。
if (($rows | Where-Object { $_.采样 -ne 57000 }).Count -gt 0) {
    throw '采样数不等于 57000 —— 取景框几何（X0/X1/Y0/Y1）与实际出图尺寸不符，上面的占比不可用。'
}