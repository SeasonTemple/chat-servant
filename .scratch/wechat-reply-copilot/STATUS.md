# 进展现状（**机器生成，勿手改**）

> 由 `scripts/ticketstatus.py` 生成。改完票据就重跑：
> `python scripts/ticketstatus.py`（`--check` 只报漂移不写文件）。

票据总数 **64** ｜ 验收项全勾 **40** ｜ 已收但留有说明项 **14** ｜ 未收 **10**

⚠ **「勾数」不等于「进度」**：已收的票里仍可能有未勾项，那些是
**条件项或永久性说明**（例如夹具 gitignored、缺件即跳过）。
反过来，Status 行是**手写**的，会漂——2026-10-15 一晚之间就有三张票的
Status 与自己的验收项矛盾。**冲突时以本表和验收项为准。**

ℹ **分类看的是 Status 行 `·` 之前那个 triage 标签，不是散文**。
「已定位」「已修完」这类词出现在说明里**不代表这张票已收**——
曾经票 44 写着「进行中 …两项复核仍未做」，却被说明里的「已定位」
判成「不是欠的活」。措辞不是状态，标签才是。

## 一、未收的票（这里就是要干的活）

⚠ **这几张剩下的几乎都不是「写代码」**，是人的眼睛与手 —— 
点一次发送、截一张图、看一眼托盘。⚠ 别拿它们当 agent 能接的活。

| 票 | 勾/总 | Status | Blocked by |
|---|---|---|---|
| `20-two-dispatch-tiers.md` | 17/19 | ready-for-human · **现行决定日期 2026-10-15** | None（可以立刻做） |
| `21-readout-phase-never-sent.md` | 5/7 | ready-for-human · **P0 · 2026-10-12 实测，不是推断** | None（可以立刻做） |
| `35-small-stroke-ratio-is-not-a-legibility-criterion.md` | 4/7 | needs-triage · **2026-10-17：四个候选全部否完，可测的量已穷尽** | **要人眼**（真 WebView2 上并排四候选过一次） |
| `40-key-input-unreachable-when-unset.md` | 8/9 | ready-for-human · **2026-10-16 修复合入并过门禁（`976735e`，此后每轮十步全绿）；只剩一项要真 key 的端到端，那项卡在你本机提供的 key 上** | (票面没写 Blocked by) |
| `41-winrt-misdetections-are-bugs-not-a-product-decision.md` | 3/7 | ready-for-human · **2026-10-18：措辞订正——这票已不是「待做」** | (票面没写 Blocked by) |
| `44-candidate-action-row-is-button-geometry-on-prose.md` | 18/18 | ready-for-human · **2026-10-18 订正：只剩一条真正未决** | (票面没写 Blocked by) |
| `45-settings-screen-left-the-paper-family.md` | 27/27 | ready-for-human · **2026-10-18 订正：Status 里那个「新撞出的缺陷」已不存在** | (票面没写 Blocked by) |
| `48-frontend-only-sweep-silently-skipped-the-rust-side.md` | 3/4 | ready-for-human · **2026-10-16 记录，不是待办——要改的是「下一次接手时先划范围」这个习惯，不是代码。最后一条验收项故意不勾：它只能由人在下个会话开头核对** | (票面没写 Blocked by) |
| `50-ocr-quality-floor-has-no-machine-guard.md` | 5/6 | ready-for-agent | None（验收项 1、2 可立即做；验收项 4 的实测部分需要真帧在手） |
| `51-release-cannot-fetch-dict.md` | 18/20 | ready-for-human | 发版那一跳需要你授权打 tag（⚠ **打 tag 会真的发一个 release**）。 |

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
| `39-ppocr-silently-fell-back-to-winrt-all-along.md` | 9/10 | done · **2026-10-18 决策已下并上线**：默认引擎 = **PP-OCRv6 small**， |
| `53-dev-ps1-run-returns-output-and-exit-code.md` | 7/8 | done · 2026-10-18。根因与实测证据见下；四个动词已改，绿红两档都自证过。 |
| `63-tick-geometry-and-pill-material.md` | 9/10 | done · 2026-10-18 · 验收项 **10/10** |

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
| `34-tray-icon-looks-grey-on-dark-taskbar.md` | 6/6 |
| `36-message-roi-left-edge-collapsed-to-zero.md` | 0/0 |
| `37-seg-pill-narrower-than-active-item.md` | 0/0 |
| `38-measured-bubble-color-in-comments-no-longer-occurs.md` | 0/0 |
| `42-settings-page-still-native-controls-and-scrollbar.md` | 8/8 |
| `43-model-config-two-groups-custom-endpoint-proxy.md` | 9/9 |
| `46-judge-surface-rescans-the-same-rect-four-times.md` | 5/5 |
| `47-pp-ocr-plan-cache-has-a-recompile-race.md` | 5/5 |
| `49-pp-ocr-plan-cache-lifetime-and-det-lock.md` | 7/7 |
| `52-compilable-control-chars-slip-past-the-fffd-scan.md` | 6/6 |
| `54-autofixer-step-cannot-report-any-warning.md` | 8/8 |
| `55-machine-acceptance-exit-paths-silence-and-stale-checklist.md` | 9/9 |
| `56-version-lives-in-four-files-but-only-one-is-gated.md` | 11/11 |
| `57-settings-second-pass-review.md` | 9/9 |
| `58-settings-third-pass-hierarchy.md` | 10/10 |
| `59-semi-transparent-backgrounds-invisible-in-browser.md` | 8/8 |
| `60-tier-row-runs-off-paper-guard-measures-the-svg-not-the-paint.md` | 14/14 |
| `61-chip-selected-state-broke-consistency.md` | 9/9 |
| `62-settings-screen-three-inconsistencies.md` | 9/9 |
| `64-window-control-still-a-web-box.md` | 6/6 |

## 四、⚠ 脚本当场抓出来的漂移（Status 行自己写的数字与实点不符）

| 票 | Status 写的 | 机器点的 |
|---|---|---|
| `56-version-lives-in-four-files-but-only-one-is-gated.md` | 4/4 | 11/11 |
| `57-settings-second-pass-review.md` | 8/8 | 9/9 |
| `58-settings-third-pass-hierarchy.md` | 7/7 | 10/10 |
| `60-tier-row-runs-off-paper-guard-measures-the-svg-not-the-paint.md` | 10/10 | 14/14 |
| `61-chip-selected-state-broke-consistency.md` | 8/8 | 9/9 |
| `63-tick-geometry-and-pill-material.md` | 10/10 | 9/10 |

> 这些不是脚本猜的，是 Status 行里的 `N/M` 与实际勾选数量对不上。
> **按本表的数字读进度**，别按 Status 读。
