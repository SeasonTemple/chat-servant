# 12: Tauri + Svelte 骨架——两个窗口跑起来

**What to build:** 一个能跑起来的 Tauri 2 + Svelte 5 应用，双窗口结构：
透明置顶的 `main`（悬浮窗）+ 隐藏的 `settings`（按需显示）。

Rust 侧暴露一个真实的 `invoke` 命令，前端能调通并拿到结果。
这条 ticket 专门用来**提前暴露窗口层的坑**，因为这些坑不在任何文档里、
只在你真的建窗口时才会撞上。

可以和其他 ticket 并行开工。

**Blocked by:** None (can start immediately)

**Status:** done（`emit_readout` 已在 ticket 16 接上真实数据；面板现在还会显示
会话身份徽标与分段调用计数）

- [x] 应用启动后 `main` 窗口是透明、无边框、置顶、不占任务栏的，
      且**不抢焦点**（`focus: false`）
- [x] `settings` 窗口启动时隐藏，能从 `main` 打开再关闭
- [x] 前端调一个真实 `invoke` 命令并渲染返回结果，Rust→前端的链路通
- [x] Rust 主动 `emit` 的事件前端能收到（不轮询）——`listen` 已接，
      ticket 16 接上真实 readout 后已在面板上渲染
- [x] 懒加载的窗口写在配置里是 `visible: true` + 停在屏幕外，
      而不是 `visible: false`（否则 WebView2 不会注册 drop target）
- [x] 窗口配置里 `dragDropEnabled` 写在配置文件里而不是只写在 Rust builder 里
- [ ] `decorations: false` 时圆角通过 DWM API 生效（没有生硬的白边）
      —— 圆角目前是 CSS 的 `border-radius`，配合 `shadow: false` 视觉上干净；
      要确认原生圆角需要真机看，DWM 那条留待实机验证
- [ ] 窗口在 150% DPI 下位置和尺寸正确 —— **未实测**，需要实机切 DPI 验证
- [x] 前端 `.svelte` 文件通过 `svelte-autofixer` 零告警（五个文件全过）

## 产出

`ui/` 整个前端 + `ui/src-tauri/` Rust 侧。tauri build 出 exe 13.2 MB、
msi 4.05 MiB、nsis 2.43 MiB。

## 工具链坑（都已写进代码注释）

1. **pnpm 11 迁移了构建脚本许可**：`package.json#pnpm.onlyBuiltDependencies`
   不再被读，新位置是 `pnpm-workspace.yaml` 的 `allowBuilds`。
   两个白名单**不能并存**，共存时新的失效。
2. **eslint-plugin-svelte v2 与 eslint 9 不兼容**：四次报错才看清根因
   （规则包装层 `context.options[1]` undefined）。升 v3 一次解决。
3. **tauri crate 必须与 @tauri-apps/api 同次版本**，且要用 `=2.12.1` 钉死，
   否则解析到别的分支。
4. **tauri.conf.json 必须是纯 JSON**：写成 `.ts` 带注释时报
   `expected value at line 1 column 1`，完全指不到真正原因。
5. **deny.toml 的 `exceptions` 必须写在 `[licenses]` 表内**，
   独立子表 `[licenses.exceptions]` 会报 `expected an array`。

## deny.toml 的两条豁免（都有复审条件）

- **license: MPL-2.0 × 6**（cssparser / selectors / dtoa-short /
  option-ext + 两个宏）。这四个是 **wry 的运行时**依赖，会进二进制。
  选 Tauri 2 就是这个代价，换框架必须重跑 cargo deny。
- **advisory: quick-xml 两条 DoS**。只能经 `plist` 进来，而 plist 1.8.0
  把它钉在 `^0.38`（修复版 0.41 跨大版本，cargo update 到不了）。
  plist 只用于 **macOS 的 Info.plist**，本项目纯 Windows，这条路不执行。

## 门禁扩到七道

`gate.ps1` 加 ui svelte-check / ui lint / ui build，
`node_modules` 不在时跳过而不算红。