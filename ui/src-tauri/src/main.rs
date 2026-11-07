// release 构建在 Windows 上不弹额外控制台窗口。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod contract;
mod debug;
mod secret;
mod watch;

use tauri::{Emitter, Manager, WindowEvent};

fn setup_app(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    configure_models_dir(app);
    mark_fixture_window(app);
    commands::set_app_handle(app.handle().clone());
    build_tray(app.handle());
    commands::start(app.handle());
    commands::refresh_tray();
    if let Some(win) = app.get_webview_window("main") {
        spawn_clickthrough(win);
    }
    Ok(())
}

fn configure_models_dir(app: &tauri::App) {
    let Ok(dir) = app.path().resource_dir() else {
        return;
    };
    let models = dir.join(BUNDLED_MODELS_DIR);
    chatservant_capture::ocr_pp::set_models_dir(&models);
    if !models
        .join(chatservant_capture::ocr_pp_pipeline::KEYS_FILE)
        .is_file()
    {
        tracing::info!(
            dir = %models.display(),
            "装包里没有 PP-OCR 权重；选到 PP-OCR 时会退回 Windows OCR"
        );
    }
}

fn mark_fixture_window(app: &tauri::App) {
    let fixture_mode = std::env::var("CHATSERVANT_FIXTURE")
        .map(|p| !p.trim().is_empty())
        .unwrap_or(false);
    if !fixture_mode {
        return;
    }
    let base = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .map(|w| w.title.clone())
        .unwrap_or_else(|| "言侍".to_string());
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_title(&format!("{base} · 夹具模式"));
    }
    tracing::info!("夹具模式：窗口标题已标注「· 夹具模式」；投递一律拒投");
}

