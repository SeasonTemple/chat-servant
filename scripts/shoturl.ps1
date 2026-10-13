param(
  [Parameter(Mandatory = $true)][string]$Url,
  [Parameter(Mandatory = $true)][string]$OutPng,
  [int]$Width = 520,
  [int]$Height = 760,
  [int]$Wait = 4000,
  [int]$Scale = 2
)

# ⚠ 量具自检：Chrome 会把窗口宽度**钳到最小 500px**，而 --screenshot 输出的
#   宽度仍等于 --window-size。所以「截图尺寸对」**不能**证明「布局宽度对」。
#   本面板内部被 cs-demo-frame 钉死 380px，与视口无关，故此处只当出图用，
#   任何尺寸结论都必须另找独立量法。
$ErrorActionPreference = 'Stop'
$chrome = 'C:\Program Files\Google\Chrome\Application\chrome.exe'
if (-not (Test-Path $chrome)) { throw "chrome not found" }

$profileDir = Join-Path $env:TEMP ("shot-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $profileDir | Out-Null

$args = @(
  '--headless=new',
  "--user-data-dir=$profileDir",
  '--no-first-run',
  '--no-default-browser-check',
  '--disable-gpu',
  '--hide-scrollbars',
  "--force-device-scale-factor=$Scale",
  "--window-size=$Width,$Height",
  "--virtual-time-budget=$Wait",
  "--screenshot=$OutPng",
  $Url
)

& $chrome @args 2>&1 | Out-Null
$code = $LASTEXITCODE
if (-not (Test-Path $OutPng)) { throw "no screenshot produced (exit=$code)" }
$img = Get-Item $OutPng
"$($img.FullName) | $($img.Length) bytes | exit=$code"
