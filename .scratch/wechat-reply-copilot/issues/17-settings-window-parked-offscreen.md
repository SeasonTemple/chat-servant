# 17: 设置窗口停在离屏，面板进不去设置

**Status:** 已定位 —— 触发对象已在 `6b2acbf` 随「设置搬进面板」删除

## 症状（2026-10-11 实机枚举该进程**全部**顶层窗口，含不可见的）

```
0xB11EC  vis=True   Tauri Window  '副驾'      380x560  @ (312,312)
0x70C70  vis=False  Tauri Window  '识别画布'  776x759  @ (104,104)
0x30E3E  vis=False  Tauri Window  '设置'      656x559  @ (-32000,-32000)
```

`tauri.conf.json` 里 `settings` 写的是 `"visible": true`。
`(-32000,-32000)` 是 Windows **最小化**窗口的标准位置。
同一份配置里 `识别画布`（`visible:false`）停在该停的地方，
主面板正常可见——**所以不是「没建」，是「建了之后被藏起来了」**。

## 根因（2026-10-11 从源码读出，确定）

**是我们自己的 `park_settings()` 主动藏的，不是最小化，也不是「没建」。**

合并前的 `ui/src-tauri/src/commands.rs` 里有一段：

```rust
/// 把 `settings` 窗口挪到屏幕外。
///
/// **为什么要挪而不是直接 `visible: false`**：Tauri 2 的窗口配置里
/// `visible: false` 会让 WebView2 跳过 drop target 与合成器的注册，…
pub fn park_settings(app: &AppHandle) {
    let Some(w) = app.get_webview_window("settings") else { return; };
    let _ = w.set_position(PhysicalPosition::new(-32000.0, -32000.0));
    let _ = w.hide();
}
```

而 `main.rs` 的 `setup` 里**无条件**调它（`174c0ed` 版）：

```rust
commands::set_app_handle(app.handle().clone());
build_tray(app.handle());
commands::park_settings(app.handle());   // ← 就是这一行
```

理由写在 ADR-0001 里（`visible: false` 会让 WebView2 少注册东西），
是**有意为之**，不是回归。

⚠️ **更正本票早先的猜测**：当时把 `(-32000,-32000)` 记成
「Windows 最小化的标准位置，所以可能被最小化了」并列为待查方向 1。
那是对**同一个现象**的两种解释，实际是第二种：代码里那个 `-32000` 是
字面量写死的 `set_position`，`hide()` 只清可见位不动位置。
两种根因的处理方式不同，这里给出确定答案。

## 触发对象已经不存在了

`6b2acbf` 并入另一台设备 2026-10-10 的进度时，**设置搬进了面板**
（`PanelSettings.svelte`），于是：

- `tauri.conf.json` 里不再有 `settings` 窗口
- `park_settings` 命令删掉了
- `open_settings` 改成两步：`show_panel()` + `emit("menu:settings")`

前端监听器**确认存在**（`ui/src/routes/+page.svelte:279`）：

```ts
un = await listen('menu:settings', () => toggleSettings(true));
```

所以托盘菜单「打开设置」→ 面板切到设置那一屏，这条路是通的。

## 验收项（按实际完成情况勾，没核实的没勾）

- [x] `scripts/winscan.ps1` 不再把人带偏——脚本头部已写明「只报可见窗口」，
      并指向 `target/probe/pidwins.ps1 <pid>`（2026-10-11 改）
- [x] 复现并分清「稳定停在离屏」与「被最小化」：是 `park_settings` 的
      `set_position(-32000,-32000)` + `hide()`，见上
- [x] 三条方向逐条排除：
      1. `show_window` 缺 `unminimize` —— **不是根因**（从来没发生过最小化）
      2. 「Tauri 2 某条路径最小化了它」—— 不是，就是本 crate 自己写的
      3. `focus: false` + 无边框的初始化时序 —— 不是，无边框的是主面板，
         它正常
- [ ] ~~`open_settings` 之后窗口真的可见（截图证据）~~ ——
      **未验，且路径已变**。合并前那条路径（`show_window` 挪到 120,120 后 show）
      从未被点开验证过：2026-10-11 那次实机里前台是游戏，鼠标被抓，
      菜单项一个都没点着。现在这条命令已经换成「叫面板 + emit」，
      要验的是**面板那一屏设置**能不能显示，同样需要空桌面环境。
      见 `docs/本机验收清单.md` 的待验清单。

## 一条留给后人的提醒

这个票是从「`winscan.ps1` 只报可见窗口」这条诊断陷阱开的。那个陷阱本身
还在（脚本已加提示，但依然只报可见窗口），**票 18 的排查也会撞上它**。