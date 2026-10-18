# 59: 半透明底色 —— 一条写着「硬约束」的规则，破了七处，而**浏览器查不出来**

**What to fix:** `ui/src/app.css` 文件头写着一条硬约束：
「**所有底色必须完全不透明**（2026-10-07 实测）：Tauri 的透明窗口走
WebView2 分层合成，子元素上的 `rgb(... / 88%)` 这类半透明底色**整层不绘制**」。

2026-10-18 全量扫了一遍：**七处违反**，其中**两处是本轮刚加的**。

**Status:** done · 2026-10-18 · **验收项 8/8**（守卫自证 **8/8**）

## 为什么这类错特别危险

**`?demo=1` 的出图评审、`svelte-check`、`pnpm build`、`autofixer`
全在 Chromium 里跑** —— 那里半透明底色画得好好的，分毫不差。
⇒ 这条硬约束违反之后，**本仓所有自动化手段都是绿的**，
只有真机 Tauri 窗口能看见。

⚠ 与票 58 记的量具坑同一族：「出图看不见 = 没改」的反面是
「出图看得见 = 改对了」——**后者同样不成立**。

## 七处违反（`grep -n 'background:.*transparent\|/ [0-9]*%'` 全量得到）

| 位置 | 原写法 | 改后 |
|---|---|---|
| `PanelSettings.svelte` `.cs-fail` | `color-mix(live 20%, transparent)` | `color-mix(live 20%, var(--cs-paper))` |
| `PanelSettings.svelte` `.cs-badge.ok` | `rgb(20 26 31 / 7%)` | `color-mix(ink 7%, var(--cs-paper))` |
| `PanelSettings.svelte` `.cs-badge.ok.env` | `rgb(20 26 31 / 5%)` | `color-mix(ink 5%, var(--cs-paper))` |
| `PaperPanel.svelte` `.cs-glyph:hover` | `rgb(20 26 31 / 7%)` | `color-mix(ink 7%, var(--cs-paper))` |
| `PaperPanel.svelte` `.cs-glyph.on` | `rgb(20 26 31 / 12%)` | `color-mix(ink 12%, var(--cs-paper))` |
| `routes/(panel)/+layout.svelte` `.cs-error code` | `rgb(255 255 255 / 22%)` | `color-mix(#fff 22%, var(--cs-live))` |
| `app.css` 滚动条拇指 | `color-mix(ink-2 42%, transparent)` | `color-mix(ink-2 42%, var(--cs-paper))` |

⚠ **混比一个都没改**（`.cs-fail` 的 12%→20% 是上一轮票 58 ⑦ 定的，
与本票无关）—— 改的只有**混色对象**：`transparent` → 背后那张纸。
浏览器里逐像素等价，真机里不再依赖那条 2026-10-07 的实测结论。

⚠ 最后一处**背后不是纸**：`.cs-error code` 坐在**不透明的红色条**上，
所以混 `var(--cs-live)` 而不是 `--cs-paper`。
⇒ 「不透明」不等于「一律混纸」，判据是**背后是什么**。

## ⚠ 不在范围内的（有意排除，不是漏了）

- `routes/studio/`、`routes/debug/` —— **独立页面**，自带深底，
  **不画在那个 Tauri 透明窗口里**，那条约束对它们不成立。
- `box-shadow` / `filter: drop-shadow(...)` 里的 alpha —— **不是底色**。
  守卫的判据只扫 `background` / `background-color` 的**值**。
  ⚠ 范围一旦放大到「所有 alpha」，这些正在用的样式会天天误报，
  而**守卫一旦天天误报就等于没有**。
- `background: transparent`（裸）—— **定案**：面板根要透出撕口外的桌面。
  `app.css` 里明确写过「真正要避开的不是 transparent，是半透明底色」，
  守卫把它写成判据而不是靠人记。

## 新增守卫 `scripts/check-opaque-bg.ps1`（挂 gate + CI Gate 5b）

范围按**不变量**选（画在透明窗口里的东西），不是按出事的文件：

```
ui/src/app.css · ui/src/lib/** · ui/src/routes/(panel)/**
```

排除项在脚本头部**点名列出**，下一个读脚本的人能知道「不是漏了」。

### ⚠ 守卫自证踩到的三个坑，每一个都表现为「全绿」

**① 断言自己错了，代码没错。** 第一版用 `/\s*[\d.]+\s*%` 当 alpha，
结果把这一行判成违规：

```
background: url('/probe-sheet.svg') center / 100% 100% no-repeat
```

那个 `/` 是 `background-position / background-size` 的简写分隔符，
**不是 alpha**。⇒ 三个模式都改成要求 `/` 出现在 `rgb(` / `hsl(` **内部**。

