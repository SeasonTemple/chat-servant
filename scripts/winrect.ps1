param([Parameter(Mandatory=$true)][int64]$Hwnd)

Add-Type -Namespace Win -Name R -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
public struct RECT { public int L, T, R, B; }
'@

# winscan/hittrace report rectangles in a different space from SetCursorPos on this
# machine (DPI virtualisation), so anything that is going to click or crop by
# coordinates has to opt into the physical space first.
[void][Win.R]::SetProcessDPIAware()

$r = New-Object Win.R+RECT
[void][Win.R]::GetWindowRect([IntPtr]$Hwnd, [ref]$r)
"hwnd=0x{0:X}  x={1} y={2} w={3} h={4}" -f $Hwnd, $r.L, $r.T, ($r.R - $r.L), ($r.B - $r.T)
