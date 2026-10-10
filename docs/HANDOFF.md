# chat-servant 交接文档

> **给接手的人（尤其是云端会话）**：这是一份**冷启动说明**。读完它你应该知道
> 现在能干什么、哪些路已经被证伪、下一刀切哪里，以及**每条命令怎么跑**。
>
> 文档里所有「实测」结论都带日期和证据来源。没写「实测」的是推断，别当事实用。
>
> 最后更新：2026-10-10

---

## 0. 三十秒摘要

Windows 桌面程序：在你自己的微信单聊窗口旁边挂一块悬浮面板，读取对方新消息，
调模型起草 2~3 条回复，落在面板上，你点一下填进输入框，再点一次才发。
**默认档只填不发。**

全链路已在真机上跑通并**真的发出去一条消息**。

> ⚠ **本次交接（2026-10-10）先读这一节。** 下面 §1~§10 是长期积累的
> 背景与「已证伪的路」，仍然有效；但**当天的进展与新发现集中在 §11**，
> 包括一个**未修的产品缺陷：切换微信会话后聊天内容不更新**。

---

## 0.1 本次交接状态快照（2026-10-10）

### 门禁（全部实跑过，不是抄的）

| 门禁 | 结果 |
|---|---|
| `cargo clippy --workspace --all-targets -- -D warnings` | **EXIT=0** |
| `cargo test --workspace --lib --bins --tests` | **250 passed / 0 failed** |
| `pnpm run check` | 0 errors 0 warnings |
| `cargo deny check` | licenses / bans / sources **ok**；**advisories FAILED**（4 条，见 §11.4） |

⚠ **doctest 跑不起来是本机缺 `rustdoc.exe`**（`rustdoc.pdb` 在、`.exe` 不在），
不是代码问题。门禁一律用 `--lib --bins --tests`。

### 进程状态

**全部已停**：应用进程 0、node 0、1420 端口无人监听。
打包时 `target/` 与 `node_modules/` 已排除。

### 做完的（当天）

1. **档位 IPC 契约缺陷**（既有缺陷，本轮修掉）——见 §11.1。
2. **分段控件改滑动指示块**——见 §11.2。
3. **cargo-deny 装好并跑通**，新建 `deny.toml`——见 §11.3。
4. **OCR 加载层跑通**（PP-OCRv4 det/rec 都能编译、能推理）——见 §11.5。

### 没做完的（别当已交付）

- **OCR 推理管线**：det 后处理、rec CTC 解码、**rec 宽度分桶**都还没写。
- **新发现的缺陷：切换微信会话后聊天内容不更新**——见 §11.6。**未修，未定位。**
- **cargo deny 的 4 条 advisories** 未决——见 §11.4。

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

---

## 11. 2026-10-10 当天进展（**先读这一节**）

> §1~§10 是长期背景。这节只记当天做的事与**当天撞到的东西**。

### 11.1 档位 IPC 契约缺陷（**已修**，既有缺陷不是当天引入的）

**症状**：面板主视图头部**档位名整格空白**；设置里的档位滑块不定位。
放大截图 `%TEMP%\cs-header-zoom.png` 证实：头部显示
「没认出会话 | ⚙ | ··· | − | ×」，**档位槽是空的**。

**根因**：`ui/src-tauri/src/commands.rs` 里，`get_tier` / `set_tier`
**返回中文显示名**，而前端契约 `ui/src/lib/types.ts` 要的是枚举 id：

```ts
export type Tier = 'Manual' | 'Confirm' | 'Auto';
```

于是 `+page.svelte:562` 拿到「只填入」，`+page.svelte:690` 的
`TIER_NAME[tier]` 是 `undefined` → 渲染空白；
`SegSlider` 拿 `"只填入"` 去匹配 `data-val` 也**一个都匹配不上** → 滑块不定位。

**修法**：把「IPC 用的 id」和「给人看的中文名」**拆成两个函数**：

| 函数 | 返回 | 谁在用 |
|---|---|---|
| `tier_id(Dispatch)` | `"Manual"` / `"Confirm"` / `"Auto"` | `get_tier` / `set_tier`（IPC） |
| `tier_name(Dispatch)` | 「只填入」/「确认后发」/「自动发送」 | 托盘 tooltip、投递回执说明文字 |

外加 `parse_tier(&str) -> Dispatch`，**解析失败一律落 `Manual`**（最低档）。

**为什么这么分**：中文名一改字，前端要跟着改代码才能跟上；而 Rust 已经是
显示名的唯一事实来源。**id 是契约，中文名是显示**，两件事不该共用一个出口。

