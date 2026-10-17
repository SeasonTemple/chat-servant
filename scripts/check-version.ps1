# 四个地方各自声明了一份版本号，**必须一致**。
#
# ## 为什么要这道
#
# 2026-10-18 实测：`ui/src-tauri/tauri.conf.json` 的 version 一直是 `0.1.0`，
# 而发版决策定的下一个 tag 是 `v0.1.1`。`release.yml` 里那道
# 「tag 与 tauri.conf.json 的 version 必须一致」的闸**会在发版那一刻才炸**——
# 也就是攒了所有改动、打完包，才知道版本对不上。
#
# 四份声明：
#   · `Cargo.toml`            `[workspace.package] version`
#   · `ui/src-tauri/Cargo.toml` `[package] version`
#   · `ui/package.json`        `.version`
#   · `ui/src-tauri/tauri.conf.json` `.version`
#
# ⚠ 后两个没有 `version.workspace = true` 那种继承可借（JSON 根本不支持），
#   所以**只能手动对齐**——而「只能手动」就意味着需要一个闸。
#
# ## 判据的形状（两处都是踩过的）
#
# ① **不许被注释满足**。上一版判据直接在 Cargo.toml 全文里搜 `version = "…"，
#    而我刚给 `[workspace.package]` 写的那段说明里就带着**别的版本号形状**的
#    文字——照着注释就能过。这与 `check-gate-parity.ps1` 里那次的教训是同一条：
#    **一个断言「CI 跑过这条命令」的守卫，被描述这条命令的注释满足**。
#    ⇒ 这里只读**去掉注释之后**的正文，且按**节**定位，不全文搜。
#
# ② **解析不出来 = 红**，不是「跳过」。
#    量具失效和「没问题」在输出上完全同形（`check-gate-parity.ps1` 自证组 M
#    踩过一次：正则写错把整个 workflow 吃掉，守卫对着残文件报「合规」）。
#    ⇒ 找不到 `[workspace.package]` / 找不到节里的 version，一律红。
#
# 退出码 0 = 四处一致；1 = 不一致或解析失败。

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

function Remove-HashComment($line) {
  if ($line -match '^\s*#') { return '' }
  return $line
}

# 从 TOML 里取某个节的 version。**逐行**扫，注释行直接剔掉。
function Get-TomlVersion([string]$path, [string]$section) {
  if (-not (Test-Path $path)) { return [pscustomobject]@{ Ok = $false; Why = "文件不存在" } }
  $lines = [IO.File]::ReadAllLines((Resolve-Path $path)) |
    ForEach-Object { Remove-HashComment $_ }
  $inSec = $false; $sawSec = $false
  foreach ($ln in $lines) {
    if ($ln -match '^\s*\[') {
      # 节名逐字比（去掉两端空白）。表头之后就该只在这个节里找了。
      $name = ($ln -replace '^\s*\[', '') -replace '\]\s*$', ''
      $inSec = ($name.Trim() -eq $section)
      if ($inSec) { $sawSec = $true }
      continue
    }
    if ($inSec -and $ln -match '^\s*version\s*=\s*"([^"]+)"') {
      return [pscustomobject]@{ Ok = $true; Value = $Matches[1] }
    }
  }
  if (-not $sawSec) { return [pscustomobject]@{ Ok = $false; Why = "没有 [$section] 这一节" } }
  return [pscustomobject]@{ Ok = $false; Why = "[$section] 里没有 version 赋值" }
}

function Get-JsonVersion([string]$path, [string]$label) {
  if (-not (Test-Path $path)) { return [pscustomobject]@{ Ok = $false; Why = "文件不存在" } }
  try {
    $o = Get-Content $path -Raw | ConvertFrom-Json
  } catch {
    return [pscustomobject]@{ Ok = $false; Why = "JSON 解析失败：$($_.Exception.Message)" }
  }
  if ($null -eq $o.version) { return [pscustomobject]@{ Ok = $false; Why = "没有 .version 字段" } }
  return [pscustomobject]@{ Ok = $true; Value = [string]$o.version }
}

$found = @()
$bad = @()

