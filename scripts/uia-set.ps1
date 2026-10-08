# 实测：能不能**不抢前台**把文字写进微信输入框。
#
# 现状（2026-10-09 实测）：
#   · 微信顶层只有 2 个 Qt 子 HWND，没有原生 Edit → WM_PASTE 无投递目标
#   · 但 UIA 树可用，聊天输入框 = `mmui::ChatInputField`，
#     是带 ValuePattern + TextPattern 的 Edit
#
# 所以有一条官方 API 的路：ValuePattern.SetValue()。
# 它**不激活窗口**，理论上完全不碰用户的鼠标键盘。
# 这份脚本就是来证伪/证实这件事的：
#   记下前台窗口 → SetValue → 再看前台窗口有没有变 → 再截像素确认真的写进去了
#
# 用法: uia-set.ps1 <微信hwnd> <要写入的文本> [截图输出]
param(
    [Parameter(Mandatory)][string]$Hwnd,
    [Parameter(Mandatory)][string]$Text,
    [string]$Shot = ''
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class FG {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, System.Text.StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
}
"@

function FgName {
    $fg = [FG]::GetForegroundWindow()
    $sb = New-Object System.Text.StringBuilder 256
    [void][FG]::GetWindowText($fg, $sb, 256)
    $p = 0
    [void][FG]::GetWindowThreadProcessId($fg, [ref]$p)
    "0x$('{0:X}' -f $fg.ToInt64()) pid=$p '$($sb.ToString())'"
}

$h = [IntPtr][Convert]::ToInt64($Hwnd, 16)
$root = [System.Windows.Automation.AutomationElement]::FromHandle($h)

# 只认 class = mmui::ChatInputField 的那个 Edit。
# 微信树里还有「搜索」框也是 Edit，按名字找会写错地方——**按 class 认**。
$cond = New-Object System.Windows.Automation.AndCondition(
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Edit)),
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ClassNameProperty,
        'mmui::ChatInputField')))
$input = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
if (-not $input) { throw "没找到 mmui::ChatInputField——微信版本可能变了，或窗口不在前台" }

$vp = $null
if (-not $input.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$vp)) {
    throw "ChatInputField 上没有 ValuePattern，这条路走不通"
}

Write-Host "写入前，前台窗口 = $(FgName)"
Write-Host "输入框当前内容 = '$($vp.Current.Value)'"

$vp.SetValue($Text)

Start-Sleep -Milliseconds 600
Write-Host "写入后，前台窗口 = $(FgName)"
$now = $null
if ($input.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$now)) {
    Write-Host "输入框现在内容 = '$($now.Current.Value)'"
}

if ($Shot) {
    Add-Type -AssemblyName System.Drawing
    $r = $input.Current.BoundingRectangle
    $x = [int]$r.X; $y = [int]$r.Y; $w = [int]$r.Width; $h2 = [int]$r.Height
    $bmp = New-Object System.Drawing.Bitmap($w, $h2)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($x, $y, 0, 0, (New-Object System.Drawing.Size($w, $h2)))
    $g.Dispose()
    $bmp.Save((Join-Path (Get-Location) $Shot), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host "已截输入框像素：$Shot"
}