**新增测试**（`commands.rs`）：
- `the_ipc_tier_is_the_frontend_enum_id_not_a_display_name`——把前端的三个
  字面量**写死进断言**。前端要改 `Tier` 的话这个测试就该红，那正是它该红的时候。
  外加一条 `assert_ne!(tier_id(d), tier_name(d))` 专门拦「两者又合回去」。
- `an_unparsable_tier_falls_back_to_manual_never_up`——8 个反例，含
  `"auto"`、`"AUTO"`、`"自动发送"`、`"Manual "`（尾随空格）。

⚠ **这条契约以前没有测试守着**，所以它能错这么久。别的 IPC 命令也值得
照这个方式补一条。

### 11.2 分段控件改滑动指示块（`SegSlider.svelte`）

用户原话：「按钮选中方式为何不做成表面一层的滑块（窗口关闭和档位切换不都可以用
这个方案？还不用加个什么文字默认）」。

- **新建 `ui/src/lib/SegSlider.svelte`**：`{@attach slideTo(value)}` + `.cs-pill`，
  墨底反白（红色只留给不可逆的动作）。
- `PanelSettings.svelte` 的「关闭窗口的行为」与 `TierSwitch.svelte` **共用同一个组件**。
- **删掉「退出（默认）」里的「（默认）」文字**——它是控件组的默认值，
  不是某枚按钮的属性，而且让同组两项长度差一倍。
- 删掉随之死掉的 `.cs-seg` / `.cs-seg-item` / `.cs-seg-item.on`。

**调用了 `ui-ux-pro-max` skill**（此前从未调用过——用户质问「设计、规范 skill
你是一点没照办？」是对的），`search.py "segmented control selected state indicator" --domain ux`
命中两条直接相关的规范，都已落地：

| 规范 | 落地方式 |
|---|---|
| **Compact Control Semantics（Critical）**：紧凑控件需原生 role + 可访问选中状态 + 键盘操作 + 可见焦点 | `role="radiogroup"` / `aria-checked`；`onKeydown` 方向键 + Home/End；全局已有 `:focus-visible`（`app.css:140`，2px 墨色） |
| **Cancellable State Transitions（High）**：快速连切要 cancel/replace 既有动效，不依赖 `animationend` | `attach.ts` 的 `slideTo` 用 `overwrite: 'auto'`；无位移时直接 `gsap.set`，不等动效结束 |

**补了 roving tabindex**：选中项 `tabindex=0`，其余 `-1`，整组 `-1`。
这是 ARIA APG 对 `role="radiogroup"` 的硬要求——每个按钮都 `tabindex=0`
会让读屏用户同时拿到两套导航，是已知反模式。

**踩到并修掉的一个坑**：搬组件时漏了 `align-self: flex-start`。
父容器是 `flex-direction: column` + `align-items: stretch`，槽会被拉成
**横贯整个面板的空杠**。

### 11.3 cargo-deny（**已装已跑**）

- 预编译 `cargo-deny 0.20.2`（`EmbarkStudios/cargo-deny` releases，
  `x86_64-pc-windows-msvc.tar.gz`），SHA256 校验通过。
