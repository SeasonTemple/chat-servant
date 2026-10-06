//! Tauri 命令层。**Rust 侧是唯一事实来源**——前端只渲染这里发过去的东西。
//!
//! 这里刻意只放**纯逻辑**与**几何探测**，不放输入注入：
//! ticket 09 的验收标准是「回读剪贴板与候选原文字节级相等」，
//! 在那之前先不接注入，免得把「以为成功了」写进产品。

use chatservant_core::dispatch::{DeliveryReceipt, Dispatch, Dispatcher};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};

use crate::secret::{self, KeyStatus};
use crate::watch::{DraftCandidate, LoopConfig, Readout, RecentN, Watcher};
use chatservant_reason::TurnContext;

/// 全局单例：一个应用只有一个观察循环。
///
/// **Mutex 里的类型必须能在循环线程里用**——所以循环自己起了线程，
/// 这里只存 `Watcher` 句柄与**纯数据**快照。
static STATE: Mutex<Option<AppState>> = Mutex::new(None);

struct AppState {
    watcher: Watcher,
    /// 最新一轮的候选。`dispatch` 要按 id 取正文。
    candidates: Vec<DraftCandidate>,
    /// 与观察循环**共用同一个格子**的设置句柄。
    ///
    /// ⚠ 这里必须是 `start()` 建的**那一份**（clone），不能新造一个。
    /// 新造的话设置写进了一个没人读的格子——界面显示「已设为 24」，
    /// 而模型那边永远还是 12。
    recent_n: RecentN,
}

/// 取全局状态。**锁中毒不算致命**——状态还在，继续用比崩掉好。
fn with_state<R>(f: impl FnOnce(&mut AppState) -> R) -> Option<R> {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    g.as_mut().map(f)
}

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
    let Some(out) = with_state(|s| s.watcher.latest()).flatten() else {
        return AnchorProbe {
            ok: false,
            anchor: String::new(),
            detail: "观察循环还没跑出第一帧".into(),
        };
    };
    if let Some(e) = &out.error {
        return AnchorProbe {
            ok: false,
            anchor: String::new(),
            detail: e.clone(),
        };
    }
    match out.anchor {
        Some(a) => AnchorProbe {
            ok: true,
            anchor: format!(
                "({},{})-({},{})",
                a.chat.x0, a.chat.y0, a.chat.x1, a.chat.y1
            ),
            detail: format!(
                "第 {} 圈｜OCR 累计 {} 次｜模型累计 {} 次",
                out.ticks, out.ocr_calls, out.model_calls
            ),
        },
        None => AnchorProbe {
            ok: false,
            anchor: String::new(),
            detail: format!("第 {} 圈：静默期，没有新消息", out.ticks),
        },
    }
}

/// 取最近一轮的完整快照。**前端刷新时用它对齐**，不轮询——用户手动触发。
#[tauri::command]
pub fn latest_readout() -> Option<Readout> {
    with_state(|s| s.watcher.latest()).flatten()
}

/// 调试画布数据。**没跑出第一帧时返回 `None`**，前端显示「等第一帧」，
/// 不画一张空白画布假装正常。
#[tauri::command]
pub fn debug_pane() -> Option<crate::debug::DebugPane> {
    crate::watch::debug_pane()
}

/// `dispatch` 的返回。**回执不是布尔值。**
#[derive(Serialize)]
pub struct DispatchReply {
    pub outcome: &'static str,
    pub detail: String,
}

#[tauri::command]
pub fn dispatch(
    app: AppHandle,
    candidate_id: String,
    action: String,
    tier: String,
) -> DispatchReply {
    let tier: Dispatch = match tier.as_str() {
        "Auto" => Dispatch::Auto,
        "Confirm" => Dispatch::Confirm,
        // 认不出来的一律当最低档。**不许因为解析失败就升档。**
        _ => Dispatch::Manual,
    };

    let mut d = Dispatcher::new();
    d.set_tier(tier);

    // Confirm 档：**没确认就不发送**。这条是硬约束，程序不许替人省这一步。
    // 第一次点「填入并确认发送」只是**准备**，第二次（tier 已是 Confirm）
    // 才真的发。所以判据是 tier 而不是 action。
    if tier == Dispatch::Confirm && action == "confirm" {
        let _ = app.emit("armed", &candidate_id);
        return DispatchReply {
            outcome: "NotSent",
            detail: "已准备好。再点一次「确认发送」才会真的发出去。".into(),
        };
    }

    let receipt = deliver(&candidate_id, &action, tier);

    // 让档位守则跑一遍，把下一步该做什么告诉前端。
    let next = d.next(receipt.clone(), 0);
    DispatchReply {
        outcome: outcome_name(&receipt),
        detail: format!("{}｜下一步：{}", receipt.explain(), next.explain()),
    }
}

