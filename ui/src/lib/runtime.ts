/**
 * 运行环境判定。**单一事实来源**。
 *
 * ## 为什么单独一个文件
 *
 * 2026-10-09 实测：`+page.svelte` 的注释写着「演示模式不碰 Tauri 也不注册
 * 监听器」，但 `+layout.svelte` 在**所有模式**下都无条件 `await listen(...)`，
 * 外面没有 try/catch。纯浏览器里没有 `window.__TAURI_INTERNALS__`，
 * `@tauri-apps/api/event` 的 `listen` 读它的 `.transformCallback` 得到
 * `undefined`，于是每次打开演示页都产生一个 **unhandledrejection**：
 *
 * ```
 * TypeError: Cannot read properties of undefined (reading 'transformCallback')
 * ```
 *
 * 这就是「状态语义没有集中的定义」——「是不是演示模式」这件事
 * 只存在于 `+page.svelte` 的一个局部常量里，
 * 别的组件够不着，于是各写各的。**把它提出来，问题就不存在了。**
 */

/**
 * 现在是不是脱离 Tauri 的演示模式。
 *
 * 判据是 **URL 上有没有 `demo`**，不是「有没有 Tauri」。
 *
 * 早先试过 `typeof window.__TAURI_INTERNALS__ !== 'undefined'`——它在
 * 打包产物里会被静态消除，换个构建模式就失效。URL 参数是显式的，
 * 不会自己变。
 */
export const IS_DEMO: boolean =
  typeof location !== 'undefined' && new URLSearchParams(location.search).has('demo');

/**
 * Tauri 的 IPC 到底在不在。
 *
 * 跟 {@link IS_DEMO} **不是一回事**：真实 Tauri 窗口里是 `true`；
 * 正式部署的静态页面（不带 `?demo`）是 `false`——那种情况下
 * 既没有 IPC 也没有演示数据，界面必须**明说连不上**，而不是假装在等。
 */
export const HAS_TAURI: boolean =
  typeof window !== 'undefined' &&
  typeof (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !== 'undefined';
