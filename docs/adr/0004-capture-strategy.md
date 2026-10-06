# ADR-0004：捕获路径分级与本机实测结论

- 状态：已接受
- 日期：2026-10-06
- 相关：[ADR-0002](0002-frame-conditioning-pipeline.md)（调理管线依赖捕获输出）

## 背景

参考项目 `examples/jev-chat-windows` 记录了一条三级捕获分级。本 ticket
的任务就是**在这台机器上逐级实测**，看哪一级真的能用。

结论先行：**第 1、2 级不可用，第 3 级能用但有硬限制。**

## 三级分级与实测结果

| 级别 | 手段 | 能否抓被遮挡 | 本机结果 |
| --- | --- | --- | --- |
| ① | Windows Graphics Capture | ✅ 能 | ❌ **不工作** |
| ② | `PrintWindow` + `PW_RENDERFULL_CONTENT` | ⚠️ 部分 | ❌ 微信拒绝（返回 0） |
| ③ | 屏幕 DC + `BitBlt` 抠矩形 | ❌ 不能 | ✅ **可用，94 ms** |

## ① WGC：本机不工作（原因未查明）

**已验证的事实**：

- 系统层面**支持**：`GraphicsCaptureSession::IsSupported()` 返回 `true`，
  OS = Windows 10.0.22631。`cargo xtask caps` 一次问出来。
- 会话启动后**立刻关闭且不报错**，一帧都不送。
- 对**三个完全不同的目标表现一致**：微信窗口、记事本窗口、**主显示器**。
- 会话线程返回 `Ok(())`，不是错误路径。

**由此可以排除的**：

- 不是微信的问题（记事本一样失败）
- 不是 `CreateForWindow` 的问题（主显示器也失败）
- 不是系统不支持（`IsSupported()` 为 true）
- 不是"目标不可见"（主显示器总是可见）
- 不是光标/边框设置的兼容性问题（改成 `Default` 后现象不变）

**代码层面的线索**：`windows-capture` 2.0.1 的 `run_message_loop` 是

```rust
match unsafe { GetMessageW(&mut message, None, 0, 0).0 } {
    -1 => return Err(FailedToRunMessageLoop),
    0  => return Ok(()),          // ← 我们观察到的就是这条
    _  => { TranslateMessage; DispatchMessageW }
}
```

我们看到的是 `Ok(())`，即 `GetMessageW` 返回 0（WM_QUIT）而消息循环从未
真正阻塞。**谁往那个线程 post 了 WM_QUIT，目前没有定位到。**

未排除的嫌疑（按可能性排序）：

1. 机器上装了 **MuMu 虚拟显示器驱动** + Intel 核显 + NVIDIA 独显三块适配器，
   WGC 内部挑合成器的方式可能出问题
2. `CreateDispatcherQueueController` 在本进程环境下的行为
3. 该 crate 在这个环境下的固有 bug（可换 `wgc` crate 2.0.1 交叉验证）

**决策**：不阻塞主线。第 ③ 级已能出帧，先用它推进；① 留作后续专项。

## ② PrintWindow：被微信拒绝，属于预期内

`PrintWindow` 返回 0。这正是参考项目 `KICKOFF.md` 预测的症状——微信把界面
自绘在 GPU 合成画布上（`MMUIRenderSubWindowHW`），这类窗口会拒绝 `WM_PRINT`，
连 `PW_RENDERFULL_CONTENT` 也不吃。

**这个结论本身是有价值的**：它独立证实了参考项目"为什么走 OCR 而不是
UIA/控件树"的判断在微信上依然成立。

## ③ 区域抓屏：能用，但必须配套遮挡检测

`GetDC(NULL)` + `BitBlt(..., SRCCOPY)` 按窗口矩形抠一块，实测 **94 ms**，
尺寸格式正确、非黑。**机制上完全可用。**

### 但它有一条会致命的前提

**读到的是屏幕坐标处的像素。窗口被别的应用盖住时，抓到的是盖在上面的东西。**

