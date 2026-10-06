# ADR-0003：依赖策略——先找轮子，净室继承

- 状态：已接受
- 日期：2026-10-06

## 背景

初期设计中，帧调理管线的变化检测、增量 ROI、LLM provider 客户端，
我都是按「自己写一遍」在设计。这是错的两个方向：

1. Win32 / WGC / LLM provider 这几块，成熟 crate 已经踩完了坑。
2. 自己写等于自己维护一堆 Windows 版本差异和 provider 的协议细节。

但同时有另一头的约束：参考项目不能直接抄。

## 决策

**任何"我要自己写"的冲动落地前，先在 crates.io 找成熟 crate。**
例外只有三类：项目特有的领域逻辑、成熟 crate 明确没覆盖的能力、
以及需要绕开上游 bug 的最小补丁（要先确认是 bug 而不是设计）。

**许可证策略：只允许 permissive，任何带来商业限制的一律不用。**

这是硬约束，不是偏好。参考项目的教训写在这里：它自己的代码是 MIT，
但发布包捆绑了 GPLv3 的 UI 组件，整个分发包因此受 GPLv3 约束。
不要重复这个错误。

| 允许 | 拒绝 |
| --- | --- |
| MIT · Apache-2.0（含 WITH LLVM-exception） · BSD-2/3 · ISC · Zlib · 0BSD · Unicode-3.0 · Unicode-DFS-2016 · BSL-1.0 · CC0-1.0 | GPL / AGPL / LGPL / MPL 及任何 copyleft |
| — | 源码可见但限制商业使用的（BUSL、SSPL、Elastic License、"非商业免费"） |
| — | 没声明许可证、或有 LICENSE 但识别不出来的 |

**不靠自觉，靠机器执行。** 根目录的 `deny.toml` 用 `cargo-deny` 落实这条策略：
没声明许可证的 crate 直接失败（`unlicensed = "deny"`），许可正文识别置信度
门槛 0.92，只允许上面那张表。

`cargo deny check` 是**七道门禁之一**，和 `clippy` / `fmt` / `test` 并列。
依赖树里悄悄混进一个 GPL crate，必须当场报红，而不是等发版前才发现。

## 一个不在 cargo 依赖树里、但同样要注意的东西

**WebView2 Runtime**（Tauri 的 Windows webview 宿主）是微软在运行时下载/预装的
独立可再分发组件，不进 `Cargo.toml`，所以 `cargo deny` 查不到它。
Evergreen 版本免费可再分发，但如果将来要做离线分发包，需要额外确认
当时的再分发条款。这条记在这里，避免"门禁全绿"给了错误的安心感。

## 净室要求（不是法律要求，是风险规避）

参考项目 `examples/jev-chat-windows` 的 `LICENSE` 是 **MIT**——单看授权，
复制代码是允许的。但它的 `NOTICE` 里写明：Windows 发布包捆绑了
**PySide6-Fluent-Widgets，该组件是 GPLv3**，"因此发布包含整体受 GPLv3 约束"。

这意味着 **MIT 标记的源文件未必是干净的**，上游可能已经引入 GPL 污染。
复制任何一行都可能把 GPL 义务带进本项目。

所以规则是：

| 允许 | 禁止 |
| --- | --- |
| 读它的**实测结论**：颜色阈值、对比度区间、锚点逻辑、OCR 参数、踩过的坑 | 复制任何一行的代码或注释 |
| 读它的 `docs/` 和 README 里的行为描述 | 沿用它的类名/函数名结构 |
| 独立重新实现，并写下自己重新标定的理由 | 用它的名称、域名做宣传 |

事实和实测数据不受版权保护，这是净室能成立的前提。

## crate 选型（2026-10-06 核实）

| 能力 | crate | 版本 / 许可 | 结论 |
| --- | --- | --- | --- |
| WGC 窗口捕获 | `windows-capture` | 2.0.0，MIT OR Apache-2.0 | **首选** |
| WGC 窗口捕获 | `wgc` | 2.0.1，MIT OR Apache-2.0 | 备选，接口更薄 |
| Win32 窗口枚举 / DPI / 剪贴板 / 输入注入 / DWM | `windows`（windows-rs） | 0.5x，MIT OR Apache-2.0 | **首选** |
| LLM provider 客户端 | `genai` | 稳定版 0.6.5，MIT OR Apache-2.0 | **首选** |
| 图像编解码与缩放 | `image` | MIT OR Apache-2.0 | **首选** |
| Tauri 外壳 | `tauri` 2.x | MIT OR Apache-2.0 | 已定，见 ADR-0001 |

许可证均取自 crates.io 实际元数据，不是从项目主页推测。
传递依赖树的最终结论以 `cargo deny check licenses` 为准。

**明确排除**：

- `rig-core`（0.42.0）——8.7k star、生产用户一堆，但它是个**完整 agent 框架**，
  附带 vector store（RAG）、typed tool use、agentic workflow、音频/图像生成。
  我们只要"两个模型角色 + 结构化 JSON 输出"。这些全是死重量。
- `llmrust` —— 已被 yank，且自述"很年轻"。
- `llmprism` —— pre-1.0，13 个 provider 按 feature flag 切分的思路不错，
  但稳定性不如 `genai`。列为备选。

## 三个具体发现

**1. `windows-capture` 有 `DirtyRegionSettings`，直接给出变化区域。**

我原本打算自己设计"变化门控 + 增量 ROI"的脏矩形逻辑——也就是比较上一帧
哪些像素变了、在哪一块变了。这个 crate 原生提供。ADR-0002 的第 1、2 级
从"要自己写"降级成"配一个开关 + 接一个回调"。

**2. `genai` 自带图像分析支持和自定义 `AuthResolver`。**

我们的两个模型角色（A 决策、B 起草）正好对上它的模型→provider 自动映射，
且 API key 走 `AuthResolver` 注入——这直接满足"key 不落普通文件、不进日志"，
不用自己写凭据层。

**3. `windows-capture` 的作者也是 Python 版 `windows-capture` 的作者。**

参考项目用的就是它的 Python 版。所以我们用的是**同一条上游链路**，
迁移到 Rust 版不存在"换了个实现导致行为不同"的风险。

## 仍然要自己写的部分

这些没有成熟 crate，或者 crate 的抽象与我们的领域不合：

- `PaneAnchor`——像素锚定推导消息区，是本项目特有的领域逻辑
- `FrameConditioner` 的语义遮罩规则——同上
- `Attribution` 的说话人/噪声判定——同上
- `DeliveryReceipt` 状态机与派发策略——安全不变量，必须自己锁住

## 影响

- 新增依赖前先确认三件事：许可证、维护活跃度、是否覆盖我们要的那一个能力
  （而不是"看起来相关"）。覆盖不全时优先在 crate 之上做薄适配，不改 crate。
- 依赖选择写进 `Cargo.toml` 的 `[dependencies]` 注释里，附选它的理由，
  免得三个月后有人问"为什么用 genai 不用 async-openai"。