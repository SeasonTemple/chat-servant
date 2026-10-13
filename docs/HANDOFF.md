# 交接说明

> **这份文件只在「换设备 / 换会话」时读。**
> 它是一次**冷启动包**：让接手的人在半小时内知道现在能干什么、
> 哪些路已经被证伪、以及每条命令怎么跑。
>
> ⚠ **不要往这里追加「YYYY-MM-DD 进展」章节。** 本文件 2026-10-13 之前
> 有过 §11~§19 共九节按日追加的日记（770 行），把真正的冷启动信息压到了
> 下面一百多行处，已经开始沉底。日常进展记在 §0 指的地方。

---

## 0. 文档地图 —— 进展去哪了

**这张表是本文件唯一的作用。** 记东西之前先查它，否则你会在三个地方各写一遍。

| 要记的东西 | 记在哪 | 形式 |
|---|---|---|
| 硬约束、常用命令、踩过的坑 | `AGENTS.md` | 逐字一致的条款 |
| 域词汇（`Frame` / `Dispatch` / …）与硬约束的**现在时** | `CONTEXT.md` | 术语表 |
| 一个决策，**尤其是被否掉的替代方案** | `docs/adr/NNNN-*.md` | 不可变，被 supersede 而非原地改 |
| **待办与进展**（做到哪一步、验收过没有） | `.scratch/<feature>/issues/*.md` | 勾 `- [x]`，与提交同一个 commit |
| 设计规范、未决设计问题 | `docs/设计规范.md` / `docs/面板设计.md` | §编号，可引用 |
| 为什么要这么改 | commit message | Conventional Commits |
| 环境事实（依赖版本、当前状态） | **直接跑命令** | `git log` / `cargo deny check` |

> ⚠ **最后一行是最容易被违反的一条。** 抄一份 `git status` 或
> `cargo test` 的输出进仓库，必然过期，而且过期之后没人会去删它——
> `docs/改动清单.md` 就是这么来的（2026-10-09 的一次 `git status` 粘贴，
> 46 行里 40 行已经在说不存在的文件，文件名还带着 UTF-8 转义乱码）。
> **已于 2026-10-13 删除。**

---

## 1. 三十秒摘要

Windows 桌面程序：在你自己的微信单聊窗口旁边挂一块悬浮面板，读取对方新消息，
调模型起草 2~3 条回复，落在面板上，你点一下填进输入框，再点一次才发。
**默认档只填不发。**

全链路已在真机上跑通并**真的发出去一条消息**。

⚠ 这是一个**个人本地工具**：只读你自己设备上、你自己有权查看的对话。
不 hook、不注入、不读对方数据库、不碰对方进程内存。截图不落盘。

---

## 2. 环境边界

### 2.1 云端会话能做 / 不能做

| 任务 | 云端 | 为什么 |
|---|---|---|
| UI 设计迭代 | ✅ **完全可以** | Svelte 5 + CSS，纯静态，见 §3.3 |
| OCR 层（`chatservant-capture` / `-perceive`） | ⚠️ 只能写代码，不能编译 | `crates/capture` 整个建在 `windows-rs` 上 |
| Tauri 外壳（`ui/src-tauri`） | ❌ 编译必挂 | 同上（Win32 / WinRT 专有 API） |
| 微信实机验证 | ❌ 只能本机 | — |

**合理的分工**：

```text
云端 LLM 写代码  ──►  源码上传  ──►  本机跑门禁 + 实机验证  ──►  结论回传
```

云端能写、不能验。凡是「跑一下才知道对不对」的东西，都得回到本机。

### 2.2 本机环境

工具链 Rust + Node ≥ 18 + pnpm（前端）。模型 key 只放环境变量
（`MINIMAX_CN_API_KEY`），**任何文档、日志、提交信息都不要写它**。

