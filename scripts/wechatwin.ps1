$ErrorActionPreference = 'Stop'
# 扫**全部**顶层窗口（不过滤可见），按进程名找微信。
# winscan.ps1 只报可见窗口，而托盘里的微信窗口是最小化的 ⇒ 会漏掉。
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  public delegate bool EnumProc(IntPtr h, IntPtr p);
  public struct RECT { public int Left, Top, Right, Bottom; }
}
'@

$procName = @{}
Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -match 'WeChat|Weixin' } | ForEach-Object {
  $procName[[uint32]$_.Id] = $_.ProcessName
}

$rows = New-Object System.Collections.ArrayList
$cb = [W+EnumProc]{
  param($h, $p)
  # 不能叫 `$pid`——PowerShell 的 `$PID` 是只读内置变量（当前进程 ID），
  #   在脚本块里遮蔽它会直接抛「无法覆盖变量 PID」。
  [uint32]$procId = 0
  [void][W]::GetWindowThreadProcessId($h, [ref]$procId)
  if ($procName.ContainsKey($procId)) {
    $sb = New-Object System.Text.StringBuilder 512
    [void][W]::GetWindowText($h, $sb, 512)
    $cn = New-Object System.Text.StringBuilder 256
    [void][W]::GetClassName($h, $cn, 256)
    $r = New-Object W+RECT
    [void][W]::GetWindowRect($h, [ref]$r)
    [void]$rows.Add([pscustomobject]@{
      Hwnd    = ('0x{0:X}' -f $h.ToInt64())
      Proc    = $procName[$procId]
      Pid     = $procId
      Class   = $cn.ToString()
      Title   = $sb.ToString()
      Visible = [W]::IsWindowVisible($h)
      Iconic  = [W]::IsIconic($h)
      Size    = "$($r.Right - $r.Left)x$($r.Bottom - $r.Top)"
      Rect    = "$($r.Left),$($r.Top)"
    })
  }
  return $true
}
[void][W]::EnumWindows($cb, [IntPtr]::Zero)

if ($rows.Count -eq 0) {
  Write-Host 'ABORT: 微信进程在跑，但一个顶层窗口都没有 —— 全部被销毁或在别的 session。'
} else {
  $rows | Sort-Object Visible, Title | Format-Table -AutoSize
  Write-Host "共 $($rows.Count) 个微信顶层窗口"
}