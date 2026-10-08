# chat-servant 交接文档

> **给接手的人（尤其是云端会话）**：这是一份**冷启动说明**。读完它你应该知道
> 现在能干什么、哪些路已经被证伪、下一刀切哪里，以及**每条命令怎么跑**。
>
> 文档里所有「实测」结论都带日期和证据来源。没写「实测」的是推断，别当事实用。
>
> 最后更新：2026-10-09

---

## 0. 三十秒摘要

Windows 桌面程序：在你自己的微信单聊窗口旁边挂一块悬浮面板，读取对方新消息，
调模型起草 2~3 条回复，落在面板上，你点一下填进输入框，再点一次才发。
**默认档只填不发。**

全链路已在真机上跑通并**真的发出去一条消息**。现在卡在三个点上：
界面还要再设计一轮、OCR 识别率不够、要一个能在没有微信的情况下改代码的姿势。

---

## 1. 环境边界（先读这条，能省掉一小时）

### 1.1 云端能做 / 不能做

| | 状态 |
|---|---|
| UI 设计迭代 | ✅ **可以完全在云端做**。Svelte 5 + CSS，纯静态，见 §5 |
| OCR 层（`chatservant-capture` / `chatservant-perceive`） | ⚠️ **只能写代码，不能编译** |
| Tauri 外壳（`ui/src-tauri`） | ❌ 云端编译必挂 |
| 微信实机验证 | ❌ 只能在本机做 |

**为什么**：`crates/capture` 整个建在 `windows-rs` 上——
WGC 抓帧（`wgc_alt.rs`）、WinRT OCR（`ocr.rs`）、UI Automation、`SendInput`
（`inject.rs`），每一样都是 Windows 专有 API。Linux 沙箱里
`cargo build --workspace` 会在 capture 这一层直接挂。

**所以合理的分工是**：

```
云端 LLM 写代码  ──►  源码上传  ──►  本机跑门禁 + 实机验证  ──►  结论回传
```

云端能写、不能验。凡是「跑一下才知道对不对」的东西，都得回到本机。

### 1.2 本机环境

本文里所有命令都**相对于仓库根目录**跑，不写死绝对路径——换台机器就照抄即可。

```
工具链       Rust（见 rust-toolchain 或 Cargo.toml 的 rust-version）
             Node.js ≥ 18 + pnpm（前端）
shell        PowerShell（**复杂命令一律写 .ps1 用 pwsh -File 跑**，
             不要用 -Command 拼 —— 引号会被吃掉，Set-Content 还会写 BOM）
模型 key     环境变量 MINIMAX_CN_API_KEY
             （值只存在于本机环境变量里，**任何文档、日志、提交信息都不要写它**；
              OPENAI_API_KEY / ANTHROPIC_API_KEY 未设置）
```

⚠️ **磁盘**：`target/` 曾经涨到 33GB（`incremental` 13GB + `deps` 17GB，
全是反复热重编造成的）。已在 2026-10-09 `cargo clean` 清掉，仓库现为 0.27GB。
下次开工前考虑设 `CARGO_INCREMENTAL=0`，别再让它涨回去。

---

## 2. 现在能干什么（全部实机验证过）

### 2.1 全链路

```text
真实微信画面
  → WGC 抓帧（不受遮挡影响，实测 6/9 采样点被盖住照样跑通）
  → 整帧 2x 最近邻放大 → Windows.Media.Ocr 离线识别
  → 逐行归类（我 / 对方 / 标题 / 碎片 / 图片内字）
  → 头部 400x40 像素指纹锁会话
  → 真实 MiniMax 模型：判断意图 → 起草候选 → 按概率排序
  → Tauri 事件推送 → Svelte 5 面板渲染（GSAP 动效）
  → 点「填入」：剪贴板字节级往返 + 和弦 Ctrl+V → 落在微信输入框
  → 点「确认发送」：抢焦点 + 回车
```

实机记录（「文件传输助手」会话，真实模型，2026-10-09）：
认出会话名 → 归并 6 条消息 → 3 条候选 → 填入输入框 → **真的发出了一条**，
内容与候选一字不差、且只有一份。

### 2.2 性能（1073 轮统计）