/// 找微信主窗口的 `HWND`。找不到就**如实报找不到**。
///
/// `WindowHit::hwnd_raw` 故意是 `isize`（跨 crate 只传不透明整数），
/// 这里按句柄反过来查窗口——`IsWindow` 是唯一可靠的验法：
/// 句柄被复用时它立刻返回 false。
fn find_wechat() -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;

    let surface = chatservant_capture::surface::WECHAT;
    let hit = chatservant_capture::find_chat_window(&surface).ok()?;
    let hwnd = windows::Win32::Foundation::HWND(hit.hwnd_raw as *mut core::ffi::c_void);
    unsafe { IsWindow(Some(hwnd)).as_bool().then_some(hwnd) }
}

/// 真正去送一次。**目前只有 Manual 档能走到这里。**
fn deliver(candidate_id: &str, action: &str, tier: Dispatch) -> DeliveryReceipt {
    let Some(hwnd) = find_wechat() else {
        return DeliveryReceipt::Failed("找不到微信窗口（已退出？换了进程名？）".into());
    };

    // ⚠ 候选取文本这一步还没接（ticket 10 起草层）。
    // 在那之前**不能**拿占位文本去注入——那会把一句假话送进别人的聊天框。
    let Some(text) = candidate_text(candidate_id) else {
        return DeliveryReceipt::Failed(format!(
            "候选 {candidate_id} 的正文还没接起草层（ticket 10），不注入占位文本"
        ));
    };

    let _ = (action, tier);
    chatservant_capture::inject::fill_input_box(&text, hwnd)
}

/// 回执 → 前端认得的字符串。
fn outcome_name(r: &DeliveryReceipt) -> &'static str {
    match r {
        DeliveryReceipt::NotSent => "NotSent",
        DeliveryReceipt::Sent => "Sent",
        DeliveryReceipt::PossiblySent => "PossiblySent",
        DeliveryReceipt::Failed(_) => "Failed",
        DeliveryReceipt::TimedOut => "TimedOut",
    }
}

/// 候选正文。**从观察循环的快照里按 id 取。**
///
/// ⚠ 取不到就返回 `None`，`deliver` 随后报「没接起草层」——
/// **绝不拿占位文本去注入**。往别人的聊天框里送一句假话，
/// 比不送严重得多。
fn candidate_text(id: &str) -> Option<String> {
    with_state(|s| {
        s.candidates
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.text.clone())
    })
    .flatten()
}

