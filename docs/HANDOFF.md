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

⚠ **但那是 2026-10-09 的旧面板。** 此后重做了面板（新路由 `/panel`，
约 570 行）、换了应用图标、改了档位标记的幅面——**这三样都还没上过真机**。
真机验收清单见 §6 末尾。

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
  → 裁到消息区（`ViewPlan::PPOCR`：ChatPane + 1× + 最近邻）→ PP-OCRv6 small 离线识别
    ⚠ **2026-10-18 订正**：原文写「整帧 2x 最近邻放大 → Windows.Media.Ocr」，
    那是 **WinRT 那条路**（`ViewPlan::WINRT`：WholeFrame + 2×），而它现在已是
    **显式回退**（`CHATSERVANT_OCR=winrt`）。默认引擎自 2026-10-18 起是 PP-OCRv6 small
    + ONNX Runtime，**看的是裁剪件、原生 1×**——两半都不再是默认路径。
    判据：`crates/perceive/src/pipeline.rs` 的 `ViewPlan`，由引擎决定（票 39 实测）。
  → 逐行归类（我 / 对方 / 标题 / 碎片 / 图片内字）
    ⚠ **2026-10-21 订正**：回退那条路（WinRT）的重采样**已从最近邻改成双线性**
    （`ocr::Engine::resample()`）。同批真机帧上 CER 0.308 → 0.233，耗时不变；
    票 41 的 ③（「个别行整行乱码」）就是这么修的。
    **归类量的是原始像素**（未放大的裁剪件），所以换重采样**没动任何阈值**。
  → 头部 400x40 像素指纹锁会话
  → 真实模型：判断意图 → 起草候选 → 按概率排序
  → Tauri 事件推送 → Svelte 5 面板渲染
  → 点「填入」：剪贴板字节级往返 + 和弦 Ctrl+V → 落在微信输入框
  → `Auto` 档：程序按最高置信度自动填入并发送（带冷却与单轮一次的护栏）
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
**这条只对 WinRT 成立。** WinRT 那条路的方案是：**永远 OCR 整帧（2×）**，
行框换算到消息区坐标后再按 ROI 过滤。不要改回裁 ROI 带子。

⚠ **2026-10-21 订正**：这一段原文写的是「2x 最近邻」——
WinRT 那条路的**重采样已改成双线性**（`ocr::Engine::resample()`）。
倍数没变（还是 2×），变的是重采样方式：
最近邻会把一行小字读崩成乱码（实测一行 50 字读成 106 字），
双线性读回来基本正确（同帧编辑距离 63 → 4，CER 0.308 → 0.233，耗时不变）。
**诊断口径（台架、`--scale` 验字号）仍然用最近邻**——两件事，别混。

⚠⚠ **2026-10-18 订正：原文这段没有限定引擎，而默认引擎已经不是 WinRT。**
**PP-OCRv6 的结论正相反**：整帧 1146 ms / 50 行，**裁到消息区 455 ms / 5 行，
而且读得更准**（票 39 实测）⇒ 默认路径**就是裁剪**，
由 `ViewPlan::PPOCR = { ChatPane, upscale: 1 }` 决定，由引擎自己带。
⇒ **别把这条禁令套到 PP-OCR 上**：照着原文去「改回整帧」，
等于把默认引擎那份已实测的提速与提准一起撤掉。
判据：`crates/perceive/src/pipeline.rs` 的 `ViewPlan`，**两个引擎的值不同**。

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

**⚠ 但 2026-10-16 实测：本面板是这条的例外，浏览器量具可以用。**

面板内部被 `cs-demo-frame` **钉死 380×560**（与 `tauri.conf.json` 的窗口、
`probe-sheet.svg` 的声明三处一致），**与视口无关**。
两张不同视口的截图实测（`ui/scripts/_cardbox.py`，内容包围盒 ÷ 2）：

| 视口 | 卡片宽 | 卡片高 |
|---|---|---|
| 520×760 | **382.0** | **561.0** |
| 1200×900 | **382.0** | **561.0** |

（382×561 含卡片自身约 1px 的投影，即窗口本体 380×560。）