| 场景 | 耗时 |
|---|---|
| 静默轮（画面没变，零模型调用） | 1.07 ~ 1.10 s |
| 起草轮（含三次**串行**模型调用） | 6.2 ~ 9.1 s |
| 进程启动 → 第一份 readout | 6.0 ~ 7.8 s |

**瓶颈是三次串行模型调用**，不是抓帧也不是 OCR。砍法见 §4.1。

### 2.3 三条不靠微信的验证姿势

这是本项目最重要的基础设施，**接手后请优先用它们，不要再去人肉点微信**。

#### 夹具帧模式（整个应用跑在冻结画面上）

```powershell
cargo xtask probe frame --method wgc          # 抓一张真实帧到 target/probe/
pwsh -File scripts/run-app-fixture.ps1 target\probe\frame-XXXX.png
```

`CHATSERVANT_FIXTURE=<png>` 一设，OCR、归类、会话锁定、起草、事件推送、
界面渲染**一条都不少**，只有「这一帧从哪来」变了。

实现见 `ui/src-tauri/src/watch.rs` 的 `FrameSource` / `load_frame`，
以及 `crates/capture/src/lib.rs` 的 `frame_from_png`。

**为什么它存在**：原来循环里写死了「找微信窗口 + WGC」，于是改一次界面就得去点
一次微信、截一次图、确认一次像素，而这些步骤每次结果都不一样——
「界面能不能跑通」这件事**没法被可靠验证**。

#### 探针命令（对 PNG 重放）

```powershell
cargo xtask ocr <png> --scale 2     # 只跑 OCR，逐行 + **逐词间隙**
cargo xtask perceive --file <png>  # 走生产接缝，一路跑到归并后的消息流
cargo xtask measure --file <png>   # 标定：底色/墨色/对比/位置
cargo xtask layout  --file <png>   # 取证帧的像素结构
cargo xtask media   <png>          # 媒体区域检测
cargo xtask e2e                    # 八段全链（含注入回读）
cargo xtask fixture --name X --source <png>   # 生成夹具 + manifest
```

`ocr --scale 2` 会打印**每一对相邻词的间隙和字高比值**——改任何跟空格、
断行、分栏有关的逻辑之前，先看这个数，别拍阈值。

#### `?demo` 模式（界面脱离 Tauri 跑）

```
https://nme1e0uxqtn1y.space.mcode.cn/?demo
```

`ui/static/readout.demo.json` 是**手写的演示数据**（假内容，形状仿真真实 OCR 输出）。
`+page.svelte` 见到 URL 带 `demo` 就读它、**不碰 Tauri、不注册任何监听器**。

演示模式下工具栏是禁用的、点候选会明说「不会真的发送」——不装假控件。
**云端改 UI 就在这个页面上迭代。**

---

## 3. 已经证伪的路（别重复踩）

这一节比前面的都重要。以下每一条都是**实机撞出来的**，不是理论推测。

### 3.1 `WM_PASTE` 投给子窗口 —— 不可行

微信 4 顶层窗口下只有 **2 个子 HWND**，全是 Qt 的，没有原生 `Edit`：

```
0x30FA0  class=Qt51514QWindowIcon  1x1  text='Weixin'
```

Qt 应用只创建一个顶层 HWND，所有控件由 Qt 自己绘制——**没有投递目标**。

证据脚本：`scripts/probe-input.ps1`

### 3.2 UI Automation `SetValue` —— 可用，但**抢不掉也躲不掉**

```
[71] Edit class=mmui::ChatInputField rect=(1214,763,819x209) [Value,Text,Invoke]
[100] Button '发送' class=mmui::XOutlineButton
[104] List  '消息' class=mmui::RecyclerListView                [Value,Invoke,Selection]
[129] List  '会话' class=mmui::XTableView                      [Value,Invoke,Selection]
```

实机写入（Chrome 在前台）：

```
写入前，前台窗口 = 0x10ECA  '...bilibili... - Google Chrome'
输入框现在内容 = '后台写入验证'
写入后，前台窗口 = 0x30F6C  '微信'          ← 微信自己激活了
```