/// 把某个隐藏窗口挪到屏幕内并显示。**共用一条路径**——
/// 三个窗口的「打开」逻辑一样，写三遍就是三处能写错的地方。
fn show_window(app: &AppHandle, label: &str) -> Result<(), String> {
    let w = app
        .get_webview_window(label)
        .ok_or_else(|| format!("{label} 窗口不存在"))?;
    w.set_position(PhysicalPosition::new(120.0, 120.0))
        .map_err(|e| e.to_string())?;
    w.show().map_err(|e| e.to_string())?;
    w.set_focus().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_settings(app: AppHandle) -> Result<(), String> {
    show_window(&app, "settings")
}

#[tauri::command]
pub fn open_debug(app: AppHandle) -> Result<(), String> {
    show_window(&app, "debug")
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

/// 当前支持的模型供应商。
///
/// 换供应商只需在这里加一项，**不用重配整个应用**——
/// 设置页按这个列表渲染，每个供应商的 key 独立存在凭据管理器里。
pub const PROVIDERS: &[&str] = &["openai", "anthropic"];

/// 设置页要读的状态。**这里没有任何 key。**
#[derive(Serialize)]
pub struct SettingsView {
    pub providers: Vec<ProviderView>,
}

#[derive(Serialize)]
pub struct ProviderView {
    pub id: String,
    /// 已经配好了。
    pub configured: bool,
    /// 值来自环境变量——这种情况下输入框应只读。
    pub from_env: bool,
    /// 掩码，仅供显示。
    pub masked: String,
    /// 没配时给的下一步指引。**不能是空字符串或静默失败。**
    pub hint: String,
}

#[tauri::command]
pub fn get_settings() -> SettingsView {
    SettingsView {
        providers: PROVIDERS
            .iter()
            .map(|p| {
                let s: KeyStatus = secret::status(p);
                ProviderView {
                    id: (*p).to_string(),
                    configured: s.configured,
                    from_env: s.from_env,
                    masked: s.masked,
                    hint: if s.configured {
                        String::new()
                    } else {
                        secret::missing_hint(p)
                    },
                }
            })
            .collect(),
    }
}

/// 提交设置。**立即生效，不用重启。**
///
/// `key` 为 `None` 或空白 = 保留原值。删 key 走 `forget_key`。
#[tauri::command]
pub fn save_setting(provider: String, key: Option<String>) -> Result<bool, String> {
    if !PROVIDERS.contains(&provider.as_str()) {
        // 认不出来的 provider 一律拒绝，**不静默接受**——
        // 拼错的供应商名会让人以为 key 存上了，其实没有。
        return Err(format!(
            "未知的供应商 {provider}。支持的是：{}",
            PROVIDERS.join(" / ")
        ));
    }
    let wrote = secret::apply(&provider, key.as_deref())?;
    if wrote {
        secret::forget_all();
    }
    Ok(wrote)
}

/// 「参考上下文」设置。**上下限由 Rust 下发**，前端不自己写死。
///
/// 硬编码 `min="4" max="40"` 在 HTML 里的话，后端一旦改了策略，
/// 滑块能拖到的范围和真实生效范围就对不上了——用户拖到 3，
/// 后端夹成 4，界面上那个数永远是假的。所以数字只从这儿来。
#[derive(Serialize)]
pub struct RecentView {
    /// 当前生效值（已夹取）。
    pub n: usize,
    pub min: usize,
    pub max: usize,
    pub default: usize,
}

fn recent_view(n: usize) -> RecentView {
    use chatservant_reason::context;
    RecentView {
        n,
        min: context::MIN_RECENT,
        max: context::MAX_RECENT,
        default: context::DEFAULT_RECENT,
    }
}

/// 读「参考上下文」条数。
#[tauri::command]
pub fn get_recent_n() -> Result<RecentView, String> {
    let n = with_state(|s| s.recent_n.get())
        .ok_or_else(|| "观察循环还没起来，读不到设置".to_string())?;
    Ok(recent_view(n))
}

/// 改「参考上下文」条数。**返回夹取后的真实生效值。**
///
/// 循环每圈读一次这个格子，所以**下一圈就用新值**，不用重启。
/// 越界不报错，直接夹——报错的话用户只会看到「设置失败」，
/// 不知道自己拖的值被改成了什么。
#[tauri::command]
pub fn set_recent_n(n: usize) -> Result<RecentView, String> {
    let applied = with_state(|s| s.recent_n.set(n))
        .ok_or_else(|| "观察循环还没起来，改不了设置".to_string())?;
    Ok(recent_view(applied))
}

#[tauri::command]
pub fn forget_key(provider: String) -> Result<(), String> {
    if !PROVIDERS.contains(&provider.as_str()) {
        return Err(format!("未知的供应商 {provider}"));
    }
    secret::erase(&provider)
}

/// 测试连通性。**只回「通不通」，不回任何响应体**——
/// 上游的错误正文可能带 key。
#[tauri::command]
pub fn probe_key(provider: String) -> Result<bool, String> {
    if !PROVIDERS.contains(&provider.as_str()) {
        return Err(format!("未知的供应商 {provider}"));
    }
    if secret::load(&provider).is_none() {
        return Err(secret::missing_hint(&provider));
    }
    // 网络层 ticket 10 接上。**现在不假装能连通**——
    // 返回 false 而不是 true，前端就知道还没接。
    Ok(false)
}

/// 起观察循环。**只在启动时调一次。**
///
/// 有 key 就组装**真实传输**（`UreqHttp` + `HttpReasoner`）；
/// 没 key 就用**明确标注的预置传输**——界面上每条候选都带「预置」字样，
/// 不能让用户以为模型接通了。
pub fn start(app: &AppHandle) {
    let cfg = build_loop_config();
    let transport_ready = cfg.transport_ready;
    // 关键：**先把句柄 clone 一份给命令层，再把 cfg move 进循环。**
    // 两边指的是同一个 `Arc<AtomicUsize>`，所以 `set_recent_n` 写进去的
    // 值，循环下一圈就能读到。少这一步，滑块就是个摆设。
    let recent_n = cfg.recent_n.clone();

    let watcher = crate::watch::spawn(cfg);
    if let Ok(mut g) = STATE.lock() {
        *g = Some(AppState {
            watcher,
            candidates: Vec::new(),
            recent_n,
        });
    }

    // 快照 → Tauri 事件。UI 只监听，不轮询。
    let app2 = app.clone();
    std::thread::Builder::new()
        .name("cs-pump".into())
        .spawn(move || {
            let mut last = 0u64;
            loop {
                if let Some(out) = with_state(|s| s.watcher.latest()).flatten()
                    && out.ticks != last
                {
                    last = out.ticks;
                    if let Ok(mut g) = STATE.lock()
                        && let Some(s) = g.as_mut()
                    {
                        s.candidates = out.candidates.clone();
                    }
                    let _ = app2.emit("readout", &out);
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        })
        .ok();

    if transport_ready {
        tracing::info!("观察循环已启动（真实模型传输）");
    } else {
        tracing::warn!(
            "观察循环已启动，但**没有可用 key**——用预置传输。候选是假的，别当成模型输出。"
        );
    }
}

/// 组装循环配置。**key 从凭据管理器读**，读不到就用预置传输。
fn build_loop_config() -> LoopConfig {
    use chatservant_reason::{HttpReasoner, Provider, UreqHttp};

    // 谁有 key 就用谁。**默认先试 OpenAI**——设置页上它是第一个选项。
    for (name, provider) in [
        ("openai", Provider::OpenAi),
        ("anthropic", Provider::Anthropic),
    ] {
        if let Some(key) = secret::load(name) {
            // `Box<dyn Reasoner>` 不是 `Send` 的边界——循环自己起线程，
            // 所以这里不需要 Send。`Http` 那边要 Send+Sync 是因为
            // `HttpReasoner` 内部用 `Arc` 共享 agent。
            let http = UreqHttp::default();
            let r = HttpReasoner::new(std::sync::Arc::new(http), provider, key);
            return LoopConfig {
                ignore_top: IGNORE_TOP,
                reasoner: Box::new(r),
                transport_ready: true,
                recent_n: RecentN::new(chatservant_reason::context::DEFAULT_RECENT),
            };
        }
    }

    LoopConfig {
        ignore_top: IGNORE_TOP,
        reasoner: Box::new(PresetReasoner),
        transport_ready: false,
        recent_n: RecentN::new(chatservant_reason::context::DEFAULT_RECENT),
    }
}

/// 顶部跳过多高（置顶公告条）。**由 ChatSurface 给，不该写死在引擎里**；
/// 还没有 `ChatSurface` 的配置入口，先用实测值 60。
const IGNORE_TOP: i32 = 60;

/// 预置传输。**每条候选都带自曝标记**，界面与日志里一眼看得出。
pub struct PresetReasoner;

impl chatservant_reason::Reasoner for PresetReasoner {
    fn judge(
        &mut self,
        _ctx: &TurnContext,
    ) -> Result<chatservant_reason::Intent, chatservant_reason::ReasonError> {
        Ok(chatservant_reason::Intent {
            intent: "预置判断（没配置 key，没读懂对话）".into(),
            need: String::new(),
            action: "blind".into(),
            tension: 0.0,
        })
    }

    fn draft(
        &mut self,
        _ctx: &TurnContext,
        _i: &chatservant_reason::Intent,
    ) -> Result<chatservant_reason::DraftSet, chatservant_reason::ReasonError> {
        Ok(chatservant_reason::DraftSet {
            candidates: vec![chatservant_reason::Candidate {
                text: "【预置候选，没配置 key，不是模型输出】".into(),
                probability: 1.0,
            }],
            blind: true,
            calls: 0,
        })
    }

    fn rank(
        &mut self,
        _c: &TurnContext,
        d: &chatservant_reason::DraftSet,
    ) -> Result<Vec<chatservant_reason::Candidate>, chatservant_reason::ReasonError> {
        Ok(d.candidates.clone())
    }
}
