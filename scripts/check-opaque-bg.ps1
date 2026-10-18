# 面板与设置屏里，**所有底色必须完全不透明**。
#
# ## 为什么要这道（`app.css` 顶头那条硬约束，2026-10-07 实测）
#
# Tauri 走**透明窗口 + WebView2 分层合成**，子元素上的半透明底色
# （`rgb(... / 88%)`、`color-mix(… , transparent)`）**整层不绘制**。
#
# ⚠⚠ **浏览器里查不出来**：`?demo=1` 的出图评审、`pnpm build`、
#   `svelte-check` 全在 Chromium 里跑，那里半透明底色画得好好的。
#   ⇒ 这类错误只在真机 Tauri 窗口上现形，而那正是「出图看不见 = 没改」
#   踩过的同一个坑（票 58 的量具一节）。
#
# 2026-10-18 实测发现**七处**半透明底色散在出货界面上，其中两处
# 是本轮刚加的（`.cs-fail`、`.cs-glyph.on`）。
# ⇒ 这道守卫不是预防假想错误，是补一个**当时真的坏了、且没人看见**的洞。
#
# ## 判据的范围（按**不变量**选，不是按出事的文件）
#
# 不变量只对**画在 Tauri 透明窗口里的东西**成立。扫：
#   · `ui/src/app.css`
#   · `ui/src/lib/**/*.svelte`
#   · `ui/src/routes/(panel)/**`
# 不扫：`routes/studio/`（手绘标记的排版台）、`routes/debug/`
# （自带深底对比台）——它们是**独立页面**，不画在那个透明窗口里。
# ⚠ 排除项在这里点名，是为了让下一个读脚本的人知道「不是漏了」。
#
# ## 三条判据各自守什么（都是 2026-10-18 实测踩出来的）
#
# ① **先剥注释再匹配**。本文件上方就写着 `rgb(20 26 31 / 7%)` 这个例子，
#    而源码的注释里也真的写着旧写法 —— 不剥注释的话，
#    **描述违规的那段注释会替违规本身交账**（与
#    `check-gate-parity.ps1` 上方记的同一族坑）。
#
# ② **`background: transparent` 不算违规**。`app.css` 面板根上那一行
#    是**定案**：纸由 SVG 自己画，根必须透出撕口外的桌面。
#    真正的禁令是「**半透明底色**」——带色调的那种。
#    ⚠ 这两件事在 `app.css` 里被反复强调过（「别把 transparent
#    和半透明底色混为一谈」），守卫把它写成判据而不是靠人记。
#
# ③ **读不到扫描范围 = 红**。目录不存在时 `Get-ChildItem` 返回空，
#    空集合会「扫到 0 个问题」然后放行 —— 那与「扫过了没问题」完全同形。
#
# 退出码 0 = 出货界面上没有半透明底色；1 = 有，或扫描范围读不到。
#
# ⚠ `-Root` 只为**自证**存在：把扫描根指到临时夹具树上，才能在不动真实
#   源码的前提下证明这道守卫真的会响。
#   ⚠⚠ **它必须是真的 `param`**，不能靠扫 `$args` 找 `-Root=`：
#   本脚本没有参数块时，PowerShell 会把 `-Root=E:\x` 拆成**两个** `$args`
#   条目（`-Root` 和 `E:\x`），`-like '-Root=*'` 谁都匹配不上 ⇒ 静默退回
#   仓库根，于是自证台架「测的是夹具」变成「测的是真仓库」。
#   **这种守卫比没有守卫更坏**：它会给出全绿，而测的根本不是那棵树。
param([string]$Root = '')

$ErrorActionPreference = 'Stop'

if ($Root) { Set-Location $Root } else { Set-Location (Join-Path $PSScriptRoot '..') }

$scopes = @(
  @{ label = 'app.css';        path = 'ui/src/app.css' }
  @{ label = 'lib/**';         path = 'ui/src/lib' }
  @{ label = 'routes/(panel)'; path = 'ui/src/routes/(panel)' }
)

# ── 收齐要扫的文件 ──────────────────────────────────────────────────────────
$files = @()
$missing = @()
foreach ($s in $scopes) {
  if (-not (Test-Path -LiteralPath $s.path)) {
    $missing += $s.label
    continue
  }
  $item = Get-Item -LiteralPath $s.path
  if ($item.PSIsContainer) {
    $files += Get-ChildItem -LiteralPath $s.path -Recurse -File -Include '*.svelte', '*.css'
  } else {
    $files += $item
  }
}

# ⚠ 判据 ③：范围读不到 = 红。空集合会「0 个问题」然后放行。
if ($missing.Count -gt 0) {
  Write-Output '底色不透明度：红（扫描范围读不到）'
  $missing | ForEach-Object { Write-Output "  · $_" }
  Write-Output '  ⚠ 读不到**不等于**「没问题」——量具坏了和合规长得一模一样。'
  exit 1
}
if ($files.Count -eq 0) {
  Write-Output '底色不透明度：红（扫描范围内一个文件都没有）'
  exit 1
}