- 装在 `C:\Users\zhanghj40\.cargo\bin\` 与 `E:\DevCache\cargo\bin\`（本机 `CARGO_HOME` 指向 E 盘）。
- ⚠ clone RustSec advisory-db **需要 `git config --global http.proxy`**。

**新建 `deny.toml`**（此前不存在 → cargo-deny 用极严默认白名单，
产生 **876 条无意义 license 报错**）。踩了四个 schema 错，记下来：

| 踩的坑 | 正确写法 |
|---|---|
| `[graph]` 不存在 | `all-features` / `exclude-dev` 就放 `[graph]` 里 |
| `[multiple-versions]` 不存在 | 应为 `[bans] multiple-versions = "warn"`（0.20 只收 LintLevel；`wildcards` / `highlight` 是旧写法） |
| `allow`/`deny`/`skip` 放错节 | 属 `[licenses]`，**不**属 `[bans]` |

**故意不加 `[advisories] ignore`**——静默掉真实漏洞是错的，留给人决定。

MPL-2.0 / LGPL 全部来自 **Tauri 传递依赖**（dom_query / cssparser /
dirs-sys / getrandom），**与 tract 无关**；理由已写进 `deny.toml` 注释。

### 11.4 cargo deny advisories：4 条待人决策

`cargo deny check` 的 `advisories` 项 FAILED（exit=1），4 条**都不是
tract-onnx 引入的**：

| ID | 组件 | 性质 |
|---|---|---|
| RUSTSEC-2024-0370 | `proc-macro-error` | unmaintained |
| RUSTSEC-2026-0194 | `xml-rs` | 重复属性检查二次复杂度 |
| RUSTSEC-2026-0195 | `xml-rs` | 命名空间分配无界 → 内存耗尽 |
| RUSTSEC-2025-0134 | `rustls-pemfile` | unmaintained（经 `ureq` → `chatservant-reason`） |

⚠ `xml-rs` 那两条**能被内存耗尽打爆**，不是纯维护性告警。
处理方式（升级 / 换依赖 / 显式 ignore 并写理由）**需要人定**，本轮没动。

### 11.5 OCR（PP-OCRv4）：**地基与加载层已通，推理管线未写**

#### 已完成

- `crates/capture/Cargo.toml` 加 `tract-onnx.workspace = true`；
  根 `Cargo.toml` 加 `tract-onnx = "0.23"`。
  **选 tract 而非 `ort`**：纯 Rust、无 C++ 运行时，避免给装进
  `C:\Program Files` 的应用塞 ONNX Runtime 动态库。
- 权重已下载到 `%APPDATA%\dev.chatservant.app\models\`（**不进仓库**），
  SHA256 与 RapidOCR 官方清单逐一校验通过。
  **权重 URL 是从 RapidOCR `python/rapidocr/default_models.yaml` 读出来的**，
  不是猜的；字典 `dict_url` 在同文件的 `paddle/` 段
  （`onnx/PP-OCRv4/rec/ppocr_keys_v1.txt` **返回 404**，正确路径是
  `paddle/PP-OCRv4/rec/ch_PP-OCRv4_rec_mobile/ppocr_keys_v1.txt`）。
- **新建 `crates/capture/src/ocr_pp.rs`**——加载层已通（7 个测试全绿）。

#### ⭐ 本机实测出来的模型形状（**别再照着 PaddleOCR 的惯例猜**）

三条假设都被实测打回，全部已写进断言与注释：

| 事实 | 原本的错误假设 | 证据 |
|---|---|---|
| rec 输入是 **`[-1,3,-1,-1]`，四维全动态** | 「高固定 48」（那是 `RecImageGenerator` 数据增强侧的行为，**不是模型**） | `model_io` 探针 |
| rec 输出是 **`[1, T, 6625]`——类在最后一维** | 「`[1, 6625, T]`」（按训练侧布局想当然） | 真跑 64px 空白图得 `[1, 8, 6625]` |
| det 输入是 **`[-1,3,-1,-1]`，batch 也是动态的** | 「batch 固定 1」（按惯例） | `model_io` 探针 |

det 输出 `[1,1,H,W]`（逐像素文字概率）由**真跑一次**确认，不是读 fact。

⚠ ⚠ **`[1, T, 6625]` 写反了不报错**，只会得到一整段空白。
所以这类形状**只能靠真跑一次钉死**，读 fact 读不到——
`model_io` 里 `analyse` 之后输出维仍然是空的（详见该函数注释里的三次踩坑记录：
①`model_for_path` 没推导 ②`analyse` 也读不到 ③`into_optimized` 必须先给具体
输入事实，而 tract 的**动态维不能用 `-1` 表示**——`TDim` 断言每维 `>= 0`，
动态维是符号 `s0`/`s1`）。

#### ⛔ 未完成（下一刀就切这里）

1. **det 后处理**：概率图 → 二值化（阈值 0.3）→ **连通域** → 外框 →
   unclip（`area * 1.6 / min_side`）→ 按 `box_thresh=0.5` 过滤。
   *刻意不搬 OpenCV 的 `minAreaRect` + `unclip`*：本项目输入是聊天窗口
   （左右对齐消息气泡），连通域 + 轴对齐外框更贴切也短得多。
2. **rec CTC 贪心解码**：按 `[t][c]` 取 argmax → 折叠重复 → 去 blank（索引 0）。
   词典 **6624 类**（`ppocr_keys_v1.txt` 6623 行 **+ blank**）。
   ⚠ Windows 上下载的权重每行带尾随 `\r`，不剥掉会让每个词末尾挂一个
   **看不见的回车符**——已处理并有测试守着。
3. **⚠ rec 宽度分桶（最大的一个坑）**：rec 是全动态的，而 tract 的形状是
   **编译期**决定的——**每个宽度不同都要重编一次图**，几十到几百毫秒。
   不能对每个文本框都这么做。要么按长度分桶缓存几个宽度，要么固定宽度
   （代价是真帧上的长文本会被缩窄）。**这一条本轮没做，是下一步最可能翻车的地方。**
4. **两套 OCR 的切换**没接：`ocr::recognize` 仍直接走 `Windows.Media.Ocr`。
   `CHATSERVANT_OCR_BACKEND` 的 `Backend` 枚举只是文档里写了，代码里还没有。
5. **与 `Windows.Media.Ocr` 的真实帧对比**没做。**对比不过就保留旧路径**——
   不为「用了模型」而替换。

### 11.6 🐛 新发现的产品缺陷（**未修、未定位**）

**切换微信会话后，面板里的聊天内容不更新。**

- **报障时间**：2026-10-10，用户实机发现。
- **现状**：**没有定位到根因，也没有修。** 不要当成已修。
- **待查的方向**（按怀疑度排序）：
  1. **观察循环是否还活着**。切会话不改变窗口 HWND，如果循环是靠
     「窗口句柄变化」或「一次性捕获会话」驱动，切会话可能根本触发不了新一轮。
     `AGENTS.md` 里记着「**WGC 会话别每帧新建**」——那么反过来，
     **这个会话是不是被缓存住、再也不刷新了**？
  2. **帧条件化 / 去重**。§9 的 `docs/adr/0002-frame-conditioning-pipeline.md`
     是「帧 → 条件化 → 归类」。切会话后画面变了，但**去重集合**里
     可能已经把这一帧判成「跟上一帧一样」丢掉了。
  3. **会话锁定**。「锁定当前会话」如果锁的是 HWND 而不是**微信内部的
     会话标识**，切会话后会拿新会话的画面配旧会话的名字——
     症状会表现为「内容不更新」或「名字对不上」，两者要分清。
  4. **感知层**。`crates/perceive` 把 OCR 框归类成对话人 / 噪声；
     切会话后左右气泡的分布变了，归类阈值可能整片失效。
- **复现前先确认**：这是「面板停在旧内容」，还是「面板清空了」？
  **两者的根因完全不同**，先分清再动手。

### 11.7 ⚠ 当天又踩一次的坑：PS 5.1 的 `Set-Content` 双重编码

`AGENTS.md` 已经写着「别用 `Set-Content -Encoding UTF8` 改源码」，**当天还是踩了**：
用 PS 5.1 的 `(Get-Content -Raw) -replace ... | Set-Content -Encoding UTF8`
做一次机械替换，结果**整份 `ocr_pp.rs` 的中文全成乱码**（UTF-8 被按 ANSI
读回再写），且有字符不可逆丢失。

**只能用 `write` / `edit` 工具改源码。** 万一必须用 shell，
走 `.NET` 直写并显式指定**无 BOM**：

```powershell
$p = "path.rs"
$c = [IO.File]::ReadAllText($p)
$c = $c.Replace("old", "new")
[IO.File]::WriteAllText($p, $c, (New-Object Text.UTF8Encoding $false))
```

编码是**静默**损坏的：编译照样过、测试照样跑，只有中文注释在变乱码。
肉眼看 diff 才发现。

---

## 12. 2026-10-11 进展（并入另一台设备的快照之后）

> §1~§11 是背景。这节记本机当天做的事。

### 12.0 先说清楚分支关系

收到另一台设备 2026-10-10 的完整工程快照（`chat-servant-handoff-20261010.zip`，
4722 个条目）。**那不是「我们落后了」，是分叉**：两边从同一祖先各走各的。

| | 本机（`chat-servant-m31f`） | 另一台（快照） |
|---|---|---|
| 有而对方没有 | `scripts/`（48 个诊断脚本）、`.scratch/` 票据、`CONTEXT.md`、`README.md`、`pnpm-lock.yaml`、`deny.toml` 的 ADR-0003 演进版、`capture/lib.rs` 的 `#![allow(missing_docs)]` 说明、49 处 `// SAFETY:`、门禁修绿 | — |
| 对方有而本机没有 | — | `SegSlider.svelte`、`PanelSettings.svelte`（设置搬进面板）、`ocr_pp.rs` + `tract-onnx`、§0.1 与 §11 的进展记录、更详细的本机待验清单 |
| 两边都改 | `commands.rs`、`main.rs`、`+page.svelte`、`deny.toml`、配置若干 | |

