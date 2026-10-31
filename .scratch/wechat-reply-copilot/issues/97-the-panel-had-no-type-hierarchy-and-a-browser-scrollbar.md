# 97: 面板归属行没有层级、状态句字号压过内容、滚动反馈是浏览器默认那条深灰柱

**Status:** ready-for-human · 2026-11-01 两轮排版修正后，第三轮按 2K 屏实看放大字号 ⇒ 代码与量测已过 ⇒ **剩下的是真机那一眼**

## 一、用户原话

> 不觉得奇怪吗这个设计？对话方名称和最新回复的文字大小没有差异 而所谓的提示文字却占了很大的范围且文字很大，也没有任何过渡效果

> 1.这个滚动条符合目前的设计风格吗？2.这个被挤到下面的观感、滚动的反馈都很差

> 字是否太小了 都比底部档位的文字小，2k屏看着都费劲
## 二、根因（两条，第一条是「写着 12px 却从来没生效过」）

### 2.1 名字与引文同号 —— `peerPx` 的默认值把 12px 顶掉了

`ui/src/lib/PaperPanel.svelte` 里 `peerPx` 原本是 `let peerPx = $state<number>(15)`，
而 `data-peer-px={peerPx ?? ''}` 是**恒设**属性 ⇒ 选择器
`[data-peer-px]:not([data-peer-px=''])` 里的 `calc(var(--peer-probe) * 1px)`
**在生产路径上永远命中** ⇒ `.cs-where` 写的 12px **一次都没生效过**。

⇒ 用户那句「名称和最新回复的文字大小没有差异」是**字面事实**，不是观感。

修法同 `data-draft-font` 的探针纪律：`peerPx` 默认改 `null`（探针不写就不设属性）。

⚠ **同族第二实例，本轮没动**：档位名 `.cs-pick-name` = `calc(11px * var(--tk, 1))`
而 `tierK` 默认 **1.35** ⇒ 实际 14.85px，11px 从未生效。不动的理由：设计规范 §4.3
记着 1.35 不是最终形态，且笔心对齐算式与字号绑定，动它要重量票 24 的 D 段。

### 2.2 滚动反馈是 WebView2 默认的深灰浮条

全文 grep `::-webkit-scrollbar` / `scrollbar-width` / `scrollbar-color` **零命中**；
`.cs-actions` 还为了宽度稳定常驻 `scrollbar-gutter: stable`（白占 15px 正文宽）。
引文侧则**没有行数上限**（一条公众号长消息 7 行 ⇒ 候选被挤到纸外）。

## 三、用户拍板（六问，全部作答）

| 问题 | 决定 |
|---|---|
| 层级 | **名字当标签、最后一句当内容**：2K 屏复看后定会话名 14px 疏排灰；引文 18px 墨色仿宋 |
| 状态句字号 | 先定 15px；2K 屏复看后升 **18px**（与引文同号，层级由颜色扛） |
| 过渡 | **只给正文位 160ms 淡入**，高度不参与动画（避免票 95「表头跳」复发） |
| 长引文 | **最多 3 行 + 省略号**，全文走 `title` |
| 滚动条 | **藏掉原生条，改纸边细指示线**（不占位，顺带撤 `scrollbar-gutter`） |
| 滚动反馈 | **上下双向渐隐 + 指示线随滚动位置进退** |

## 四、落地

- `ui/src/lib/PaperPanel.svelte`：`peerPx` 默认 `null`；`.cs-where` 14px、`.cs-where-quote` 18px / `-webkit-line-clamp: 3` + 标准 `line-clamp` 双写 / `title` 全文；四支状态句 18px、正文位 `min-height: 43px`；`animation: cs-draft-fade 160ms`；`scrollbar-width: none` + `::-webkit-scrollbar` 双写；`.cs-scroll-mark` 是 `.cs-body-col` 的绝对定位子元素；双向渐隐折进 `--fade-top` / `--fade-bottom` 两个变量。
- `docs/设计规范.md` 新增 §4.2c-quater（3 行夹取 + title / 藏原生条与纸边线 / 双向渐隐，含「为什么」）。
- `ui/static/demo/longquote.json`：末句 200 个**合成**字符，让 3 行夹取有真夹具覆盖（此前 12 张夹具共用同一组消息、末句最长 4 字 ⇒ 只能靠 DOM 注入验收）。
- `ui/scripts/probe-header.mjs`：加 `--scroll` 与 `--inject`；修一处**量具时序 bug**——它原先「滚 → 量 → 等 900ms → 截图」，而那 900ms 是量「停手淡出」用的，带着 screenshot 一起等 ⇒ **截图永远拍在归零之后**（第一版据此判断「线不存在」的判断是对的，但证据是量具自己造的）。现在截图前重新点一次滚动。

