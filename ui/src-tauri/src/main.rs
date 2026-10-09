// release 构建在 Windows 上不弹额外控制台窗口。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod debug;
mod secret;
mod watch;

use tauri::{Emitter, Manager, WindowEvent};

fn main() {
    init_tracing();
    tauri::Builder::default()
        .setup(|app| {
            // 托盘**先建**，因为「关闭时 = 收进托盘」这一档要验证托盘存在。
            // 顺序反了的话，设置页可能在托盘建好之前就把那一档存进去了，
            // 于是「收起」这个动作没有落点。
            // ⚠ **handle 必须最先记下来。** 托盘刷新和「档位变了要刷托盘」
            // 都从全局取它，晚一步的话第一次刷新就会静默失败。
            commands::set_app_handle(app.handle().clone());
            build_tray(app.handle());
            commands::park_settings(app.handle());
            // 观察循环**在这里起**。它自己的线程里跑
            // （WGC 会话对象不是 Send，不能跨线程 move）。
            commands::start(app.handle());
            // 建完循环再刷一次托盘 —— 循环一上来就知道档位和会话名了。
            commands::refresh_tray();
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // ⚠ **默认行为是错的**：Tauri 收到关闭请求会退出整个应用。
                // 而这个程序是 alwaysOnTop + skipTaskbar + 无边框——
                // 一旦它从别的途径（比如系统「关闭窗口」快捷键）被关掉，
                // 用户连同托盘图标一起弄丢了，那才叫找不回来。
                //
                // 所以：**只有托盘菜单里的「退出」能真正结束进程。**
                // 其余一律吞掉。
                let app = window.app_handle().clone();
                let label = window.label().to_string();
                match label.as_str() {
                    // 设置 / 识别画布：本来就有标题栏，关掉就关掉。
                    "settings" | "debug" => {}
                    // 主面板：关不掉。它只能「收进托盘」。
                    _ => {
                        let _ = commands::hide_panel(app);
                        api.prevent_close();
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::probe_anchor,
            commands::latest_readout,
            commands::debug_pane,
            commands::dispatch,
            commands::open_settings,
            commands::open_debug,
            commands::get_settings,
            commands::save_setting,
            commands::forget_key,
            commands::probe_key,
            commands::get_recent_n,
            commands::set_recent_n,
            commands::get_token_budget,
            commands::set_token_budget,
            commands::get_images_out,
            commands::set_images_out,
            commands::get_tier,
            commands::set_tier,
            commands::relock_session,
            // 窗口生命周期。见 commands.rs 里那一节的说明——
            // 早先这个程序**没有任何办法停掉**。
            commands::get_close_behavior,
            commands::set_close_behavior,
            commands::hide_panel,
            commands::show_panel,
            commands::quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("启动失败");
}

/// 建托盘图标。
///
/// **这是一个必须存在的出口。** 主窗口同时设了 `decorations: false`
/// （没有标题栏）、`skipTaskbar: true`（不在任务栏）、`alwaysOnTop: true`，
/// 三者叠在一起意味着系统层面没有任何可见的关闭入口。托盘是唯一的。
///
/// ⚠ 这里原本写成 `+ skipTaskbar: true + alwaysOnTop: true`——
/// 行首那个 `+` 被 Markdown 当成**列表项**，rustdoc 会把这段渲染成一串
/// 列表而不是一句话，clippy 也报 `doc list item without indentation`。
///
/// `tray-icon` 这个 feature 一直开着，只是**一次都没被用上**。
fn build_tray(app: &tauri::AppHandle) {
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    // 菜单在 `commands` 里构造 —— 它要读档位和会话名。
    let menu = match commands::build_tray_menu(app) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("托盘菜单建不起来：{e}");
            return;
        }
    };

    let built = TrayIconBuilder::with_id(commands::TRAY_ID)
        .menu(&menu)
        .tooltip("chat-servant")
        // 菜单默认**只在右键**出现。左键不接管的话，用户点了图标
        // 什么都不会发生 —— 那个图标看着就是坏的。
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                let _ = commands::show_panel(app.clone());
            }
            "relock" => {
                // 和面板上那颗按钮走**同一条 Rust 路径**：
                // 事件 → 前端 → `relock_session`。两处不各写一遍。
                let _ = app.emit(events::MENU_RELOCK, ());
            }
            "settings" => {
                let _ = commands::open_settings(app.clone());
            }
            "debug" => {
                let _ = commands::open_debug(app.clone());
            }
            "quit" => {
                let _ = commands::quit_app(app.clone());
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle().clone();
                let visible = app
                    .get_webview_window("main")
                    .and_then(|w| w.is_visible().ok())
                    .unwrap_or(false);
                if visible {
                    let _ = commands::hide_panel(app);
                } else {
                    let _ = commands::show_panel(app);
                }
            }
        })
        .build(app);

    if let Err(e) = built {
        // 建不出来**不 panic**。程序照常跑，只是没有托盘 ——
        // 而此时设置里的「收进托盘」会被 `tray_available` 拒掉，
        // 「关闭时 = 退出」照常工作。**降级要看得见。**
        tracing::error!("托盘图标建不起来（程序仍可运行，但没有后台出口）：{e}");
    } else {
        tracing::info!("托盘图标已就绪");
    }
}

/// 装上日志 subscriber。
///
/// ## 为什么以前没有、现在必须有
///
/// 早先 `main()` 里**根本没有这一句**——`tracing` 当了依赖却从没有 subscriber
/// 去消费，于是 `commands.rs` / `watch.rs` 里的 `tracing::info!`、
/// `tracing::warn!` 全部静默丢弃，应用**从头到尾没输出过一行**。
///
/// 后果不是「日志好看点」：托盘建不起来、观察循环抓不到帧、模型调用失败、
/// 线程数异常增长——这些全都无从得知，只能靠现象反推。
/// 不可观测的系统，没法判断自己有没有在工作。
///
/// 级别由 `RUST_LOG` 决定，默认只到 `info`：
/// 设 `RUST_LOG=debug` 能看到每轮耗时与抓帧线程号。
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,wgc=info"));
    // `try_init` 而不是 `init`：测试里可能已经装过一个，别因此 panic。
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}

/// 事件名。前端只认这两个，**不轮询**。
pub mod events {
    pub const READOUT: &str = "readout";
    pub const TELEMETRY: &str = "telemetry";
    /// 托盘菜单点了「锁定当前会话」。前端监听它并调 `relock_session`，
    /// **和面板上那颗按钮走同一条 Rust 路径** —— 两处不各写一遍。
    pub const MENU_RELOCK: &str = "menu:relock";
}
