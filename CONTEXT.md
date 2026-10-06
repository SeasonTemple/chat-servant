# CONTEXT

桌面聊天回复助手的领域语言。写代码前先读这里；用词和这里不一致就是缺陷。

`chat-servant` 挂在用户自己的聊天窗口旁，观察对话、判断意图、生成候选回复，
并按用户选定的档位把文本送回输入框。它**不理解**对话，只**观察**和**转述**。

---

## Glossary

### 采集层（capture）

**Frame**
一次窗口捕获得到的完整位图。RGB8，常驻内存，**永不落盘**。一个 `Frame` 对应
目标窗口某一时刻的完整画面，包含标题栏、侧边栏、聊天区、输入框。

**ChatPane**
`Frame` 中属于消息列表的那块矩形。聊天区是唯一需要被"读"的区域；其余部分
（联系人列表、工具栏、标题栏）永远不进入 OCR。

**PaneAnchor**
从一帧像素里推导出的 `ChatPane` 几何与面板底色。必须由像素内容锚定，
**不写死坐标**——同一份代码要同时吃深色主题、浅色主题、125%/150% DPI、
窗口任意尺寸。推导失败返回 `None`，上层降级重试，不猜。

**ChatSurface**
目标聊天软件的一份适配描述：进程名、主窗口标题规则、自己的气泡底色判定、
OCR 拼接规则、顶部要跳过的区域。接一个新软件 = 新增一份 `ChatSurface`，
不改任何引擎代码。

### 调理层（condition）

**FrameConditioner**
把 `Frame` 变成"值得送去识别"的小图的管线。它是成本控制的主要杠杆，
因为**视觉 token 成本与图像面积成正比**——宽度给两倍，token 花四倍。
详见 [ADR-0002](adr/0002-frame-conditioning-pipeline.md)。

**NoiseClass**
一个区域被丢弃的原因。`Avatar` / `Sticker` / `ImageText` / `SenderLabel` /
`Timestamp` / `QuotedBlock` / `LinkCard` / `PinnedNotice` / `SystemNotice` /
`TinyGlyph` / `Chrome`。调试视图按这个分类上色——**认错了要一眼看出是哪一步的锅**。

**Bubbled**
调理层的输出：一组紧贴文字的 `Bubble` 矩形 + 一张只保留有效文字像素的背景图。
不含头像、表情包、时间戳、引用块、链接卡片、发言人名、系统提示。

### 感知层（perceive）

**OcrLine**
OCR 返回的一个文本框：四角坐标、文本、置信度。坐标以 `Bubble` 为原点。

**Speaker**
`Me` | `Them` | `System`。
用 `Me`/`Them` 而不是 `me`/`her`——关系是运行时配置，不是词法事实。

**Attribution**
对一个 `OcrLine` 的判定结果：`Speaker` + `NoiseClass`。噪声类统一归到 `System`。
**误判必须可见**：每条 `Attribution` 都要能追到它的判定依据（底色、对比度、
字号比、坐标），供调试视图和回归测试消费。

**Message**
一次归并后的对话轮次：`{speaker, sender_name?, text, y}`。同一个 `Bubble` 内的
多行 `OcrLine` 已合并。`sender_name` 只在群聊有值。

**Novelty**
一条 `Message` 相对本会话已见集合是不是新的。滚动翻出的旧消息不算新；
同一人连发两句完全一样算一条（对触发无害）。

### 会话层（session）

**Session**
一段被持续观察的对话。由 `(ChatSurface, 窗口句柄, 对端名)` 标识。
切会话不清状态：历史、上下文、候选、`Novelty` 集合都按 `Session` 分开存。

**PeerName**
会话头部 OCR 出来的对端名，作为 `Session` 的稳定 key。OCR 会抖
（「小分队」↔「小分认」），所以按相似度归并，抖一下不能分裂出新 `Session`。

### 推理层（reason）

**Intent**
决策模型的判断结果：`{intent, need, action, tension}`。
它是一段**小抄**，不是最终回复；起草模型读它来写候选。

**Candidate**
一条起草的候选回复 + 胜出概率。`DraftSet` 里有 1..N 条。

**VisionBridge**
**回复生成模型 B 的一个配置项**，不是独立的第三条链路，默认关闭。

仅当 B 是**无视觉能力的纯文本 LLM** 且用户显式开启时存在：
把对方发的**图片消息那一小段 `Bubble`** 转成描述，再让 B 据此起草。

边界：
- 它的输入**永远只是图片消息区域**，绝不是整个窗口。
- 它的输出**永远不用于读气泡文字**——气泡文字只走 OCR。
- B 具备视觉能力时，VisionBridge 不存在，图片随文本一起进 B。

### 行动层（act）

**Dispatch**
发送档位，三档且**默认最低**：

| 档 | 行为 | 进入方式 |
| --- | --- | --- |
| `Manual` | 只填入输入框，停在光标处 | 默认；会话结束自动回落到此档 |
| `Confirm` | 填入后二次确认才发送 | 用户在候选卡片上显式选择 |
| `Auto` | 填入并发送，不打断 | 需在设置里开启，会话内每条都可见可打断 |

**DeliveryReceipt**
一次输入注入的结果。**不是布尔值**：

| 值 | 含义 |
| --- | --- |
| `NotSent` | 确定没发（剪贴板都没写成） |
| `Sent` | 送达且已回读验证 |
| `PossiblySent` | 可能已经发了——**禁止自动重放** |
| `Failed(reason)` | 明确失败，可按退避重试 |
| `TimedOut` | 超时，结果未知，按 `PossiblySent` 对待 |

`PossiblySent` 存在的唯一理由：重放一条已经发出的消息无法撤销。

---

## 系统边界

一条数据流，六个阶段，每个阶段只依赖它上游的输出结构，不依赖上游的实现：

```
Frame ──▶ PaneAnchor ──▶ Bubbled ──▶ Attribution ──▶ Message ──▶ Session
   capture    condition       perceive              context
                                                        │
                                                        ▼
                              Candidate ◀── Intent ◀──  reason
                                                        │
                                                        ▼
                                          Dispatch ◀── present ──▶ 填入 / 发送
```

- `capture` 不认识聊天软件，只认识 HWND。
- `condition` 不发网络请求，不认识消息。
- `perceive` 不发网络请求。
- `reason` 是**唯一**允许出网做模型调用的阶段，且只被 `Novelty` 为真的新消息触发。
- `present` 和 `act` 不出网。

## 不做什么

这些不是"以后再说"，是这个产品的定义边界：

- 不 hook、不注入、不读数据库、不解密、不碰对方进程内存。
- 不做通用 RPA。不是"让 Agent 操作电脑"，是"在聊天窗口旁边当个副驾"。
- 不自动处理转账、红包、收款。
- 不替用户决定发不发——程序可以建议，可以按确认发，但升到 `Auto` 必须是人按的。
- 截图不落盘。