**结论：「写进微信自己的输入框」本身就会抢前台**，换哪条 API 都一样——
这不是投递方式的问题，是那个控件的语义（给输入框赋值 → `SetFocus`）。

完整推理见 `docs/adr/0005-delivery-and-focus.md`。

证据脚本：`scripts/uia-set.ps1`、`scripts/dump-uia.ps1`

### 3.3 OCR 不能裁紧 —— 「乐」气泡被整段丢掉

同一块画面：

```
整帧 2x        5 行，读到「乐」（对方消息）
ROI 带子 2x   4 行，读不到「乐」
```

`Windows.Media.Ocr` **需要行周围有内容才收**，裁得越紧漏得越多。
现方案：**永远 OCR 整帧（2x 最近邻），行框换算到消息区坐标后再按 ROI 过滤**。
不要改回裁 ROI 带子。

### 3.4 空格阈值不要拍脑袋

Windows OCR 把中文**逐字切成独立 word**，`OcrLine.Text()` 一律用空格拼，
于是「古文山」变成「古 文 山」。现在改成按几何间隙决定补不补空格。

阈值 **1/2 字高**，依据是 `xtask ocr --scale 2` 在真实帧上量到的分布：

| 场景 | 间隙/字高 |
|---|---|
| 相邻汉字（文│件│传│输│助│手） | 0.00 ~ 0.08 |
| 中英紧排（卜│a2025│全） | 0.10 |
| 误识别连排（1│g│3） | 0.33 |
| 真空格（`口│.tgz`） | **1.18** |
| 真空格（`]`│`h`） | **1.73** |

0.33 与 1.18 之间是**大空档**，0.5 正中间。
⚠️ 曾经降到 1/3 去迁就一个**自己编的测试几何**，结果真帧上「B0」被拼成「B 0」。
`crates/capture/src/ocr.rs` 里七条测试全部用**实测几何**，别改成等宽方块。

### 3.5 `probe frame` 不要用 `printwindow`

`PrintWindow` 对走 DirectComposition 的窗口（WebView2）返回**黑帧**。
整屏截图才是真相。`scripts/shotwin.ps1` 会骗人。

### 3.6 WGC 会话别每帧新建

每帧新建整套 WGC 会话会**泄漏 12 个线程/帧**（实测增速 9 线程/秒）。
现在是一条长驻工作线程 + 一个会话（`wgc_alt.rs` 的 `worker()`）。
改这块之前先跑 `cargo xtask thread-probe --n N`。

### 3.7 别用 `Set-Content -Encoding UTF8` 改源码

PowerShell 5.1 下它会**写 BOM**。这次它把 BOM 加到了 `+page.svelte` 第一行：
prettier 容忍、`svelte-check` 也过，**只有 svelte-autofixer 报错**，
而且报的是 `at line undefined` 这种**没有位置的错**——差点被当成工具抽风。

排查手法：把文件 `git stash` 掉再跑 autofixer，二分定位是不是自己刚写进去的。
改源码一律用 `edit` / `write` 工具。

---

## 4. 接下来该做什么

按「价值 ÷ 风险」排序。前两项在云端就能做。

### 4.1 档位重构：三档 → 两档

用户 2026-10-09 提出：「按理说只需要两档：只填不发、自动填发」。这个直觉是对的，
但真正的问题不是档数，是**「档位」和「动作」被混成了一个东西**。

⚠️ 这条**和 §5 的轨道 B（契约重构）是同一件事的两种说法**：
档位之所以能变成三档、Confirm 之所以会变成死胡同，
根源就是「什么状态存在哪一层」没有定义清楚。要改档位语义，
得连契约一起改，不要只改 `TierSwitch` 的按钮。

| 现在 | 实际行为 |
|---|---|
| Manual | 程序永远不发，你自己在微信里按回车 |
| Confirm | 程序发，但每条要点两次 |
| Auto | 程序直接填并发送 |

`Confirm` 长得像一个**档位**，干的却是「每条候选上的一个**一次性动作**」的活。

**改法**：

- **档位只剩两档**：`Manual`（默认，程序永不发）/ `Auto`（程序自己填并发送）
- 每条候选上保留一个**「现在就发这一条」**的按钮，它**不改档位**、发完就消失

