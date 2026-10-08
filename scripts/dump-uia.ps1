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

for ($i = 0; $i -lt $all.Count; $i++) {
    $e = $all.Item($i)
    $c = $null; $r = $null
    try { $c = $e.Current; $r = $c.BoundingRectangle } catch { continue }
    if ($X1 -gt $X0) {
        if ($r.X + $r.Width -lt $X0 -or $r.X -gt $X1) { continue }
        if ($r.Y + $r.Height -lt $Y0 -or $r.Y -gt $Y1) { continue }
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
        [int]$r.X, [int]$r.Y, [int]$r.Width, [int]$r.Height,
        ($flags -join ','))
}