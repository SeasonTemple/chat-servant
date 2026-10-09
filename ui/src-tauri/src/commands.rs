//! Tauri 命令层。**Rust 侧是唯一事实来源**——前端只渲染这里发过去的东西。
//!
//! 这里刻意只放**纯逻辑**与**几何探测**，不放输入注入：
//! ticket 09 的验收标准是「回读剪贴板与候选原文字节级相等」，
//! 在那之前先不接注入，免得把「以为成功了」写进产品。

use chatservant_core::dispatch::{DeliveryReceipt, Dispatch, Dispatcher};
use serde::Serialize;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};

use crate::secret::{self, KeyStatus};
use crate::watch::{DraftCandidate, LoopConfig, Readout, RecentN, TokenBudget, Watcher};
use chatservant_reason::TurnContext;

/// 全局单例：一个应用只有一个观察循环。
///
/// **Mutex 里的类型必须能在循环线程里用**——所以循环自己起了线程，
/// 这里只存 `Watcher` 句柄与**纯数据**快照。
static STATE: Mutex<Option<AppState>> = Mutex::new(None);

/// 全局的 `AppHandle`。
///
/// **为什么需要它**：`set_tier` / 观察循环这些地方只有 `&mut AppState` 或
/// 连它都没有，而「档位变了要刷托盘」这条要求跟档位是同一个触发点。
/// 往 `AppState` 里塞 `AppHandle` 会把它变成自引用；用一个 OnceLock
/// 是干净的。`AppHandle` 是 `Clone + Send + Sync`，存全局安全。
static APP: OnceLock<AppHandle> = OnceLock::new();

/// 记下 handle。由 `main.rs` 的 `setup` 在观察循环启动**之前**调。
pub fn set_app_handle(h: AppHandle) {
    let _ = APP.set(h);
}

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
    /// 图片 token 预算。同样必须与循环共用。
    token_budget: TokenBudget,
    /// 图片外发总闸。**默认关。**
    images_out: crate::watch::ImagesOut,
    /// 发送档位。**唯一事实来源**，前端只请求改。
    tier: crate::watch::TierCell,
    /// 「重新锁定当前会话」的请求。与循环共用同一个格子。
    relock: crate::watch::Relock,
    /// **Confirm 档的「已装填」状态。** 见 [`Arming`]。
    armed: Arming,
    /// 程序**锁定并正在处理**的会话名。托盘的状态行读它。
    ///
    /// ⚠ **不是「窗口里当前显示的那个」。** 那是 `locked_peer` 之外的事：
    /// 会话身份闸要求程序只对锁定会话动手，所以托盘首行要报的是
    /// 「我到底在替谁说话」。空串 = 还没认出来，那同样是不动手。
    peer: String,
}

/// Confirm 档的二次确认：**程序不许替人省这一步，但人也不能被卡在这一步。**
///
/// ## 这里曾经是一个死胡同
///
/// 早先的实现是：
///
/// ```rust
/// if tier == Dispatch::Confirm && action == "confirm" {
///     let _ = app.emit("armed", &candidate_id);
///     return DispatchReply { outcome: "NotSent", detail: "已准备好。再点一次…" };
/// }
/// ```
///
/// 第二次点击进来时，`tier` **仍然是 Confirm**、`action` **仍然是 confirm**
/// ——**同一个分支又命中**，于是永远走不到下面的 `deliver`。
///
/// 后果不是「多一步确认」，而是 **Confirm 档根本发不出去**：
/// 用户点一次回一句「再点一次」，点两次还是「再点一次」，
/// 点一百次也还是。而那句话是一个**明晃晃的假承诺**——
/// 它承诺下一次会发，下一次不会。
///
/// 2026-10-09 实机复现：点「填入并确认发送」→ 文字进了输入框、
/// 没有发出去、回执说「再点一次」。再点，无限循环。
///
/// `armed` 事件当时也没有任何前端监听者——装填状态发出去就没了。
///
/// ## 为什么状态必须在 Rust 侧
///
/// 「这一次点击是装填还是发送」是**决定一条消息会不会真的发出去**的判断。
/// 放在前端就等于把这条判断交给一个可以随手改的变量，
/// 硬约束第 5 条（升档必须人按、默认档最低）当场失效。
///
/// ## 装填什么时候失效
///
/// 三种情况都必须失效，否则「装填」会变成一张可以囤起来的许可证：
/// · 候选换了（`ticks` 变了）——上一轮的候选已经不是这一轮的候选了；
/// · 档位变了；
/// · 真的发出去了。
#[derive(Debug, Default)]
struct Arming {
    /// 已装填的那条候选的 id。
    id: Option<String>,
    /// 装填时观察循环跑到第几圈。用来判断候选是否已经换过。
    ticks: u64,
}

impl Arming {
    /// 装填。**只在同一条候选、且候选没换过时才成立。**
    ///
    /// 返回 `false` 表示「之前那次装填已经作废，这次必须重新装填」——
    /// 宁可让用户多点一次，也不要把上一轮的确认挪到这一轮的候选上。
    fn arm(&mut self, id: &str, ticks: u64) -> bool {
        if self.id.as_deref() == Some(id) && self.ticks == ticks {
            return true;
        }
        self.id = Some(id.to_string());
        self.ticks = ticks;
        false
    }