⇒ 所以 `shoturl.ps1` 默认的 `-Width 520 -Height 760` 里，**520/760 是视口、
不是面板**——面板在两种视口下量到的几何完全相同。
「截图尺寸证明不了布局尺寸」这条**对别的页面仍然成立**，对**本面板不成立**。

⚠ 曾经因为把 520 当成面板尺寸，在票 44 里记下一条「量具尺寸与真机对不上」的
   **假未决项**。教训：先确认量到的那个矩形**是被测对象还是它的容器**。

### 4.10 测试文件也可能是死的

`ui/src-tauri/src/contract.rs` 从落盘到 2026-10-13 **一次都没编译过**——
`main.rs` 里从来没写过 `mod contract;`。里面十几条看起来很认真的测试
**一次都没运行过**。

它是被一条 commit（`d67487b fix(构建): 让没编译过的 Rust 编译通过`）引进来的，
那条 commit 修的是「让它能编过」，没人核对它**有没有被 `mod` 声明**。

⚠ **`cargo build` / `clippy` / `test` 绿，只证明你编译到的那些代码是对的。**
「新增一个 `.rs` 文件」和「新增一个函数」不是同一件事：后者的错编译器会告诉你，
前者**一声不吭**。详见 `.scratch/…/26-contract-module-was-never-compiled.md`。

### 4.11 「模型不准」的结论，可能是在**模型没跑**的时候得出的 ⭐

2026-10-15 实锤：`BucketCache::plan_path()` 拼出的 rec 权重文件名
（`ch_PP-OCRv4_rec_mobile_w{w}.onnx`）**在磁盘上不存在**，
每帧都加载失败 → 整轮报错被吞成 `None` → **回落 WinRT**，
而那条回落 warning 被 `xtask` 的 `with_env_filter` 丢掉了，**一条日志都没有**。

⇒ 所有关于「PP-OCR 认不准」的历史结论**都作废**——
量的是 WinRT 的逐字切词，却当成了 PP-OCR 的 det 行为。

**判据不是读日志，是比耗时：**

| 强制引擎 | 耗时 | 输出 |
|---|---|---|
| `CHATSERVANT_OCR=ppocr` | **13456 ms** | 两者**逐字节相同** |
| `CHATSERVANT_OCR=winrt` | **309 ms** | ↑ |

13 秒和 0.3 秒不可能是同一个引擎产出的。
**「跑一次特别慢」本身就是「它在失败」的信号**，比任何日志都可靠。

修好之后（票 39）PP-OCR 真能跑，但**仍然不成句**（det 框纵向只盖住字形上半），
而且 2× 单帧 **108.9 s**（debug）——比 WinRT 慢 200~600 倍。
**该选哪条路还没定**，票 39 是待决。

⚠ 顺带一条量法：**报 OCR 准确率必须带「放大 N×」**。
1× 下读得很差、2×（生产条件）下大部分正文是对的，
用 1× 的结论去否定整条 OCR 路线会推错方向。

---

## 5. 本机怎么跑

```powershell
# 一次跑完全部门禁
# ⚠ 通篇别数「几步」。`gate.ps1` 里 `Step` 调用的**总数**（2026-10-18 实测 12）
#   与**实际执行数**天生不等：`secret scan` 在没设 `CHATSERVANT_PROBE_KEY` 时跳过，
#   CI 用占位 key 必跑 ⇒ 两侧执行数也不相等。
#   2026-10-16 起脚本会打印分步耗时表，那是唯一准确的清单，别把任何计数抄进文档。
pwsh -NoProfile -File scripts\gate.ps1

# 开发内循环：只跑其中几步（结论只覆盖跑过的步，脚本会打 GATE_SCOPE: PARTIAL）
pwsh -NoProfile -File scripts\gate.ps1 -Skip 'deny,ui build'
```

