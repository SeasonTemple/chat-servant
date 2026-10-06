// `chat-servant` 前端是**纯渲染层**。
//
// 三条规矩，任何一条都不能破：
//
// 1. **不轮询。** 所有状态变化都靠 Tauri 的 `invoke` / `listen` 推过来。
//    前端不缓存会话状态——Rust 侧是唯一事实来源。
// 2. **不碰网络。** 所有网络调用都在 Rust 侧。前端拿不到 API key，
//    也不该知道模型路由。
// 3. **不重算几何。** `PaneAnchor` 那些像素判定全在 Rust 侧，
//    前端只拿结果渲染。
//
// 静态导出：Tauri 打的是 `file://`/`tauri://` 本地协议，不是 HTTP。
// 所以每个路由都要有独立的 html，两个窗口才能各自打开自己的页面。
// `ssr: false` — 这个应用永远跑在 WebView 里，不需要服务端渲染。

export const prerender = true;
export const ssr = false;
