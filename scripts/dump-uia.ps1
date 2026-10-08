# 把微信窗口的 UI Automation 树打出来，用来回答：
#   「不抢前台，能不能把文字送进聊天输入框？」
#
# 已知事实（2026-10-09 实测）：
#   · 微信顶层窗口只有 2 个子 HWND，全是 Qt 的，没有原生 Edit
#     → WM_PASTE 投给子窗口这条路**没有投递目标**
#   · 但 UIA 树是通的，根节点 class = `mmui::MainWindow`
#     → 官方 ValuePattern / TextPattern 可能是出路
# 这份脚本把整棵树连同控件类型、名字、矩形、可用模式列出来。
#
# 用法: dump-uia.ps1 <hwnd> [最大深度]
param(
    [Parameter(Mandatory)][string]$Hwnd,
    [int]$MaxDepth = 14
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$h = [IntPtr][Convert]::ToInt64($Hwnd, 16)
$root = [System.Windows.Automation.AutomationElement]::FromHandle($h)
if (-not $root) { throw "拿不到 UIA 根元素" }

$walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker

function Show($el, $depth, $path) {
    if ($depth -gt $MaxDepth) { return }
    try {
        $c = $el.Current
        $r = $c.BoundingRectangle
        $flags = @()
        foreach ($p in @(
            @{ n = 'Value'; t = [System.Windows.Automation.ValuePattern]::Pattern },
            @{ n = 'Text'; t = [System.Windows.Automation.TextPattern]::Pattern },
            @{ n = 'Legacy'; t = [System.Windows.Automation.LegacyIAccessiblePattern]::Pattern },
            @{ n = 'Invoke'; t = [System.Windows.Automation.InvokePattern]::Pattern }
        )) {
            $o = $null
            if ($el.TryGetCurrentPattern($p.t, [ref]$o)) { $flags += $p.n }
        }
        $name = $c.Name
        if ($name.Length -gt 28) { $name = $name.Substring(0, 28) + '…' }
        Write-Host ("{0}{1,-14} '{2}' class={3,-22} rect=({4},{5},{6},{7}) [{8}]" -f `
            ('  ' * $depth), $c.ControlType.ProgrammaticName -replace 'ControlType\.', '',
            $name, $c.ClassName,
            [int]$r.X, [int]$r.Y, [int]$r.Width, [int]$r.Height,
            ($flags -join ','))
    } catch {
        Write-Host ("{0}<读不到属性>" -f ('  ' * $depth))
        return
    }
    $child = $walker.GetFirstChild($el)
    while ($child) {
        Show $child ($depth + 1) "$path/$($child.Current.AutomationId)"
        $child = $walker.GetNextSibling($child)
    }
}

Write-Host "根：'$($root.Current.Name)' class='$($root.Current.ClassName)'" -ForegroundColor Cyan
Show $root 0 ''