    fn disarm(&mut self) {
        self.id = None;
    }
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
                "第 {} 圈｜{}｜OCR 累计 {} 次",
                out.ticks,
                out.counts.explain(),
                out.ocr_calls
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
    /// 档位守则给出的「下一步」。`None` = 到此为止，没什么要接着做的。
    pub next_hint: Option<String>,
}

#[tauri::command]
pub fn dispatch(app: AppHandle, candidate_id: String, action: String) -> DispatchReply {
    // ⚠ **夹具模式下绝不投递。**
    //
    // 夹具模式是给「没有微信可用时改界面」用的，可它拿到的是**真微信的帧**。
    // 这时候按「确认发送」会把内容打进**一个真的对话里**——
    // 而这个人并没有要求发这条消息。
    //
    // 「我以为我在测试」不构成免责理由：不可逆动作的前提是**用户确实要它**。
    let fixture_mode = std::env::var("CHATSERVANT_FIXTURE")
        .map(|p| !p.trim().is_empty())
        .unwrap_or(false);
    if fixture_mode {
        return DispatchReply {
            outcome: "Failed",
            detail: "当前跑在**冻结的夹具帧**上（CHATSERVANT_FIXTURE），不投递。\
                     要真的发送，去掉该环境变量重启。"
                .into(),
            next_hint: None,
        };
    }

    // ⚠ **档位从 Rust 自己的格子读，不看前端传什么。**
    // 早先的签名收一个 `tier: String` 并直接采信——那等于把「这条消息
    // 会不会真的发出去」交给一个前端可以随便改的字符串。
    let tier = with_state(|s| s.tier.get())
        .ok_or_else(|| "观察循环还没起来".to_string())
        .unwrap_or(Dispatch::Manual);

    let mut d = Dispatcher::new();
    d.set_tier(tier);

    // Confirm 档：**没确认就不发送**。这条是硬约束，程序不许替人省这一步。
    //
    // ⚠⚠ 判据是「**这一条候选已经装填过、且候选没有换过**」，
    // 不是 action。早先这里写 `tier == Confirm && action == "confirm"`——
    // 两次点击进来的条件完全相同，于是第二次又命中这里，永远发不出去，
    // 而回执每次都写着「再点一次就会发出去」。见 [`Arming`]。
    let ticks = with_state(|s| s.watcher.latest())
        .flatten()
        .map(|o| o.ticks);
    if tier == Dispatch::Confirm && action == "confirm" {
        let already = {
            let Ok(mut g) = STATE.lock() else {
                return DispatchReply {
                    outcome: "Failed",
                    detail: "内部状态锁中毒".into(),
                    next_hint: None,
                };
            };
            match g.as_mut() {
                Some(s) => match ticks {
                    Some(t) => s.armed.arm(&candidate_id, t),
                    // 拿不到当前这轮（循环还没跑出快照）→ 不许装填。
                    // 宁可让他等一轮，也不要在没有基准的情况下放行发送。
                    None => false,
                },
                None => false,
            }
        };
        if !already {
            // 装填是**状态变化**，要告诉界面：按钮文案从「填入并确认发送」
            // 变成「确认发送」，样式也换掉。早先这个 emit 发出去之后
            // **没有任何前端监听者**，状态就丢了——于是装填既不生效也不可见。
            let _ = app.emit("armed", &candidate_id);
            // 第一次点击：**只填入，不发**。这一步同时是让人读、让人改的窗口。
            let filled = deliver(&candidate_id, false);
            let next = d.next(filled.clone(), 0);
            return DispatchReply {
                outcome: outcome_name(&filled),
                detail: format!(
                    "{}。已填入输入框——**看一眼、改一改**，再点一次「确认发送」才真的发出去。",
                    filled.explain()
                ),
                next_hint: Some(next.explain()),
            };
        }
        // 第二次点击：**只按回车，不再往输入框里写任何东西。**
        //
        // ⚠ 两次都填会发出去两份拼接——`fill_input_box` 的语义是
        // 「在光标处追加」，它不动用户已经打了一半的字。
        // 实测（2026-10-09）：发出去的是同一句话连着两遍。
        let Some(hwnd) = find_wechat() else {
            return DispatchReply {
                outcome: "Failed",
                detail: "找不到微信窗口（已退出？换了进程名？）".into(),
                next_hint: None,
            };
        };
        let receipt = chatservant_capture::inject::send_only(hwnd);
        if let Ok(mut g) = STATE.lock()
            && let Some(s) = g.as_mut()
        {
            // 一次装填只服务一次发送；发过就作废。
            s.armed.disarm();
        }
        let next = d.next(receipt.clone(), 0);
        return DispatchReply {
            outcome: outcome_name(&receipt),
            detail: format!(
                "档位 {}｜{}｜下一步：{}",
                tier_name(tier),
                receipt.explain(),
                next.explain()
            ),
            next_hint: Some(next.explain()),
        };
    }

    // 改档就解除装填：上一档的确认不能带到下一档。
    // ⚠ 放在这里而不是 `set_tier` 里：`set_tier` 是命令层的入口，
    // 而这里保证「**任何一次投递动作**之前，装填都是有效的」。
    if let Ok(mut g) = STATE.lock()
        && let Some(s) = g.as_mut()
    {
        s.armed.disarm();
    }

    let receipt = deliver(&candidate_id, matches!(tier, Dispatch::Auto));

    // 让档位守则跑一遍，把下一步该做什么告诉前端。
    let next = d.next(receipt.clone(), 0);
    DispatchReply {
        outcome: outcome_name(&receipt),
        detail: format!(
            "档位 {}｜{}｜下一步：{}",
            tier_name(tier),
            receipt.explain(),
            next.explain()
        ),
        next_hint: Some(next.explain()),
    }
}