这样档位回到它唯一该表达的意思：**「程序会不会不问我就发」**，
而「我现在就想发这一条、下一条我还是手动」的诉求也没丢。

涉及：`crates/core/src/dispatch.rs`、`ui/src-tauri/src/commands.rs`
（`Arming`、`deliver`、`dispatch`）、`ui/src/lib/TierSwitch.svelte`、
`ui/src/lib/CandidateCard.svelte`。

### 4.2 OCR 换预下载模型

现在的 `Windows.Media.Ocr` 识别率不够。实测错误样例：

| 真值 | 读成 |
|---|---|
| 涉及 | <按硬约束移除的 OCR 碎片> |
| <按硬约束移除的 OCR 碎片>（文件名片段） | <按硬约束移除的 OCR 碎片> |
| `80%-ASD-STE100.md` | `80％一ASD一訂EIOO.md` |

**许可证已逐个核实，全部过门禁**：

| 组件 | 版本 | 许可证 | 核实方式 |
|---|---|---|---|
| RapidOCR（PP-OCRv4 的 ONNX 导出） | — | **Apache-2.0** | 仓库 `LICENSE` 原文读取 |
| `tract-onnx` | 0.23.8 | MIT OR Apache-2.0 | `cargo info` |
| `ort` | 2.0.0-rc.13 | MIT OR Apache-2.0 | `cargo info` |
| `ocrs` / `rten` / `ndarray` | — | MIT OR Apache-2.0 | `cargo info` |

模型权重约 **16MB**（det + rec mobile）。`tract-onnx` 是**纯 Rust**，
不需要 C++ 运行时——比 `ort`（包 ONNX Runtime C++）少一层二进制依赖。

**做法**：新开一个 `ocr_pp` 模块，与现有 `ocr.rs` 并存，用环境变量切换，
拿今天那批真实帧做**前后对比**（行数、错字数、耗时）。对比不过就保留现状，
不要为了「用了模型」而替换。

⚠️ 新增依赖后必须跑 `cargo deny check`——这是门禁之一。

### 4.3 首屏提速（**本机做**，因为要测模型耗时）

三选一或组合：

1. **首屏不等模型**：先推一帧「已认出会话 + 已读到消息」（纯本地，~1.1 s），
   候选区显示「正在起草…」，模型结果后到。体感 8 s → 1.1 s。
2. **合并调用**：judge + draft 并成一次请求。
3. **并行**：rank 与 draft 并行。

实现位置：`ui/src-tauri/src/watch.rs` 的 `step()`，
现在 `if newest.is_some() { ... 推理 ... }` 整段是同步的。

### 4.4 票据对账

`.scratch/wechat-reply-copilot/issues/` 下 16 张票，还有 6 张带未勾项：
浅色主题夹具、多 DPI 实测、多行气泡归并、调试指标（像素/OCR 次数/视觉 token）、
自己的口吻样本、DWM 圆角。

**规矩**：每完成一项立刻勾，别攒到最后。漂移过的票要按实机证据更正，
没核实的宁可留着并注明「未逐条勾选」——假勾比不勾更糟。

---

## 5. 重做：这是**两条轨道**，不是一件

> ⚠️ **接手的人请先读完这一节再动手。**
>
> 用户 2026-10-09 的原话：「没让你直接做选择，是让你写清楚 UI 重做是要按产品 UI
> 设计重做，因此需要找到适合的 skill，然后出设计，最后改好、调好 UI。
> 还有 Rust 部分的设计，比如与 UI 的通信交互等，都是需要打磨的，
> 需要把相关代码风格、规范的 skill 拿到后重构。」

### 5.1 两条轨道

| | 轨道 A：UI / 产品设计 | 轨道 B：Rust 侧 |
|---|---|---|
| 范围 | 面板的视觉、信息层级、组件形态、交互反馈 | 与 UI 的**通信契约**、代码风格与规范、模块边界 |
| 主要文件 | `ui/src/routes/`、`ui/src/lib/*.svelte`、`ui/src/app.css`、`ui/static/readout.demo.json` | `crates/*/src/**`、`ui/src-tauri/src/{watch,commands,debug}.rs` |
| 验收方式 | 浏览器里看 `?demo`，对着设计稿核对 | 门禁全绿 + 实机行为不 regress |

