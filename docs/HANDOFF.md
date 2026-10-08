# chat-servant 现状交接

写于 2026-10-09。这份文档回答两个问题：**现在能干什么**，以及
**接下来该干什么**。所有结论都来自实机或门禁，不是推测。

---

## 1. 一句话

在 Windows 上盯住你自己的微信单聊窗口，读取对方新消息 → 调模型起草
2~3 条回复 → 落在旁边一块悬浮面板上 → 你点一下填进输入框，
再点一次才发。**默认档只填不发**（硬约束第 5 条）。

---

## 2. 已经跑通的东西（实机验证，非推测）

### 全链路

```text
真实微信画面
  → WGC 抓帧（不受遮挡影响）
  → 整帧 2x 最近邻放大 → Windows.Media.Ocr 离线识别
  → 逐行归类（我 / 对方 / 标题 / 碎片 / 图片内字）
  → 头部 400x40 像素指纹锁会话
  → 真实 MiniMax 模型：判断意图 → 起草候选 → 按概率排序
  → Tauri 事件推送 → Svelte 5 面板渲染（GSAP 动效）
  → 点「填入」：剪贴板字节级往返 + 和弦 Ctrl+V → 落在微信输入框
  → 点「确认发送」：抢焦点 + 回车
```

在「文件传输助手」会话上实测：认出会话名、归并出 6 条消息、
起草 3 条候选、填入输入框、**真的发出去一条**（内容与候选一字不差、
且只有一份）。

### 性能（实机 1073 轮统计）

| 场景 | 耗时 |
|---|---|
| 静默轮（画面没变，零模型调用） | 1.07 ~ 1.10 s |
| 起草轮（含三次串行模型调用） | 6.2 ~ 9.1 s |
| 进程启动 → 第一份 readout | 6.0 ~ 7.8 s |

**主要成本是三次串行模型调用**，不是抓帧也不是 OCR。这是下一步最值得砍的地方。

---

## 3. 夹具模式：不需要微信也能跑（新增）

```powershell
# 抓一帧当前画面
cargo xtask probe frame --method wgc
# 让整个应用吃这张冻结的帧
pwsh -File scripts/run-app-fixture.ps1 target\probe\frame-XXXX.png
```

`CHATSERVANT_FIXTURE=<png>` 一设，OCR、归类、会话锁定、起草、
事件推送、界面渲染**一条都不少**，只有「这一帧从哪来」变了。

**为什么要有它**：原来循环里写死了「找微信窗口 + WGC」，于是改一次界面
就得去点一次微信、截一次图、确认一次像素，而这些步骤每次结果都不一样。
于是「界面能不能跑通」这件事**没法被可靠地验证**，只能靠一次次人肉点击碰运气。

夹具模式下画面每轮一样，所以第一轮起草之后循环自然进入静默期——
这正好也是要验的行为（硬约束第 6 条）。

**夹具模式下 `dispatch` 直接拒绝投递**：夹具拿的是真微信的帧，
这时候按「确认发送」会把内容打进一个真对话里，而这个人并没有要求发。

> 真实帧**不入库**：`crates/capture/fixtures/*.toml` 里明写了
> 「原本都含真实联系人名与消息正文，不入库」。要提交测试夹具得先脱敏。

---

## 4. 硬约束与它们各自的落点

| # | 约束 | 代码落点 | 实机验证 |
|---|---|---|---|
| 1 | 只读自己有权查看的对话 | `watch.rs` 的头部像素指纹闸；认不出 = 不动手 | ✅ |
| 2 | 只用窗口级捕获 + 本地离线 OCR | `wgc_alt.rs` + `ocr.rs`（WinRT，本地） | ✅ |
| 3 | 截图不落盘 | 帧只存在于内存；唯一例外 `xtask probe` 写 `target/probe/` | ✅ |
| 4 | 不碰钱 | 起草 system prompt 禁转账/红包/收款 | ✅ |
| 5 | 发送分档、默认最低 | `Dispatcher` + Rust 侧 `TierCell`（前端只请求改） | ✅ 三档都实测过 |
| 6 | 静默期零调用 | `PaneWatcher::observe` 在画面没变时返回 `Quiet` | ✅ 静默轮零模型调用 |
| 7 | key 不落普通文件 | `secret::load` 只读凭据管理器/环境变量；文本全过 `redact` | ✅ `cargo xtask secret-scan` |

许可门槛：**只允许 permissive**。策略由根目录 `deny.toml` 机器执行，
`cargo deny check` 是门禁之一。新增依赖时许可证从 crates.io 元数据确认，
不从项目主页推测（ADR-0003）。

---

## 5. 本轮修掉的缺陷（全部实机复现，非推测）

