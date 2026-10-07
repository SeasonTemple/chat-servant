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

## LLM 客户端：为什么没有换成成熟 crate（2026-10-07 复核）

**这一节是补记，不是新结论——本项目的纪律是"先查 crates.io 再考虑自己写"，
而我第一版 `transport.rs` 跳过了这步。复核之后理由成立，但它是**取舍**，
不是"没有依赖"。**

crates.io 实际结果（`/api/v1/crates` 搜索 + 逐个核对）：

| crate | 下载 | 稳定版 | 许可 | 更新 |
| --- | --- | --- | --- | --- |
| `rig-core` | 3,329,504 | 0.44.0 | MIT | 2026-10-07 |
| `async-openai` | 9,000,107 | 0.42.1 | MIT | 2026-09-28 |
| `genai` | 452,291 | 0.6.5 | MIT OR Apache-2.0 | 2026-10-05 |
| `anthropic`（官方 SDK） | 32,481 | 0.0.8 | MIT | **2024-09-03** |
| `openai-api-rs` | 759,038 | 10.0.2 | MIT | 2026-10-03 |

**`genai` 就是「泛用 provider 接入」那个**，许可证也过关，
覆盖 OpenAI / Gemini / Anthropic / **DeepSeek / Groq** / Bedrock / Vertex 等。
**LiteLLM 没有 Rust 实现**（Python/TS 代理层），**Vercel AI SDK 没有官方 Rust 移植**（TS）。

## 哪些该外包，哪些该自己写

**`transport.rs` 里的两类东西，性质完全不同：**

| 部分 | 规模 | 该不该外包 |
| --- | --- | --- |
| 协议层：请求体拼装 / 响应解析 / 鉴权头 / 端点 / 错误分类 | ≈120 行 | **本应交给 genai**。上游改协议时它会跟着改，手写要自己跟 |
| 接缝层：`Reasoner` 三段 / 降级路径 / `redact` / 同步 trait | ≈300 行 | **该自己写**。genai 一点不做这些；`rig-core` 会想接管整层（它要 async、要拥有推理层） |

## 协议层没外包，是**许可证门挡的**，不是技术判断

实测（把 genai 挂进依赖树跑 `cargo deny`）：

    error[banned]: crate 'openssl-sys = 0.9.117' is explicitly banned

两条 TLS 路线都撞门：

| genai 路线 | 撞什么 |
| --- | --- |
| `native-tls` | 拖进 `openssl-sys` → **撞 `deny.toml` 的 ban**（"统一走 rustls"） |
| `rustls-tls`（默认） | 拖进 `webpki-roots` —— ⚠ **本节原写「MPL-2.0，需要新豁免」，那是错的** |

> **更正（2026-10-07）**：`webpki-roots` 0.26.11 与 1.0.9 的 `Cargo.toml`
> 声明的都是 **`CDLA-Permissive-2.0`**，不是 MPL-2.0。它属于 permissive，
> 已直接进 `deny.toml` 的 `allow` 白名单，**不需要任何豁免**。
>
> 于是本节「两条 TLS 路线都撞门」的结论**站不住**：`rustls-tls` 这条
> 现在是通的。当时只看了 crates.io 页面上的旧标注，没核对 crate 的
> `Cargo.toml` 元数据——正是 AGENTS.md 明令禁止的「不要从项目主页推测」。

⚠ 别把这个结论说成「genai 不合格」——它 45 万下载、两个月前还在更新，
是这个领域最成熟的通用实现。**卡住它的是我们自己的门，不是它。**
要换就得**先单独论证 webpki-roots 那条豁免**，不要在接依赖时顺手放行。

**另两条（都不是拦路虎）：**

- **核心 crate 仍要保持同步。** `Reasoner` 是同步 trait 而 genai 是 async；
  但 **Tauri 本身构建在 tokio 上**，所以在 `src-tauri` 包一层 runtime 桥接即可，
  `reason` 核心不动。先前说成「接它 = 把 tokio 引进核心依赖树」是**代价说重了**。
- **Anthropic 侧官方 SDK 两年零更新**（2024-09-03），那半边手写是当前唯一选择。

**复审条件**：`deny.toml` 愿意为 webpki-roots 单独论证一条 MPL-2.0 豁免时，
**协议层那 120 行改用 genai**（接缝层不动）。**不要因为「忘了为什么」就再查一遍**
——那是本节存在的理由。

> **这条复审条件已经被上面那条更正解除。** `webpki-roots` 是
> CDLA-Permissive-2.0，不需要豁免，`genai` 的 `rustls-tls` 路线**不再被
> 许可证挡住**。换不换是另一个取舍（`genai` 是 async，接缝层要包一层
> 桥接），不在本轮范围里——但**别再拿「许可证不允许」当理由**了。