实测时微信窗口 hwnd `0x10AFC` 明明存在，但那个位置上是 MCode 自己的窗口，
抓回来的是 MCode 界面。**一个非黑的、尺寸正确的"成功"帧，内容完全是错的。**

这与产品承诺的"被其他窗口盖住也能读"直接冲突。因此第 ③ 级**不能单独使用**，
必须配套：

- **`region::is_occluded(hwnd, sample_points)`** —— 用 `WindowFromPoint`
  在窗口内取几个采样点，比对命中的顶层窗口。任一点被别的窗口占据即判定被遮挡。
- **内容校验** —— 抓到非黑帧**不等于**抓到的是目标窗口。`PaneAnchor`
  推导出的消息区位置必须与预期布局吻合，对不上就是抓错了。
- **UI 上如实提示** —— 被遮挡时要告诉用户"把聊天窗口露出来"，
  而不是默默用错误内容继续。

**「没报错」在第 ③ 级不构成有效帧的证据。** 这和 ADR 里
`PossiblySent` 禁止重放是同一条原则：不能拿"动作返回成功"当结果证明。

## 影响

- 捕获层对外暴露**三档 + 一个能力探测**，而不是单一 `capture()`：
  `capture_one_frame`（WGC）/ `printwindow::capture` / `region::capture`，
  外加 `xtask caps` 一次性问出系统支持情况。
- **生产路径暂定第 ③ 级**，但必须带 `is_occluded` + 内容校验双重守卫。
  这意味着 v1 的"被遮挡也能读"承诺要打折，UI 上要明确说明。
- 第 ① 级的 WGC 保留代码但不作为默认。后续单独开一条去查 WM_QUIT 的来源，
  以及换 `wgc` crate 交叉验证。
- ADR-0002 的五级调理管线**不受影响**——它作用在 `Frame` 上，
  帧从哪来与调理策略正交。但第 1 级（变化门控）依赖可靠的变化检测，
  第 ③ 级在窗口被遮挡时"内容不变"反而是好事（盖住的窗口区域像素不变），
  这一点在实测中正好被验证了。

## 遮挡检测已实测有效

`cargo xtask occluded` 在本机跑出的真实数据：

```
窗口 = 微信 (weixin.exe)  0x10AFC
屏幕矩形 = (700, 251) - (1976, 1216)   1276x965

  左上  ( 724,  275)  命中 0x8F0C70  ← 被别的窗口盖住
  右上  (1952,  275)  命中 0x10AC6  ← 被别的窗口盖住
  中心  (1338,  733)  命中 0x10AFC   目标窗口
  ...（9 点中 6 点被遮挡，只有中间一列命中目标）

❌ 有采样点被遮挡    is_occluded() 判定 = true
```

判定与实际一致。**同时也解释了那次"抓到了但内容全错"**：
左侧被 `0x8F0C70`、右侧被 `0x10AC6` 盖住，只有中间一列是微信。

## 遗留

- **`wgc` crate 2.0.1 交叉验证 WGC** —— 值得做。它用的是
  **`os::windows-apis` 而不是 `windows`**，是一套完全独立的绑定和实现，
  所以 `windows-capture` 的问题它可能不受影响。API 已确认：
  `wgc::window::new_item_from_hwnd(hwnd) -> GraphicsCaptureItem`，
  `Wgc::new(item, WgcSettings)` 之后 `Wgc` 本身就是 `Iterator`，
  `Frame::size()` / `Frame::pixels()` 取数据。
  接入要跨两套 `HWND` 类型做桥接（两者都包 `*mut c_void`），
  本轮上下文不够做完，依赖已回退，树里不留半截实验。
  **如果它能工作，v1 就能保住"被遮挡也能读"，不用在 UI 上给用户降级提示。**
- 给 `region::is_occluded` 补单元测试（现在只有实机数据，没有回归测试）。
- `PaneAnchor`（ticket 02）落地后，它就是内容校验那一环——
  推导出的消息区位置与预期布局对不上，就说明抓到的是别的窗口。
