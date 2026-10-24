# 扫源码与文档里的 U+FFFD（Unicode 替换字符），以及**游离控制符**。
#
# 为什么值得单独立一步：**受损的全是注释与文档**，所以 fmt / clippy / test /
# svelte-check 一样绿，损坏能静静躺着一直没人发现。
#
# 成因见 ticket 19：PS 5.1 的 `(Get-Content -Raw) -replace … | Set-Content
# -Encoding UTF8` 做机械替换，UTF-8 被按 ANSI 读回再写——字符已经丢了，
# 事后无法从文件里还原。
#
# ⚠ 第二类损伤见 ticket 52：**转义序列被当成真字符写进了源码**。
#   实测在 `crates/capture/src/inject.rs` 抓到四种：
#   - `bytes` 的 `b` 变成 U+0008（退格符）——读出来看着像 `ytes`，肉眼扫不出来
#   - `focus` / `fill_input_box` 的 `f` 变成 U+000C（换页符）
#   - `type_unicode` 的 `t` 变成 TAB
#   - 另一处留了字面反引号 + r（本文件早先也是这样，所以下面连它一起扫）
#   **四种全部编译得过、测试跑得过**，因为受损的是注释；
#   而 `ocus(target)` 这种把字母吃掉的注释，clippy 同样不会管。
#   ⇒ 只查 U+FFFD 的那一档扫不到它们，必须单独扫控制符。
#
# **只扫项目自己的目录，不扫 git ls-files 的全量。**
# `.agents/` `.claude/` `.minimax/` `.zcode/` 是 CLI 为别的运行时装的，四份
# 共 3150 个文件；`git ls-files` 全量逐个 ReadAllText 要几十秒，而那里由
# `skills-lock.json` 管版本，改内容不是本仓库该做的事。

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$ext = '\.(rs|svelte|ts|js|md|json|toml|ps1|py|css|html|yml|yaml)$'
# `ui\scripts` 必须在这里（2026-10-13 补）。原先只有 `ui\src`，
#   而 `ui/scripts/` 恰好是**中文注释最密的地方**——量具脚本
#   （`sheetprobe.ps1` / `pencilgen.py` / `icongen.py`）每一条都在解释
#   「为什么这么量」，全是长句中文，最经不起编码往返。
#   漏掉它的实测证据：那天 `stateink.ps1` 里一个 U+FFFD 就摆在注释里，
#   而这一步扫了 179 个文件报「干净」。
#   同理补了 `py` 扩展——`pencilgen.py` / `icongen.py` 也在这个目录里。
$roots = @('crates', 'xtask', 'ui\src', 'ui\src-tauri\src', 'ui\scripts', 'scripts', 'docs', '.scratch')
$files = @()
foreach ($r in $roots) {
  if (Test-Path $r) {
    $files += Get-ChildItem -Path $r -Recurse -File | Where-Object { $_.FullName -notmatch '\\target\\|node_modules|\.svelte-kit' }
  }
}
# ⚠⚠ **2026-10-24 修一个静默的洞**：原来这两行用的是
#   `Get-ChildItem -Path . -File -Include *.md,*.toml` 与
#   `Get-ChildItem -Path ui -File -Include *.json,*.ts,*.js`。
# ⇒ **PowerShell 的 `-Include` 配字面路径（路径里没有通配符）会过滤掉一切**
#   （实测：`Get-ChildItem -Path ui -File -Include *.json` 返回 **0** 个，
#   而去掉 `-Include` 返回 **10** 个）⇒ 这两行**从来没扫到过任何文件**
# ⇒ 根目录的 `AGENTS.md` / `README*.md` / `Cargo.toml`、
#   `ui/` 根下的 `package.json` / `tsconfig.json` / `vite.config.ts` /
#   `pnpm-lock.yaml` / `pnpm-workspace.yaml`
#   ——**全都在扫描范围外**，而它们恰恰是中文注释最密的地方。
#   ⇒ 改用**扩展名过滤**，不靠 `-Include`。
$files += Get-ChildItem -Path . -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match '\.(md|toml)$' }
$files += Get-ChildItem -Path ui -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match '\.(json|ts|js|yaml|yml)$' }
$files = $files | Where-Object { $_.Name -match $ext } | Sort-Object FullName -Unique

