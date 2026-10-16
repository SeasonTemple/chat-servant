param([string]$OutDir = "$PSScriptRoot\..\target\shots")

# 九具全出图。2026-10-14：Chrome CLI 出空白时**先确认应用没崩**——
#   那次空白不是量具坏了，是我把面板改崩了（null 上读 status_hint），
#   而 `pnpm build` 与 svelte-autofixer 全过。⇒ 出图后必须核对字节数。
$ErrorActionPreference = 'Stop'
$chrome = 'C:\Program Files\Google\Chrome\Application\chrome.exe'
if (-not (Test-Path $chrome)) { throw 'chrome not found' }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$base = 'http://127.0.0.1:4173/?demo=1&state='
$states = 'drafted','confirm','armed','drafting','failed','noprep','silent','fresh','parked'

foreach ($s in $states) {
  $out = Join-Path $OutDir "st-$s.png"
  # **必须重试**：2026-10-14 实测同一 URL 连出三次，前一次空白、
  #   后两次 580465 字节且逐字节相同。空白与水合时序有关，不是稳定故障。
  #   而空白恰好是「应用崩了」与「量具抖了」共同的表现 ⇒ 单次结果不能定责。
  $len = 0
  for ($try = 1; $try -le 4; $try++) {
    $profile = Join-Path $env:TEMP ("shot-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force -Path $profile | Out-Null
    & $chrome '--headless=new' "--user-data-dir=$profile" '--no-first-run' '--disable-gpu' `
      '--hide-scrollbars' '--force-device-scale-factor=2' '--window-size=520,760' `
      '--virtual-time-budget=9000' "--screenshot=$out" "$base$s" 2>&1 | Out-Null
    if (Test-Path $out) { $len = (Get-Item $out).Length }
    if ($len -ge 50000) { break }
  }
  # 空白图约 7.5 KB。真面板约 580 KB。低于 50 KB 一律当作「没渲染」。
  if ($len -lt 50000) { throw "$s 连出四次都是空白（$len bytes）——先查应用是不是崩了" }
  "{0,-9} {1} bytes  (第 {2} 次)" -f $s, $len, $try
}
