# 12: Tauri + Svelte 骨架——两个窗口跑起来

**What to build:** 一个能跑起来的 Tauri 2 + Svelte 5 应用，双窗口结构：
透明置顶的 `main`（悬浮窗）+ 隐藏的 `settings`（按需显示）。

Rust 侧暴露一个真实的 `invoke` 命令，前端能调通并拿到结果。
这条 ticket 专门用来**提前暴露窗口层的坑**，因为这些坑不在任何文档里、
只在你真的建窗口时才会撞上。

可以和其他 ticket 并行开工。

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] 应用启动后 `main` 窗口是透明、无边框、置顶、不占任务栏的，
      且**不抢焦点**
- [ ] `settings` 窗口启动时隐藏，能从 `main` 打开再关闭
- [ ] 前端调一个真实 `invoke` 命令并渲染返回结果，Rust→前端的链路通
- [ ] Rust 主动 `emit` 的事件前端能收到（不轮询）
- [ ] 懒加载的窗口写在配置里是 `visible: true` + 停在屏幕外，
      而不是 `visible: false`（否则 WebView2 不会注册 drop target）
- [ ] 窗口配置里 `dragDropEnabled` 写在配置文件里而不是只写在 Rust builder 里
- [ ] `decorations: false` 时圆角通过 DWM API 生效（没有生硬的白边）
- [ ] 窗口在 150% DPI 下位置和尺寸正确
- [ ] 前端 `.svelte` 文件通过 `svelte-autofixer` 零告警