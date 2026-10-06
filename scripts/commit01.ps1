$ErrorActionPreference = "Continue"
Set-Location "E:\devSpace\chat-servant-m31f"

Write-Output "=== 门禁 ==="
& pwsh -NoProfile -File scripts\gate.ps1 2>&1 | Select-String -Pattern '^(=====|>>> |GATE|advisories)' | ForEach-Object { $_.Line }
$gate = $LASTEXITCODE

Write-Output ""
Write-Output "=== 提交 ==="
git add -A
git status --short
git -c core.autocrlf=false commit -q -m @"
feat: 抓帧探针与 Rust 工作区骨架（ticket 01）

建立 workspace、域类型、窗口发现与两条捕获路径，并用探针在实机上
测出 WGC 在本机不工作这一关键结论。

- crates/core: Rect + Frame(RGB8, 常驻内存)，含全黑检测与裁剪
- crates/capture: ChatSurface / EnumWindows 窗口发现 /
  WGC 单帧捕获 / PrintWindow 兜底
- xtask: cargo xtask probe frame，支持 --hwnd 与 --method
- deny.toml: permissive-only 许可证策略，cargo deny check 门禁
- 16 个单元测试；fmt / clippy -D warnings / test / deny 四道全绿

实机结论（见 docs/adr/0004-capture-strategy.md）：
- 窗口发现正常：weixin.exe pid 18180, 1276x965
- WGC：会话启动后立即关闭且无错误码，微信与记事本表现一致
- PrintWindow+PW_RENDERFULL_CONTENT：微信直接拒绝（返回 0）
  即参考项目预测的 GPU 合成窗口症状
"@
Write-Output "COMMIT_EXIT=$LASTEXITCODE"
git log --oneline | Select-Object -First 3
Write-Output "GATE_EXIT=$gate"