# U+0000–U+0008 / U+000B / U+000C / U+000E–U+001F / U+007F：这些字符在任何一份
# 源码或文档里都没有合法用途，看到就是损坏。TAB / LF / CR **排除**——
# TAB 本身合法，见下面「词中 TAB」。
$ctrlRe = [regex]"[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]"
# 词中间的 TAB：ticket 52 里 `type_unicode` 的 `t` 被换成了 TAB。
# 只报「字母/数字 夹 TAB 夹 字母/数字」——那不是缩进风格，是字母被吃掉了。
$midTabRe = [regex]"[\p{L}\p{N}]\t[\p{L}\p{N}]"
# 字面转义残留：PowerShell 的 `` `r `` / `` `n `` 被当成普通字符写进了源码。
# 判据收窄到「行尾是 反引号 + 一个字母 + 只有空白」——源码里没有任何合法写法
# 长这样，而 markdown 满地都是（行内代码就在行尾），所以源码之外不适用。
$litEscRe = [regex]'`[a-zA-Z][ \t]*$'

$hits = @()
$ctrlHits = @()
foreach ($f in $files) {
  try { $c = [IO.File]::ReadAllText($f.FullName) } catch { continue }
  $rel = $f.FullName.Replace((Get-Location).Path + '\', '')
  if ($c -match "\uFFFD") {
    $n = ([regex]::Matches($c, "\uFFFD")).Count
    $idx = ([regex]::Match($c, "\uFFFD")).Index
    $ln = ($c.Substring(0, $idx) -split "`n").Count
    $hits += ('  {0}:{1}  {2} 个 U+FFFD' -f $rel, $ln, $n)
  }
  foreach ($m in $ctrlRe.Matches($c)) {
    $ln = ($c.Substring(0, $m.Index) -split "`n").Count
    $ctrlHits += ('  {0}:{1}  U+{2:X4}（游离控制符）' -f $rel, $ln, [int][char]$m.Value)
  }
  foreach ($m in $midTabRe.Matches($c)) {
    $ln = ($c.Substring(0, $m.Index) -split "`n").Count
    $ctrlHits += ('  {0}:{1}  词中间的 TAB：…{2}…（字母被换成了制表符）' -f $rel, $ln, ($m.Value -replace "`t", '<TAB>'))
  }
  # 字面转义残留**只对源码扫，markdown 一律跳过**。
  # 教训：v1 的判据是「反引号紧跟在 `*` 后面」，第一跑就在三张票里各误报一次
  # ——markdown 的 **`y 51..63`** 这种加粗行内代码长得一模一样。
  # ⇒ 宁可漏，也别让守卫天天喊狼来了。
  if ($f.Extension -ne '.md') {
    foreach ($ln2 in ($c -split "`n")) {
      if ($litEscRe.IsMatch($ln2)) {
        $ctrlHits += ('  {0}  字面转义残留：…{1}…（PowerShell 的 `r / `n 被写成了字面字符）' -f $rel, $ln2.Trim())
      }
    }
  }
}

Write-Output ("扫了 {0} 个文件。" -f $files.Count)
if ($hits.Count -gt 0) {
  Write-Output "发现已损坏字符（U+FFFD）："
  $hits | ForEach-Object { Write-Output $_ }
  Write-Output ''
  Write-Output '这些位置编译得过、测试也跑得过——受损的是注释与文档，所以没有任何'
  Write-Output '其他门禁会报它们。处理办法见 .scratch/…/19-mojibake-in-source-comments.md'
  exit 1
}
if ($ctrlHits.Count -gt 0) {
  Write-Output '发现游离控制符 / 词中 TAB / 字面转义残留：'
  $ctrlHits | ForEach-Object { Write-Output $_ }
  Write-Output ''
  Write-Output '这一类 U+FFFD 那一档扫不到：它们是**能编译的控制符**，而且往往顶掉了'
  Write-Output '一个字母（bytes→ytes、focus→ocus）。见 .scratch/…/52-*.md'
  exit 1
}

Write-Output 'U+FFFD / 控制符 扫描：干净'
exit 0