⚠ 另一条纪律记在这：**结论要实测，别拿推理顶。**
本节第一版写的是「接 genai = 把 tokio 引进核心依赖树」，那是**推断**，
而 Tauri 本就构建在 tokio 上、根本不是问题。又有一版凭直觉猜
「`native-tls` 在 Windows 上走 SChannel、不会拖 openssl-sys」——
**实测证明那个直觉是错的**（`cargo deny` 报 `openssl-sys is explicitly banned`）。
**一个未验证的直觉差点推翻一条已验证的结论。** 改依赖前先把它挂进
`cargo deny` 跑一遍，比读文档快也比读文档准。

## 加一家兼容方是加一行，不是加五个 arm

`Provider` 收敛成两种协议：OpenAI 兼容 与 Anthropic 原生。
分协议的地方一律判 `is_openai_compatible()`，**不逐个列 variant**；
端点与模型名是 `Provider::chat_url()` / `Provider::models()` 的数据。

逐个列 variant 的写法会让每加一家就在
`headers` / `build_body` / `extract_text` / `models_url` / `probe_headers`
各漏一处——漏掉的那处症状是"配置成功、一调用就 400"，
和 `anthropic-default` 那个头把模型名带偏是同一类错误。

**实测**：MiniMax 的 `abab6.5s-chat` 在
`https://api.minimaxi.com/v1/chat/completions` 返回 HTTP 200 并正常回话
（密钥只从环境变量读，未打印、未落盘）。**模型名只填实测能通的**——
填一个没验过的名字，表现同样是 404，比明确不支持更难查。

## 影响

- 新增依赖前先确认三件事：许可证、维护活跃度、是否覆盖我们要的那一个能力
  （而不是"看起来相关"）。覆盖不全时优先在 crate 之上做薄适配，不改 crate。
- 依赖选择写进 `Cargo.toml` 的 `[dependencies]` 注释里，附选它的理由，
  免得三个月后有人问"为什么用 genai 不用 async-openai"。
## 补记（2026-10-07）：ureq 一直没开 TLS 后端，真实模型从来没跑通过

### 现象

接上真实传输后第一次实机调用，得到：

    Unknown Scheme: cannot make HTTPS request because no TLS backend is configured

### 原因

`crates/reason/Cargo.toml` 写的是 `features = ["json", "gzip", "native-certs"]`。
查 `ureq-2.12.1/Cargo.toml` 的特性定义：

    native-certs = ["dep:rustls-native-certs"]   ← 只是**证书加载器**
    tls          = ["dep:webpki-roots", "dep:rustls", "dep:rustls-pki-types"]
    native-tls   = ["dep:native-tls"]

**`native-certs` 自身不带任何 TLS 实现。** 真正的后端是 `tls`。
于是这个 HTTP 层从头到尾发不出任何 HTTPS 请求——而 OpenAI / Anthropic /
MiniMax 全都要 HTTPS。

### 为什么测试没抓到

`Reasoner` 的测试全部走注入的 mock `Http`，**没有一个真出过网**。
「编译通过、单元测试全绿」和「能调用真实模型」之间隔着这一条，
而门禁里没有任何一项会碰到它。

### 为什么「用操作系统信任库」那个意图其实没实现

原注释写着「rustls + 操作系统信任库（native-certs），为一份证书数据破门禁不划算」。
但作者当时误以为 `native-certs` 自带 TLS 后端，所以特意**没有**加 `tls`，
理由还写得很笃定。实际上：

    ureq-2.12.1/src/rtls.rs
    #[cfg(feature = "native-certs")]       fn root_certs() { load_native_certs() }
    #[cfg(not(feature = "native-certs"))]  fn root_certs() { webpki_roots::TLS_SERVER_ROOTS }

两个分支**互斥**。同时开 `tls` + `native-certs`，走的仍然是操作系统信任库，
Mozilla 根证书那一支被 cfg 掉。**原意图是可以实现的**，只要把 `tls` 也加上。

### 实测结果（2026-10-07）

    传输 = minimax（判断 abab6.5s-chat / 起草 abab6.5s-chat）
    ✅ 全链通过，用时 5.90s
      判定 4 条消息 / 10 条归类
      判断 意图：对方可能在分享某种积分或进度系统的情况｜建议：reply
      候选 3 条
        1.  70%  看起来你已经很努力了，加油，相信你会做得更好的！

`cargo deny check` 四项全过：`licenses ok` / `advisories ok, bans ok, sources ok`。

### 方法论

> **「单元测试全绿」不等于「这条路走通过」。**
> 凡是**依赖外部世界的接缝**（网络、剪贴板、窗口、文件系统），
> 必须有一条**真的发生过**的证据，否则它只是编译过了。