处理原则：**逐项并，不整包覆盖**。基建留本机的，产品改动按下面的判断走。

### 12.1 🐛 托盘图标画不出来（**已定位并修**）

**症状**：通知区里**留了一格空白**。悬停 tooltip 读出来是
「没认出会话 · 只填入」——那正是 `tray_status_line()` 的输出，
**说明图标确实注册成功了**，只是画不出东西。

**根因**：Tauri 在 Windows 上是这么取默认应用图标的
（`tauri-codegen-2.7.1/src/context.rs:211`）：

```rust
if target == Target::Windows {
  quote!(#root::image::default_window_icon_from_app_icon_resource())
}
```

——从**可执行文件的图标资源**里读。而那份资源只有 `tauri build`
打安装包时才嵌进去。`cargo` / `tauri dev` 出来的二进制里没有，
于是 dev 模式下 `default_window_icon()` **恒为 `None`**，
托盘 `NIM_ADD` 带了个空 `hIcon`。shell 对一个没有图的图标
**照样留了通知区的位**，tooltip 也照常（那是我们自己写的文本）。

**为什么这个缺陷特别坏**：托盘是 `decorations:false` +
`skipTaskbar:true` + `alwaysOnTop:true` 这个组合下**唯一的后台出口**。
它画不出来，意味着用户收进托盘之后就找不回来了。