**两条轨道要一起做。** 只改 UI 不动契约，等于把界面刷在一套没验证过的接口上——
本项目已经吃过这个亏（见 §3「Rust `Message` 缺 `id`，界面整块空白」）。

### 5.2 共同流程：**找 skill → 出设计 → 改 → 调**

**不要跳过「出设计」直接写代码。** 顺序是有理由的：
跳过设计直接改代码，产出就是训练数据里最常见的那一套，也就是用户说的「AI 味」。

```
① 找 skill          在技能站/生态里检索，挑**适合本项目**的（见 5.3 / 5.4 的检索入口与筛法）
② 出设计           先产出设计说明：信息层级、组件清单、状态矩阵、token 取值、动效参数
                   ——写下来，不是画在脑子里
③ 改               按设计改代码
④ 调               对着**实机证据**调，不是凭感觉
```

第 ④ 步的「实机证据」在本项目是具体的：

- UI：`?demo` 页面的截图 + `docs/HANDOFF.md` §5.6 的验收判据表逐条对
- Rust：八道门禁全绿 + 对真实微信帧重放（`cargo xtask ocr/perceive --file`）

### 5.3 轨道 A：去哪儿找 UI 设计 skill

**检索入口**（这是工具，具体选哪个由接手的人判断）：

```bash
npx skills find <关键词>                   # 在 skills.sh 生态里搜
npx skills add <repo-url> --skill <名字>   # 装某一个
```

站点：`https://skills.sh`

**我 2026-10-09 搜过一轮**，以下是搜到的**候选池**——是给你筛的原料，不是结论。
安装量那一列只说明「被用得多」，不代表「适合本项目」：

| skill | 安装量 | 出处 | 它在管什么 |
|---|---|---|---|
| `frontend-design` | 862k | Vercel 官方 | 描述里就写着 *avoid generic AI aesthetics* |
| `web-design-guidelines` | 613k | Vercel 体系 | 对照 Vercel Web Interface Guidelines **审查**代码 |
| `design-taste-frontend` | 451k | 社区 | *Override generic LLM frontend output* |
| `ui-ux-pro-max` | 347k | 社区 | UI/UX 决策、组件推荐 |
| `high-end-visual-design` | 334k | 社区 | 让 AI 界面像高端机构做的 |
| `redesign-existing-projects` | 330k | 社区 | **把现有项目从 AI 味升级** |
| `ui-skills` | — | 社区 | 禁令清单（禁紫渐变 / glow / 动 layout 属性等） |
| `design` | — | smithery/ai | 动手前先**定设计人格**（7 种方向选一种） |
| `visual-design-foundations` | 11.8k / ★39.4k | wshobson/agents | 排版比例、8 点网格、OKLCH 色阶、WCAG 计算 |
| `geist-design-system` | — | **社区作者** | Geist 风格 |

**筛选判据**（比安装量重要）：

- 这个 skill 的**产出形态**是不是「设计说明 + 组件规格」，
  而不是「给我一堆 CSS」？我们要的是前者。
- 它有没有**反对意见**（anti-pattern / 禁令）？只会说「要优雅」的没有约束力。
- 它假设的技术栈跟本项目合不合（Svelte 5 + 原生 CSS 变量，没有 Tailwind）？
- 出处是官方还是社区？社区的就当**判据来源**，不当**官方规范**引用——
  这和「依赖许可证要从 crates.io 元数据确认，不从项目主页推测」是同一条纪律。
- 装之前看 `skills.sh` 上的 Security Audits。

⚠️ **不要装 `vercel/vercel-plugin` 来代替检索。** 那是 Vercel **自家产品**的
开发指引包（deploy / ai-sdk / flags / workflow / storage…33 个 skill），
跟「找优秀 UI 设计范式」是两件事——其中跟 UI 相关的只有 `frontend-design`。

### 5.4 轨道 B：Rust 侧要打磨什么

**同样要先找 skill**，检索入口同上，关键词换成
`rust` / `code style` / `architecture` / `api design` / `naming conventions`。
候选池我没系统搜过，**这一条留给接手的人去搜**。

要打磨的三块，按本项目的实机教训排：

