# 每个工作区成员的 license 声明，**必须等于**根 `[workspace.package] license`。
#
# ## 为什么要这道（2026-10-18 实测打出来的，不是推演）
#
# `ui/src-tauri/Cargo.toml` 一直写着 `license = "MIT OR Apache-2.0"`，
# 而根 manifest、工作区其余四个 crate 与仓库根的 `LICENSE` 都是 **BUSL 1.1**。
#
# 本来指望 `cargo deny check` 顺手抓得住——**它抓不住，而且方向正好相反**：
#   · `MIT` / `Apache-2.0` 都在 `deny.toml` 的 permissive 白名单里，
#     所以写错成 MIT **不会**触发 "license is not explicitly allowed"；
#   · `deny.toml` 里那条 `{ allow = ["BUSL-1.1"], crate = "chat-servant-app" }`
#     于是变成了**用不上的例外**，cargo-deny 只发一条
#     `warning[license-exception-not-encountered]`；
#   · 而门禁只看**退出码**，warning 不影响 `deny_exit`。
#
# 实测：把 license 改回 `MIT OR Apache-2.0` ⇒ `cargo deny check` **exit=0**。
# ⇒ 在这个方向上 `cargo deny` 是**瞎的**，必须另开一道。
#
# ## 判据的形状（照 `check-version.ps1` 的两条纪律）
#
# ① **成员清单由 `cargo metadata --no-deps` 给**，不硬编码包名。
#    硬编码就等于「按出事的那个文件选范围」——下一个新增 crate 的人不会
#    想起来把包名加进守卫。加成员时自动被覆盖，才是按不变量选范围。
#    （`deny.toml` 的注释已经写过同一句：「补的时候让 `cargo metadata` 当裁判」。）
#
# ② **读不到 = 红**，不是「跳过」。
#    量具失效和「没问题」在输出上完全同形。某个成员解析不出 license
#    （没写、又没继承 ⇒ metadata 里是 null）一律红。
#
# ⚠ 为什么比对 `cargo metadata` 的**解析后**值，而不是逐个文件 grep `license = "…"`：
#   成员可以用 `license.workspace = true` 继承——继承与字面写法在文件里
#   **长得不一样**，但实际许可必须相同。metadata 给的是继承展开后的值，
#   两种写法都盖得住；而逐文件 grep 只认字面写法，会漏掉「继承的成员」，
#   也会被**解释该字段的那段注释**满足。
#
# 退出码 0 = 全一致；1 = 有不一致、或有成员/根读不到 license。

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

function Remove-HashComment($line) {
  if ($line -match '^\s*#') { return '' }
  return $line
}

# ── 唯一出处：根 [workspace.package] license ─────────────────────────────────
# 逐行扫，注释行剔掉，按节定位——否则说明这段继承关系的**注释**
# 里那个 license 字面量就能替真正的声明交账。
$rootPath = 'Cargo.toml'
if (-not (Test-Path $rootPath)) {
  Write-Output 'license 一致性：红（根 Cargo.toml 不存在）'
  exit 1
}
$lines = [IO.File]::ReadAllLines((Resolve-Path $rootPath)) |
  ForEach-Object { Remove-HashComment $_ }
$inSec = $false; $sawSec = $false; $want = $null
foreach ($ln in $lines) {
  if ($ln -match '^\s*\[') {
    $name = (($ln -replace '^\s*\[', '') -replace '\]\s*$', '').Trim()
    $inSec = ($name -eq 'workspace.package')
    if ($inSec) { $sawSec = $true }
    continue
  }
  if ($inSec -and $ln -match '^\s*license\s*=\s*"([^"]+)"') { $want = $Matches[1]; break }
}
if (-not $want) {
  Write-Output 'license 一致性：红（根 [workspace.package] 里读不到 license）'
  if (-not $sawSec) { Write-Output '  · 没有 [workspace.package] 这一节' }
  else { Write-Output '  · 那一节里没有 license 赋值' }
  Write-Output '  ⚠ 读不到**不等于**「没关系」——量具坏了和许可不一致长得一模一样。'
  exit 1
}

