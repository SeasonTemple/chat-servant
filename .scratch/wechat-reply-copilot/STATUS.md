# 进展现状（**机器生成，勿手改**）

> 由 `scripts/ticketstatus.py` 生成。改完票据就重跑：
> `python scripts/ticketstatus.py`（`--check` 只报漂移不写文件）。

票据总数 **36** ｜ 验收项全勾 **21** ｜ 已收但留有说明项 **11** ｜ 未收 **4**

⚠ **「勾数」不等于「进度」**：已收的票里仍可能有未勾项，那些是
**条件项或永久性说明**（例如夹具 gitignored、缺件即跳过）。
反过来，Status 行是**手写**的，会漂——2026-10-15 一晚之间就有三张票的
Status 与自己的验收项矛盾。**冲突时以本表和验收项为准。**

## 一、未收的票（这里就是要干的活）

⚠ **这几张剩下的几乎都不是「写代码」**，是人的眼睛与手 —— 
点一次发送、截一张图、看一眼托盘。⚠ 别拿它们当 agent 能接的活。

| 票 | 勾/总 | Status | Blocked by |
|---|---|---|---|
| `20-two-dispatch-tiers.md` | 16/18 | ready-for-human · **现行决定日期 2026-10-15** | None（可以立刻做） |
| `21-readout-phase-never-sent.md` | 5/7 | ready-for-human · **P0 · 2026-10-12 实测，不是推断** | None（可以立刻做） |
| `34-tray-icon-looks-grey-on-dark-taskbar.md` | 4/6 | in-progress · **2026-10-14 载体已重新设计并定案（A′）· 正式图标已生成并入库**（`8eeb244`） | (票面没写 Blocked by) |
| `35-small-stroke-ratio-is-not-a-legibility-criterion.md` | 4/7 | needs-triage · **2026-10-15 连否三个候选后主动收手** | None（票 22 已把反例测出来了，可以立刻做） |

## 二、已收但留着说明项（**不是欠的活**）

| 票 | 勾/总 | Status |
|---|---|---|
| `01-probe-frame.md` | 6/7 | done（2026-10-07 实机复核。`probe frame` 的默认捕获路径在 |
| `02-probe-layout.md` | 4/7 | 部分 done（2026-10-07 对账。命令名与票面不一致；多 DPI 未实测） |
| `03-probe-ocr.md` | 7/8 | done（2026-10-07 实机复核。⚠ 命令名与票面写的不一致，见下） |
| `04-freeze-fixtures.md` | 4/5 | done（2026-10-07 对账。夹具只有**深色**两份，浅色与多 DPI 维度未做） |
| `06-conditioning-incremental-roi.md` | 6/7 | done |
| `09-input-injection-readback.md` | 6/7 | 部分 done（注入通路与回读已完成；候选取正文已在 ticket 10 接上， |
| `12-tauri-svelte-skeleton.md` | 7/9 | done（`emit_readout` 已在 ticket 16 接上真实数据；面板现在还会显示 |
| `15-vision-bridge.md` | 8/9 | done，但默认路径**不启用** |
| `17-settings-window-parked-offscreen.md` | 3/4 | 已定位 —— 触发对象已在 `6b2acbf` 随「设置搬进面板」删除 |
| `24-small-type-floor-remeasure.md` | 6/7 | done · **2026-10-14 量具重建 + 三档数据 + 字重对照，结论如下** |
| `33-attribution-thresholds-are-dark-theme-only.md` | 3/5 | done · 2026-10-14 真机浅色主题实测发现并已修 · 根因定位到具体数值 |

## 三、已收（验收项全勾）

| 票 | 勾/总 |
|---|---|
| `05-pipeline-seam-golden-tests.md` | 5/5 |
| `07-conditioning-mask-and-scale.md` | 8/8 |
| `08-delivery-state-machine.md` | 7/7 |
| `10-reasoning-seam.md` | 8/8 |
| `11-context-trim-and-lazy-summary.md` | 7/7 |
| `13-credentials-and-settings.md` | 8/8 |
| `14-overlay-candidates-and-dispatch-ui.md` | 11/11 |
| `16-e2e-full-chain.md` | 9/9 |
| `18-panel-stale-after-session-switch.md` | 6/6 |
| `19-mojibake-in-source-comments.md` | 5/5 |
| `22-handwritten-body-typeface.md` | 7/7 |
| `23-button-component-family.md` | 8/8 |
| `25-main-window-loads-deprecated-panel.md` | 4/4 |
| `26-contract-module-was-never-compiled.md` | 5/5 |
| `27-compact-mode-is-dead-code.md` | 5/5 |
| `28-two-event-names-never-emitted.md` | 5/5 |
| `29-app-icon-was-never-designed.md` | 3/3 |
| `30-real-frame-ocr-accuracy.md` | 9/9 |
| `31-perceive-has-no-relayout-duty.md` | 8/8 |
| `32-parked-panel-tells-a-lie.md` | 7/7 |
| `36-message-roi-left-edge-collapsed-to-zero.md` | 0/0 |

## 四、⚠ 脚本当场抓出来的漂移（Status 行自己写的数字与实点不符）

| 票 | Status 写的 | 机器点的 |
|---|---|---|
| `25-main-window-loads-deprecated-panel.md` | 5/5 | 4/4 |

> 这些不是脚本猜的，是 Status 行里的 `N/M` 与实际勾选数量对不上。
> **按本表的数字读进度**，别按 Status 读。
