# ADR-0001：Rust 核心 + Tauri 2 + Svelte 5，而不是 egui

- 状态：已接受
- 日期：2026-10-06

## 背景

核心链路（窗口捕获、像素调理、OCR、Win32 输入注入）用 Rust 写，这没有争议。
有争议的是 GUI 层选什么。

最初倾向 `egui`/`eframe`：纯 Rust、单二进制、无 node 工具链、体积小。
这个推荐的推理是"Rust 性能取向"。

## 决策

**Tauri 2 外壳 + Svelte 5 前端，核心链路是纯 Rust，不经过 webview。**

## 为什么推翻

推理错在把性能预算用在了错误的层。逐层拆开看实际负载：

| 层 | 每秒工作量 | 是否在 UI 线程 |
| --- | --- | --- |
| WGC 捕获 | 数十帧 | 否，独立线程 |
| 帧调理 | 每帧数百万像素运算 | 否 |
| Windows.Media.Ocr | 单次 200–800ms | 否 |
| 模型调用 | 单次数百 ms 到数秒 | 否 |
| **UI 渲染** | **几行文字 + 几个按钮 + 一张 canvas** | **是** |

唯一在 UI 线程上的是渲染。它每秒要处理的工作量是几十个 DOM 节点和一张
`<canvas>`——这在任何栈上都是无负担的。为一个无负担的层牺牲开发效率，
是本末倒置。

而真正决定"使用体验"的恰恰是 UI 层：悬浮窗布局、三条候选卡片、设置页的
key 表单与模型选择器、中文排版、聊天记录滚动、调试视图的实时画框。
`egui` 做这些要多写 3–5 倍代码，做出来还是工具风。被替代的参考项目
（Python + PySide6）体验差的根因在交互设计，不在渲染性能。

参考项目还有一个证据：那 58 KB 的 `overlay.py` 里有相当篇幅在处理
PySide6 的事件循环和重绘抖动，这些工作量在 web 栈里是免费的。

## 备选方案

### egui / eframe

- 优点：单二进制、无 node 工具链、冷启动快、体积小、无 webview 依赖。
- 缺点：设置页和聊天记录 UI 成本高；中文 IME 需要额外处理；
  immediate mode 下高频重绘的调试画布反而不如 `<canvas>` 省心；
  成品观感是工具风。
- **真正的代价**：把开发时间花在 UI 样板上，而 UI 不是瓶颈也不是痛点。

### Slint

- 优点：声明式、原生观感好、纯 Rust。
- 缺点：生态和现成控件少于 web 栈；调试视图的 canvas 式绘制要自己搭。

### Tauri + React

- 与 Svelte 同构，选择 Svelte 的理由是更小、编译期反应式无虚拟 DOM，
  且有官方 MCP server 与 skills（`@sveltejs/mcp`，含 `svelte-autofixer`）
  可以在写码时自动校验。

## 代价（明确接受）

1. **引入 node 工具链**。`ui/` 需要 pnpm，CI 和本地都要装。
2. **依赖 WebView2 Evergreen 运行时**。Win10 1803+ / Win11 一般预装，
   但仍是外部依赖；缺失时需要引导安装或离线 bootstrapper。
3. **冷启动多几百毫秒**。可接受——这是常驻托盘工具，不是要秒开的 CLI。
4. **Windows 上透明置顶窗口有一组已知坑**，必须在实现时按下面处理：
   - 懒加载的窗口要写成 `visible: true` + 停在 `x: -10000`（屏幕外），
     **不能**写 `visible: false`。WebView2 只在窗口首次被 DWM 合成时注册
     OLE drop target，隐藏的窗口在拖拽开始时还没有 drop target。
   - `dragDropEnabled` 只在 `tauri.conf.json` 里声明才生效；设到 Rust
     `WebviewWindowBuilder` 上会被静默忽略（Tauri #13761）。
   - `decorations: false` 时 Tauri 不暴露圆角，需要自己调
     `DwmSetWindowAttribute(DWMWA_WINDOW_CORNER_PREFERENCE)`。
   - Acrylic/Mica 效果在 `hide()` → `show()` 之后会丢失，需要重新 apply。

## 影响

- 架构约束：**API key 不进前端**。所有网络调用在 Rust 侧，前端只拿结构化结果。
  这不只是安全考虑，也让 Tauri 的 capabilities 权限面保持最小。
- 双窗口结构：`main`（透明置顶悬浮窗）+ `settings`（`visible: false`，按需 show）。
  与 BongoCat 的形状一致。