shell 是 PowerShell。**复杂命令一律落 `.ps1` 用 `pwsh -NoProfile -File` 跑**，
不要用 `-Command` 拼——引号会被吃掉，`Set-Content -Encoding UTF8` 还会写 BOM
（§3.7 有完整的后果）。

⚠ **磁盘**：`target/` 没有 GC，只增不减。改 `[profile.*]` 或增删 feature
会让旧产物**永不删除**（实测一次 profile 改动 + 一次 feature 改动：
0.27 GB → 10.4 GB）。改完立刻 `cargo clean --profile dev`。

---

## 3. 现在能干什么

### 3.1 全链路（实机验证过）

```text
真实微信画面
  → WGC 抓帧（不受遮挡影响，实测 6/9 采样点被盖住照样跑通）
  → 整帧 2x 最近邻放大 → Windows.Media.Ocr 离线识别
  → 逐行归类（我 / 对方 / 标题 / 碎片 / 图片内字）
  → 头部 400x40 像素指纹锁会话
  → 真实模型：判断意图 → 起草候选 → 按概率排序
  → Tauri 事件推送 → Svelte 5 面板渲染
  → 点「填入」：剪贴板字节级往返 + 和弦 Ctrl+V → 落在微信输入框
  → 点「确认发送」：抢焦点 + 回车
```

实机记录（「文件传输助手」会话，真实模型，2026-10-09）：
认出会话名 → 归并 6 条消息 → 3 条候选 → 填入输入框 → **真的发出了一条**，
内容与候选一字不差、且只有一份。

### 3.2 性能（1073 轮统计）

| 场景 | 耗时 |
|---|---|
| 静默轮（画面没变，零模型调用） | 1.07 ~ 1.10 s |
| 起草轮（含三次**串行**模型调用） | 6.2 ~ 9.1 s |
| 进程启动 → 第一份 readout | 6.0 ~ 7.8 s |

**瓶颈是三次串行模型调用**，不是抓帧也不是 OCR。

### 3.3 三条不靠微信的验证姿势