#### B-1 与 UI 的通信契约（**优先级最高**）

现在的形状：

- `ui/src-tauri/src/watch.rs` 的 `Readout` / `Message` / `DraftCandidate` / `CallCounts`
- `ui/src/lib/types.ts` 的 TS 镜像类型
- 事件名：`readout` / `debug_pane` / `armed`
- 命令面：`probe_anchor` / `latest_readout` / `dispatch` / `debug_pane` /
  `set_tier` / `get_tier` / `open_debug` / `open_settings` / `relock_session` …
- `ui/src-tauri/capabilities/default.json` 的权限白名单

**这份契约没有任何一处是「设计」出来的**，它是一路长出来的。已知问题：

- **两端手工镜像，没有任何机制保证同步。** Rust 的 `Message` 少一个 `id`，
  前端的 `{#each (m.id)}` 就让 7 条消息的 key 全变 `undefined`——
  面板整块空白而后端日志一切正常。
  **重构方向**：让契约成为可校验的东西（生成 TS 类型，或加一条跑在门禁里的
  契约测试），而不是两处手写的巧合。
- **状态语义没有集中的定义。** 比如「装填」这件事，
  曾经同时存在于 Rust 状态、前端 `armedId`、按钮文案三处，任何一处漏改就是 bug。
- **`reason` 是自由文本，被前端正则匹配来判断要不要报警**（`/失败|错误|…/`）。
  这是把结构化信息塞进字符串再猜回来。
- **事件名散落在 `main.rs` 的常量、Rust 的 `emit`、前端的 `listen` 三处。**

#### B-2 代码风格与规范

`AGENTS.md` 里有硬约束和命令，但没有**代码风格**层：
错误处理怎么写、注释写什么不写什么、模块边界怎么划、公共 API 的命名。
本项目反复出现的几类问题（都可作为重构目标）：

| 问题 | 例子 | 后果 |
|---|---|---|
| **恒哑的错误处理** | `let _ = app.emit(...)` / `let _ = (action, tier)` | 序列化失败、档位被丢弃，**全程无报错** |
| **注释与实现分叉** | 注释写「第二次点击才发」，代码里第二次点走不到那条路 | Confirm 档变成死胡同 |
| **命名撒谎** | 变量叫 `full`，实际是 `frame.crop(anchor.chat)` 之后的**消息区** | 坐标换算连着错三次 |
| **该失败的地方糊过去** | 锁中毒时「继续用比整个进程崩掉好」 | 状态悄悄错乱 |
| **注释解释「为什么」不足** | 大量注释在解释「这是什么」而不是「为什么是这样」 | 改的人看不懂约束 |

#### B-3 模块边界

`chatservant-capture` 同时装着**窗口发现**、**WGC 抓帧**、**OCR**、
**UI 注入**、**媒体编码**、**锚点推导**——六件事耦合在一个 crate 里，
导致「夹具模式」这种测试能力必须横穿整个依赖链才能加进去
（见 §2.3）。**这也是为什么云端编译不了它。**

### 5.5 轨道 A 的验收：0 AI 味的可检查判据

不要写「要有品味」这种没法执行的话。下面每条**都能在代码里 grep 出来**，
做完逐条对。前四条来自社区 skill 的禁令，后几条是本项目实机撞出来的。

