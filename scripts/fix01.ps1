$ErrorActionPreference = "Continue"
Set-Location "E:\devSpace\chat-servant-m31f"

Write-Output "=== 1) examples/ 是第三方参考项目，不该进我们的仓库 ==="
git rm -r --cached examples -q
Add-Content -Path .gitignore -Value "`n# 第三方参考项目（自带 LICENSE，MIT/GPL 混合）。只读，不入库。`nexamples/"
Remove-Item -Recurse -Force .git\modules -ErrorAction SilentlyContinue

Write-Output "=== 2) 修 fmt ==="
cargo fmt --all

Write-Output "=== 3) 修 clippy ==="
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | Select-String -Pattern '^(error|warning)\[|^  -->' | ForEach-Object { $_.Line } | Select-Object -First 20
Write-Output "CLIPPY_EXIT=$LASTEXITCODE"