$ErrorActionPreference = "Continue"
Set-Location "E:\devSpace\chat-servant-m31f"

# 把探针源码搬进 xtask 会越滚越大，夹具生成独立成一个脚本命令。
cargo xtask fixture --name wechat-dark-2side --source target\probe\xiaohao.png
cargo xtask fixture --name wechat-dark-myside --source target\probe\text-chat.png
Get-ChildItem crates\capture\fixtures | Select-Object Name, Length
