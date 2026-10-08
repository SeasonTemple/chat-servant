# ADR-0005：投递路径——为什么不能做到「完全不碰前台」

**状态**：accepted（2026-10-09 实机结论）
**相关**：ADR-0004（采集策略）、`crates/capture/src/inject.rs`、`scripts/uia-set.ps1`

## 背景

产品的硬约束第 2 条写着「不 hook、不注入、不读对方数据库、不解密、
不碰对方进程内存」。但现有实现里有一处与这条约束的精神相冲突：
投递走 `SetForegroundWindow` + `AttachThreadInput` + ALT 松锁 + `SendInput`
（`inject.rs` 的 `prime_foreground_lock` / `chord_events`）。

它能工作，但代价是**每次填入都会把用户的鼠标键盘抢走**。用户在使用别的
软件时，面板里的任何一次投递都会把微信拽到前台。这是「副驾」最难接受的
行为——副驾的意义就在于你不用看着它。

## 实测（2026-10-09，微信 4.x，窗口类 `Qt51514QWindowIcon`）

### 路径 A：往子 HWND 发 `WM_PASTE` —— 不可行

```text
微信顶层窗口下共有 2 个子 HWND
  0x30FA0  class=Qt51514QWindowIcon  1x1  text='Weixin'
```

**没有任何原生 Edit / RichEdit / WebView 子窗口**。Qt 应用只创建一个顶层
HWND，所有控件由 Qt 自己绘制，于是「投递目标」根本不存在。这条路在
WxWidgets / 原生控件的应用上可用，在微信上不可用。

### 路径 B：UI Automation `ValuePattern.SetValue()` —— 可用，但微信仍会自己抢前台

微信 4 暴露了完整的 UIA 树，类名带 `mmui::` 前缀，**聊天输入框是一个
带 `ValuePattern` 和 `TextPattern` 的 `Edit`**：

```text
[71]  Edit  class=mmui::ChatInputField   rect=(1214,763,819x209)  [Value,Text,Invoke]
[100] Button '发送' class=mmui::XOutlineButton
[104] List  '消息' class=mmui::RecyclerListView                 [Value,Invoke,Selection]
[129] List  '会话' class=mmui::XTableView                       [Value,Invoke,Selection]
```

实机写入（Chrome 在前台）：

```text
写入前，前台窗口 = 0x10ECA  '...bilibili... - Google Chrome'
输入框当前内容 = ''
写入后，前台窗口 = 0x30F6C  '微信'          ← 微信自己激活了
输入框现在内容 = '后台写入验证'
截输入框像素 → 「后台写入验证」确实在里面
```

**结论：`SetValue` 精确、不模拟按键、不动鼠标、不碰剪贴板、不依赖像素坐标，
但 WeChat 会在输入框获得值时调用 `SetFocus`，顺带激活自己的窗口。**

也就是说：**「写进微信自己的输入框」这件事本身就会抢前台**，
换哪条 API 都一样——这不是投递方式的问题，是被投递的那个控件的语义。

## 决策

分两层，各取能做到的最好：

### 读：已经是零干扰，且不可退化

采集走 WGC（Windows Graphics Capture）+ 本地离线 OCR，**从不触碰前台，
从不发送任何输入**。这一层不需要改。

### 写：默认**不**写进微信

1. **默认档（Manual）是「复制」而不是「填入」。** 面板上每条候选配一个
   复制按钮，文本进剪贴板，用户在**自己正在用的窗口**里 Ctrl+V。
   这是唯一真正零干扰的路径，也应该是默认。
2. **「填入微信输入框」降级为显式的、需要人点的动作**，并且：
   - 用 UIA `SetValue` 而不是 `SendInput`——不模拟按键、不动鼠标、
     不依赖像素坐标、不受用户中途切窗口影响（不会出现「打到了别的窗口里」）；
   - 动作发生时面板自身高亮一瞬，让用户知道**是谁**把焦点拿走了；
   - 仍然不发送。发送只在 Confirm/Auto 档，且升档必须人按。
3. **自动发送（Auto 档）与「不干扰」互斥**：Auto 档一旦成立，抢焦点就是
   必然的。界面上必须写明这一点，而不是让用户事后才发现。

### 为什么「虚拟 UI 层」仍然是这个产品的核心，而不是装饰

因为**输出必须有地方落**。如果候选只能出现在微信自己的输入框里，
那每一次「生成回复」都必然是一次焦点抢夺，产品没法在旁边安静地工作。
面板存在的意义是：识别完全静默，产出落在旁边，人只在**自己决定**的时刻
把文本带走。抢焦点那条路是可选的旁路，不是必经之路。

## 后果

- `inject.rs` 里基于 `SendInput` 的投递保留为**兜底**：UIA 元素找不到时
  （微信升级、换了版本、无障碍层被关）仍能工作，代价是抢焦点。
  两条路都要有，并在 `reason` 里如实说明走了哪条。
- `scripts/uia-set.ps1` 与 `scripts/probe-input.ps1` 留在仓库里：它们是
  得出本 ADR 结论的证据，下次微信升级后应当先重跑一遍，确认
  `mmui::ChatInputField` 还在、ValuePattern 还可用。

## 待验证

- `ValuePattern.SetValue` 是否会**替换**输入框已有内容（而不是追加）。
  本次是在空输入框上测的。正式实现前必须在有内容的输入框上再测一次，
  并按结果决定是直接 `SetValue` 还是先读 `Current.Value` 再拼接。
- 微信是否会记住/上报「有程序在改我的输入框」。这属于微信的合规问题，
  不影响技术结论，但影响这个默认档该不该开。