**这是本项目最重要的基础设施。接手后优先用它们，不要再去人肉点微信。**

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
cargo xtask e2e                    # 全链（含注入回读）
cargo xtask fixture --name X --source <png>   # 生成夹具 + manifest
```

`ocr --scale 2` 会打印**每一对相邻词的间隙和字高比值**——改任何跟空格、
断行、分栏有关的逻辑之前，先看这个数，别拍阈值。

#### `?demo` 模式（界面脱离 Tauri 跑）

```bash
cd ui && pnpm dev      # 然后开 http://localhost:1420/?demo=1&state=drafted
```

`ui/static/demo/*.json` 是**手写的演示数据**（假内容，形状仿真真实 OCR 输出）。
每个状态一份文件，`state=` 选哪一态（`drafted` / `drafting` / `failed` /
`armed` / `confirm` / `noprep` / `silent`）。

演示模式下工具栏禁用、点候选会明说「不会真的发送」——**不装假控件**。
**云端改 UI 就在这个页面上迭代。**

⚠ 夹具里手写的字段（尤其 `phase`）曾经骗过人：真实链路少发一个字段，
八态却照样在 demo 里全渲染得出来。改数据契约时，**demo 绿不等于真机通**。

---

## 4. 已经证伪的路 ⭐

**这一节比前面都重要。** 以下每一条都是**实机撞出来的**，不是理论推测。
重复踩一遍的代价远高于读完这 90 行。

### 4.1 `WM_PASTE` 投给子窗口 —— 不可行

微信 4 顶层窗口下只有 **2 个子 HWND**，全是 Qt 的，没有原生 `Edit`：

```text
0x30FA0  class=Qt51514QWindowIcon  1x1  text='Weixin'
```

Qt 应用只创建一个顶层 HWND，所有控件由 Qt 自己绘制——**没有投递目标**。

证据脚本：`scripts/probe-input.ps1`

### 4.2 UI Automation `SetValue` —— 可用，但**抢不掉也躲不掉**

```text
[71] Edit class=mmui::ChatInputField rect=(1214,763,819x209) [Value,Text,Invoke]
[100] Button '发送' class=mmui::XOutlineButton
[104] List  '消息' class=mmui::RecyclerListView                [Value,Invoke,Selection]
[129] List  '会话' class=mmui::XTableView                      [Value,Invoke,Selection]
```

实机写入（Chrome 在前台）：

```text
写入前，前台窗口 = 0x10ECA  '...bilibili... - Google Chrome'
输入框现在内容 = '后台写入验证'
写入后，前台窗口 = 0x30F6C  '微信'          ← 微信自己激活了
```

**结论：「写进微信自己的输入框」本身就会抢前台**，换哪条 API 都一样——
这不是投递方式的问题，是那个控件的语义（给输入框赋值 → `SetFocus`）。

完整推理见 `docs/adr/0005-delivery-and-focus.md`。
证据脚本：`scripts/uia-set.ps1`、`scripts/dump-uia.ps1`

### 4.3 OCR 不能裁紧 —— 「乐」气泡被整段丢掉

同一块画面：

```text
整帧 2x        5 行，读到「乐」（对方消息）
ROI 带子 2x   4 行，读不到「乐」
```

`Windows.Media.Ocr` **需要行周围有内容才收**，裁得越紧漏得越多。
现方案：**永远 OCR 整帧（2x 最近邻），行框换算到消息区坐标后再按 ROI 过滤**。
不要改回裁 ROI 带子。

### 4.4 空格阈值不要拍脑袋

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
⚠ 曾经降到 1/3 去迁就一个**自己编的测试几何**，结果真帧上「B0」被拼成「B 0」。
`crates/capture/src/ocr.rs` 里七条测试全部用**实测几何**，别改成等宽方块。

### 4.5 `probe frame` 不要用 `printwindow`

`PrintWindow` 对走 DirectComposition 的窗口（WebView2）返回**黑帧**。
整屏截图才是真相。`scripts/shotwin.ps1` 会骗人。

### 4.6 WGC 会话别每帧新建

每帧新建整套 WGC 会话会**泄漏 12 个线程/帧**（实测增速 9 线程/秒）。
现在是一条长驻工作线程 + 一个会话（`wgc_alt.rs` 的 `worker()`）。
改这块之前先跑 `cargo xtask thread-probe --n N`。

### 4.7 别用 `Set-Content -Encoding UTF8` 改源码

PowerShell 5.1 下它会**写 BOM**。它先后坑过两次：

1. 把 BOM 加到 `+page.svelte` 第一行——prettier 容忍、`svelte-check` 也过，
   **只有 svelte-autofixer 报错**，而且报的是 `at line undefined` 这种
   **没有位置的错**，差点被当成工具抽风。
   排查手法：把文件 `git stash` 掉再跑 autofixer，二分定位是不是自己刚写进去的。
2. 2026-10-13 做票 20 时用它做批量替换，直接给 `commands.rs` 加了 BOM。

**改源码一律用 `edit` / `write` 工具。** 批量替换后立刻验：

```powershell
$b = [IO.File]::ReadAllBytes($p)
if ($b[0] -eq 0xEF -and $b[1] -eq 0xBB -and $b[2] -eq 0xBF) {
  [IO.File]::WriteAllBytes($p, $b[3..($b.Length-1)]); 'BOM stripped'
}
```

### 4.8 合成夹具会骗人

四个用**亮度差**造出来的「另一个会话」夹具全部通过，而会话身份闸
**从来没关过**——那是硬约束第 1 条的实测破口。

⚠ 造夹具时，差异必须在**字形/位置**上，不许在亮度上。同一类毛病还有：
二值裁剪（`SetWindowRgn`、mask 阈值化）没有中间灰阶，边缘必然是阶梯，
放大 4~8 倍看全是锯齿——读起来像「图像缺了一块」而不是材质。

### 4.9 验收工具报的尺寸 ≠ 被测布局的尺寸

Chrome 把窗口宽度钳到最小 500px，而 `--screenshot` 输出的图**仍等于**
`--window-size`——也就是拿到的是「500px 布局被裁成 380px」的图，
**输出尺寸证明不了布局尺寸**。`--force-device-scale-factor` 只放大输出像素、
CSS 视口不变；`--headless=old` 也钳。

> 靠这条假测量得出过两个「bug」（面板溢出、`background` 简写被 Svelte 丢掉），
> 两个都不存在，其中一个还被写进了代码注释。
> **判据要独立于工具自报值**：放一个已知尺寸的标记，或直接读
> `innerWidth` / `getBoundingClientRect` 再反查截图。
> 窄视口不要用浏览器量，改用真机窗口截图（屏幕 DC + BitBlt，不是 PrintWindow）。

### 4.10 测试文件也可能是死的

`ui/src-tauri/src/contract.rs` 从落盘到 2026-10-13 **一次都没编译过**——
`main.rs` 里从来没写过 `mod contract;`。里面十几条看起来很认真的测试
**一次都没运行过**。

它是被一条 commit（`d67487b fix(构建): 让没编译过的 Rust 编译通过`）引进来的，
那条 commit 修的是「让它能编过」，没人核对它**有没有被 `mod` 声明**。

⚠ **`cargo build` / `clippy` / `test` 绿，只证明你编译到的那些代码是对的。**
「新增一个 `.rs` 文件」和「新增一个函数」不是同一件事：后者的错编译器会告诉你，
前者**一声不吭**。详见 `.scratch/…/26-contract-module-was-never-compiled.md`。

---

## 5. 本机怎么跑

```powershell
# 一次跑完全部门禁（九步）
$env:HTTPS_PROXY='http://127.0.0.1:7897'; $env:HTTP_PROXY='http://127.0.0.1:7897'
pwsh -NoProfile -File scripts\gate.ps1
```

⚠ `cargo deny` 要联网拉 advisory-db，而 github.com 直连不通，
**先把代理挂上**再跑完整门禁（`gate.ps1` 会继承环境变量）。

⚠ **别只看门禁的退出码，要看它打印的那两行**
（`GATE: ALL GREEN` / `GATE: FAILED`）。2026-10-11 实测：
`gate.ps1` 原来只打印而没有 `exit`，于是 `pwsh -File` 永远返回 0——
一个带红门禁的 commit 被提交了。现在补了 `exit $fail`，两者一致；
但判断时仍以打印的那两行为准。

启动与探针：

```powershell
cd ui && pnpm tauri dev           # 前端 dev server 在 http://localhost:1420
cargo xtask probe frame           # 抓一帧聊天窗口到 target/probe/，人工看像素
pwsh -NoProfile -File scripts\fgwin.ps1      # 前台窗口
pwsh -NoProfile -File scripts\winscan.ps1    # 只报可见窗口
pwsh -NoProfile -File scripts\shotwin.ps1 <hwnd> <out.png>
```

⚠ **改前端靠 HMR，不要重启 exe。** dev server（1420）开着时改
`.svelte` / `.css` 会实时推给运行中的进程。只有改了 Rust 才重新编译。

⚠ `target/probe/` 里的证据图**可能含真实聊天内容，不要提交进 git**。

---

## 6. 下一刀看哪里

**不要在本文件里找待办**——那属于 `.scratch/`，而且会立刻过期。

- 票据与阻塞关系：`.scratch/wechat-reply-copilot/issues/`
  （`Blocked by` 写明了谁挡谁，`Status` 写了能不能立刻做）
- 未决的设计问题：`docs/设计规范.md` §10「未决」
- 两个待拍板（需要人决定，agent 不该替他们定）：
  - **票 22**：正文手写体的字号 19 / 21 / 23px
  - **票 25**：`PanelSettings` 这类共用组件留是移
- 需要本机微信才能做的验收（云端做不了）：票 21、票 20 各自的真机项
