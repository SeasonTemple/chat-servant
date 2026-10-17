# 核对**装包内部**真的有 OCR 权重。
#
# ## 为什么不能只看 bundle 目录里有没有 .onnx
#
# `bundle.resources` 的文件是**进安装包内部**的（`<安装目录>/models/ppocrv6-small/`），
# 不会以散文件形式躺在 `target/release/bundle/` 下。
# 所以 2026-10-18 那个核对步骤写的是
#
#     Get-ChildItem $BundleDir -Recurse -Include '*.onnx','ppocrv6_dict.txt'
#
# ——它**必然找不到**，于是 `release.yml` 的 dry run 在这一步红了
# （run `39038993181`）。而权重其实好好地在装包里。
#
# ⚠ 这不是「写错了路径」，是**查错了地方**：目录里的东西是「打包输入」，
#   要证明的是「打包**产物**里有」。两者不是一个空间。
#
# ## 两个安装包的查法不一样，且各自的理由写在输出里
#
# | 安装包 | 怎么查 | 依据 |
# |---|---|---|
# | NSIS | `7z l` 列条目，**按路径**匹配 `models\ppocrv6-small\<文件名>` | NSIS 归档保留原文件名 |
# | MSI | `7z l` 列条目，**按体积**匹配 | MSI 内部用 `PathFile_I<hash>` 存流，**文件名被哈希掉了** |
#
# MSI 那边只能比体积，所以体积是**当场从刚取回来的权重文件读的**，不是写死的常量——
# 换模型时它自动跟着变，不会变成一个「早就过期的数字」。
#
# 代价要说清：MSI 的体积匹配是**代理判据**（同名不同物理论上可能撞体积）。
# 真正的名字级判据由 NSIS 那一路提供，两者都过才算数。
param(
  [string]$BundleDir,
  [string]$WeightsDir = 'target/models/ppocrv6-small'
)

$ErrorActionPreference = 'Stop'

# 目录优先取 `BUNDLE_DIR` 环境变量，让 release.yml 里的那个 env 仍是**唯一出处**
# ——否则这里再写一遍字面量，就等于我刚修掉的「同一路径三份副本」又回来了。
if (-not $BundleDir) {
  $BundleDir = if ($env:BUNDLE_DIR) { $env:BUNDLE_DIR } else { 'target/release/bundle' }
}

$sevenZip = (Get-Command 7z -ErrorAction SilentlyContinue)?.Source
if (-not $sevenZip) {
  # Windows runner 镜像清单里列了 7zip（Tools 段），所以它**应该**在；
  # 真不在就红，不要静默跳过——「查不到」与「没问题」长得一模一样。
  throw "找不到 7z。runner 镜像清单（actions/runner-images Windows2025）Tools 段列有 7zip，装不上说明镜像变了。"
}
Write-Output "7z: $sevenZip"

if (-not (Test-Path $BundleDir)) { throw "没找到装包目录：$BundleDir（由 BUNDLE_DIR / -BundleDir 决定）" }
if (-not (Test-Path $WeightsDir)) { throw "没找到权重目录：$WeightsDir（应先跑 fetch_models.py）" }

Write-Output "装包目录：$BundleDir"

$weights = Get-ChildItem $WeightsDir -File | Sort-Object Name
if ($weights.Count -lt 3) {
  throw ("权重只有 $($weights.Count) 个，应为 3（det / rec / dict）：" +
         ($weights.Name -join ', '))
}
Write-Output ""
Write-Output "权重来源：$WeightsDir"
foreach ($w in $weights) {
  Write-Output ("  {0,-28} {1,10} 字节" -f $w.Name, $w.Length)
}

$fail = [System.Collections.Generic.List[string]]::new()

# ── NSIS：按路径匹配（名字级判据）────────────────────────────────────
$nsis = Get-ChildItem (Join-Path $BundleDir 'nsis') -Filter '*.exe' -File -ErrorAction SilentlyContinue
if (-not $nsis) { $fail.Add("没找到 NSIS 安装包：$BundleDir\nsis\*.exe") }
else {
  $listing = (& $sevenZip l $nsis[0].FullName 2>&1) -join "`n"
  Write-Output ""
  Write-Output "── NSIS $($nsis[0].Name)（按**路径**匹配，名字级判据）"
  foreach ($w in $weights) {
    # 7z 的路径分隔符随平台；两边都接受
    $ok = $listing -match ("models[\\/]ppocrv6-small[\\/]" + [regex]::Escape($w.Name))
    Write-Output ("  {0,-28} {1}" -f $w.Name, $(if ($ok) { '在' } else { '**不在**' }))
    if (-not $ok) { $fail.Add("NSIS 安装包里没有 $($w.Name)") }
  }
}

# ── MSI：按体积匹配（代理判据，名字被哈希掉了）─────────────────────────
$msi = Get-ChildItem (Join-Path $BundleDir 'msi') -Filter '*.msi' -File -ErrorAction SilentlyContinue
if (-not $msi) { $fail.Add("没找到 MSI 安装包：$BundleDir\msi\*.msi") }
else {
  # MSI 这边只能**抽取**后比体积：
  #   · `7z l` 对 MSI 只列出一个 `app.cab` 条目，**不下沉**到内部流
  #     （实测只解析出 1 个体积，而真实流有 4 个）；
  #   · `7z e` 才会下沉，流名是 `PathFile_I<hash>`——文件名被哈希掉了。
  # 体积是**当场从刚取回来的权重文件读的**，不是写死的常量。
  $tmp = Join-Path ([IO.Path]::GetTempPath()) ("msi-probe-" + [guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Force -Path $tmp | Out-Null
  try {
    & $sevenZip e -y "-o$tmp" $msi[0].FullName *>$null
    $extracted = Get-ChildItem $tmp -File -ErrorAction SilentlyContinue
    $sizes = @($extracted | ForEach-Object { $_.Length })
    Write-Output ""
    Write-Output "── MSI $($msi[0].Name)（按**体积**匹配；MSI 内部流名是 PathFile_I<hash>，文件名不可见）"
    Write-Output "   从 MSI 里抽出 $($extracted.Count) 个流："
    $extracted | Sort-Object Length -Descending | ForEach-Object {
      Write-Output ("     {0,12:N0} 字节  {1}" -f $_.Length, $_.Name)
    }
    foreach ($w in $weights) {
      $ok = $sizes -contains $w.Length
      Write-Output ("  {0,-28} {1,10} 字节  {2}" -f $w.Name, $w.Length, $(if ($ok) { '在' } else { '**不在**' }))
      if (-not $ok) { $fail.Add("MSI 里没有体积为 $($w.Length) 的条目（对应 $($w.Name)）") }
    }
  } finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
  }
}

Write-Output ""
if ($fail.Count) {
  Write-Output '装包内**没有**完整的 OCR 权重：'
  $fail | ForEach-Object { Write-Output "  · $_" }
  Write-Output ''
  Write-Output '⇒ 装出去的应用会走不了 PP-OCR 那条路（表现为「装上了但 OCR 一直是慢的那条」）。'
  Write-Output '  多半是 bundle.resources 的映射写错，或取到的权重目录不对。'
  exit 1
}

Write-Output '装包内有权重：NSIS 路径级 + MSI 体积级，两边都齐。'
Write-Output '⇒ 装出去的应用会在 <安装目录>\models\ppocrv6-small\ 找到 det / rec / dict。'
exit 0