$found += [pscustomobject]@{ Where = 'Cargo.toml [workspace.package]'; R = Get-TomlVersion 'Cargo.toml' 'workspace.package' }
$found += [pscustomobject]@{ Where = 'ui\src-tauri\Cargo.toml [package]';   R = Get-TomlVersion 'ui\src-tauri\Cargo.toml' 'package' }
$found += [pscustomobject]@{ Where = 'ui\package.json';                       R = Get-JsonVersion 'ui\package.json' 'package.json' }
$found += [pscustomobject]@{ Where = 'ui\src-tauri\tauri.conf.json';         R = Get-JsonVersion 'ui\src-tauri\tauri.conf.json' 'tauri.conf.json' }

Write-Output '版本声明（逐处点名）：'
foreach ($f in $found) {
  if ($f.R.Ok) {
    Write-Output ("  {0,-38} = {1}" -f $f.Where, $f.R.Value)
  } else {
    Write-Output ("  {0,-38} = **读不到**（{1}）" -f $f.Where, $f.R.Why)
    $bad += ("$($f.Where) 读不到版本：$($f.R.Why)")
  }
}

if ($bad.Count -gt 0) {
  Write-Output ''
  Write-Output '版本一致性：红（至少一处读不到）'
  $bad | ForEach-Object { Write-Output "  · $_" }
  Write-Output ''
  Write-Output '⚠ 读不到**不等于**「没关系」——量具坏了和版本对不上，'
  Write-Output '  在这一屏上长得一模一样。修法：确认那个文件的结构是不是变了。'
  exit 1
}

$vals = $found | ForEach-Object { $f = $_; $f.R.Value }
# ⚠ 必须 @() 包一层：四处一致时 `Sort-Object -Unique` 只回**一个**对象，
#   而 PowerShell 会把单元素结果拆包成标量字符串 ⇒ `$uniq[0]` 取到的是
#   **第一个字符**（"0.1.1" → "0"）。本会话里这个坑在 autofixer 那张票上
#   已经踩过一次，这里是第二次，所以写进注释。
$uniq = @($vals | Sort-Object -Unique)
if ($uniq.Count -ne 1) {
  Write-Output ''
  Write-Output '版本一致性：红（四处不一致）'
  Write-Output ''
  Write-Output '  发版闸（release.yml 的 Report publish gate）只比 **tag ↔ tauri.conf.json**，'
  Write-Output '  另三处对不上它不会拦——所以必须在这道里一起对齐。'
  Write-Output ''
  $found | ForEach-Object { Write-Output ("    {0,-38} = {1}" -f $_.Where, $_.R.Value) }
  Write-Output ''
  Write-Output '  改的时候四处一起改，并同步 `docs/HANDOFF.md` 里记的下一个版本号。'
  exit 1
}

Write-Output ''
Write-Output ("版本一致性：四处一致 = {0}" -f $uniq[0])

# ─────────────────────────────────────────────────────────────────────────
# 附一条：**本版本有没有双语发布说明**。
#
# ⚠ 这里刻意**镜像** `release.yml` 的策略，而不是自己定规矩：
#   · 说明文件**存在** ⇒ 必须同时有 `## 中文` 与 `## English`，否则发版**失败**；
#   · 说明文件**不存在** ⇒ 只警告，沿用 GitHub 自动生成的 notes。
# 本地提前验是为了「改了版本号却忘了写说明」在提交里就暴露，
# 而不是攒到打 tag 那天才炸。正则与 release.yml 里的**逐字相同**。
$note = Join-Path 'docs\release-notes' ('v{0}.md' -f $uniq[0])
if (Test-Path $note) {
    $body = [IO.File]::ReadAllText((Resolve-Path $note))
    $hasCn = $body -match '(?m)^##\s*中文'
    $hasEn = $body -match '(?m)^##\s*English'
    if (-not ($hasCn -and $hasEn)) {
        Write-Output ("发布说明 {0} 缺少 {1} 段" -f $note, $(if (-not $hasCn -and -not $hasEn) { "'## 中文' 与 '## English'" } elseif (-not $hasCn) { "'## 中文'" } else { "'## English'" }))
        Write-Output '    ⇒ 双语发布说明是门禁不是建议：release.yml 在这一步会 exit 1。'
        exit 1
    }
    Write-Output ("发布说明     : {0}（双语齐全）" -f $note)
}
else {
    Write-Output ("发布说明     : **没有 {0}**" -f $note)
    Write-Output '    ⇒ 发版不会失败，但只会得到 GitHub 自动生成的 notes。'
    Write-Output '      要策展就写一份（同时含 ## 中文 与 ## English），随这个版本一起提交。'
}
exit 0
