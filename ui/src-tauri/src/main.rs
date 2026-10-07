// release 构建在 Windows 上不弹额外控制台窗口。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod debug;
mod secret;
mod watch;

fn main() {
    init_tracing();
    tauri::Builder::default()
        .setup(|app| {
            commands::park_settings(app.handle());
            // 观察循环**在这里起**。它自己的线程里跑
            // （WGC 会话对象不是 Send，不能跨线程 move）。
            commands::start(app.handle());
            Ok(())
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
        ])
        .run(tauri::generate_context!())
        .expect("启动失败");
}

/// 装上日志 subscriber。
///
/// ## 为什么以前没有、现在必须有
///
/// 早先 `main()` 里**根本没有这一句**——`tracing` 当了依赖却从没有 subscriber
/// 去消费，于是 `commands.rs` / `watch.rs` 里的 `tracing::info!`、
/// `tracing::warn!` 全部静默丢弃，应用**从头到尾没输出过一行**。
///
/// 后果不是「日志好看点」：观察循环抓不到帧、模型调用失败、
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
}