**修法**：编译期把 PNG 嵌进二进制，自己解一份带进去。
`Cargo.toml` 开 `tauri` 的 `image-png` feature——
`image` / `png` **本来就在依赖树里**，不引入新的许可证面。

**实机验证**：溢出浮层第一格从空白变成绿气泡图标；右键菜单正常弹出，
首行「没认出会话 · 只填入」是**灰色不可点**的状态行，下面
显示面板 / 锁定当前会话 / 打开设置 / 识别画布 / 退出（Ctrl+Q）。

### 12.2 档位 IPC 契约缺陷：**本机确认同病**，已修（并入 §11.1 的修法）

§11.1 记的是另一台设备发现的。本机核对后发现**同一个缺陷在**，
只是症状不同：

- 面板头部**正常**显示「只填入」——因为那里的 `tier` 来自
  readout 事件，Rust 推的是枚举 id。
- 出问题的是**设置页**：`get_tier` / `set_tier` 回的是中文名，
  前端 `TIER_NAME["只填入"]` 是 `undefined`，档位控件定位不到。

修法照搬：拆出 `tier_id`（IPC 契约）/ `parse_tier`（失败落 `Manual`），
`tier_name` 只留给托盘与回执文字。并补上那两条测试——
`the_ipc_tier_is_the_frontend_enum_id_not_a_display_name` 把前端的
三个字面量写死进断言，`an_unparsable_tier_falls_back_to_manual_never_up`
用 8 个反例钉住「解析失败不许升档」。两条都实测通过。

### 12.3 🐛 设置窗口停在离屏（**新发现，未定位**）

`tauri.conf.json` 里 `settings` 窗口写的是 `"visible": true`，
但实机枚举该进程的**全部**顶层窗口（含不可见的）得到：

```
0x30E3E  vis=False  Tauri Window  '设置'  656x559 @ (-32000,-32000)
```

`(-32000,-32000)` 是 Windows 最小化窗口的标准位置。
同一份配置里的 `识别画布`（`visible: false`）停在该停的地方，
主面板 `副驾` 正常可见。**所以不是「没建」，是「建了但被最小化/藏起来了」**，
而 `winscan.ps1` 只报可见窗口，会把这两件事混为一谈。
下一步要么查 `open_settings` 的调用路径，要么复现一次确认是不是误操作。

### 12.4 ⚠ 本轮没验成的：光标被前台游戏抓走了

`点 − 收进托盘`、`Alt+F4 拦截`、`拖动窗口` 这三条**都没验成**，
**不是应用缺陷，是环境**：当时前台是《地下城与勇士》，
`SetCursorPos` 直接**不生效**（设置 (639,349)、读回仍是 (1707,824)）。
症状是面板所有按钮的 `:hover` 底色都不出现——事件压根没进 WebView。

⚠ **`SetCursorPos` 不生效时不要去查「按钮为什么不响应」**，
先读回 `GetCursorPos` 确认指针真的动了没有。

### 12.5 🐛 切换微信会话后面板内容不更新（§11.6 复述，仍未定位）

另一台设备 2026-10-10 用户实机报过。**本轮没碰**，状态不变：
未定位、未修。§11.6 那四条怀疑方向仍然有效。

### 12.6 并进来的东西

⚠ **本节在 2026-10-11 被改写过一次。** 最初这里写的是「没并进来的东西」，
理由是「产品级结构改动要人拍板」。用户回的是：
**「包里的进度你给我采纳上了，别自作主张不要；至于对齐的问题，你不就该处理？」**
于是全部并入，对齐由本机负责。下面的 §13 记并入过程与三处**故意不并**的地方。