## 五、量测（父代理独立复现，不是听子代理报告）

| 量 | 改前 | 改后 |
|---|---|---|
| 四态 `.cs-draft` clientHeight | 47.5 / 47.5 / 47.5 / **79** | **39 / 39 / 39 / 39** |
| 状态句 | 21px，无过渡 | 15px，`cs-draft-fade 160ms`，scrollHeight 23 ≤ 39 不被裁 |
| 「对方最后一句」 | 15px（与名字同号） | 15px / FangSong / `rgb(20,26,31)` / clamp 3 |
| 会话名 | 15px（12px 从未生效） | **12px** |
| 候选容器 | `offsetWidth` ≠ `clientWidth`（滚动条占 15px） | **316 == 316** |
| 原生滚动条 | 有 | `barWidth 0` + `scrollbar-width: none` |
| 双向渐隐 | 只有底部 32px 且仅 `[data-more]` | 顶/中/底 = 0/32、32/32、32/0 |
| 长引文 | 无上限 | `quoteLines 2.99` / `quoteTitleLen 200`（夹具，不靠 inject） |

### 5.1 第三轮：2K 屏可读性（2026-11-01）

| 量 | 改前 | 改后 |
|---|---|---|
| 会话名 | 12px | **14px** |
| 引文 / 状态句 | 15px / 15px | **18px / 18px**（均大于档位名 14.85px） |
| 单行状态容器 | clientHeight 39 / scrollHeight 23 | **clientHeight 43 / scrollHeight 43**，不再裁切 |
| 长引文 | clamp 3 | 18px 下仍为 **2.99 行**，`title` 全文 200 字 |
| 候选 / 档位 | 正常 | 候选区吸收新增高度，档位 top **451.7** 不变 |

2× 像素密度截图：`target/probe/type-readable-2x/panel-drafted-drafted@2x.png`。

本轮门禁：`pwsh -NoProfile -File scripts\dev.ps1 ui-check` ⇒ 三步 exit=0、
`AUTOFIXER_ALL_CLEAN (12 files)`；`-Filter the_demo_fixtures` ⇒ 3 passed（新夹具不破 Rust 守卫）。

## 六、指示线：从「量得到、看不见」到达标

判据是**合成后的对比度 ≥ 3:1**（WCAG 1.4.11 非文本元件），不是「看着有」。

- 第一版 `--cs-pencil` rgb(76,90,99) @0.55 ⇒ 合成 rgb(151,153,141)，对纸底 **2.33:1** ⇒ 不达标。
- 现 `--cs-pencil-2` rgb(66,80,89) @0.9 ⇒ 合成 rgb(84,95,99)，**5.29:1**（用户取样纸底上 5.54:1；
  落到撕口那种暗纸 rgb(232,222,196) 上 **4.97:1**）。三条纸底都过 3:1。
- 宽度：先 3px，**用户看图后定案改 2px 发丝线**（「仍读成一根细滚动条」）。
  ⚠ **粗细不参与对比度判据**——2px 与 3px 用的是同一个合成色。

## 七、验收

- [x] 会话名 14px 疏排 / 引文 18px 墨色（2K 屏实看后放大）
- [x] 单行正文位等高 43px，状态句不被裁；失败提示可自然换行
- [x] 正文位 160ms 淡入，高度不参与动画
- [x] 引文 3 行夹取 + `title` 全文（`longquote` 夹具，不靠 DOM 注入）
- [x] 两个滚动容器都无原生滚动条，`scrollbar-gutter` 已撤
- [x] 指示线对比 ≥3:1、宽 2px、仅滚动中出现、停手 800ms 淡出
- [x] 双向渐隐随滚动位置进退，`forced-colors` 下无条件撤 mask
- [x] `ui-check` 三步 OK；夹具守卫 `the_demo_fixtures_*` 仍绿
- [ ] **真机那一眼**：面板在自己屏幕上「名字 / 引文 / 状态句」的层级读着顺不顺，
      以及滚动时那道 2px 线是否刚好（纸纹深处会不会断）