⚠ `cargo deny` 要联网拉 advisory-db，但**本机直连 github.com 是通的**，
不必先挂代理（2026-10-15 核实 `git ls-remote` ⇒ exit=0；
2026-10-16 完整门禁实测：`cargo deny check` 直连 **11.9s 通过**）。
`scripts\deny.ps1` 默认直连，只有设了 `CHATSERVANT_PROXY` / `HTTPS_PROXY`
才走代理。**这一节原先写「直连不通、必须先挂代理」，是过期结论。**

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
⚠ **2026-10-13 该目录已整体清理**（228 个文件 / 86 张图，含真实微信窗口
截图）。票据与 `设计规范.md` 引用过的取证帧（`text-chat.png` /
`panel-tick-big*.png` / `ring-size-compare.png`）**清理前就已全部丢失**，
所以那次清理没有再丢任何东西——详见票 07 的更正段。

---

## 6. 下一刀看哪里

**不要在本文件里找待办**——那属于 `.scratch/`，而且会立刻过期。

- 票据与阻塞关系：`.scratch/wechat-reply-copilot/issues/`
  （`Blocked by` 写明了谁挡谁，`Status` 写了能不能立刻做）
- 未决的设计问题：`docs/设计规范.md` §10「未决」

**当前队列的先后**（2026-10-13 晚，细节以票据为准）：

1. ✅ **旧面板已删、主窗口已切到纸面板**（票 25，本轮完成）。
   `/` 与 `/panel` 现在共用 `$lib/PaperPanel.svelte`，窗口控制挂在撕口那条
   上沿（同时是拖拽带），设置是纸面板内的一块互斥主区。
   ⚠ **只剩「冷启动实机」一项未验**——本轮改动全在 `ui/src`，但真 Tauri
   窗口里显示的是新面板这件事只有起一次进程才算数。
2. ~~**正文区的动作行 + 按钮组件族**~~ **已做**（2026-10-15，提交 `6417804`）。
   三变体按 `设计规范.md` §4.2c 落地，映射是「不可逆优先于推荐」：
   `Auto` 档整组红批，`Manual` 档首选实墨、备选墨线。
   ⚠ **真机只验了 `Manual` 档**，`Auto` 档那半条仍欠票 20。

   > ⛔ **本机不要在真机上把档位切到「自动发送」去试。**
   > 切档那一刻起，观察循环里的 `AutoGate` 就会在**对方下次发消息时自动发**，
   > 不只是「点按钮才发」。而本机没配 API key，起草器是 `PresetReasoner`，
   > 候选正文恒为「预置候选，没配置 key，不是模型输出」——
   > 往真人会话里发出去的是这句占位话。
   > 要看 `Auto` 档长什么样，用夹具出图：
   > `pwsh -NoProfile -File scripts\shoturl.ps1 -Url "http://localhost:1420/panel?demo=1&state=auto" -OutPng target\probe\act-auto.png`
   > ⚠ **别用 `?tier=Auto`**：夹具里的 `tier` 字段会盖掉那个探针口（实测）。
   > ⚠ 真机面板截图用 `shotwin.ps1` 收**十六进制**句柄、**相对**输出路径，
   > 且它走 `PrintWindow`（截托盘溢出那类 XAML island 会压成竖条，
   > 但普通面板窗口没问题）。
   >
   > ✅ **顺带修掉一个假警报（同一个提交）**：「仅填入」档点候选后，
   > 面板曾显示「可能已经发出去了」而对话流根本没有新消息——
   > `fill_input_box` 全程没按回车却返回了 `PossiblySent`。
   > 现已拆成 `DeliveryReceipt::Filled`，面板如实写「没有发出去」。

   ⚠ 顺带收掉一个现在看得见的分裂：设置页里的档位控件仍是旧 `SegSlider`
   （墨底滑块），与顶边那道红铅笔勾是两套形态语言，并排出现在同一屏。
   → `.scratch/…/23-button-component-family.md`
3. **手写体落到 `ui/src`。** 字号已定（21px，见票 22），
   但字体标尺本身还没进前端，**demo 绿不等于真机可读**。
4. ~~**补两个从未 emit 的事件名。**~~ —— **票 28 已于 2026-10-14 结掉**，
   选的是**删干净**而不是补 emit（理由见票内）。调试画布改成明摆着的
   「重新拉取」按钮，离线横幅改由一次真实的 `invoke('latest_readout')` 触发。
   ⇒ 新增 `contract::commands`（28 条命令）并配了两条双向对账测试。
