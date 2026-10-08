# 探测「不抢前台也能把文字送进微信输入框」的两条现实路径：
#   A. 往子窗口 HWND 发 WM_PASTE / WM_CHAR（PostMessage，不激活窗口）
#   B. UI Automation 的 ValuePattern.SetValue（官方 API，不改前台）
#
# 用途：现在唯一的投递路径是 SetForegroundWindow + SendInput，
# **必然把用户的鼠标键盘抢走**。要回答「能不能不干扰人」，
# 光看文档不算数，得在真机上知道这两条路到底通不通。
# 用法: probe-input.ps1 <微信窗口 hwnd>
param([Parameter(Mandatory)][string]$Hwnd)

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public class PI {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr p, EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    public static List<IntPtr> Children(IntPtr parent) {
        var l = new List<IntPtr>();
        EnumChildWindows(parent, (h, x) => { l.Add(h); return true; }, IntPtr.Zero);
        return l;
    }
}
"@

$h = [IntPtr][Convert]::ToInt64($Hwnd, 16)

Write-Host "=== A. 子窗口枚举（PostMessage 目标必须是真 HWND）===" -ForegroundColor Cyan
$kids = [PI]::Children($h)
Write-Host "微信顶层窗口下共有 $($kids.Count) 个子 HWND"
$editLike = @()
foreach ($k in $kids) {
    $c = New-Object System.Text.StringBuilder 256
    [void][PI]::GetClassName($k, $c, 256)
    $t = New-Object System.Text.StringBuilder 256
    [void][PI]::GetWindowText($k, $t, 256)
    $r = New-Object PI+RECT
    [void][PI]::GetWindowRect($k, [ref]$r)
    $cls = $c.ToString()
    $vis = [PI]::IsWindowVisible($k)
    # 只打印可能承载输入的类名，其余按类名归并计数
    if ($cls -match 'Edit|RichEdit|Text|WebView|Chrome|Cef|Qt') {
        $editLike += $k
        Write-Host ("  0x{0:X} vis={1,-5} {2}x{3} class={4,-22} text='{5}'" -f `
            $k.ToInt64(), $vis, ($r.R - $r.L), ($r.B - $r.T), $cls, $t.ToString())
    }
}
if ($editLike.Count -eq 0) {
    Write-Host "  ⚠ 没有任何 Edit/RichEdit 类子窗口。Qt 应用通常只创建一个顶层 HWND，" -ForegroundColor Yellow
    Write-Host "    所有控件由 Qt 自己绘制——那样 WM_PASTE 就**没有投递目标**。" -ForegroundColor Yellow
} else {
    Write-Host "  → 存在 $($editLike.Count) 个可能的文本控件，PostMessage 有投递目标"
}

Write-Host ""
Write-Host "=== B. UI Automation 树（看有没有可用的 Edit/Text 模式）===" -ForegroundColor Cyan
try {
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($h)
    Write-Host "  根元素 = '$($root.Current.Name)' class='$($root.Current.ClassName)'"
    $cond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Edit)
    $edits = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)
    Write-Host "  找到 Edit 控件 $($edits.Count) 个"
    for ($i = 0; $i -lt [Math]::Min($edits.Count, 6); $i++) {
        $e = $edits.Item($i)
        $vp = $null
        if ($e.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$vp)) {
            Write-Host "   [$i] '$($e.Current.Name)'  ValuePattern **可用** = $($vp.Current.Value)"
        } else {
            Write-Host "   [$i] '$($e.Current.Name)'  ValuePattern 不可用"
        }
    }
    if ($edits.Count -eq 0) { Write-Host "  ⚠ 没有 Edit 控件 → UIA 设值这条路也走不通" -ForegroundColor Yellow }
} catch {
    Write-Host "  UIA 探测失败：$($_.Exception.Message)" -ForegroundColor Yellow
}