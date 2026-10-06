# ADR-0004：捕获路径定案——用 `wgc`，并附本机实测证据

- 状态：已接受
- 日期：2026-10-06
- 相关：[ADR-0002](0002-frame-conditioning-pipeline.md)（调理管线依赖捕获输出）、
  [ADR-0003](0003-dependency-strategy.md)（依赖策略）

## 结论

**采用 `wgc` 2.0.1 作为唯一的窗口捕获路径。**

微信窗口在**被完全遮挡**时仍能抓到内容正确的帧，帧内 OCR 能读出与
窗口实际一致的中文文字。产品承诺的「被其他窗口盖住也能读」得以保留，
不需要向用户降级。

`windows-capture` 2.0.1 与第 ③ 级区域抓屏保留为诊断手段，不作产品默认。

## 分级与实测结果

| 级别 | 手段 | 能否抓被遮挡 | 本机结果 |
| --- | --- | --- | --- |
| ① | `wgc` 2.0.1（Windows Graphics Capture） | ✅ | ✅ **可用，322 ms** |
| ①′ | `windows-capture` 2.0.1（同一个 WGC） | ✅ | ❌ 会话启动即关闭且无错 |
| ② | `PrintWindow` + `PW_RENDERFULL_CONTENT` | ⚠️ | ❌ 微信拒绝（返回 0） |
| ③ | 屏幕 DC + `BitBlt` 抠矩形 | ❌ | ✅ 94 ms，但内容可能错 |

## ①′ 的问题不在环境，在那个实现

`windows-capture` 2.0.1 在本机对**微信窗口、记事本窗口、主显示器**
三者表现完全一致：会话启动后立刻关闭、不报错、一帧不来，而
`GraphicsCaptureSession::IsSupported()` 明确返回 `true`
（`cargo xtask caps` 可复现，OS = 10.0.22631）。

当时排除掉的：目标特殊、`CreateForWindow`、系统不支持、目标不可见、
光标/边框设置兼容性。代码落点在它的 `run_message_loop`——
`GetMessageW` 一上来就返回 0，消息循环从未阻塞。WM_QUIT 来源至今未定位，
但**这已经不影响决策**。

**换 `wgc` 2.0.1（github.com/Atliac/wgc，另一个独立实现）后立刻工作。**
两者用同一个 `windows` crate 的 `HWND`，但事件循环、D3D 设备创建、
会话管理各自独立实现、无代码关联。所以这是**那个 crate 的问题**。

> 方法论：区分「环境问题」和「某个实现的问题」，唯一可靠的手段是
> **换一个独立实现复测**。这一条已并入 ADR-0003。

## 证据

**1. 部分遮挡（6/9 采样点被盖）**：抓到微信窗口，322 ms，1262×958。

**2. 完全遮挡（8/9 采样点被最大化的记事本盖住）**：仍抓到微信窗口。

两种遮挡状态下抓到的帧 **SHA256 完全相同**：

```
frame-…2518.png  195474 bytes  1733FEC97F11A5A2E229AA832857A082…
frame-…2549.png  195474 bytes  1733FEC97F11A5A2E229AA832857A082…
```

遮挡状态变了，捕获内容字节级不变——**抓的是窗口，不是屏幕**。
对照：第 ③ 级区域抓屏那张哈希 `6ED52939…` 不同，因为它抓到的是
被盖在上面的另一个窗口。

**3. 帧内 OCR**（`cargo xtask read`，Windows.Media.Ocr，224 ms，26 行）：

```
文 件 传 输 助 手          ← 会话名
G8W / HXH / 微 信 支 付     ← 联系人名
什 邡 项 目 组   @广发裉行   昨 天
04 ： 25                   ← 时间戳
```

与抓到的画面逐项对得上。个别字有误（嘢/秘、裉/银），属识别率问题，
留给 ticket 03 的阈值调优；字间空格是引擎在深色小字号上的分词行为。

## 尺寸差异：PaneAnchor 必须处理

WGC 抓到的是 **1262×958**，`GetWindowRect` 给的是 **1276×965**。
差 14 px / 7 px——阴影与不可见缩放边框。

**`PaneAnchor`（ticket 02）的坐标换算要把这个偏移算进去**，否则框整体偏移。

## 仍然保留的两级，以及为什么

**② `PrintWindow`**：微信返回 0 拒绝重绘。这独立证实了参考项目
`KICKOFF.md` 的判断——微信把界面自绘在 GPU 合成画布上，这类窗口拒绝
`WM_PRINT`，连 `PW_RENDERFULL_CONTENT` 都不吃。
保留它的理由是**诊断价值**：用来区分「窗口拒绝重绘」和「会话起不来」。

**③ 区域抓屏**：`region::is_occluded` 的配套兜底。有了 ① 之后它不是必需的，
但保留两个理由：一是多一层保险，二是 `is_occluded` 本身要在 UI 上用来
告诉用户「这个窗口我读不到」——**这与抓不抓得到是两件事**。

> 第 ③ 级有个陷阱值得记住：抓到**非黑的、尺寸正确的**帧，
> **不等于**抓到的是目标窗口。它读的是屏幕坐标处的像素。
> 「动作返回成功」不构成结果证明——这与 ADR 里 `PossiblySent`
> 禁止重放是同一条原则。

## 实现约束

`wgc` 与 `windows-capture` 都要求 `windows 0.62.2`，版本锁死
（ADR-0003 已说明）。`windows-core 0.62.2` 的 `NonNull<c_void>` 不是 `Send`，
所以 **WGC 会话对象不能跨线程 move**：必须在目标线程内部创建会话、
跑迭代器，只把 `(w, h, Vec<u8>)` 这种纯数据传出来。
`wgc_alt.rs` 就是这个形状。

## 影响

- 捕获层对外以 `wgc_alt::capture_one_frame` 为默认实现。
- `windows-capture` 降级为「用于对照的诊断实现」，其失败不影响产品。
- OCR 走 `Windows.Media.Ocr`（系统自带、离线、零 token），
  像素路径为 `Frame(RGB8) → BGRA8 → DataWriter::DetachBuffer
  → SoftwareBitmap::CreateCopyFromBuffer → RecognizeAsync`，
  **不经过任何文件**。
- 下一步 ticket 02（`PaneAnchor`）在此基础上开工。
