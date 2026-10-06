// release 构建在 Windows 上不弹额外控制台窗口。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod debug;
mod secret;
mod watch;

fn main() {
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
        ])
        .run(tauri::generate_context!())
        .expect("启动失败");
}

/// 事件名。前端只认这两个，**不轮询**。
pub mod events {
    pub const READOUT: &str = "readout";
    pub const TELEMETRY: &str = "telemetry";
}
