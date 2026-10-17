param(
  [string]$ProcName = 'chat-servant-app',
  [string]$Out = '',
  [int]$Pad = 8
)

# 默认输出路径必须**从仓库位置算出来**，不能写死。
# 曾经写死 `E:\workspace\chat-servant\target\probe\v11_full.png`——那是本仓
# 换过位置之前的路径。E: 盘在，所以 New-Item -Force 会把那个目录凭空建出来，
# 脚本照样打印 "saved"，而截图躺在仓库外面没人看得见。
if (-not $Out) {
  # 注意只跳**一层**：`Split-Path` 自己就会去掉叶子（`-Leaf`），
  # 再套一层 -Parent 会退到仓库的上一级去，路径反而跑到仓库外面。
  $repo = Split-Path $PSScriptRoot -Parent
  $Out = Join-Path $repo 'target\probe\panel.png'
}

# Find the panel window by owning process, force it visible, bring it forward,
# then capture the screen region around it. fgwin.ps1 targets by title, which
# no longer matches: this build's window title is empty while parked in tray.

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -Namespace Cap -Name P -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
[DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
[DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
[DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
[DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr h);
[DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, System.Text.StringBuilder s, int n);
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleDC(IntPtr hdc);
[DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleBitmap(IntPtr hdc, int w, int h);
[DllImport("gdi32.dll")] public static extern IntPtr SelectObject(IntPtr hdc, IntPtr o);
[DllImport("gdi32.dll")] public static extern bool BitBlt(IntPtr d,int x,int y,int w,int h,IntPtr s,int sx,int sy,int rop);
[DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr o);
[DllImport("gdi32.dll")] public static extern bool DeleteDC(IntPtr dc);
[DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
[DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h, IntPtr dc);
public delegate bool EnumProc(IntPtr h, IntPtr p);
public struct RECT { public int L, T, R, B; }
'@

[void][Cap.P]::SetProcessDPIAware()

$proc = Get-Process -Name $ProcName -ErrorAction Stop | Select-Object -First 1
$targetPid = [uint32]$proc.Id
Write-Host "pid $targetPid"

$found = New-Object System.Collections.ArrayList
$cb = [Cap.P+EnumProc]{
  param($h, $p)
  $wpid = 0
  [void][Cap.P]::GetWindowThreadProcessId($h, [ref]$wpid)
  if ($wpid -eq $targetPid) {
    $len = [Cap.P]::GetWindowTextLength($h)
    $sb = New-Object System.Text.StringBuilder ($len + 2)
    [void][Cap.P]::GetWindowText($h, $sb, $sb.Capacity)
    [void]$found.Add([pscustomobject]@{
      H = $h; Vis = [Cap.P]::IsWindowVisible($h)
      Title = $sb.ToString(); Len = $len
    })
  }
  return $true
}
[void][Cap.P]::EnumWindows($cb, [IntPtr]::Zero)

if ($found.Count -eq 0) { Write-Host "no top-level window for pid"; exit 2 }
foreach ($w in $found) {
  Write-Host ("  hwnd=0x{0:X} visible={1} len={2} title='{3}'" -f `
    [int64]$w.H, $w.Vis, $w.Len, $w.Title)
}

$win = $found | Where-Object { $_.Len -gt 0 } | Select-Object -First 1
if (-not $win) { $win = $found | Select-Object -First 1 }
$hwnd = $win.H

# SW_RESTORE = 9, SW_SHOW = 5
[void][Cap.P]::ShowWindow($hwnd, 9)
Start-Sleep -Milliseconds 400
[void][Cap.P]::ShowWindow($hwnd, 5)
[void][Cap.P]::BringWindowToTop($hwnd)
[void][Cap.P]::SetForegroundWindow($hwnd)
Start-Sleep -Milliseconds 900

$r = New-Object Cap.P+RECT
[void][Cap.P]::GetWindowRect($hwnd, [ref]$r)
$w = $r.R - $r.L; $h = $r.B - $r.T
Write-Host ("target hwnd=0x{0:X} rect {1},{2} {3}x{4} visible={5}" -f `
  [int64]$hwnd, $r.L, $r.T, $w, $h, [Cap.P]::IsWindowVisible($hwnd))

$x = [Math]::Max(0, $r.L - $Pad); $y = [Math]::Max(0, $r.T - $Pad)
$cw = $w + 2 * $Pad; $ch = $h + 2 * $Pad

$sdc = [Cap.P]::GetDC([IntPtr]::Zero)
$dc  = [Cap.P]::CreateCompatibleDC($sdc)
$bmp = [Cap.P]::CreateCompatibleBitmap($sdc, $cw, $ch)
[void][Cap.P]::SelectObject($dc, $bmp)
[void][Cap.P]::BitBlt($dc, 0, 0, $cw, $ch, $sdc, $x, $y, 0x00CC0020)
$img = [System.Drawing.Image]::FromHbitmap($bmp)

New-Item -ItemType Directory -Force -Path (Split-Path $Out) | Out-Null
$img.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$img.Dispose()
[void][Cap.P]::DeleteObject($bmp)
[void][Cap.P]::DeleteDC($dc)
[void][Cap.P]::ReleaseDC([IntPtr]::Zero, $sdc)
Write-Host "saved: $Out ($cw x $ch at $x,$y)"