/// 档位 → 界面认得的字符串。**回读用**——让用户看到程序此刻认定的档位，
/// 而不是他记得自己点过什么。
#[tauri::command]
pub fn get_tier() -> String {
    with_state(|s| s.tier.get())
        .map(tier_name)
        .unwrap_or_else(|| tier_name(Dispatch::Manual))
}

/// 改档。**返回真实生效值**——界面上要显示程序认定的那个。
#[tauri::command]
pub fn set_tier(tier: String) -> String {
    // 认不出来的一律当 Manual。**不许因为解析失败就升档。**
    let d = match tier.as_str() {
        "Auto" => Dispatch::Auto,
        "Confirm" => Dispatch::Confirm,
        _ => Dispatch::Manual,
    };
    let real = with_state(|s| s.tier.set(d))
        .map(tier_name)
        .unwrap_or_else(|| tier_name(Dispatch::Manual));
    // ⭐ 在 `with_state` **之后**刷——锁已经放掉了，这里再拿就是死锁。
    // 档位变了，托盘首行和图标状态必须跟着变：面板看不见的时候，
    // 它是唯一告诉用户「开着自动」的地方。
    refresh_tray();
    real
}

fn tier_name(d: Dispatch) -> String {
    match d {
        Dispatch::Manual => "只填入",
        Dispatch::Confirm => "确认后发",
        Dispatch::Auto => "自动发送",
    }
    .to_string()
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

/// 真正去送一次。
///
/// ## 档位在这里才第一次真正起作用
///
/// 早先的版本写的是 `let _ = (action, tier);`——**把档位整个丢掉**，
/// 只调 `fill_input_box`。于是 Auto 档和 Manual 档行为完全一样：
/// 都只填不发。而设置页写着「自动发送，填入即发送，不打断」——
/// 界面承诺的是整个产品里**最危险的那个能力**，代码却根本没实现。
///
/// 现在由调用方把「这一轮到底该不该发」算成 [`send`] 传进来：
/// Auto 恒为 true；Confirm 要先在 [`Arming`] 里成立；Manual 恒为 false。
///
/// `send` 是**显式参数**而不是再读一次 tier——「发不发」这个判断只允许
/// 有一个地方做（`dispatch`），这里再读一遍 tier 就是第二份判断，
/// 两份迟早会分叉，而且分叉的方向会是「多发」。
fn deliver(candidate_id: &str, send: bool) -> DeliveryReceipt {
    let Some(hwnd) = find_wechat() else {
        return DeliveryReceipt::Failed("找不到微信窗口（已退出？换了进程名？）".into());
    };

    // 取不到正文就**不注入**。拿占位文本去注入等于把一句假话送进
    // 别人的聊天框，那比不送严重得多。
    //
    // ⚠ 早先的报错写「正文还没接起草层（ticket 10）」——ticket 10 早做完了，
    // 起草层是接好的。真正的失败原因是**这个 id 不在当前快照里**：
    // 对方又发了新消息、readout 已经推进到新一轮，旧候选就没了。
    // 报错要指向真实原因，否则会把人引到完全错误的方向。
    let Some(text) = candidate_text(candidate_id) else {
        return DeliveryReceipt::Failed(format!(
            "候选 {candidate_id} 已不在当前这轮的候选里（对方又发了新消息，候选被换掉了）。重新等一轮。"
        ));
    };

    if send {
        chatservant_capture::inject::fill_and_send(&text, hwnd)
    } else {
        chatservant_capture::inject::fill_input_box(&text, hwnd)
    }
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

// ═══════════════════════════════════════════════════════════════
// 窗口生命周期
//
// **这个程序启动之后必须有办法停掉。**
//
// 早先这个 crate 里一条退出路径都没有：主窗口 `decorations: false`
// （没有标题栏）、`skipTaskbar: true`（不在任务栏）、`alwaysOnTop: true`，
// 而 `tray-icon` 这个 feature 开了却一次都没用上。于是 Alt+F4 不生效、
// 任务栏看不见、托盘没有图标——**一个只能靠任务管理器杀掉的程序，
// 而它还在盯着别人的聊天窗口**。
// ═══════════════════════════════════════════════════════════════

/// 「关闭时」的行为。与前端 `types.ts` 的 `closeBehavior` 镜像。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CloseBehavior {
    /// 按 × 就是退出。**默认值** —— 没有托盘时这是唯一安全的落点。
    Exit,
    /// 按 − 收进托盘，程序继续跑。托盘菜单里才有的那个「退出」。
    Tray,
}

impl Default for CloseBehavior {
    /// **默认 `Exit`，不是 `Tray`。**
    ///
    /// 新装一个替你说话的程序，用户不该一关窗口它就悄悄跑着。
    /// `Tray` 得是用户明确选过的 —— 那意味着他知道自己把它放进了后台。
    fn default() -> Self {
        CloseBehavior::Exit
    }
}

/// 关闭行为那个设置文件的**词干**（不含 `.txt`）。
///
/// ⚠ 早先它是个没人用的常量，而 `prefs_path` 里另写了一份
/// `"close_behavior.txt"`——**同一个事实两份拷贝**，改名时必然只改一处。
/// 现在 `prefs_path` 直接用它拼文件名，这里是唯一的事实来源。
const CLOSE_BEHAVIOR_FILE: &str = "close_behavior";

/// 读「关闭时」。**读不到就按默认的 `Exit`** —— 那是最不容易出事的那个。
#[tauri::command]
pub fn get_close_behavior(app: AppHandle) -> CloseBehavior {
    read_close_behavior(&app)
}

/// 写「关闭时」。前端设置页调它。
///
/// ⚠ **参数收 `String`，不收 `CloseBehavior`。**
/// Tauri 的命令参数必须实现 `CommandArg`，也就是要能 `Deserialize`。
/// `CloseBehavior` 只派生了 `Serialize`（那是**往外发**用的），
/// 直接当参数会报 `the trait bound CloseBehavior: CommandArg is not satisfied`。
///
/// 收字符串再自己解析，和 [`set_tier`] 保持同一套写法——
/// **一个记法认不出来的值一律走默认档并说明原因**，而不是静默升档。
#[tauri::command]
pub fn set_close_behavior(app: AppHandle, behavior: String) -> Result<(), String> {
    let behavior = match behavior.as_str() {
        "exit" => CloseBehavior::Exit,
        "tray" => CloseBehavior::Tray,
        other => return Err(format!("「{other}」不是合法的关闭行为（只认 exit / tray）")),
    };
    // ⭐ **先验证落点存不存在，再写。**
    // 选了「收进托盘」却没有托盘 = 一个没有落点的动作 = 用户把面板
    // 收走之后它就变成了看不见的僵尸。所以这一档必须在托盘可用时才成立。
    if behavior == CloseBehavior::Tray && !tray_available(&app) {
        return Err("托盘不可用，不能设成「收进托盘」—— 那样关掉面板它就藏起来了。".into());
    }
    let path = prefs_path(&app);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("建目录失败：{e}"))?;
    }
    std::fs::write(&path, behavior_str(behavior)).map_err(|e| format!("写设置失败：{e}"))?;
    // 托盘菜单的标题行也要跟着变（Auto 档开着会带红点）。
    refresh_tray();
    Ok(())
}

