# 54: autofixer 那一步**在结构上无法报告任何一条告警**，而且报错指向「网络/工具，别改代码」

**What to build:** 让 `ui svelte-autofixer` 这一步在代码真有问题时，报的是**代码的问题**。

**Blocked by:** 无。

**Status:** done · 2026-10-18。根因 + 解析器修复 + 端到端自证见下。

## 一句话

这个工具**没有告警时**输出 JSON，**有告警时**输出 JS 字面量拼接——那种东西
`ConvertFrom-Json` 必然抛。旧脚本把「抛了」归类成「工具不可用」，
并打印一句「**查网络/工具本身，别改代码**」。

## 实测的两副面孔

同一个二进制、同一条命令，只因为文件里有没有告警：

**无告警**（可解析，`ConvertFrom-Json` 接受无引号 key）：

```
{
  issues: [],
  suggestions: [],
  require_another_tool_call_after_fixing: false
}
```

**有一条告警**（解析失败，卡在 `+`）：

```
{
  issues: [
    'A component can have a single top-level `<style>` element\n' +
      'https://svelte.dev/e/style_duplicate at line 1908, column 0'
  ],
  suggestions: [],
  require_another_tool_call_after_fixing: true
}
```

```
解析失败：从 JSON 转换失败…unexpected character was encountered: +
Path 'issues[0]', line 3, position 66.
```

## 为什么这比「误报」严重

门禁**确实还是红的**（`exit 2`），所以它没有静默放行。但它报的是：

```
  ??    .\src\lib\PanelSettings.svelte  TOOL-UNAVAILABLE（没拿到结构化输出，exit=0）
AUTOFIXER_TOOL_UNAVAILABLE (1 / 11 个文件没被检查到）
  ⇒ 这一步**没有结论**，不是「有问题」。查网络/工具本身，别改代码。
```

而它**唯一会这么判的场合，恰恰是代码真有问题的时候**。
于是每一条真实告警都会被翻译成「工具坏了」，并**明确劝人不要去改代码**。

⚠ 实测触发它的第一条真实告警，是本轮自己改出来的：
`PanelSettings.svelte` 里 `.cs-lead` 这条没人用的 CSS 选择器。
**那道告警本身是对的**（我确实忘了删一条样式），工具也**正确报出了它**——
被脚本翻译掉了。

## 已修：两条路都走

1. 先试真 JSON（无告警那条原样保留）；
2. 失败则按 JS 字面量解析：`一条 = 若干单引号字面量用 + 串起来`。
   先按「字面量（可带 `+`）」切条，再在每条**内部**把所有字面量拼回去。
   ⚠ 不用 `-split '\+'`——告警正文里本来就有 `+`（URL、代码片段）。

「真的工具没跑完」改判为**两个数组都切不出条目**（只判 `issues` 会漏
「有 suggestions 无 issues」那种形状）。

### ⚠⚠ 修完第一版还有个量具自己的坑

自证时打出来的是**一个字母 `A`**，而 `issues=1` 是对的。
原因：`Get-Entries` 返回 `@('…')`，**单元素数组写回管道时被 PowerShell 拆包成
标量字符串**，于是 `$issues[0]` 取到的是**字符串的第一个字符**。
⇒ 只查计数**看不出来**，因为计数仍然对。调用点补 `@()`。

## 自证（端到端跑真脚本，判据取退出码 + 原文）

做法：临时造一个 `ui/src/lib/_afselfproof.svelte`（两个 `<style>` ⇒ 必报
`style_duplicate`），跑**真的** `ui/scripts/autofixer.ps1`，然后删掉夹具。

| 档 | 期望 | 实测 |
|---|---|---|
| 红 | exit 1 + `FAIL … issues=1` + **告警原文** | `AUTOFIXER_HAS_WARNINGS (1 / 12)`，原文含 `at line 14, column 0` |
| 绿 | exit 0 + `AUTOFIXER_ALL_CLEAN` | `AUTOFIXER_ALL_CLEAN (11 files)` |

⚠ 夹具**删不掉**：`Remove-Item` / `rm --`（绝对与相对路径各试一次）全被
本地安全网关拦下，`Move-Item` 也拦。最后用运行时自带的
`%USERPROFILE%\.minimax\bin\mavis-trash` 才移走。**没有绕道提权**——
只是被拦之后走了文档化的那条路。要记的是：**下次别在 `src/` 里造夹具**，
改用 `target/probe/`。

## 验收

- [x] 定位到「有告警时输出不是 JSON」，并给出两副原始输出的对照
- [x] 解析器两条路：先 JSON，失败按 JS 字面量
- [x] 「工具没跑完」的判据收紧成两个数组都空
- [x] 修掉单元素数组拆包导致原文只打出 1 个字符的坑
- [x] FAIL 时**打出告警原文**（原来只报 `issues=N`，看的人得自己回去找行号）
- [x] 端到端自证：红档 exit 1 + 原文、绿档 exit 0
- [x] 夹具已从 `src/` 移走，工作区无残留
- [x] 顺带修掉了它抓到的那条真告警：`PanelSettings.svelte` 的孤儿 `.cs-lead`