| 对方的东西 | 处置 |
|---|---|
| `PanelSettings.svelte` + 设置搬进面板（删了 config 里的 `settings` 窗口与 `park_settings`） | 并入 |
| `SegSlider.svelte` 滑动指示块（`TierSwitch` 与设置共用） | 并入 |
| `ocr_pp.rs` + `tract-onnx` | 并入（加载层；推理管线仍未写完，状态见 §11.5） |
| `commands.rs` 的托盘三态（`TRAY_PENDING/READY/FAILED`） | 并入，**取代**本轮自己写的 `tray_available()` |
| `commands.rs` 的 `tray_icon(auto)` + Auto 红点 | 并入，**取代**本轮只做「补图标」的那版 |
| 根 `Cargo.toml` 的 `[profile.dev] debug=1` + `[profile.dev.package."*"] debug=0` | 并入（磁盘：`target` 实测 8.4 GB → 见 §13.3） |
| `crates/{core,perceive,reason}/Cargo.toml` 的 `[lints] workspace = true` | 并入，并因此补齐 90 条 `missing_docs`（见 §13.2） |
| 3 处 `sort_by` → `sort_by_key` | 并入 |
| `tauri.conf.json` 删掉 `settings` 窗口与 `appDirectoriesOverride` | 并入 |
| `deny.toml` | **不并**。本机这份是 ADR-0003 演进出来的（7753 vs 3979 字节），对方那份是初版 |
| `ui/{tsconfig,svelte.config,vite.config,pnpm-workspace}`、`.gitignore` | **不并**。对方那几份是**未 prettier 的脚手架**：Tab 缩进、`tsconfig` 缺 `"types": ["node"]`（`vite.config.ts` 用得到）、`vite.config.ts` 丢了 `TAURI_DEV_HOST`。照搬会直接挂 lint 门禁 |
| `.agents/` `.claude/` `.minimax/` `.zcode/` `skills-lock.json` | **不并**。那是那台机器的 agent 环境，不是工程源码 |

---

## 13. 2026-10-11 第二天：把另一台设备的进度全部并进来

### 13.1 两条分支是**分叉**，不是先后

拿到 4722 条的快照后，第一件事是分清「哪些差异是真的」。做法是
`target/probe/diffcode.ps1`：对每个文件跑 `git diff --no-index -U0`，
**滤掉注释行和空行**，只看代码改动。

为什么必须滤：两个分支**各自独立**给同一批 `unsafe` 补了 `// SAFETY:` 注释。
不滤的话 `crates/capture/` 下九个文件每个都显示成「大改」，
据此做整包覆盖会把另一边那台的实机修复退回去。

滤完之后结论很干净：

- `capture/` 的 `ocr.rs` / `inject.rs` / `printwindow.rs` / `region.rs` /
  `wgc.rs` / `wgc_alt.rs` / `window.rs`、`core|perceive|reason/src/lib.rs`
  —— **零代码改动**，本机版本是严格超集。**保留本机。**
- `commands.rs` / `main.rs` / `+page.svelte` / `TierSwitch.svelte`
  —— 真实重写，**以对方为准**。

### 13.2 `[lints]` 一开，欠了 90 条文档

对方给 `crates/{core,perceive,reason}` 也加了 `[lints] workspace = true`，
于是 `missing_docs = "warn"` 第一次在这三个 crate 上真正生效：
**90 条**（core 30 / perceive 37 / reason 23）。

⚠ **这件事本身暴露了一个更值得记的问题**：本机 `crates/capture/src/lib.rs`
的豁免说明里白纸黑字写着「纯逻辑的那三个 crate **没有**这条豁免，
它们的公开 API 仍然受 `missing_docs` 约束」——**而它们从没 opt-in，
那句话一直是空话**。加上继承等于把它变成真的，于是欠账一次性暴露。

对方的选择是**把 `missing_docs` 从工作区 lint 表里删掉**（他们那边
166 条编不过）。本机**没有照做**，改为补文档：90 条全部补齐，
其中 44 个是结构体字段、18 个枚举变体，恰好是 `Dispatch` / 回执 /
`Outcome` / 归类 / `Message` 这一整套**领域词汇**——补上它们对读代码的人
是真的有用，删一条 lint 不是。

> 这一步是派两个子代理按 crate 并行做的（文件互不重叠）。
> 两个都**主动报出**了「还剩 2 条在 `crates/core/src/scale.rs`，不在我范围内」——
> 那 2 条是本机漏补的 `Billing::Patch` / `Billing::Tile` 变体本身。
> **子代理报「不属于我」时先去看它说的那处，不要默认它是对的。**

### 13.3 对方那份托盘实现**取代**了本轮自己写的那版

本轮先写的是「显式补一个 PNG 图标」。对方那版更深，且修掉一个
本机代码里**同样存在**的竞态：

> Tauri 是在 `.setup()` **之前**创建窗口的，面板的第一次 IPC 可能跑在
> `build_tray` 之前。那时 `tray_by_id()` 还是 `None`，于是
> 「收进托盘」这一档被判成不可用 → `closeBehavior` 读成 `exit` → **「−」永远不出现**，
> 两秒后设置页再问一次又读成 `tray`。同一进程、同一文件，答案不一样。