fn behavior_str(b: CloseBehavior) -> &'static str {
    match b {
        CloseBehavior::Exit => "exit",
        CloseBehavior::Tray => "tray",
    }
}

/// 设置文件放哪。与 `appDirectoriesOverride` 一起，落在应用自己的目录下。
fn prefs_path(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(format!("{CLOSE_BEHAVIOR_FILE}.txt"))
}

fn read_close_behavior(app: &AppHandle) -> CloseBehavior {
    let Ok(s) = std::fs::read_to_string(prefs_path(app)) else {
        return CloseBehavior::default();
    };
    match s.trim() {
        "tray" if tray_available(app) => CloseBehavior::Tray,
        // 「tray」但托盘没了 → 退回 Exit。**一个失效的设置比没有设置更危险**：
        // 用户以为它还在后台跑，其实它可能已经退出了。
        _ => CloseBehavior::Exit,
    }
}

/// 托盘是不是真的建起来了。
///
/// **不查「我建过托盘吗」，查「现在还能拿到托盘吗」**——托盘初始化失败
/// 时 `TrayIcon` 会是 None，而设置里可能还留着 `tray`。
fn tray_available(app: &AppHandle) -> bool {
    app.tray_by_id(TRAY_ID).is_some()
}

/// 托盘图标 id。与 `main.rs` 里建的那一个必须是同一个。
pub const TRAY_ID: &str = "cs-tray";

/// 收进托盘。**不是退出** —— 观察循环继续跑。
///
/// 只有「关闭时 = 收进托盘」时前端才会调它，所以这个命令存在
/// 就意味着用户已经选过托盘那一档。
#[tauri::command]
pub fn hide_panel(app: AppHandle) -> Result<(), String> {
    let Some(w) = app.get_webview_window("main") else {
        return Err("主窗口不存在".into());
    };
    w.hide().map_err(|e| format!("隐藏失败：{e}"))
}

/// 显示主面板（托盘菜单 →「显示面板」）。
#[tauri::command]
pub fn show_panel(app: AppHandle) -> Result<(), String> {
    let Some(w) = app.get_webview_window("main") else {
        return Err("主窗口不存在".into());
    };
    w.show().map_err(|e| format!("显示失败：{e}"))?;
    w.unminimize().map_err(|e| format!("取消最小化失败：{e}"))?;
    w.set_focus().map_err(|e| format!("聚焦失败：{e}"))
}

/// **退出。**
///
/// 菜单里点「退出」和面板上点 × 都走这里，**共用同一条路径**——
/// 两处能写错的地方越少越好。
///
/// ⚠ 不做二次确认：Auto 档开着时退出可能留下半句没发出去。拦它的
/// 地方在**别处** —— 顶边写着当前档位，托盘图标在 Auto 时带红点，
/// 菜单首行写着「小周 · 自动发送」。用户是看得见的，不该在点下去
/// 的那一瞬间才拦住他。
#[tauri::command]
pub fn quit_app(app: AppHandle) -> Result<(), String> {
    app.exit(0);
    Ok(())
}