| # | 禁令 | 判据 |
|---|---|---|
| 1 | 紫粉渐变 / 多色渐变 | `grep -iE '#(6366f1\|a855f7\|8b5cf6)'` 应为空 |
| 2 | 玻璃拟态 | `grep 'backdrop-filter'` 应为空（它也禁得对大模糊面做动画） |
| 3 | glow / 光晕当主要可点性提示 | 不用 `box-shadow: 0 0 24px` 这类发光 |
| 4 | 不加装饰性渐变 | 除非显式要求 |
| 5 | 一律超大圆角 | 面板是**工具**不是营销页。圆角 ≤ 8px，同层内统一，不混用多套圆角体系 |
| 6 | Emoji 或单字当图标 | 图标一律 SVG；**图标按钮必须有 `aria-label`** |
| 7 | 营销词堆进工具界面 | 「✨」「🚀」「立即开始」——工具里出现就是错的 |
| 8 | 三等分平级卡片 | 产品只有一个**主产出**，不该有三张平级卡片 |
| 9 | 通用聊天布局（气泡 + 头像） | **我们是仪表不是第二个聊天软件**。用说话人窄列，不做气泡 |
| 10 | 所有元素等宽等距 | 必须有层级差：正文 / 次要 / 判据，字号与对比度分档；数字用 `tabular-nums` |
| 11 | 所有按钮长得一样 | 主次分明：默认实底、次要描边、危险警示色；每视图**只用一种强调色** |
| 12 | 空状态只说「暂无数据」 | **必须给一个明确的下一步动作** |
| 13 | 动画动 layout 属性 | 只动 `transform` / `opacity`；交互反馈 ≤ 200ms，进场 ease-out |
| 14 | 假数据占位 | 演示数据要说清是演示的（`readout.demo.json` 顶部有说明） |

**再加一条否定式判据**：做完问自己——
「如果这是个真实商业软件的某个面板，而不是一个 demo，设计师会认吗？」
认不出来就说明还在 AI 的舒适区里。

### 5.6 已定的设计定位（**已定，别推翻**）

- 面板是**仪表**，不是第二个聊天软件
- 底色暖石墨 `#1B1A18`，**刻意偏离微信的冷调近黑**——两块窗口并排时
  要能一眼分清哪个是微信、哪个是仪表
- 强调色黄铜 `#D9A441`；微信绿**只留给「我发的」这一条语义**，
  界面自身的强调色不能抢掉它
- 签名元素是左侧 3px 导轨：顶端状态点，通体带当前档位。
  它编码的是「程序现在是活着 / 在等 / 可能发出去」这个事实
- **正文是最大的字，按钮反而最小**——候选是这个面板的产出
- 判据（`bg=rgb(...) 对比=...`）**不常驻**，点说话人才展开
- 设计 token 全在 `ui/src/app.css`

### 5.7 动效

现有三处：候选卡入场、档位指示块滑动、消息流滚到底，
都走 `ui/src/lib/attach.ts` 的 attachment（Svelte 5 的 `{@attach fn}`）。
`prefers-reduced-motion` 在 `ui/src/lib/motion.ts` 里有现成处理。
⚠️ 这些是**现状**，不是设计——轨道 A 重做时动效要重新出方案，别默认沿用。

---
## 6. 云端会话怎么改 UI

```
线上：https://nme1e0uxqtn1y.space.mcode.cn/?demo
源码：仓库里的 ui/src/  +  ui/src-tauri/src/  +  ui/static/readout.demo.json
```

- 改 Svelte 组件 → `ui/src/routes/+page.svelte`、
  `ui/src/lib/{CandidateCard,MessageStream,TierSwitch,ReasonBar}.svelte`
- 设计 token 全在 `ui/src/app.css`（暖石墨 `#1B1A18` / 黄铜 `#D9A441`）
- 演示数据在 `ui/static/readout.demo.json`，**改它就能试各种极端排版**
  （超长句、中英混排、多行气泡、文件名、空列表）

本地起预览（不用 Tauri）：

```bash
cd ui && pnpm dev          # 然后打开 http://localhost:1420/?demo
```

### 写 Svelte 之前必须做的事

```bash
npx -y @sveltejs/mcp svelte-autofixer src/xxx.svelte     # 必须 issues: []
npx -y @sveltejs/mcp list-sections
npx -y @sveltejs/mcp get-documentation "<section>"
```

⚠️ autofixer 对 BOM 敏感（见 §3.7），报 `at line undefined` 先查 BOM。

（设计约束不在这里重复——见 §5.3「0 AI 味」判据表与 §5.5 设计定位。）

---

## 7. 本机怎么跑