**② `[regex]@'…'@ -replace …` 会静默废掉正则。**
`-replace` 作用在 **Regex 对象**上，而
`System.Text.RegularExpressions.Regex.ToString()` 返回的是
**选项串**（`IgnoreCase`），**不是 pattern**
⇒ 正则被换成字面量 `IgnoreCase`，**什么都不匹配、全程绿**。
自证里 A1 / A2 / B2 三组「注入违规却判绿」就是这么来的。

**③ 单引号 here-string 里的 `#` 是字面量，不是注释。**
把 `# rgb(0 0 0 / 7%)` 这类说明留在 pattern 内部，
会要求输入里出现那行注释，匹配同样归零。

⇒ 三条都收敛到同一个写法：**判据写成一行、注释写在外面、
最后才 `[regex]::new()`**。

### 判据有**两条轴**，用例表也按轴写

| 组 | 轴 | 期望 | 实得 |
|---|---|---|---|
| A1 `rgb(20 26 31 / 7%)` | 形状 | 红 | 红 ✅ |
| A2 `color-mix(…, transparent)` | 形状 | 红 | 红 ✅ |
| A3 裸 `background: transparent`（定案） | 形状 | 绿 | 绿 ✅ |
| A4 违规写在 CSS 注释里 | 形状 | 绿（须剥注释） | 绿 ✅ |
| A5 `url()` 简写的斜杠 | 形状 | 绿 | 绿 ✅ |
| B1 违规在 `routes/debug/` | 位置 | 绿（不在范围） | 绿 ✅ |
| B2 违规在 `lib/` 下**新增文件** | 位置 | 红 | 红 ✅ |
| B3 范围文件读不到 | 位置 | 红 | 红 ✅ |

⚠ A4 与 B1 是**防误报**的方向：只测「注入违规 ⇒ 红」的话，
一个「什么都报红」的守卫同样能全过。
⚠ B2 守的是**范围按目录推、不按文件名硬编码**——
否则下一个新增组件的人不会想起来把文件名加进守卫。
⚠ B3 守的是「读不到 = 红」：空集合会「0 个问题」然后放行，
那与「扫过了没问题」完全同形。

### ⚠ 台架自己也坏过一次

第一版自证汇总永远打印 `SELFPROOF 0/0` 然后 `exit 0` ——
`$results +=` 写在 `function` 里改的是**函数自己的作用域**。
⇒ 现在用 `ArrayList` + `[void]$results.Add(...)`，
并加了一条：**0 组用例也判红**（「一组都没跑到」是台架坏了，不是守卫过了）。

同一轮还修了 `-Root` 参数：脚本没有 `param` 块时，
PowerShell 把 `-Root=E:\x` 拆成**两个** `$args` 条目，
扫 `$args` 找 `-Root=` 谁都匹配不上 ⇒ **静默退回仓库根**，
于是「测的是夹具」变成「测的是真仓库」——
**这种守卫比没有守卫更坏：它给全绿，而测的根本不是那棵树**。
现在改成真的 `param([string]$Root)`，并让台架每组先打印传进去的树路径。

## 验收

- [x] 全量扫出货界面上的半透明底色（7 处）
- [x] 7 处全部改成「混背后那张纸」，**混比一个没动**
- [x] `.cs-error code` 混的是红条不是纸（判据是**背后是什么**）
- [x] 新增 `scripts/check-opaque-bg.ps1`，范围按不变量选，排除项点名列出
- [x] 挂 `gate.ps1`（新步 `opaque backgrounds`）+ `ci.yml` Gate 5b +
      `check-gate-parity.ps1` 见证表 —— parity 守卫 exit 0（15 步全对齐）
- [x] 守卫自证 **8/8**，覆盖形状与位置两条轴，红绿两个方向都测
- [x] 修掉守卫自己的三个 bug（简写斜杠误报 / `Regex.ToString()` 废掉正则 /
      here-string 里的 `#` 是字面量）
- [x] 修掉台架自己的两个 bug（作用域导致恒 0/0 恒绿 / `-Root` 静默失效）

## 仍然未结

- ⚠ 那条硬约束的原始实测是 **2026-10-07**，本轮**没有**在真机 Tauri
  窗口上复验它是否仍然成立（需要起壳子，本轮没做）。
  ⇒ 这次改成不透明，**不再依赖那条结论**，但「它是否仍然成立」这个
  问题本身还没答。
- ⚠ `.cs-glyph` 的 `box-shadow: inset …` 描边是**不透明**色，没问题；
  但**阴影**本身在透明窗口里能不能画、画出来是什么样，同样没复验。