/// 托盘图标跟着档位变。Auto 档开着时带红点 —— 面板看不见的时候，
/// 这是唯一告诉用户「它还在，而且开着自动」的地方。
pub fn refresh_tray() {
    let Some(app) = APP.get() else {
        // 观察循环有可能在 `set_app_handle` 之前起？不该，但真发生了
        // 也只是托盘少刷一次，**不能 panic**。
        tracing::debug!("还没有 AppHandle，托盘这次刷不了");
        return;
    };
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let title = tray_status_line();
    let _ = tray.set_tooltip(Some(&title));
    // 菜单首行就是状态行。**只有这一处** —— 面板和菜单不各写一遍。
    if let Ok(menu) = build_tray_menu(app) {
        let _ = tray.set_menu(Some(menu));
    }
}

/// 托盘状态行：「谁 · 什么档位」。
///
/// ⚠ 报的是**程序锁定并正在处理的**那个会话，不是窗口里当前显示的
/// 那个 —— 会话身份闸要求只对锁定会话动手，所以这里要回答的是
/// 「我到底在替谁说话」。
pub fn tray_status_line() -> String {
    let peer = current_peer();
    let who = if peer.is_empty() {
        "没认出会话".to_string()
    } else {
        peer
    };
    format!("{who} · {}", current_tier_cn())
}

/// 托盘菜单。**构造在这里，不在 `main.rs`** —— 它要读档位和会话名，
/// 那是这个模块的事；`main.rs` 只负责把图标建出来。
///
/// ## 为什么不用「build 失败就退化成一条分隔线」
///
/// 早先这里给每个 item 写了 `.unwrap_or_else(|_| sep())`，意思是
/// 「这一项建不出来就拿分隔线顶上」。那是**把失败藏起来**：
/// 菜单里会安静地少一项，而界面上没有任何地方说明少了什么。
///
/// 分隔线本来就该用 `.separator()`——它已经在链上了，不需要兜底。
/// 真要失败，让它**响**地失败（`?`），菜单建不出来时上层会看到原因。
pub fn build_tray_menu(app: &AppHandle) -> Result<tauri::menu::Menu<tauri::Wry>, tauri::Error> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    let mb = MenuBuilder::new(app);
    // `MenuItemBuilder::build(&M)` 与 `MenuBuilder::build(&M)` **都要传 manager**。
    // 这是 Tauri 2 的签名，云端那版两处都写成了零参。
    let item = |id: &str, text: &str, enabled: bool| {
        MenuItemBuilder::with_id(id, text)
            .enabled(enabled)
            .build(app)
    };
    // ⚠ 末尾**不用** `Ok(...?)`：`MenuBuilder::build()` 返回的就是
    // `Result<Menu<R>, tauri::Error>`，与本函数的返回类型一致，
    // 多包一层 clippy 会报 "enclosing Ok and ? operator are unneeded"。
    //
    // 另注：这里零参、而上面的 `MenuItemBuilder::build(app)` 要传 manager——
    // 两个同名方法在同一段代码里，签名还不一样。
    mb
        // 首行是**状态，不可点**。面板收起来之后，这是唯一告诉用户
        // 「它还在、而且开着自动」的地方。
        .item(&item("status", &tray_status_line(), false)?)
        .separator()
        .item(&item("show", "显示面板", true)?)
        .item(&item("relock", "锁定当前会话", true)?)
        .item(&item("settings", "打开设置", true)?)
        .item(&item("debug", "识别画布", true)?)
        .separator()
        .item(
            &MenuItemBuilder::with_id("quit", "退出")
                .accelerator("CmdOrCtrl+Q")
                .build(app)?,
        )
        // 零参的是 `MenuBuilder`，**要传参的是 `MenuItemBuilder`**。
        .build()
}

/// 当前档位的中文名。
///
/// ⚠ **复用 `get_tier` 那条路径**（`with_state` + `tier_name`），不自己
/// 再解析一遍。读锁拿不到就按最低档 `Manual` —— **绝不能猜一个更宽松的值**。
fn current_tier_cn() -> String {
    with_state(|s| s.tier.get())
        .map(tier_name)
        .unwrap_or_else(|| tier_name(Dispatch::Manual))
}

/// 程序锁定并正在处理的会话名。空 = 没认出来。
fn current_peer() -> String {
    with_state(|s| s.peer.clone()).unwrap_or_default()
}

/// 当前支持的模型供应商。
///
/// 换供应商只需在这里加一项，**不用重配整个应用**——
/// 设置页按这个列表渲染，每个供应商的 key 独立存在凭据管理器里。
pub const PROVIDERS: &[&str] = &["openai", "anthropic", "minimax"];

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

/// 「单张图片的视觉 token 预算」。
///
/// ## 为什么滑块默认不出现
///
/// 界面上的这个滑块**只在 `visible` 为真时渲染**。`visible` 的含义是
/// 「这一轮真的可能有图会发出去」——模型 B 是多模态的，或者视觉桥开着。
///
/// 默认配置（B 多模态、桥关闭）下**没有一张图会外发**，此时摆一个滑块
/// 就是那个熟悉的坏东西：**调了没反应的控制项**。用户在 ticket 11 里
/// 刚为此付过代价（`recent_n` 的假即时生效），同一个错误不犯第二次。
#[derive(Serialize)]
pub struct BudgetView {
    /// 当前生效值（已夹取）。
    pub budget: u32,
    pub min: u32,
    pub max: u32,
    /// 滑块**该不该出现**。前端照这个渲染，不自己判断。
    pub visible: bool,
    /// 供应商的固定底账。预算低于它就**无解**，界面要如实说。
    pub provider_base: u32,
    /// 当前供应商名。
    pub provider: String,
}

