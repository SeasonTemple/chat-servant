# 查候选依赖在 crates.io 上的许可证与最新版本。
#
# 用途：本仓库的硬性门禁是「只允许 permissive 许可证」（见 deny.toml / ADR-0003），
#      所以评估任何新方案时**先从 crates.io 元数据确认许可证**，
#      不能凭项目主页或印象——这是 ADR-0003 明写的要求。
# 用法: lic.ps1 <crate> [<crate> ...]
param([Parameter(Mandatory)][string[]]$Crates)

$ErrorActionPreference = 'Continue'
foreach ($c in $Crates) {
    try {
        $r = Invoke-RestMethod -Uri "https://crates.io/api/v1/crates/$c" `
            -Headers @{ 'User-Agent' = 'chat-servant-license-audit (local)' } -TimeoutSec 25
        $lic = $r.crate.license
        if (-not $lic) { $lic = '未声明（走 deny.toml 的未声明许可证规则）' }
        $d = ($r.crate.description -replace '\s+', ' ')
        if ($d.Length -gt 66) { $d = $d.Substring(0, 66) + '…' }
        "{0,-16} v{1,-9} license={2,-34} {3}" -f $c, $r.crate.max_stable_version, $lic, $d
    } catch {
        "{0,-16} 查不到：{1}" -f $c, $_.Exception.Message
    }
}