5. ✅ **图标：分离度 → 载体重做 → 画布裁边距（三轮，票 29 / 34）**
   - **2026-10-14 票 29** 拍板「压暗纸」：`PAPER_BRIGHTNESS = 0.52`
     （浅侧 3.91:1 / 深侧 3.80:1）。⚠ **这条路已被票 34 推翻**——两侧中位最平衡，
     但深色侧 3.80 意味着它在托盘里**不跳出来**，正是用户抱怨的「一团灰」。
   - **2026-10-14 票 34** 改为**双色分置**：`PAPER_BRIGHTNESS = 1.0`（奶白纸）
     \+ 1px 深墨硬描边（给浅色任务栏）\+ 实心深红勾。深侧 median **10.75:1**。
   - **2026-10-15 票 34 续** 修了**画布死边距**：勾宽 350 / 画布 512 ⇒ 内容只占
     **68.4%**，而 Windows 的托盘格子是**系统定的**，于是同一格里邻居约 90% 满、
     我们 68% 满 ⇒ 真机肉眼「小一号」（实测邻居 15.6px、本图 11.7px）。
     修法是 `icongen.py` 的 `crop_dead_margin()` + `CANVAS_FILL = 0.96`
     （扫档量出来的，不单调，见票 34 的表）。**设计坐标一个字没改**，
     所以 `PAPER_STROKE_PX=32 ⇔ 托盘 16px 上 1px` 那条换算继续成立。
     代价据实记：深侧 median 10.75 → **9.33**（仍过 8.0 门槛）。
     ⚠ 图标 `include_bytes!` 编进二进制 ⇒ **改了要重启程序**才看得见。

6. ✅ **票 24 · 小字下限（2026-10-14 结掉）** —— 量具**已重建**在
   `ui/scripts/analysetype.py`（⚠ 不能放 `target/`，会被清理带走），
   工作台加了 D 段 `/studio?only=small`。结论**与 v1 的「下限 12px」相左**：
   12px 与 11px 的断笔风险几乎一样，**杠杆是字重不是字号**（400→600 把 ≤1px
   细笔画从 ~78% 压到 ~16%）。⇒ **11px 下限，三行小字全部 600**，已落码
   （`设计规范.md` §4.3）。⚠ 口径：纯色纸底 + Chrome 3× 出图，是**乐观下界**，
   真机（WebView2 + 纸纹）只会更差，**仍要真机复看**。
7. ⛔ **票 30 · 会话闸：根因已定案并修完，但只有测试验过。**
   根因不是坐标算错，而是**先裁后取样**——`chat.y0 == header_top` 是定义，
   标题栏在 `chat.y0` 之上、裁剪时已丢掉。已改为从**整帧**取样带，
   两条新判据都自证过会响（退回旧语义 ⇒ 5 条红）。
   **待验三条都要开真实微信**：`header_top` 实测 31 时 400×31 带子够不够
   `MIN_PIXELS`；那 31px 里到底有没有会话名；两个会话标题栏像素实际差多少。
   ⇒ 文档里「闸认的是会话身份」在真机复验前**只能算代码上成立**。

**需要本机微信才能做的验收（云端做不了）**：两档发送、phase 事件、
外观 IPC 往返这三项的实机部分，**加上票 25 的冷启动与票 30 的待验三条**。
**它们一次都没验过**——文档里「全链路已跑通并真的发出去一条消息」
指的是 2026-10-09 的那一次，纸面板与新图标都还没上过真机。

⚠ **票 22 仿宋 75.9% 细笔画**与**票 24 的小字数据**都出自
纯色纸底 + Chrome 3× 出图，是**乐观下界**；票 23 的焦点环 / 禁用态
也还没验（且 `grep cs-btn` 回归那一项未勾）。

⚠ **门禁别用管道缓冲看**：`gate.ps1` 每一步都接
`Select-Object -Last N`，从外面再套一层管道会一直缓冲到结束，
中途看不到任何输出，看起来像卡死（本轮就撞过一次：pwsh 零 CPU、
没有 cargo 子进程）。**要么等它跑完，要么分步跑**：
`scan-mojibake.ps1` → `sheetpack.ps1 -Check` → `cargo fmt --check`
→ `cargo clippy` → `cargo test` → `pnpm check` / `pnpm lint`。