fn spawn_clickthrough(win: tauri::WebviewWindow) {
    std::thread::Builder::new()
        .name("cs-clickthrough".into())
        .spawn(move || {
            let mut ignoring = false;
            loop {
                if !win.is_visible().unwrap_or(true) {
                    if ignoring && win.set_ignore_cursor_events(false).is_ok() {
                        ignoring = false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    continue;
                }
                std::thread::sleep(std::time::Duration::from_millis(40));
                update_clickthrough(&win, &mut ignoring);
            }
        })
        .ok();
}

fn update_clickthrough(win: &tauri::WebviewWindow, ignoring: &mut bool) {
    let (Ok(cur), Ok(pos), Ok(scale)) = (
        win.cursor_position(),
        win.outer_position(),
        win.scale_factor(),
    ) else {
        return;
    };
    let lx = (cur.x - f64::from(pos.x)) / scale;
    let ly = (cur.y - f64::from(pos.y)) / scale;
    let in_outer = (0.0..=380.0).contains(&lx) && (0.0..=560.0).contains(&ly);
    let on_paper = (16.0..=364.0).contains(&lx) && (4.0..=508.0).contains(&ly);
    let want_ignore = in_outer && !on_paper;
    if want_ignore != *ignoring && win.set_ignore_cursor_events(want_ignore).is_ok() {
        *ignoring = want_ignore;
    }
}

fn main() {
    init_tracing();
    tauri::Builder::default()
        .setup(setup_app)
        .on_window_event(|window, event| {
            if matches!(event, WindowEvent::Focused(false)) && window.label() == "tray" {
                let _ = window.hide();
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                // **默认行为是错的**：Tauri 收到关闭请求会退出整个应用。
                // 而这个程序是 alwaysOnTop + skipTaskbar + 无边框——
                // 一旦它从别的途径（比如系统「关闭窗口」快捷键）被关掉，
                // 用户连同托盘图标一起弄丢了，那才叫找不回来。
                //
                // ⚠⚠ **但「一律吞掉」同样是错的**（2026-11-01 改，票 99）。
                // 设置页「窗口 → 按窗口控制键时：退出 / 收进托盘」那一档，
                // 在此之前**只写不读**：分支无条件 `hide_panel`，于是两个选项
                // 结果一样，而设置页写着「按 × 直接退出。程序不会在后台继续跑」。
                // ⇒ 落点由偏好定：`Exit` 就真退，只有 `Tray` 才收起来。
                let app = window.app_handle().clone();
                let label = window.label().to_string();
                match label.as_str() {
                    // 主面板：按偏好分岔。
                    "main" => {
                        let action = commands::close_action(&app);
                        // 落点必须进日志：这条分支在票 99 之前无条件 `hide_panel`，
                        // 于是「选了退出却关不掉」在日志里一个字都没有。
                        tracing::info!(
                            target: "chat_servant_app::close",
                            label = %label,
                            action = ?action,
                            "收到关闭请求"
                        );
                        if action == commands::CloseAction::Quit {
                            // `quit_app` 走 `app.exit(0)`，窗口随进程一起没了，
                            // 这条事件链到此为止 ⇒ 不需要也不该再 prevent。
                            let _ = commands::quit_app(app);
                        } else {
                            let _ = commands::hide_panel(app);
                            api.prevent_close();
                        }
                    }
                    // 识别画布：本来就有标题栏，关掉就关掉。
                    "debug" => {}
                    // 托盘纸面菜单：收起窗口而不销毁，下次右键直接原位弹出。
                    _ => {
                        let _ = window.hide();
                        api.prevent_close();
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::latest_readout,
            commands::debug_pane,
            commands::dispatch,
            commands::open_settings,
            commands::open_debug,
            commands::get_settings,
            commands::save_setting,
            commands::forget_key,
            commands::probe_key,
            commands::set_model,
            commands::set_effort,
            commands::set_route,
            commands::get_proxy,
            commands::set_proxy,
            commands::get_recent_n,
            commands::set_recent_n,
            commands::get_token_budget,
            commands::set_token_budget,
            commands::get_images_out,
            commands::set_images_out,
            commands::get_tier,
            commands::set_tier,
            commands::relock_session,
            commands::get_session_mode,
            commands::set_session_mode,
            // 窗口生命周期。见 commands.rs 里那一节的说明——
            // 早先这个程序**没有任何办法停掉**。
            commands::get_close_behavior,
            commands::set_close_behavior,
            commands::get_mark_shape,
            commands::set_mark_shape,
            commands::tray_ready,
            commands::hide_panel,
            commands::show_panel,
            commands::drag_window_by,
            commands::set_always_on_top,
            commands::quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("启动失败");
}

/// 建托盘图标。
///
/// **这是一个必须存在的出口。** 主窗口 `decorations: false`（没有标题栏）、
/// `skipTaskbar: true`（不在任务栏）、`alwaysOnTop: true`，三者叠在一起
/// 意味着系统层面没有任何可见的关闭入口。托盘是唯一的。
///
/// `tray-icon` 这个 feature 一直开着，只是**一次都没被用上**。
fn handle_menu_event(app: &tauri::AppHandle, event: tauri::menu::MenuEvent) {
    match event.id().as_ref() {
        "show" => {
            let _ = commands::show_panel(app.clone());
        }
        "relock" => {
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
    }
}

fn handle_tray_click(tray: &tauri::tray::TrayIcon, event: tauri::tray::TrayIconEvent) {
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
    let TrayIconEvent::Click {
        button,
        button_state: MouseButtonState::Up,
        position,
        ..
    } = event
    else {
        return;
    };
    let app = tray.app_handle().clone();
    match button {
        MouseButton::Left => {
            if let Some(tw) = app.get_webview_window("tray") {
                let _ = tw.hide();
            }
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
        MouseButton::Right => show_tray_menu(&app, position),
        _ => {}
    }
}

fn build_tray(app: &tauri::AppHandle) {
    use tauri::tray::TrayIconBuilder;

    // **图标必须显式给。**（实测 2026-10-10）
    //
    // `TrayIconBuilder::with_id(...)` 不像 `new(app)` 那样自动取应用图标，
    // 而 `tauri.conf.json` 的主窗口也没有 `icon` 字段（`bundle.icon` 只喂
    // 安装包，不喂窗口）。两头都没有 → 托盘**条目在、图标是空的**：
    // 日志照样打「托盘图标已就绪」，但用户看到的是一个空白格子。
    //
    // 这比「图标没出来」更难查，因为它看起来像图标加载慢。
    // 所以图直接编译进二进制（`include_bytes!`），不依赖运行期资源路径。
    //
    // 解码与「会发出去的档盖红点」都在 `commands::tray_icon` 里——**只有那一份**，
    // 这样 `refresh_tray` 换档时不会做出一个跟初始图标不一致的版本。
    let icon = match commands::tray_icon(commands::tier_can_send()) {
        Ok(i) => i,
        Err(e) => {
            tracing::error!("托盘图标读不出来（程序仍可运行，但没有可见的托盘入口）：{e}");
            commands::set_tray_ready(false);
            return;
        }
    };

    // 菜单在 `commands` 里构造 —— 它要读档位。
    let menu = match commands::build_tray_menu(app) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("托盘菜单建不起来：{e}");
            commands::set_tray_ready(false);
            return;
        }
    };

    let mut builder = TrayIconBuilder::with_id(commands::TRAY_ID)
        .icon(icon)
        .tooltip(commands::tray_tooltip())
        // 菜单默认**只在右键**出现。左键不接管的话，用户点了图标
        // 什么都不会发生 —— 那个图标看着就是坏的。
        .show_menu_on_left_click(false)
        .on_menu_event(handle_menu_event)
        .on_tray_icon_event(handle_tray_click);
    if app.get_webview_window("tray").is_none() {
        builder = builder.menu(&menu);
    }
    let built = builder.build(app);
    if let Err(e) = built {
        // 建不出来**不 panic**。程序照常跑，只是没有托盘 ——
        // 而此时设置里的「收进托盘」会被 `tray_available` 拒掉，
        // 「关闭时 = 退出」照常工作。**降级要看得见。**
        tracing::error!("托盘图标建不起来（程序仍可运行，但没有后台出口）：{e}");
        commands::set_tray_ready(false);
    } else {
        // **成功就是权威结论，不是「等它能被查到」。**
        //
        // 「关闭时 = 收进托盘」这一档的成立与否由这里说了算。见
        // `commands::set_tray_ready` 的注释：查 `tray_by_id` 在启动后
        // 头 ~200ms 里恒为 `false`，会让面板把自己的出口判成「退出」。
        commands::set_tray_ready(true);
        tracing::info!("托盘图标已就绪");
    }
}

/// 在托盘右键点击处弹出 118px 撕边棉纸菜单窗口（替代 Win32 原生 `#32768` 菜单）。
fn show_tray_menu(app: &tauri::AppHandle, cursor: tauri::PhysicalPosition<f64>) {
    let Some(w) = app.get_webview_window("tray") else {
        return;
    };
    let scale = w.scale_factor().unwrap_or(1.0);
    let pw = (134.0 * scale).round() as i32;
    let ph = (204.0 * scale).round() as i32;
    let gap = (6.0 * scale).round() as i32;
    let cx = cursor.x.round() as i32;
    let cy = cursor.y.round() as i32;

    let mut x = cx - pw / 2;
    let mut y = if cy > ph + gap {
        cy - ph - gap
    } else {
        cy + gap
    };

    if let Ok(Some(mon)) = w.current_monitor() {
        let mpos = mon.position();
        let msize = mon.size();
        let min_x = mpos.x + gap;
        let max_x = mpos.x + (msize.width as i32) - pw - gap;
        let min_y = mpos.y + gap;
        let max_y = mpos.y + (msize.height as i32) - ph - gap;
        if max_x >= min_x {
            x = x.clamp(min_x, max_x);
        }
        if max_y >= min_y {
            y = y.clamp(min_y, max_y);
        }
    }

    let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
    let _ = w.show();
    let _ = w.set_focus();
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

/// 事件名清单。**这里只是转出，定义在 [`crate::contract::events`]。**
///
/// 2026-10-13：本文件原先自己有一份 `pub mod events`（只有 3 个常量），
/// 而 `contract.rs` 里另有一份（4 个）。两份都是「事件名的集中处」，
/// 等于集中了两次——而 `contract.rs` 那份连编译都没进过，
/// 于是守住这件事的 `no_event_literal_outside_this_file` 从未运行过。
/// 现在单一出处是 `contract.rs`。
pub use crate::contract::events;

/// 装包里权重落在 `resource_dir()` 下的**哪一层**。
///
/// 这个字面量出现在**两个地方**——`tauri.conf.json` 的 `bundle.resources` 映射目标，
/// 和 [`main`] 的 `setup` 里那次 `dir.join(...)`。两处任一改错，
/// 装包**照样成功**、权重**照样没进去**、产品**照样静默退回 Windows OCR**，
/// 而且 Tauri 自己不报错（它只管「源路径存不存在」）。
///
/// 所以它必须是**一个常量**，由
/// [`wiring_tests::the_bundled_models_dir_is_the_one_the_shell_joins`] 钉住两侧一致。
const BUNDLED_MODELS_DIR: &str = "models";

/// **`bundle.resources` 与 `ModelSet` 的相对路径必须指向同一处。**
///
/// 装机后 `resource_dir()/models/ppocrv6-small/` 才是权重真正落地的位置，
/// 而 [`chatservant_capture::ocr_pp_pipeline::ModelSet`] 里三个文件名都是
/// **相对 `models_dir()`** 的。两份配置各写各的时候：
///
/// - 打包**照样成功**（资源映射写错常常只是少拷几个文件，Tauri 不报错）；
/// - 产品启动时找不到权重 ⇒ **静默退回 Windows.Media.Ocr**，
///   而那条路会漏掉 17.6% 的对方消息（ADR-0006 的真帧检出率）。
///
/// ⇒ 这是一条**失败面完全静默**的接线，值得一条测试。
///
/// ⚠ 它只由 [`wiring_tests`] 调用，**不在 `main()` 里跑**：
/// 这两条配置都是**编译期常量**，不配就是构建期的问题，
/// 让它在开发机上变成一个「启动即崩」只会更难查——
/// 该响的地方是门禁（`cargo test`），不是用户的启动流程。
///
/// 函数本身也带 `cfg(test)`：唯一的调用点在 `#[cfg(test)] mod wiring_tests` 里，
/// 而 `--all-targets` **同时编译测试与非测试两个产物**——不标的话，
/// 非测试那一半会看见一个没人调用的函数，门禁以 dead_code 变红
/// （实测 exit=101，`error: function ... is never used`）。
#[cfg(test)]
fn assert_resources_and_model_set_agree() {
    let conf = include_str!("../tauri.conf.json");
    let prefix = "ppocrv6-small";
    let det = chatservant_capture::ocr_pp_pipeline::DET_MODEL;
    let rec = chatservant_capture::ocr_pp_pipeline::REC_MODEL;
    let keys = chatservant_capture::ocr_pp_pipeline::KEYS_FILE;
    assert!(
        conf.contains(prefix),
        "tauri.conf.json 的 bundle.resources 里没有 {prefix}/ —— \
         装包不会带权重，产品会静默退回 Windows OCR"
    );
    for name in [det, rec, keys] {
        assert!(
            name.starts_with(prefix),
            "ModelSet 的 {name} 不在 resources 映射的 {prefix}/ 底下"
        );
    }
}

#[cfg(test)]
mod wiring_tests {
    use super::BUNDLED_MODELS_DIR;

    /// **外壳 join 的那层目录，必须就是 resources 映射目标的第一段。**
    ///
    /// 这是同一条接线的**第三块**，早先没人盯：
    /// [`super::assert_resources_and_model_set_agree`] 只比了
    /// 「conf 里有 `ppocrv6-small`」与「`ModelSet` 的文件名以它开头」，
    /// **没比外壳那次 `dir.join(..)`**。
    ///
    /// 后果与另两处一样：把常量改成 `resources` 而 conf 还写 `models/`
    /// ⇒ 装包成功、权重进去、产品找不到、静默退回 WinRT，且 Tauri 不报错
    /// （它只管「源路径存不存在」）。
    #[test]
    fn the_bundled_models_dir_is_the_one_the_shell_joins() {
        let conf = include_str!("../tauri.conf.json");
        let prefix = "ppocrv6-small";
        // 只看**源路径指向 target/models 的那些条目**——将来加图标之类
        // 别的资源时，不该被这条判为不合法。
        let mut checked = 0usize;
        for line in conf.lines().filter(|l| l.contains("target/models/")) {
            let Some((_, dest)) = line.split_once("\": ") else {
                panic!("resources 条目解析不出来（格式变了？）：{line}");
            };
            let dest = dest.trim().trim_matches(|c| c == ',' || c == '"');
            let first = dest.split('/').next().unwrap_or_default();
            assert_eq!(
                first, BUNDLED_MODELS_DIR,
                "resources 目标 `{dest}` 的第一段与外壳 join 的 \
                 BUNDLED_MODELS_DIR(`{BUNDLED_MODELS_DIR}`) 不一致"
            );
            assert!(
                dest.starts_with(&format!("{BUNDLED_MODELS_DIR}/{prefix}")),
                "resources 目标 `{dest}` 不在 `{BUNDLED_MODELS_DIR}/{prefix}/` 底下"
            );
            checked += 1;
        }
        assert!(
            checked > 0,
            "tauri.conf.json 的 bundle.resources 里**一条权重映射都没有**——\
             装包不会带权重，产品会静默退回 Windows OCR"
        );
    }

    /// 装机形态那条接线**失败时完全静默**，所以必须有测试盯着。
    ///
    /// 这条测试存在的理由是它验的是**两份配置之间的一致性**，
    /// 而不是任何一份配置本身对不对——两份都对但互相不认，是最容易漏的那类错。
    #[test]
    fn the_bundle_resources_and_the_model_set_point_at_the_same_directory() {
        super::assert_resources_and_model_set_agree();
    }

    /// `models_dir()` 的优先级：**环境变量 > 外壳 > 自动查找**。
    ///
    /// ⚠ `set_models_dir` 是 `OnceLock`，**只设一次**，所以这条只验
    /// 「没设过的时候它仍然落在仓库 `target/models`」——
    /// 设过之后的优先级只能靠另一个进程验（`xtask` 就是干这个的）。
    #[test]
    fn models_dir_still_finds_the_repo_weights_without_a_shell() {
        let d = chatservant_capture::ocr_pp::models_dir();
        assert_eq!(
            d.file_name().and_then(|s| s.to_str()),
            Some("models"),
            "自动查找必须落在名为 models 的目录上：{}",
            d.display()
        );
        assert!(
            !d.to_string_lossy().contains("%APPDATA%") && !d.to_string_lossy().contains("AppData"),
            "权重又落回系统盘了：{}（硬约束：产物一律留项目所在盘）",
            d.display()
        );
    }

    /// 系统关闭请求**必须先读偏好，再决定吞不吞**。
    ///
    /// 这条守的是 2026-11-01 之前那个形态（票 99）：`CloseRequested` 分支不读
    /// 偏好、无条件 `hide_panel` + `api.prevent_close()`，于是设置页里
    /// 「退出 / 收进托盘」两个选项结果一样，而文案写着「按 × 直接退出」。
    ///
    /// 为什么是**源码扫描**而不是调一遍：判据本体（`commands::close_action`）
    /// 由 `commands.rs` 的 `close_action_follows_the_preference` 量，可它证明不了
    /// 「这里有没有调它」——`AppHandle` 起不来，闭包也没法单独执行。
    /// ⇒ 量的是**接线**：这段分支里要读偏好、要有真退出那支、且不许在它前面预防。
    #[test]
    fn the_close_request_reads_the_preference_before_hiding() {
        let source = include_str!("main.rs");
        let prod = source.split("#[cfg(test)]").next().unwrap_or(source);

        // 从 `CloseRequested` 切到下一个顶层注册点：中间只有这一个处理器。
        let start = prod
            .find("WindowEvent::CloseRequested")
            .expect("main.rs 里没有 CloseRequested ⇒ 关闭拦截整段没了");
        let end = prod[start..]
            .find(".invoke_handler(")
            .expect("找不到 invoke_handler ⇒ 切窗锚点变了，先修这条守卫")
            + start;
        let handler = &prod[start..end];

        let at = handler.find("commands::close_action(").unwrap_or_else(|| {
            panic!("关闭分支没有读「关闭时」偏好（票 99 那个形态回来了）：{handler}")
        });
        assert!(
            handler.contains("commands::CloseAction::Quit") && handler.contains("quit_app("),
            "偏好为「退出」时没有真正的退出分支：{handler}"
        );
        // `prevent_close` 必须在读偏好**之后**——排在前面就说明它又变回无条件吞掉。
        let prevent = handler
            .find("api.prevent_close()")
            .expect("主面板不再拦关闭请求了？那收进托盘会连窗口一起没");
        assert!(
            prevent > at,
            "`prevent_close` 出现在读偏好之前 ⇒ 关闭请求还是被无条件吞掉：{handler}"
        );
        // 识别画布有自己的标题栏，那条放行分支不该被顺手改成「也收起来」。
        let debug_arm = ["\"debug\"", "=> {}"].join(" ");
        assert!(
            handler.contains(&debug_arm),
            "识别画布的放行分支没了：{handler}"
        );
    }
}