```bash
# 八道门禁（必须全绿才算数）
pwsh -NoProfile -File scripts/gate.ps1

# 现场模式
cd ui && pnpm tauri dev          # 日志落 target/app.log

# 夹具模式（不需要微信）
pwsh -File scripts/run-app-fixture.ps1 <png>

# 诊断脚本（都在 scripts/）
pwsh -File scripts\winscan.ps1          # 顶层窗口列表
pwsh -File scripts\shotat.ps1 <hwnd> out.png   # 整屏截图 + 按窗口实际矩形裁
pwsh -File scripts\clickat.ps1 <x> <y>  # 屏幕绝对坐标点击
pwsh -File scripts\imgdiff.ps1 a.png b.png     # 逐像素差（验 hover 用）
pwsh -File scripts\hittrace.ps1 <x> <y> # 该点上所有窗口的 z 序
pwsh -File scripts\cursorspy.ps1 [x y]  # 光标位置 + 前景窗口
pwsh -File scripts\focuswin.ps1 <hwnd>  # 把前台切到某窗口
pwsh -File scripts\uia-set.ps1 <hwnd> "文本"   # UIA 写入，不抢鼠标
```

**`shotat.ps1` 是唯一可信的截图方式**：`shotwin.ps1`（PrintWindow）
对 WebView2 返回黑帧。凭印象写死裁剪坐标是上一轮反复裁偏的原因——
**每次都用窗口的真实矩形**。

---

## 8. 硬约束（动代码前先读）

来自 `AGENTS.md`，代码里必须能看出对应实现：

1. **只读自己设备上、自己有权查看的对话。** 不代替任何人查看别人的聊天。
2. **采集只用窗口级捕获 + 本地离线 OCR。** 不 hook、不注入、不读对方数据库、
   不解密、不碰对方进程内存。
3. **截图不落盘。** 帧常驻内存，不写磁盘、不进日志、不上传。唯一例外是
   `xtask probe` 这类显式调试命令写入 `target/probe/`，且必须提示用户。
4. **不碰钱。** 转账、红包、收款相关界面元素一律不碰；起草 prompt 里也禁了。
5. **发送分档，默认档最低。** 升档必须显式、可回退。
6. **静默期零调用。** 没有新对方消息时不调任何模型。
7. **API key 不落普通文件、不进日志。** 只从凭据管理器或环境变量读，
   报错文本一律脱敏。

**许可门槛**：只允许 permissive 许可证。GPL / AGPL / LGPL / MPL、
源码可见但限制商用的（BUSL/SSPL/Elastic）、未声明许可证的依赖一律不用。
新增依赖时许可证**从 crates.io 元数据确认**（`cargo info <crate>` 看 `license:`），
不从项目主页推测。策略由 `deny.toml` 机器执行，`cargo deny check` 是门禁之一。
理由与选型记录见 `docs/adr/0003-dependency-strategy.md`。

**参考项目**：`examples/jev-chat-jarvis/` 是被替代的参考实现。
遵守**净室**：只读实测结论（`docs/KICKOFF.md`、`app/ocr.py`、`app/capture.py`），
**不复制任何代码**。

---

## 9. 文档地图

| 文件 | 内容 |
|---|---|
| `AGENTS.md` | 项目约定与硬约束 |
| `CONTEXT.md` | 领域术语（写代码前读） |
| `docs/adr/0002-frame-conditioning-pipeline.md` | 帧 → 条件化 → 归类 |
| `docs/adr/0003-dependency-strategy.md` | 许可门槛与选型 |
| `docs/adr/0004-capture-strategy.md` | 为什么用 WGC 而不是抓屏 |
| `docs/adr/0005-delivery-and-focus.md` | **投递路径与「不抢前台」** |
| `docs/HANDOFF.md` | 本文 |
| `.scratch/wechat-reply-copilot/issues/` | 16 张票据（含未完成项） |

---

## 10. 提交历史（本次会话）

| 提交 | 内容 |
|---|---|
| `306f3dc` | 六个实机缺陷：面板空白、候选活不过两秒、重锁后读不到东西、候选被挤出屏幕、OCR 插空格、闸关上面板无反馈 |
| `d1a1270` | UI 重做（真图标 / 消息行去卡片化 / 候选区重排）+ OCR 阈值实测定标 + ADR-0005 |
| `d55ec51` | Confirm 档死胡同 + 两份拼接 + 夹具模式 + `docs/HANDOFF.md` |
| `3adb9c6` | `?demo` 模式——界面脱离 Tauri 在浏览器跑 |
| `9d88249` | 部署用瘦源码副本脚本 |
