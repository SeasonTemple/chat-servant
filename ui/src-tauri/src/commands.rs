//! Tauri 命令层。**Rust 侧是唯一事实来源**——前端只渲染这里发过去的东西。
//!
//! 这里刻意只放**纯逻辑**与**几何探测**，不放输入注入：
//! ticket 09 的验收标准是「回读剪贴板与候选原文字节级相等」，
//! 在那之前先不接注入，免得把「以为成功了」写进产品。

use chatservant_core::dispatch::{DeliveryReceipt, Dispatch, Dispatcher};
use serde::Serialize;
use tauri::{AppHandle, Manager, PhysicalPosition};

/// `probe_anchor` 的返回。前端只渲染，不自己算几何。
#[derive(Serialize)]
pub struct AnchorProbe {
    pub ok: bool,
    pub anchor: String,
    /// 推不出来时的真实原因。**不假装成功**。
    pub detail: String,
}

#[tauri::command]
pub fn probe_anchor() -> AnchorProbe {
    // 现在还没有常驻的观察循环，所以只报「没在观察」而不是编一个锚点。
    // 编出来的锚点会让前端渲染一个假的框，比报错难查得多。
    AnchorProbe {
        ok: false,
        anchor: String::new(),
        detail: "观察循环尚未启动（ticket 16 接上采集层后才有锚点）".into(),
    }
}

/// `dispatch` 的返回。**回执不是布尔值。**
#[derive(Serialize)]
pub struct DispatchReply {
    pub outcome: &'static str,
    pub detail: String,
}

#[tauri::command]
pub fn dispatch(candidate_id: String, action: String, tier: String) -> DispatchReply {
    let tier: Dispatch = match tier.as_str() {
        "Auto" => Dispatch::Auto,
        "Confirm" => Dispatch::Confirm,
        // 认不出来的一律当最低档。**不许因为解析失败就升档。**
        _ => Dispatch::Manual,
    };

    let mut d = Dispatcher::new();
    d.set_tier(tier);

    // 注入还没接（ticket 09）。这里如实说没送出去，
    // 而不是假装成功——`Sent` 会让上层认为不需要回读。
    let receipt = DeliveryReceipt::NotSent;
    let next = d.next(receipt.clone(), 0);

    DispatchReply {
        outcome: "NotSent",
        detail: format!(
            "注入尚未接线（ticket 09）。候选 {candidate_id}，动作 {action}，档位 {tier}。{}｜下一步：{}",
            receipt.explain(),
            next.explain()
        ),
    }
}

#[tauri::command]
pub fn open_settings(app: AppHandle) -> Result<(), String> {
    let w = app
        .get_webview_window("settings")
        .ok_or_else(|| "settings 窗口不存在".to_string())?;
    // 从屏幕外挪回屏幕内再显示。`park_settings` 把它藏到了 (-32000, -32000)，
    // 直接 show 会在左上角闪一下。
    w.set_position(PhysicalPosition::new(120.0, 120.0))
        .map_err(|e| e.to_string())?;
    w.show().map_err(|e| e.to_string())?;
    w.set_focus().map_err(|e| e.to_string())
}

/// 把 `settings` 窗口挪到屏幕外。
///
/// **为什么要挪而不是直接 `visible: false`**：Tauri 2 的窗口配置里
/// `visible: false` 会让 WebView2 跳过 drop target 与合成器的注册，
/// 首次 show 出来可能是白屏或黑屏。挪到屏幕外让它正常初始化，
/// 用户看不见，但合成器已经就位。
///
/// 这条是 ADR-0001 记的 Windows 坑之一。
pub fn park_settings(app: &AppHandle) {
    let Some(w) = app.get_webview_window("settings") else {
        return;
    };
    let _ = w.set_position(PhysicalPosition::new(-32000.0, -32000.0));
    let _ = w.hide();
}

/// 启动观察循环的占位。现在是空的——**不假装在跑**。
///
/// ticket 16 会把 `PaneWatcher` + `pipeline::read` 接进来。
/// 那之前 `probe_anchor` 如实说「没在观察」，不编一个锚点出来。
#[allow(dead_code)]
pub fn start(app: &AppHandle) {
    let _ = app;
}