对方用**三态**（`TRAY_PENDING` / `TRAY_READY` / `TRAY_FAILED`）解决：
「还没报」和「报失败了」不是同一件事，只有**明确失败**才降级。
本机原版的 `tray_available() = app.tray_by_id(..).is_some()` 把两者
当成一件——**同一个坑**。

对方还多做了一件本轮完全没做的事：**托盘图标在 Auto 档带红点**
（`stamp_auto_dot` 在 RGBA 缓冲上就地画，半径与圆心都是量出来的，
不与气泡包围盒相交）。

### 13.4 磁盘

对方那份根 `Cargo.toml` 里有 `[profile.dev] debug = 1` 与
`[profile.dev.package."*"] debug = 0`。并入后**本机 `target` 里那些
按旧 profile 编出来的产物仍然占着 9.5 GB**——profile 改了不会回收旧产物。
下一次 `cargo clean` 之后才是真实体积。

### 13.5 没并的三处，以及为什么

| 东西 | 为什么留本机的 |
|---|---|
| `deny.toml` | 本机这份是 ADR-0003 演进出来的，对方那份是初版。**降级覆盖是倒退** |
| `ui/{tsconfig,svelte.config,vite.config,pnpm-workspace}`、`.gitignore` | 对方那几份是未 prettier 的脚手架，照搬直接挂 lint |
| `.agents/` `.claude/` `.minimax/` `.zcode/` `skills-lock.json` | 那是那台机器的 agent 环境，不是工程源码 |

⚠ **「对方那份更小 / 更朴素」不等于「对方那份更旧」。** 这几份里恰恰是
本机那份更完整。任何一次并入都要先跑 `diffcode.ps1` 分清方向，
再决定谁覆盖谁。

---

## 14. 2026-10-11：会话名闸的两处定位

并入 `6b2acbf` 之后继续查票 17 / 票 18，两张票的根因都定了，
其中一张是真缺陷，已修。

### 14.1 票 17 的根因是**我们自己的代码**（不是最小化）

症状是设置窗口停在 `(-32000,-32000)`、`vis=False`，而配置写的是
`"visible": true`。当时记的猜测是「可能被最小化了」——
**那个猜测是错的**。合并前的 `commands.rs` 里就写着：

```rust
/// **为什么要挪而不是直接 `visible: false`**：Tauri 2 的窗口配置里
/// `visible: false` 会让 WebView2 跳过 drop target 与合成器的注册，…
pub fn park_settings(app: &AppHandle) {
    let _ = w.set_position(PhysicalPosition::new(-32000.0, -32000.0));
    let _ = w.hide();
}
```

`main.rs` 的 `setup` 里无条件调它。那是**有意为之**（ADR-0001），
`-32000` 是字面量，不是 Windows 的最小化位置。
而「面板进不去设置」这个症状**从来没被复现过**——那天实机前台是游戏，
菜单项一个都没点着。