# ── 剥注释 ─────────────────────────────────────────────────────────────────
function Remove-CssComment([string]$text) {
  $out = [Text.StringBuilder]::new()
  $i = 0; $n = $text.Length
  while ($i -lt $n) {
    $two = if ($i + 1 -lt $n) { $text.Substring($i, 2) } else { '' }
    if ($two -eq '/*') {
      $end = $text.IndexOf('*/', $i + 2)
      if ($end -lt 0) { break }          # 没闭合的块注释：后面全是注释
      $i = $end + 2
      # 注释换行留着，免得把两行声明黏成一行造出假阳性
      [void]$out.Append("`n")
      continue
    }
    if ($two -eq '//') {                 # Svelte <style> 里 SCSS 风格的行注释
      $nl = $text.IndexOf("`n", $i)
      if ($nl -lt 0) { break }
      $i = $nl
      continue
    }
    [void]$out.Append($text[$i]); $i++
  }
  $out.ToString()
}

# ── 判据本体 ───────────────────────────────────────────────────────────────
# 只看 `background:` / `background-color:` 的**值**。border-color、box-shadow、
# filter:drop-shadow 里的 alpha 是合法的（不是「底色」），不在范围内。
# ⚠ 这条边界是有意的：把范围放大到「所有 alpha」会把 drop-shadow 全判成违规，
#   而那是一条真正在用的样式 —— 守卫一旦天天误报就等于没有。
#
# ⚠⚠ **alpha 必须认得出它属于哪个函数**。第一版用 `/\s*[\d.]+\s*%` 当 alpha，
#   结果把这一行判成违规：
#     background: url('/probe-sheet.svg') center / 100% 100% no-repeat
#   —— 那个 `/` 是 `background-position / background-size` 的简写分隔符，
#   **不是 alpha**。⇒ 「断言失败先怀疑断言自己」当场兑现一次。
#   下面三个模式都要求 `/` 出现在 `rgb(` / `hsl(` **内部**。
$reDecl = [regex]'(?im)(?:^|[;{])\s*(?<prop>background(?:-color)?)\s*:\s*(?<val>[^;{}]*)'
# ⚠⚠ **用「数组 + join」而不是 here-string**。本轮在这里连踩两次：
#   ① `[regex]@'…'@ -replace …` 里的 `-replace` 作用在 **Regex 对象**上，
#      而 `Regex.ToString()` 返回的是**选项串**（`IgnoreCase`）不是 pattern
#      ⇒ 正则被悄悄换成字面量，**什么都不匹配、全程绿**。
#   ② 单引号 here-string 里 `#` 不是注释而是**字面量**；把注释留在
#      pattern 内部会要求输入里出现那行注释，匹配同样归零。
#   两种失败都表现为「注入违规却判绿」——**量具自己坏了，输出却像结论**。
# ⇒ 判据本身写成**一行**、注释写在**外面**，两条一起消掉。
$alphaSrc = '\brgba?\s*\([^)]*/\s*[\d.]+%|\brgba\s*\(|\bhsla?\s*\([^)]*/\s*[\d.]+%|\bhsla\s*\(|,\s*transparent\s*\)'
$reAlpha = [regex]::new($alphaSrc, [Text.RegularExpressions.RegexOptions]::IgnoreCase)

$bad = @()
foreach ($f in $files) {
  $raw = [IO.File]::ReadAllText($f.FullName)
  $src = Remove-CssComment $raw
  foreach ($m in $reDecl.Matches($src)) {
    $val = $m.Groups['val'].Value.Trim()
    # 判据 ②：裸 `transparent` = 「没有底色」，是定案，不算。
    if ($val -match '^transparent$') { continue }
    if ($val -match '^none$') { continue }
    if ($reAlpha.IsMatch($val)) {
      $line = 1 + ([regex]::Matches($raw.Substring(0, $m.Index), "`n")).Count
      $bad += ('{0}:{1}  background: {2}' -f
        [IO.Path]::GetRelativePath((Get-Location).Path, $f.FullName), $line, $val)
    }
  }
}

if ($bad.Count -gt 0) {
  Write-Output ("底色不透明度：红（{0} 个文件里扫到 {1} 处半透明底色）" -f $files.Count, $bad.Count)
  $bad | ForEach-Object { Write-Output "  · $_" }
  Write-Output ''
  Write-Output '  ⚠ Tauri 透明窗口 + WebView2 分层合成会把半透明底色**整层不绘制**，'
  Write-Output '    而浏览器里它画得好好的 ⇒ 这类错只有真机窗口能看见。'
  Write-Output '  改法：混**背后那张纸**（`color-mix(…, var(--cs-paper))`），不混 `transparent`。'
  Write-Output '    背后是别的颜色就混那个（例：红条里的 `code` 混 `var(--cs-live)`）。'
  Write-Output '  ⚠ 裸 `background: transparent` 是定案（面板根要透出撕口外的桌面），别改它。'
  exit 1
}

Write-Output ''
Write-Output ("底色不透明度：{0} 个文件，出货界面上没有半透明底色（裸 transparent 不算）" -f $files.Count)
exit 0