/// 预算默认值。取标准档上限 1568 的约 3/4，够看清一张聊天图。
pub const BUDGET_DEFAULT: u32 = 1200;

fn budget_view(budget: u32, provider: &chatservant_reason::Provider) -> BudgetView {
    use chatservant_reason::Provider;
    // 底账是**随模型变**的：gpt-4o-mini 是 2833，Claude 的 patch 计费是 0。
    // 这里如实反映当前 provider，界面据此显示「预算不够」的警告。
    let provider_base = match provider {
        Provider::Anthropic => 0,
        Provider::OpenAi => 2833,
        // ⚠ **MiniMax 的视觉 token 计费没有实测过。**
        // 拿不到权威数字时**不许编一个**——编出来的预算会让用户以为
        // 花了那个数，实际账单却是别的。这里如实报 0 并标「未核实」，
        // 让界面把图片外发说成「成本未知」而不是给个假精确的数。
        Provider::MiniMax => 0,
    };
    BudgetView {
        budget: budget.min(budget_ceiling(provider)),
        min: crate::watch::BUDGET_MIN,
        // **上界按供应商给。** 早先写死 1568（Anthropic 的上限），
        // 于是 gpt-4o-mini 上滑块永远到不了它的底账 2833，
        // 图片功能在默认模型上完全不可用而界面看不出来。
        max: budget_ceiling(provider),
        // ⚠ **这里不能写死 true。**
        // 早先的版本恒返回 true，理由是「模型 B 多模态，图随文本进 B」——
        // 但图片链路**根本没接进观察循环**，一张图也出不去。
        // 那正是同一段注释里批评的「调了没反应的控制项」，自己却犯了一遍。
        // 现在它读总闸：闸关着就不显示滑块。
        visible: with_state(|s| s.images_out.get()).unwrap_or(false),
        provider_base,
        provider: provider.key().to_string(),
    }
}

/// 读图片外发总闸。
#[tauri::command]
pub fn get_images_out() -> Result<bool, String> {
    with_state(|s| s.images_out.get()).ok_or_else(|| "观察循环还没起来，读不到设置".to_string())
}

/// 改图片外发总闸。
///
/// ⚠ **这是个危险开关**：打开之后，对方发来的图片区域会被裁出、缩放、
/// base64 编码，然后发给第三方模型。媒体检测的准确率尚未在真实帧上
/// 完整验证，所以这里**只允许显式开启**，没有「自动」档。
#[tauri::command]
pub fn set_images_out(on: bool) -> Result<bool, String> {
    with_state(|s| s.images_out.set(on)).ok_or_else(|| "观察循环还没起来，改不了设置".to_string())
}

/// 当前供应商能接受的**单张上限**。
///
/// - Anthropic：标准档 token 上限 **1568**（官方文档）。
/// - OpenAI `gpt-4o-mini`：底账 2833 + 一个 512px tile 的 5667 = **8500**。
///   它比 Anthropic 贵 5.4 倍，因为它的底账与图的大小无关。
fn budget_ceiling(provider: &chatservant_reason::Provider) -> u32 {
    use chatservant_reason::Provider;
    match provider {
        Provider::Anthropic => 1568,
        Provider::OpenAi => 2833 + 5667,
        // MiniMax 的视觉计费未核实（见上面 `budget_view` 的说明），
        // 所以给 Anthropic 同级的保守上界，**并让界面标「成本未知」**。
        Provider::MiniMax => 1568,
    }
}

/// 「把当前这个会话锁成要处理的那个」。
///
/// 指纹闸在**第一帧读到头部**时建锁，所以启动时开着的会话就是被锁的。
/// 用户在错的会话上启动了应用之后，这条命令是**唯一的修正手段**——
/// 没有它就只能关掉重开，那不是设计，是放弃。
#[tauri::command]
pub fn relock_session() -> Result<(), String> {
    with_state(|s| s.relock.request()).ok_or_else(|| "观察循环还没起来".to_string())
}

/// 读预算。
#[tauri::command]
pub fn get_token_budget() -> Result<BudgetView, String> {
    let budget = with_state(|s| s.token_budget.get())
        .ok_or_else(|| "观察循环还没起来，读不到设置".to_string())?;
    Ok(budget_view(budget, &active_provider()))
}

#[tauri::command]
pub fn set_token_budget(budget: u32) -> Result<BudgetView, String> {
    let applied = with_state(|s| s.token_budget.set(budget))
        .ok_or_else(|| "观察循环还没起来，改不了设置".to_string())?;
    Ok(budget_view(applied, &active_provider()))
}

/// 当前生效的供应商。**没配 key 时退回 OpenAI**——
/// 纯展示用的，不参与任何路由决策。
fn active_provider() -> chatservant_reason::Provider {
    if secret::load("anthropic").is_some() {
        chatservant_reason::Provider::Anthropic
    } else {
        chatservant_reason::Provider::OpenAi
    }
}

