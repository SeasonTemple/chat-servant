# 打印窗口的原始扩展样式并逐位解读。
# 用途：面板看得见但收不到鼠标时，嫌疑最大的是
#       **WS_EX_LAYERED(0x80000) + 逐像素 alpha 命中测试**——
#       分层窗口上 alpha=0 的像素不吃鼠标事件，
#       表现就是「画面在那儿，点哪儿都没反应」。
#       这件事只能看原始样式位，靠猜必错。
# 用法: exstyle.ps1 <hwnd>
param([Parameter(Mandatory)][string]$Hwnd)

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class EX {
    [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtr(IntPtr h, int i);
    [DllImport("user32.dll")] public static extern bool GetLayeredWindowAttributes(IntPtr h, out uint c, out byte a, out uint f);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@

$h = [IntPtr][Convert]::ToInt64($Hwnd, 16)
$ex = [EX]::GetWindowLongPtr($h, -20).ToInt64()
$st = [EX]::GetWindowLongPtr($h, -16).ToInt64()
"窗口 $Hwnd"
"扩展样式 = 0x{0:X16}" -f $ex
"基本样式 = 0x{0:X16}" -f $st

$bits = @(
    @{ n = 'WS_EX_TRANSPARENT    '; v = 0x00000020; d = '整窗点击穿透' },
    @{ n = 'WS_EX_LAYERED        '; v = 0x00080000; d = '分层窗口——鼠标按逐像素 alpha 命中测试' },
    @{ n = 'WS_EX_NOACTIVATE     '; v = 0x08000000; d = '点击不激活（focus:false）' },
    @{ n = 'WS_EX_TOOLWINDOW     '; v = 0x00000080; d = '不在任务栏/Alt-Tab' },
    @{ n = 'WS_EX_TOPMOST        '; v = 0x00000008; d = '置顶' },
    @{ n = 'WS_EX_APPWINDOW      '; v = 0x00040000; d = '独立应用窗口' },
    @{ n = 'WS_EX_NOREDIRECTION  '; v = 0x00200000; d = '跳过 DWM 重定向' },
    @{ n = 'WS_EX_NOREDIRECTIONB '; v = 0x00400000; d = '无重定向位图' }
)
foreach ($b in $bits) {
    if (($ex -band $b.v) -ne 0) { "  [有] $($b.n)  $($b.d)" }
}

$col = 0; $alpha = 0; $flag = 0
if ([EX]::GetLayeredWindowAttributes($h, [ref]$col, [ref]$alpha, [ref]$flag)) {
    "分层属性：颜色键=0x{0:X8} 整体 alpha={1} 标志={2}" -f $col, $alpha, $flag
    if ($flag -band 0x02) { "  LWA_COLORKEY —— 全透明像素处的点击会穿透到下层" }
} else {
    "读不到分层属性（窗口不是分层窗口，或该窗口不支持）"
}