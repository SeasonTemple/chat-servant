# 列出微信窗口 UIA 树里的**全部**后代元素（控件类型 / 名字 / 类名 / 矩形 / 可用模式）。
#
# 用途：回答「不抢前台能不能把文字送进聊天输入框」。
# 已知：微信顶层只有 2 个 Qt 子 HWND，没有原生 Edit → WM_PASTE 无投递目标；
#       但 UIA 根是 `mmui::MainWindow`，FindAll 能找到 Edit 且 ValuePattern 可用。
# 问题是**聊天输入框**到底是哪个元素、以及它支持什么模式——只能把树打出来看。
#
# 用法: dump-uia.ps1 <hwnd> [只打印矩形在某区域内的元素]
param(
    [Parameter(Mandatory)][string]$Hwnd,
    [int]$X0 = 0, [int]$Y0 = 0, [int]$X1 = 0, [int]$Y1 = 0
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$h = [IntPtr][Convert]::ToInt64($Hwnd, 16)
$root = [System.Windows.Automation.AutomationElement]::FromHandle($h)
if (-not $root) { throw "拿不到 UIA 根元素" }
$rr = $root.Current.BoundingRectangle
Write-Host ("根：'{0}' class='{1}' rect=({2},{3},{4},{5})" -f `
    $root.Current.Name, $root.Current.ClassName, [int]$rr.X, [int]$rr.Y, [int]$rr.Width, [int]$rr.Height)

$all = $root.FindAll(
    [System.Windows.Automation.TreeScope]::Descendants,
    [System.Windows.Automation.Condition]::TrueCondition)
Write-Host "后代元素共 $($all.Count) 个"

$patterns = @(
    @{ n = 'Value'; t = [System.Windows.Automation.ValuePattern]::Pattern },
    @{ n = 'Text'; t = [System.Windows.Automation.TextPattern]::Pattern },
    @{ n = 'Invoke'; t = [System.Windows.Automation.InvokePattern]::Pattern },
    @{ n = 'Selection'; t = [System.Windows.Automation.SelectionPattern]::Pattern }
)

# UIA 的 `BoundingRectangle` 是 `double`，而**未渲染/空元素的宽高是 ±∞**
# （不是 0）。原实现直接 `[int]$r.Width` ⇒ 抛
# 「无法将值 "∞" 转换为类型 System.Int32」，整个脚本在**第一个**这种元素上死掉。
#
# ⚠⚠ 这里不能简单地把 ∞ 当 0：区域过滤（第 41–44 行）会因此把
#   「位置未知」的元素判成「在 0,0 附近」，于是**过滤器把量具坏了这件事藏起来**。
#   ⇒ 非有限值一律标成 `?` 并**不过滤**，让它显形。
function Get-SafeNum([double]$v) {
    if ([double]::IsNaN($v) -or [double]::IsInfinity($v)) { return '?' }
    if ($v -gt [int]::MaxValue -or $v -lt [int]::MinValue) { return '?' }
    return [int][math]::Round($v)
}

for ($i = 0; $i -lt $all.Count; $i++) {
    $e = $all.Item($i)
    $c = $null; $r = $null
    try { $c = $e.Current; $r = $c.BoundingRectangle } catch { continue }
    $rx = Get-SafeNum $r.X
    $ry = Get-SafeNum $r.Y
    $rw = Get-SafeNum $r.Width
    $rh = Get-SafeNum $r.Height
    # 位置未知的元素**不做区域过滤**（见上面的理由）。
    if ($X1 -gt $X0 -and $rx -ne '?' -and $ry -ne '?' -and $rw -ne '?' -and $rh -ne '?') {
        if ($rx + $rw -lt $X0 -or $rx -gt $X1) { continue }
        if ($ry + $rh -lt $Y0 -or $ry -gt $Y1) { continue }
    }
    $flags = @()
    foreach ($p in $patterns) {
        $o = $null
        try { if ($e.TryGetCurrentPattern($p.t, [ref]$o)) { $flags += $p.n } } catch { }
    }
    $name = $c.Name
    if ($name.Length -gt 30) { $name = $name.Substring(0, 30) + '…' }
    Write-Host ("[{0,4}] {1,-12} '{2}' class={3,-20} rect=({4},{5},{6}x{7}) [{8}]" -f `
        $i, ($c.ControlType.ProgrammaticName -replace 'ControlType\.', ''),
        $name, $c.ClassName,
        $rx, $ry, $rw, $rh,
        ($flags -join ','))
}