| 缺陷 | 症状 | 根因 |
|---|---|---|
| 面板整块空白 | 后端 `messages=7`，界面一条不显示 | Rust `Message` 没有 `id`，前端 `{#each ... (m.id)}` 的 key 全是 `undefined` |
| 候选活不过两秒 | tick=1 有 3 条，tick=3 变 0 | 增量带里只有自己说的话时无条件 `candidates.clear()`，而水位线又不推进 |
| 重锁后永远读不到东西 | 点「锁」锁上了但一条也不出 | `apply_relock` 清了历史/摘要/候选，唯独漏了水位线 |
| 候选被挤出屏幕 | 9 条消息时整块候选看不见 | grid 按位置行排版，条件渲染的异常条让所有孩子错一格 |
| 「古 文 山」插空格 | 标题栏显示带空格的会话名 | Windows OCR 逐字切分中文，`OcrLine.Text()` 一律用空格拼 |
| Confirm 档发不出去 | 点一次「再点一次」，点一百次还是 | 判据写成 `tier==Confirm && action=="confirm"`，两次点击条件完全相同 |
| 发出去是两份拼接 | 同一句话连着两遍 | 装填成立后又调 `fill_and_send`，而 `fill_input_box` 是**追加** |
| 闸关上面板零反馈 | 微信里换会话，面板一动不动还挂着旧会话 | 闸关闭时 `step` 返回 `Ok(None)`，一条 readout 都不发 |

每一条都配了回归测试，合计新增 **11 条**。

---

## 6. 投递路径与「不抢前台」——ADR-0005

实测两条路：

- **A. 往子 HWND 发 `WM_PASTE`** —— 微信只有 2 个 Qt 子窗口、
  没有原生 Edit，**没有投递目标**。不可行。
- **B. UI Automation `ValuePattern.SetValue()`** —— 可用。
  聊天输入框是 `mmui::ChatInputField`（Edit，带 Value/TextPattern），
  精确、不模拟按键、不动鼠标、不碰剪贴板、不依赖像素坐标。

**但微信仍会自己抢前台**：Chrome 在前台时写入，`SetValue` 之后前台变成了微信
——它给输入框赋值的瞬间调了 `SetFocus`。

结论：**「写进微信自己的输入框」本身就会抢前台**，换哪条 API 都一样。
这不是投递方式的问题，是被投递的那个控件的语义。

「虚拟 UI 层」的意义因此清楚了：**输出必须有地方落**。候选只能出现在
微信输入框里的话，每次生成回复必然是一次焦点抢夺。面板让识别完全静默、
产出落在旁边，人只在自己决定的时刻把文本带走。

证据脚本：`scripts/probe-input.ps1`、`scripts/uia-set.ps1`、
`scripts/dump-uia.ps1`。

---

## 7. 还没解决的（按价值排序）

### 7.1 启动到首屏 6~8 秒 —— **最容易拿到收益**

三次模型调用是串行的。两条路：

1. **首屏不等模型**：先推一帧「已认出会话 + 已读到消息」（纯本地，~1.1 s），
   候选区显示「正在起草…」，模型结果后到。体感从 8 s 降到 1.1 s。
2. **合并调用**：judge + draft 并成一次请求，或让 rank 与 draft 并行。

### 7.2 OCR 质量一般 —— **已核完许可，等决策**

Windows OCR 的实际错误率不低（实测把「涉及」读成「步及」、「伽02」
读成「伽02」、文件名里的下划线读成破折号）。备选方案**许可证已逐个核实**：

| 组件 | 许可证 | 来源 |
|---|---|---|
| RapidOCR（PP-OCRv4 的 ONNX 导出） | **Apache-2.0** | 仓库 LICENSE 原文核实 |
| `tract-onnx` v0.23.8 | MIT OR Apache-2.0 | `cargo info` |
| `ort` v2.0.0-rc.13 | MIT OR Apache-2.0 | `cargo info` |
| `ocrs` / `rten` / `ndarray` | MIT OR Apache-2.0 | `cargo info` |

模型权重约 16MB（det + rec mobile）。`tract-onnx` 是纯 Rust，
不需要 C++ 运行时。

### 7.3 票据里未勾的项

`.scratch/wechat-reply-copilot/issues/` 下还有 6 张票带未勾项：
浅色主题夹具、多 DPI 实测、多行气泡归并、调试指标（像素/OCR 次数/视觉 token）、
自己的口吻样本、DWM 圆角。

---

## 8. 怎么跑

```bash
# 八道门禁（fmt / clippy -D warnings / test / secret-scan / cargo deny
#           / svelte-check / lint / svelte-autofixer / build）
pwsh -File scripts/gate.ps1

# 现场模式
cd ui; pnpm tauri dev

# 夹具模式（不需要微信）
pwsh -File scripts/run-app-fixture.ps1 <某张 frame png>

# 感知层探针（都对 PNG 重放）
cargo xtask perceive --file <png>
cargo xtask ocr <png> --scale 2      # 会打印逐词间隙
cargo xtask measure --file <png>
cargo xtask layout  --file <png>
cargo xtask e2e                      # 八段全链
```

---

## 9. 迁移到云端的边界（2026-10-09 核实）

**迁不了的部分**：`chatservant-capture` 整个建在 `windows-rs` 上——
WGC 抓帧、WinRT OCR、UIA、`SendInput`，每一样都是 Windows 专有 API。
Linux 沙箱里 `cargo build --workspace` 会在 capture 这一层直接挂。

**能迁的部分**：UI。Svelte 5 + CSS 是纯静态的，`pnpm build` 出来能部署成
一个 URL，界面设计可以在浏览器里全速迭代。

所以「云端继续」= UI 那半搬到云端，Rust 那半留在本机、但改成吃冻结帧，
不再需要人肉点微信。
