# 100: 「退出」那枚键的失败分支是空的，而「空的 catch 会藏 bug」从来没有机器守卫

**Status:** done · 2026-11-01 改成 `console.error`，并补上第一条机器守卫（2/2 验收项通过）

## 一、事实

`AGENTS.md` 的「容易踩的坑」里写着 **「空的 `catch {}` 会把这类 bug 藏到看不见」**，
但全仓**没有一条机器守卫**管它（`rg '空空 catch|empty catch' scripts/` 零命中）。

2026-11-01 核面板源码时发现两枚窗口键是**两种待遇**（`ui/src/lib/PaperPanel.svelte`）：

| 键 | 命令 | 失败分支 |
|---|---|---|
| `−` 收进托盘 | `hide_panel` | `console.error('收进托盘失败：', e)`（票 66，2026-10-18 改的） |
| `×` 退出 | `quit_app` | `.catch(() => {})` —— **一句原因都不打** |

⚠ 而 `×` 是**唯一**能让进程停下来的按钮：它失败时用户看到的是「按了没反应」，
日志里连一行解释都没有。这正是 AGENTS.md 那条坑描述的形状。

## 二、为什么当时没被抓到

- `clippy -D warnings` 不管 `.svelte`；
- `svelte-check` / `eslint` / `prettier` 都不把空 catch 当错误（`no-empty` 只覆盖 `catch {}`，
  不覆盖 `.catch(() => {})` 这种**回调**形态）；
- 票 66 只改了 `−` 那一支，**没有把判据写成守卫** ⇒ 另一支原样躺着。

## 三、改动

1. `ui/src/lib/PaperPanel.svelte`：`×` 那支改成
   `void invoke('quit_app').catch((e) => console.error('退出失败：', e))`，
   并写明为什么不能空（`quit_app` 自己不会失败——它直接 `app.exit(0)`——但 IPC 桥可以）。
2. `ui/src-tauri/src/commands.rs` 新增 `mod window_button_tests`（读面板源码，与既有
   `fixture_reply_tests` / `key_draft_tests` 同一套做法）：
   - `the_panel_never_swallows_a_failure_silently`：面板**任何**非注释行里出现
     `catch(() => {})` 即红，并打印行号与原行。
   - `both_window_buttons_say_why_they_failed`：`hide_panel` 与 `quit_app` 两处的失败分支
     都必须含 `console.error`。⚠ 判据不是「不是空的就行」——`catch(() => 0)` 在用户眼里一样。

## 四、验收

- [x] 反向自证：把 `quit_app` 那行**注入**回 `.catch(() => {})` ⇒ 两条守卫**同时红**
      （`panicked at ui\src-tauri\src\commands.rs:4066` / `:4044`，报的正是
      `1139:void invoke('quit_app').catch(() => {})}`）；写回后
      `cargo test -p chat-servant-app window_button_tests` = **2 passed**。
- [x] `cargo fmt --all` / `clippy -D warnings` / `dev.ps1 ui-check`（`AUTOFIXER_ALL_CLEAN (12 files)`）
      与完整 `gate.ps1` **ALL GREEN** 全过。

## 五、还剩下的判据边界

守卫只覆盖 **`PaperPanel.svelte`**。其余 `.svelte` / `.ts` 里的空 catch 没有机器管
（实测当前只有 `PanelSettings.svelte:401` 的 `.catch(() => proxyDraft)` 属于「有回退值」的
合法形态，不是空分支）。要扩面时按同一套读源码的做法加，**不要**上启发式——
「这个 catch 空得有没有道理」判不准，硬做成守卫只会得到一条永远绿着的假判据。
