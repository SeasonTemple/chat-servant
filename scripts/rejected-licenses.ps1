# 一次性诊断：从 cargo deny 的 licenses 报告里列出被拒的 crate 名。
# 不靠正则猜 crate 名——报告里的树形字符宽度随终端宽度变，猜不准。
param([string]$Log = 'target/probe/lic.log')

$ErrorActionPreference = 'Stop'
$ErrorActionPreference = 'Continue'

# 被拒条目的特征：紧跟在 "rejected: license is not explicitly allowed" 之后的
# 那个 "│ <name> v<version> registry+..." 行。取每段的第一个。
$names = New-Object System.Collections.Generic.HashSet[string]
$armed = $false
foreach ($line in Get-Content -LiteralPath $Log) {
    if ($line -match 'rejected: license is not explicitly') { $armed = $true; continue }
    if ($armed) {
        $t = $line.Trim()
        if ($t -match '^│\s*([a-zA-Z0-9_-]+)\s+v[0-9]') {
            [void]$names.Add($Matches[1])
            $armed = $false
        }
    }
}
$names | Sort-Object