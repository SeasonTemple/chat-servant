$ErrorActionPreference = "Continue"
Set-Location $PSScriptRoot\..
$fail = 0

function Step($name, $scriptblock) {
  Write-Output ""
  Write-Output "========== $name =========="
  & $scriptblock
  if ($LASTEXITCODE -ne 0) { Write-Output ">>> $name FAILED (exit=$LASTEXITCODE)"; $script:fail = 1 }
  else { Write-Output ">>> $name OK" }
}

Step "cargo fmt --all --check" { cargo fmt --all --check 2>&1 | Select-Object -Last 40 }
Step "cargo clippy -D warnings" { cargo clippy --workspace --all-targets -- -D warnings 2>&1 | Select-Object -Last 60 }
Step "cargo test --workspace"  { cargo test --workspace 2>&1 | Select-Object -Last 60 }
if (Get-Command cargo-deny -ErrorAction SilentlyContinue) {
  Step "cargo deny check" { cargo deny check 2>&1 | Select-Object -Last 60 }
} else {
  Write-Output ""
  Write-Output ">>> cargo deny check SKIPPED（cargo-deny 未安装：cargo install cargo-deny）"
}

Write-Output ""
Write-Output "=========================="
if ($fail -eq 0) { Write-Output "GATE: ALL GREEN" } else { Write-Output "GATE: FAILED" }
Write-Output "GATE_EXIT=$fail"