#[tauri::command]
pub fn forget_key(provider: String) -> Result<(), String> {
    if !PROVIDERS.contains(&provider.as_str()) {
        return Err(format!("未知的供应商 {provider}"));
    }
    secret::erase(&provider)
}

/// 测试连通性。**只回「通不通」**，不回任何响应体——
/// 上游的错误正文可能带 key。
#[tauri::command]
pub fn probe_key(provider: String) -> Result<bool, String> {
    use chatservant_reason::{Provider, UreqHttp, probe_connectivity};

    let p = match provider.as_str() {
        "openai" => Provider::OpenAi,
        "anthropic" => Provider::Anthropic,
        "minimax" => Provider::MiniMax,
        // 认不出来的一律拒绝，**不静默接受**。
        _ => return Err(format!("未知的供应商 {provider}")),
    };
    let Some(key) = secret::load(&provider) else {
        return Err(secret::missing_hint(&provider));
    };
    // 真的发一次请求。早先这里恒返回 false，注释说「网络层 ticket 10 接上」——
    // 但网络层早就是真的了，于是这个按钮**永远说不通**。
    // 恒假的指示器比没有更糟：用户会以为 key 坏了，反复重填。
    let http = UreqHttp::new(std::time::Duration::from_secs(10));
    match probe_connectivity(&http, p, &key, std::time::Duration::from_secs(10)) {
        Ok(()) => Ok(true),
        // 失败原因已经过 `redact`，可以安全显示。
        Err(e) => Err(e),
    }
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
    let token_budget = cfg.token_budget.clone();
    let images_out = cfg.images_out.clone();
    let tier = cfg.tier.clone();
    let relock = cfg.relock.clone();

    let watcher = crate::watch::spawn(cfg);
    if let Ok(mut g) = STATE.lock() {
        *g = Some(AppState {
            watcher,
            candidates: Vec::new(),
            recent_n,
            token_budget,
            images_out,
            tier,
            relock,
            armed: Arming::default(),
            peer: String::new(),
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
                    // ⭐ **先把锁释放，再刷托盘。**
                    // `refresh_tray` 要读 `STATE` 拿档位和会话名，
                    // 在锁里调它就是自己等自己 —— 直接死锁。
                    let mut peer_changed = false;
                    if let Ok(mut g) = STATE.lock()
                        && let Some(s) = g.as_mut()
                    {
                        s.candidates = out.candidates.clone();
                        if s.peer != out.peer {
                            s.peer = out.peer.clone();
                            peer_changed = true;
                        }
                    }
                    if peer_changed {
                        // 托盘首行写的是「我到底在替谁说话」，
                        // 会话换了就得跟着换。
                        //
                        // ⚠ 直接调 `refresh_tray()`，**不要写 `commands::`**——
                        // 这段代码就在 `commands` 模块里，`commands::`
                        // 在自己模块内不是一个可解析的路径。
                        refresh_tray();
                    }
                    // ⚠ **不要吞掉 emit 的错误。**
                    //
                    // 早先这里写的是 `let _ = app2.emit(...)`，于是序列化失败
                    // （字段里有 serde 编不出来的东西）时：日志照打「emit
                    // readout tick=N」、Rust 侧一切正常，而界面**一个字都收不到**，
                    // `latest_readout` 也走同一条序列化路径所以同样失败——
                    // 表现就是「后端认出会话了，界面永远停在空白」，
                    // 而日志里看不出任何异常。这正是**恒哑的错误处理**。
                    if let Err(e) = app2.emit("readout", &out) {
                        tracing::error!(
                            target: "chat_servant::pump",
                            "readout 推不到界面：{e}（tick={}）",
                            out.ticks
                        );
                    }
                    tracing::info!(
                        target: "chat_servant::pump",
                        "emit readout tick={} peer={:?} messages={} candidates={}",
                        out.ticks,
                        out.peer,
                        out.messages.len(),
                        out.candidates.len()
                    );
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

/// 要锁定的会话名。**空串 = 不锁**——那时程序宁可不做，也不猜。
///
/// 锁定的意义是「只对这个会话动手」：窗口里是别的聊天时不推理也不投递。
/// OCR 认错名字的概率不为零（`reason::peer` 记着两个字的中文名无法
/// 归并），所以**认不出名字和名字对不上，都按「不是目标会话」处理**，
/// 不去猜「大概是它」。
const LOCKED_PEER: &str = "小号";

/// 组装循环配置。**key 从凭据管理器读**，读不到就用预置传输。
fn build_loop_config() -> LoopConfig {
    use chatservant_reason::{HttpReasoner, Provider, UreqHttp};

    // 谁有 key 就用谁。**默认先试 OpenAI**——设置页上它是第一个选项。
    for (name, provider) in [
        ("openai", Provider::OpenAi),
        ("anthropic", Provider::Anthropic),
        ("minimax", Provider::MiniMax),
    ] {
        if let Some(key) = secret::load(name) {
            // `Box<dyn Reasoner>` 不是 `Send` 的边界——循环自己起线程，
            // 所以这里不需要 Send。`Http` 那边要 Send+Sync 是因为
            // `HttpReasoner` 内部用 `Arc` 共享 agent。
            let http = UreqHttp::default();
            let r = HttpReasoner::new(std::sync::Arc::new(http), provider, key);
            return LoopConfig {
                source: crate::watch::FrameSource::from_env(),
                ignore_top: IGNORE_TOP,
                reasoner: Box::new(r),
                transport_ready: true,
                recent_n: RecentN::new(chatservant_reason::context::DEFAULT_RECENT),
                locked_peer: LOCKED_PEER.to_string(),
                token_budget: TokenBudget::new(BUDGET_DEFAULT),
                images_out: crate::watch::ImagesOut::off(),
                tier: crate::watch::TierCell::default_manual(),
                relock: crate::watch::Relock::new(),
                provider,
            };
        }
    }

    LoopConfig {
        source: crate::watch::FrameSource::from_env(),
        ignore_top: IGNORE_TOP,
        reasoner: Box::new(PresetReasoner),
        transport_ready: false,
        recent_n: RecentN::new(chatservant_reason::context::DEFAULT_RECENT),
        locked_peer: LOCKED_PEER.to_string(),
        token_budget: TokenBudget::new(BUDGET_DEFAULT),
        images_out: crate::watch::ImagesOut::off(),
        tier: crate::watch::TierCell::default_manual(),
        relock: crate::watch::Relock::new(),
        provider: chatservant_reason::Provider::OpenAi,
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
        _ctx: &TurnContext,
        d: &chatservant_reason::DraftSet,
    ) -> Result<Vec<chatservant_reason::Candidate>, chatservant_reason::ReasonError> {
        Ok(d.candidates.clone())
    }
}

#[cfg(test)]
mod budget_ceiling_tests {
    use super::*;
    use chatservant_reason::Provider;

    /// **上界必须按供应商不同。**
    ///
    /// 早先把 1568（Anthropic 的上限）当成通用值，于是 gpt-4o-mini 上
    /// 滑块永远到不了 2833 的底账——图片功能在**默认模型**上完全不可用，
    /// 而界面上看不出任何异常。
    #[test]
    fn the_ceiling_differs_per_provider() {
        assert_eq!(budget_ceiling(&Provider::Anthropic), 1568);
        assert_eq!(budget_ceiling(&Provider::OpenAi), 8500);
        assert!(
            budget_ceiling(&Provider::OpenAi) > budget_ceiling(&Provider::Anthropic),
            "gpt-4o-mini 的底账让它必须给出更高的上界"
        );
    }

    /// **上界必须 ≥ 该供应商的底账。**
    ///
    /// 上界比底账还低的话，那张模型**永远发不出任何图**，
    /// 而用户能拖到的最大值就在底账之下——这个组合必须被挡住。
    #[test]
    fn the_ceiling_is_always_at_least_the_provider_base() {
        for (p, base) in [(Provider::Anthropic, 0u32), (Provider::OpenAi, 2833)] {
            assert!(
                budget_ceiling(&p) >= base,
                "{:?}: 上界 {} 低于底账 {base}，图永远发不出去",
                p,
                budget_ceiling(&p)
            );
        }
    }

    /// 夹取后显示的值必须落在滑块的可达范围内。
    #[test]
    fn the_reported_budget_fits_inside_the_slider_range() {
        let v = budget_view(20_000, &Provider::OpenAi);
        assert_eq!(v.max, 8500);
        assert!(v.budget <= v.max && v.budget >= v.min, "超出滑块范围");
    }

    /// **Confirm 档的二次确认必须真的能走完。**
    ///
    /// 2026-10-09 实机事故：判据写成 `tier == Confirm && action == "confirm"`，
    /// 两次点击条件完全相同，于是第二次又命中「装填」分支，
    /// **Confirm 档永远发不出去**，而回执每次都写着「再点一次就会发」。
    /// 界面上那是一个明晃晃的假承诺。
    ///
    /// 这条测试按用户的手来走：点一下（装填）→ 再点一下（发送）。
    #[test]
    fn confirm_arm_needs_two_clicks_and_then_releases() {
        let mut a = Arming::default();
        assert!(!a.arm("c0", 10), "第一次点击只能装填，不能放行发送");
        assert!(a.arm("c0", 10), "第二次点击（候选没变）应当放行");
        a.disarm();
        assert!(
            !a.arm("c0", 10),
            "发过一次之后要重新装填——一次装填只服务一次发送"
        );
    }

    /// **装填不能挪到另一条候选上。**
    ///
    /// 不然就是「我确认了 A，结果发出去的是 B」。
    #[test]
    fn an_arming_never_moves_to_another_candidate() {
        let mut a = Arming::default();
        assert!(!a.arm("c0", 10));
        assert!(!a.arm("c1", 10), "换了一条候选就必须重新装填");
        assert!(a.arm("c1", 10), "c1 自己装填后再点一次才放行");
    }

    /// **候选换过之后，上一次的装填立刻作废。**
    ///
    /// `ticks` 前进意味着观察循环出了新一轮 readout，候选集合已经不是
    /// 装填时那一批了。拿旧确认去放行新的候选，是把一次确认扩大成两次。
    #[test]
    fn an_arming_dies_when_the_readout_advances() {
        let mut a = Arming::default();
        assert!(!a.arm("c0", 10));
        assert!(!a.arm("c0", 11), "readout 前进后旧装填必须作废");
        assert!(a.arm("c0", 11), "在新一轮里重新装填后，第二次点击放行");
    }
}
