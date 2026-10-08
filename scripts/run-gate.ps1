$ErrorActionPreference = 'Continue'
$log = Join-Path $PSScriptRoot '..\target\gate.log'
New-Item -ItemType Directory -Force -Path (Split-Path $log) | Out-Null
& pwsh -NoProfile -File (Join-Path $PSScriptRoot 'gate.ps1') *>&1 |
    Tee-Object -FilePath $log
$code = $LASTEXITCODE
Write-Host "GATE_EXIT=$code"
exit $code