现在那个窗口已经不存在了：`6b2acbf` 把设置搬进了面板，
`open_settings` 变成「`show_panel()` + `emit("menu:settings")」，
前端监听器确认在 `+page.svelte:279`。

⚠ **`(-32000,-32000)` 在本项目里就是「我们自己藏的」的标记。**
下一个人看到它，别再往「最小化 / 没建成」上想。

### 14.2 票 18 的根因：闸管住了推理，**没管住消息累积**

`watch.rs` 的 `step` 里，`merge_into_stream` 原来排在身份闸**前面**：

```rust
s.messages = merge_into_stream(&s.messages, &messages);   // 先把 B 并进 A
let peer_ok = …;
if !peer_ok { s.candidates.clear(); /* 但 s.messages 没清 */ return …; }
```

用户在微信里切到会话 B 时，B 的消息**先被并进 A 的消息流**，闸才关。
闸挡住的是推理与投递，消息累积已经发生了。而 `s.peer` 每轮都在更新，
所以面板的状态是：**标题写着 B 的名字 · 下面挂着 A 的历史夹着 B 的新消息**——
正是那段注释原本要防的「看起来在处理 A，其实已经在看 B」。

**没变成隐私事故**：闸在推理之前 return，`history` 只在闸开时追加，
`collect_images` 也在闸之后。别的会话内容**没有出网**。
这是一处界面诚实性缺陷，不是硬约束第 1 条的破口。

已改：累积挪到闸之后；抽出 `apply_gate_closed()`，
在「指纹确实不同」那一支清 `s.messages`；「认不出头部」那一支**不清**
（认不出 ≠ 换了会话，否则消息列表会闪）。三条回归测试。

⚠ **那处顺序没有自动化测试守着**（`step` 要真帧真 OCR 才跑得起来，
本仓库没有读自己源码做结构断言的先例）。只有一行注释在解释它为什么重要。
谁把它挪回闸前面，不会有任何测试变红。

### 14.3 方法论：诊断脚本会把人带偏

票 17 整个开在一条诊断陷阱上：`scripts/winscan.ps1` **只报可见窗口**，
于是「窗口没被创建」和「窗口被藏起来了」长得一模一样——
输出里两者都不出现。脚本头部已补上提示并指向
`target/probe/pidwins.ps1 <pid>`。票 18 的排查还会撞上它。
---

## 15. 2026-10-11 实机：会话身份闸从来没关过（硬约束第 1 条实测破口）

§14.2 记的是「闸管住了推理、没管住消息累积」。这一轮实机复现时发现
**更严重的一层：闸本身就没关过。**

### 15.1 现象

在「古文山」和「文件传输助手」之间切一次会话：

```text
切之前  emit readout  peer="古文山"      messages=8  candidates=3
切之后  emit readout  peer="文件传输助手" messages=12 candidates=3
```

面板标题变成「文件传输助手」，`对方 4 · 我 8`，底下写着
「在等文件传输助手的新消息」。
**程序在给一个用户从未锁定的会话起草回复。**

### 15.2 根因：指纹量化太粗，而测试只测了亮度差

`identity.rs` 的指纹是「每通道 2 bit → 64 桶颜色直方图」。真实帧实测：

```text
区域 x308..708 y31..71（现状用的那块，16000 px）
  最大桶占比 = 98.8%     ← rgb(17,17,17) / (34,34,51) / (51,51,51) 全进第 0 桶
  直方图相似度 = 0.982   ← 阈值 0.90，永远判「同一个」
  会话名的墨迹          = 189 px（占 1.2%），推不动那 98.8%
```

**而旧测试一直是绿的**：`a_different_name_prints_differently` 用
`tint 225` 对 `tint 120`，**亮度差 105 级**。真实会话名之间亮度完全一样
（都是白字），差的是**字形**。那条测试证明的全是与现实无关的事。

### 15.3 改成二值墨迹掩码

背景取本区域**众数亮度**，差过 60 的像素算「墨」，再用 Sørensen–Dice。
比对时**只放宽「命中」一侧**（±1 px），分母仍是各自的真实墨迹数——
不先膨胀，因为膨胀会把两个不同名字的重叠也拉起来（0.2793 → 0.5483 → 0.6354），
那是往危险方向走。

| 量 | 同一会话 | 不同会话 |
|---|---|---|
| 旧：64 桶直方图 | — | **0.982**（闸失效） |
| 新：墨迹 Dice | **1.0000** | **0.2793** |

### 15.4 一并补上的：闸必须**看得见**

闸关着的时候界面原来一个字都不说（`reason` 是「本轮不动手」，
而前端理由栏只放行含「失败/错误/抓不到/收不到/异常」的条目）。
硬约束第 1 条要求这道闸是看得见的。现在 `Readout` 多了显式的
`gate_open`，面板据此显示「不认这个人，就不动手」+ 原因 +「锁定当前会话」。
**不靠中文串匹配。**

### 15.5 复验（同一对会话）

```text
切到 文件传输助手 → messages=0 candidates=0，面板显示「不认这个人，就不动手」
切回 古文山       → messages=6 candidates=0，闸自动开，消息重读回来
```

### 15.6 方法论：合成夹具会骗人

这一轮有**四处夹具**用亮度差造「另一个会话」（`identity.rs` 一处、
`watch.rs` 三处），全绿，而闸从来没关过。

> **造「另一个会话」的夹具，差别必须在字形/位置上，不许在亮度上。**

现在统一用 `named(tint, style)`：三个 style 亮度完全一样，只改笔画落点。

### 15.7 探针（都在 `target/probe/`，未跟踪）

| 脚本 | 干什么 |
|---|---|
| `uia-click.ps1` | 按**名字**点微信会话（截图换算屏幕坐标会偏 260 多 px；UIA 的矩形是 DPI 正确的物理坐标） |
| `idmeasure.ps1` | 量现状那块区域的桶占比与相似度 |
| `idmeasure2.ps1` | 候选签名对比（墨迹 / 灰度格） |
| `idmeasure3.ps1` | 膨胀半径与降采样格的取舍 |

⚠ 另记一条环境事实：**`winscan.ps1` / `hittrace.ps1` 报的窗口矩形与
`SetCursorPos` 用的屏幕坐标不是一个空间**（DPI 虚拟化）。
按它们算出来的坐标点击会偏 260 多像素——2026-10-11 实测。
要按坐标点谁，**用 UIA 的 `BoundingRectangle`**。