# ── 成员清单：让 cargo metadata 当裁判 ────────────────────────────────────────
$metaRaw = & cargo metadata --no-deps --format-version 1 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) {
  Write-Output 'license 一致性：红（cargo metadata 跑失败）'
  Write-Output '  · 这一步取不到工作区成员清单，下面无从比对——量具坏了，当红处理。'
  Write-Output '  · cargo 的原始输出：'
  ($metaRaw -split "`r?`n") | Select-Object -First 8 | ForEach-Object { Write-Output "    $_" }
  exit 1
}
try {
  $meta = $metaRaw | ConvertFrom-Json
} catch {
  Write-Output 'license 一致性：红（cargo metadata 输出解析失败）'
  Write-Output "  · $($_.Exception.Message)"
  exit 1
}

# workspace_members 是 id 数组（形如 "path+file:///…#chatservant-core@0.1.1"），
# 与 packages[].id 一一对应。不按名字匹配——重名包在 metadata 里是分开的条目。
$byId = @{}
foreach ($p in $meta.packages) { $byId[$p.id] = $p }
$members = @()
foreach ($id in $meta.workspace_members) {
  if (-not $byId.ContainsKey($id)) {
    Write-Output 'license 一致性：红（workspace_members 里有 id 对不上 packages）'
    Write-Output "  · $id"
    exit 1
  }
  $members += $byId[$id]
}
$members = @($members | Sort-Object { $_.manifest_path })

if ($members.Count -eq 0) {
  Write-Output 'license 一致性：红（工作区成员一个都没有）'
  Write-Output '  · 这几乎不可能是对的——量具一定坏了，按红处理。'
  exit 1
}

# ── 逐个点名 ────────────────────────────────────────────────────────────────
Write-Output ("license 声明（唯一出处 [workspace.package] = {0}）：" -f $want)
$bad = @()
foreach ($p in $members) {
  # 从 manifest_path 反推一个人类看得懂的相对位置。
  # ⚠ 不要把仓库目录名写死在正则里——那是在这台机器上量出来的形状，
  #   换个 clone 路径就静默失配（`-replace` 匹配不上会**原样返回整条绝对路径**，
  #   输出变得很长但**不会报错**，看起来只是「路径不好看」）。
  $rel = [IO.Path]::GetRelativePath((Get-Location).Path, $p.manifest_path)
  if ($null -eq $p.license -or $p.license -eq '') {
    Write-Output ("  {0,-34} = **没有 license**" -f $rel)
    $bad += ('{0}：既没写 license，也没继承 license.workspace = true' -f $rel)
  } elseif ($p.license -eq $want) {
    Write-Output ("  {0,-34} = {1}" -f $rel, $p.license)
  } else {
    Write-Output ("  {0,-34} = {1}  ← 与唯一出处不一致" -f $rel, $p.license)
    $bad += ('{0}：写的是 "{1}"，根是 "{2}"' -f $rel, $p.license, $want)
  }
}

if ($bad.Count -gt 0) {
  Write-Output ''
  Write-Output 'license 一致性：红'
  $bad | ForEach-Object { Write-Output "  · $_" }
  Write-Output ''
  Write-Output '  ⚠ `cargo deny check` 在这个方向上**抓不到**：MIT/Apache-2.0 都在'
  Write-Output '    白名单里，它只会把 `deny.toml` 里那条 BUSL-1.1 例外标成'
  Write-Output '    `warning[license-exception-not-encountered]` 并仍然 exit 0。'
  Write-Output '    ⇒ 唯一能守住这条的是这道脚本本身。'
  Write-Output ''
  Write-Output '  改法：让所有成员继承 `license.workspace = true`（app 壳子不跟工作区'
  Write-Output '  继承那一套，只能手写 `license = "BUSL-1.1"`），值必须与根一字不差。'
  exit 1
}

Write-Output ''
Write-Output ("license 一致性：{0} 个成员全一致 = {1}" -f $members.Count, $want)
exit 0
