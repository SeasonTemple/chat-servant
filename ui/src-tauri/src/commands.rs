//! Tauri 命令层。**Rust 侧是唯一事实来源**——前端只渲染这里发过去的东西。
//!
//! 这里刻意只放**纯逻辑**与**几何探测**，不放输入注入：
//! ticket 09 的验收标准是「回读剪贴板与候选原文字节级相等」，
//! 在那之前先不接注入，免得把「以为成功了」写进产品。

use chatservant_core::dispatch::{DeliveryReceipt, Dispatch, Dispatcher, Next};
// `Deserialize` 不只是给某个命令用的：`CloseBehavior` 是 `#[tauri::command]`
// 的**入参**，Tauri 要求命令参数类型实现 `CommandArg`，
// 而它的 blanket impl 建立在 `Deserialize` 上。
// 只 derive `Serialize` 的话编译报 E0277（2026-10-10 本机实测）。
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};

use crate::contract::Action;
use crate::secret::{self, KeyStatus};
use crate::watch::{DraftCandidate, LoopConfig, Readout, RecentN, TokenBudget, Watcher};
use chatservant_reason::{ModelGroup, Protocol, Provider, Route, TurnContext};

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
    /// 「用哪个模型」。**同样必须与推理器共用同一格**——
    /// 新造一个的话设置页会显示「已切到 X」，而出网时发的还是旧模型。
    ///
    /// **两组**（`None` = 还没有真实传输）。为什么要两组见 [`ModelGroup`]：
    /// 「决策走 A、文字走 B」在只有一格模型名时表达不了。
    models: Option<ModelGroups>,
    /// 出站代理。**网络层的属性，两组共用**——所以界面上是单独一栏。
    proxy: chatservant_reason::ProxyCell,
    /// 发送档位。**唯一事实来源**，前端只请求改。
    tier: crate::watch::TierCell,
    /// 「重新锁定当前会话」的请求。与循环共用同一个格子。
    relock: crate::watch::Relock,
    /// **防重状态：「这条候选本轮是不是已经发过了」。** 见 [`Arming`]。
    armed: Arming,
    /// 程序**锁定并正在处理**的会话名。托盘的状态行读它。
    ///
    /// ⚠ **不是「窗口里当前显示的那个」。** 那是 `locked_peer` 之外的事：
    /// 会话身份闸要求程序只对锁定会话动手，所以托盘首行要报的是
    /// 「我到底在替谁说话」。空串 = 还没认出来，那同样是不动手。
    peer: String,
}

/// 防重状态：**这条候选本轮是不是已经发过了。**
///
/// ## 它现在防的是什么（2026-10-15 改了语义）
///
/// 早先它是「**已装填**」——`Confirm` 档第一次点击后记一笔，
/// 等人第二次点击才发。砍掉 `Confirm` 之后**没有第二次点击了**，
/// 那个语义随之消失。
///
/// 现在它只挡一件事：**同一条候选在同一轮里被发两遍**。
/// 点一下就发的情况下，连点两下就会往对方的聊天框里塞两条一样的消息，
/// 而发出去的撤不回来。
///
/// ## 这里曾经是一个死胡同
///
/// `Confirm` 档早先的实现是：
///
/// ```text
/// if tier == Dispatch::Confirm && action == "confirm" {
///     let _ = app.emit(ARMED, &candidate_id);
///     return DispatchReply { outcome: "NotSent", detail: "已准备好。再点一次…" };
/// }
/// ```
///
/// 第二次点击进来时，`tier` **仍然是 Confirm**、`action` **仍然是 confirm**
/// ——**同一个分支又命中**，于是永远走不到下面的 `deliver`。
///
/// 后果不是「多一步确认」，而是 **`Confirm` 档根本发不出去**：
/// 用户点一次回一句「再点一次」，点两次还是「再点一次」，
/// 点一百次也还是。而那句话是一个**明晃晃的假承诺**——
/// 它承诺下一次会发，下一次不会。
///
/// 2026-10-09 实机复现：点「填入并确认发送」→ 文字进了输入框、
/// 没有发出去、回执说「再点一次」。再点，无限循环。
///
/// ## 为什么状态必须在 Rust 侧
///
/// 「这一条能不能发出去」是**决定一条消息会不会真的发出去**的判断。
/// 放在前端就等于把这条判断交给一个可以随手改的变量，
/// 硬约束第 5 条（升档必须人按、默认档最低）当场失效。
///
/// ## 防重什么时候失效
///
/// 两种情况都会清空整份记录，否则它会变成一张可以囤起来的许可证：
/// · 换轮了（`ticks` 变了）——上一轮的候选已经不是这一轮的候选了；
/// · 档位变了。
///
/// ⚠⚠ **它只挡「同一条被重复发」，不表示「已装填」**（2026-10-15）。
/// 砍掉两段式之后生产里只有一个调用点（`dispatch` 的 `Auto` 分支），
/// 用法是「这一轮这条候选发过了没有？」
/// ⚠ 文档跟着实现走之前，这里曾经写着「装填。返回 `false` 表示之前那次装填
///   已经作废」——而 `arm` 返回 `true` 的含义是**这条已经发过了、拒绝重发**。
///   一个把「拒绝」写成「放行」的注释，会让下一个人以为重发是被鼓励的。
///
/// ## 为什么记的是「一整个列表」而不是「最近那一条」
///
/// 原来只有一个 `id` 槽，于是：发 c0 → 记 c0；发 c1 → 记 c1 覆盖掉 c0；
/// 这时再点 c0，槽里存的是 c1 ⇒ **c0 被当成没发过，再发一遍**。
/// 「一条消息被重复发出去」撤不回来，而触发它只需要连点三下——
/// 所以槽改成**本轮发过的全部 id**。
#[derive(Debug, Default)]
struct Arming {
    /// **本轮已经发过**的候选 id。顺序不表意，只表「这一轮发过哪些」。
    sent: Vec<String>,
    /// `sent` 属于哪一轮。用来判断记录是不是上一轮留下来的。
    ticks: u64,
}

impl Arming {
    /// 记一次发送，并回答「这条是不是本轮已经发过了」。
    ///
    /// **返回 `true` = 已经发过 ⇒ 拒绝重发。** 返回 `false` = 没发过，
    /// 调用方可以发（并且这一次发送被记下来了）。
    ///
    /// ⚠ **它在发送被尝试之前就记下了。** 所以像「窗口句柄拿不到」这种
    ///   发送根本没起步的失败，也会把这一轮的位置占掉 ⇒ 用户得等下一轮。
    ///   这是故意的：占错了只是多点一次，放过了就是重复发一条真消息。
    ///
    /// ⚠ 「宁可让用户多点一次，也不要把上一轮的记录挪到这一轮的候选上」
    /// ——这条仍然成立，只是主语从「确认」换成了「已发」。
    fn arm(&mut self, id: &str, ticks: u64) -> bool {
        if self.ticks != ticks {
            self.sent.clear();
            self.ticks = ticks;
        }
        if self.sent.iter().any(|s| s == id) {
            return true;
        }
        self.sent.push(id.to_string());
        false
    }

    fn disarm(&mut self) {
        self.sent.clear();
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
    if let Some(e) = out.error() {
        return AnchorProbe {
            ok: false,
            anchor: String::new(),
            detail: e.to_string(),
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
pub fn dispatch(candidate_id: String, action: String) -> DispatchReply {
    // ⚠ **这里曾经收一个 `app: AppHandle`，只为 `app.emit(ARMED, …)`。**
    //   2026-10-15 砍掉两段式之后那次 emit 就没有存在理由了（`armed`
    //   的语义从「已装填」变成「本轮已发过」，而前端至今没有监听者——
    //   见 `contract.rs` 的 `KNOWN_UNLISTENED`）。留着它就是留一个
    //   只为不存在的功能存在的参数，`-D warnings` 会先把它喊出来。
    //
    // ⚠ **动作先解析，再谈别的。**（票 26）
    //
    // `action` 从 IPC 过来是 `String`，任何字符串都能到这一行。以前这里是
    // `action == "confirm"` 的字符串比较——拼错、传空、传中文，全都悄悄
    // 落进最底下那条「只填入」的分支：**界面上写着一个看不懂的动作，
    // 程序却替你按了最温和的那一下。** 现在它在进业务逻辑之前就失败，
    // 而且失败的话是能直接给人看的。
    let act = match Action::parse(&action) {
        Ok(a) => a,
        Err(why) => {
            return DispatchReply {
                outcome: "Failed",
                detail: why,
                next_hint: None,
            };
        }
    };

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

    // ⚠ `Auto` 档：**点一下就填入并发送，没有第二次确认**（2026-10-15 决定，
    // 砍掉的是 `Confirm`）。所以这里**没有装填分支**；`Arming` 在 `Auto` 档上
    // 不再表示「人已装填过一次，等他再点」，而只挡「同一条被重复发」。
    let ticks = with_state(|s| s.watcher.latest())
        .flatten()
        .map(|o| o.ticks);
    if tier == Dispatch::Auto && act == Action::Confirm {
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
                    // 拿不到当前这轮（循环还没跑出快照）→ 不发。
                    // 宁可让他等一轮，也不要在没有基准的情况下放行发送。
                    None => false,
                },
                None => false,
            }
        };
        if already {
            // ⚠ 防重：同一条候选、同一轮里已经发过一次 ⇒ 不许再发一遍。
            // 重复发送撤不回来，所以这里直接拒绝而不是「再发一次看看」。
            return DispatchReply {
                outcome: "NotSent",
                detail: "这条候选本轮已经发送过了，没有重复发。".into(),
                next_hint: None,
            };
        }
        // ⚠⚠ **这是整个代码库里唯一会把消息真的发出去的地方。**
        // `sends_for_real` 把条件穷举成一句话；`debug_assert` 让
        // 「哪天有人改了上面的分支条件」当场炸，而不是等发错一条消息。
        debug_assert!(
            sends_for_real(tier, act),
            "走到自动发送分支却不是「Auto 档 + confirm 动作」"
        );
        let Some(hwnd) = find_wechat() else {
            return DispatchReply {
                outcome: "Failed",
                detail: "找不到微信窗口（已退出？换了进程名？）".into(),
                next_hint: None,
            };
        };
        // ⚠ 填入 + 发送**一步做完**。
        //
        // 这里必须用 `deliver_send`（填了再按回车），**不能**用老的
        // `send_only`（只按回车、不写字）——那个路径成立的前提是
        // 「上一步已经装填过了」，而 `Auto` 档**没有上一步**。
        // 用错会得到「回执说发成功了，输入框里其实什么都没有」。
        let receipt = deliver_send(hwnd, &candidate_id);
        if let Ok(mut g) = STATE.lock()
            && let Some(s) = g.as_mut()
        {
            // 这一轮的防重记录已经用掉了，换候选要重新 arm。
            s.armed.disarm();
        }
        let next = d.next(receipt.clone(), 0);
        return DispatchReply {
            outcome: outcome_name(&receipt),
            detail: receipt_detail(tier, &receipt, &next),
            next_hint: Some(next.explain()),
        };
    }

    // 改档就清掉防重记录：上一档的记录不能带到下一档。
    // ⚠ 放在这里而不是 `set_tier` 里：`set_tier` 是命令层的入口，
    // 而这里保证「**任何一次投递动作**之前，防重状态都是干净的」。
    if let Ok(mut g) = STATE.lock()
        && let Some(s) = g.as_mut()
    {
        s.armed.disarm();
    }

    // ⚠ 走到这里只有一种可能：**tier == Manual**。
    //
    // `Auto` 在上面那个分支里已经 return 了（填入并发送）。
    // 所以这一行的 `send` 恒为 `false`，而且**恒为 false 是对的**。
    //
    // ⚠⚠ 别把它「简化」成 `deliver(&candidate_id)` 或加个可变参数——
    // 这个 `false` 是硬约束第 5 条的落点之一。两档之下，
    // **整个代码库里按下回车的地方只剩一处**：`inject::fill_and_send`
    // 末尾的 `press_enter`，而它**只被 `deliver_send` 调**——
    // 也就是只被上面那个 `Auto` 分支调。少一个条件就多一条
    // 「不问就发出去」的路。
    //
    // ⚠ 2026-10-15 之前这里写的是 `inject::send_only`。那个函数已经删了，
    // 因为砍掉两段式之后它失去了存在理由：它的全部价值是「上一步装填过了，
    // 这一步只补一个回车」，而 `Auto` 档没有上一步。它只剩注释不改，
    // 下一个读的人会以为它是当前路径。
    let receipt = deliver(&candidate_id, false);

    // 让档位守则跑一遍，把下一步该做什么告诉前端。
    let next = d.next(receipt.clone(), 0);
    DispatchReply {
        outcome: outcome_name(&receipt),
        detail: receipt_detail(tier, &receipt, &next),
        next_hint: Some(next.explain()),
    }
}

/// 档位 → **枚举 id**（`"Manual"` / `"Auto"`）。**IPC 契约用这个。**
///
/// ⚠ 这里和 `tier_name` 是**两回事**，2026-10-10 实测踩过：
/// 早先 `get_tier` / `set_tier` 直接返回 `tier_name`（中文「仅填入」），
/// 而前端 `types.ts` 的契约是 `type Tier = 'Manual' | 'Auto'`。
/// 于是 `TIER_NAME[tier]` 是 `undefined` → 面板头部档位名**整格空白**，
/// `SegSlider` 拿中文去匹配 `data-val` 也**一个都匹配不上**，
/// 滑块因此不定位。**id 归 Rust 定、显示名归前端 `TIER_NAME` 与托盘**——
/// 中文名一改字，前端要跟着改代码才能跟上。
#[tauri::command]
pub fn get_tier() -> String {
    tier_id(with_state(|s| s.tier.get()).unwrap_or(Dispatch::Manual))
}

/// 改档，并按「档位真的变了」清空防重记录。**返回真实生效的 id。**
///
/// ## 这条规则原先只活在注释里（2026-10-14 实测）
///
/// `Arming` 的文档写着「换轮了、**档位变了**，都要清空」，`dispatch` 里
/// 那句注释又写着「改档就清记录」——**两处都声称有，代码里一处都没有**：
/// `set_tier` 从头到尾没碰过 `armed`。
///
/// 当时的漏网之鱼长这样（`Confirm` 档，2026-10-15 已砍）：Auto 档点候选 A
/// （只装填，不发）→ 切到 Manual → 在微信里改成自己想发的内容 →
/// 切回 Auto → 再点 A，`arm("A", 同一 ticks)` 返回 true，
/// **两段式被跳过，直接发送**，用户写的内容被覆盖。测试见
/// `changing_tier_disarms`。
///
/// ## 方向：只会更严，绝不放松
///
/// 这条只让**重复发送更难发生**，不打开任何新路径。⚠ 硬约束第 5 条的
/// 档位集合没动，动的只是「防重记录在什么时候作废」。
///
/// ## 为什么抽成独立函数
///
/// 它住在 `set_tier` 的行内时**没法测**——`AppState` 得靠
/// `watch::spawn()` 才造得出来（那会真的起一条观察循环线程）。
/// ⇒ 把「档位格 + 防重记录」这一小块单独拿出来，测试才能**真的走一遍
/// `set_tier` 跑的那段代码**。
///
/// ⚠ 第一版的测试是在测试里**复刻**了这段判断，于是把这里的
///   `disarm` 短路掉（`if before != after && false`）测试**照样全绿**——
///   **复印件不是守卫**。
fn apply_tier(tier: &crate::watch::TierCell, armed: &mut Arming, d: Dispatch) -> String {
    // ⚠ `TierCell::set` 不告诉你是「从 A 变到 B」还是「本来就是 B」——
    //   所以「是否真的变了」只能**先读后写**再比。
    //
    //   ⚠ 比的是 **`set` 的返回值**而不是传进去的 `d`。今天是同一个值
    //   （`set` 原样返回入参），但若哪天 `set` 加了夹取，比 `d` 就会在
    //   「请求变了、生效值没变」时**误清防重记录**。比返回值对那种情况免疫。
    let before = tier.get();
    let after = tier.set(d);
    if before != after {
        armed.disarm();
    }
    tier_id(after)
}

/// 改档。**返回真实生效的 id**——界面上要显示程序认定的那个。
#[tauri::command]
pub fn set_tier(tier: String) -> String {
    // 认不出来的一律当 Manual。**不许因为解析失败就升档。**
    let d = parse_tier(&tier);
    // ⚠⚠ **改档必须清空防重记录**（原理由与漏出来的路见 `apply_tier` 的
    //   文档与测试 `changing_tier_disarms`）。这条以前只活在注释里。
    let real = with_state(|s| apply_tier(&s.tier, &mut s.armed, d)).unwrap_or("Manual".into());
    // ⭐ 在 `with_state` **之后**刷——锁已经放掉了，这里再拿就是死锁。
    // 档位变了，托盘首行和图标状态必须跟着变：面板看不见的时候，
    // 它是唯一告诉用户「开着自动」的地方。
    refresh_tray();
    real
}

/// 前端 id → 档位。**解析失败一律落到 `Manual`**（最低档），
/// 不升档、不报错——面板刚起来时还没有状态可读，报错只会让用户看见一句红字。
fn parse_tier(raw: &str) -> Dispatch {
    match raw {
        "Auto" => Dispatch::Auto,
        // ⚠ **两处历史值都落进这一支 ⇒ 读成 Manual**（最低档）：
        //
        // ① `"Confirm"`——2026-10-15 之前那一档的名字。本机 prefs 里
        //    **很可能还存着这个值**，解析不到就必须回落。
        // ② 任何未知字符串（前端版本不一致、手改过 prefs）。
        //
        // 降档方向是对的：解析不出来时得到的必须是最低档，
        // 绝不因为「不认识」就升上去——升上去意味着**自动发送**。
        _ => Dispatch::Manual,
    }
}

/// **两个条件同时成立**才允许按下回车。
///
/// 抽成纯函数是为了让测试能**穷举全部组合**——写在 `dispatch` 里的话，
/// 「能不能发出去」只能靠人读分支顺序，读错一次就是发错一条真消息。
///
/// ⚠ 2026-10-15 起条件从三个减到两个：`Confirm` 档取消后
/// 「已装填过」不再是独立条件（`Auto` 一步到位，没有上一步）。
/// ⚠ **注意这里成立的那一档就是不问就发的那一档**——
///   砍掉 `Confirm` 正是拆掉了「发送前再问一次」那道闸。
fn sends_for_real(tier: Dispatch, act: Action) -> bool {
    matches!((tier, act), (Dispatch::Auto, Action::Confirm))
}

/// 档位 → IPC 契约里的枚举 id。见 `get_tier` 的说明：这是**给前端拿去索引
/// `TIER_NAME` / `data-val` 的键**，不是给人看的字。
fn tier_id(d: Dispatch) -> String {
    match d {
        Dispatch::Manual => "Manual",
        Dispatch::Auto => "Auto",
    }
    .to_string()
}

/// 档位 → **中文显示名**。**只给人看**：托盘 tooltip、投递回执的说明文字。
/// 前端拿不到这个，它自己有 `TIER_NAME`。
fn tier_name(d: Dispatch) -> String {
    match d {
        Dispatch::Manual => "仅填入",
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
/// 现在由调用方把「这一轮到底该不该发」算成 [`send`] 传进来。
/// ⚠ 2026-10-15 之后**所有调用点传的都是 `false`**：
///   `Manual` 档只填入；`Auto` 档走的是 [`deliver_send`]，不走这里。
///   这个 `false` 是「默认档下程序不替人说话」这条约束的落点。
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

/// **`Auto` 档专用：填入 + 发送，一步做完。**（2026-10-15 新增）
///
/// ## 为什么要有它，而不是直接调 [`deliver`]
///
/// 老结构里「填入」和「发送」是两次独立调用：第一次 `fill_input_box`
/// 让人读、让人改，第二次 `send_only` 只按回车。
/// `Auto` 档砍掉第二次之后，那条「只按回车」的路径就失去了它的前提——
/// **输入框里根本没有字**，按回车等于在空输入框上发一条空消息，
/// 而回执会写「已送达」。
///
/// ⚠ 所以这里必须走 `fill_and_send`（填了再按回车），
/// **不能**沿用 `send_only`。这是个静默失败：不报错、回执说成功、
/// 而对方的聊天窗口里出现一条空白。
///
/// ## 为什么它不接受 `tier` 参数
///
/// 「该不该发」这个判断只允许在 `dispatch` 里做一次。
/// 这里再读一遍 tier 就是第二份判断，两份迟早分叉，而分叉的方向
/// 永远是「多发」。档位判定在 [`sends_for_real`] 那里，穷举测试守它。
fn deliver_send(hwnd: windows::Win32::Foundation::HWND, candidate_id: &str) -> DeliveryReceipt {
    let Some(text) = candidate_text(candidate_id) else {
        return DeliveryReceipt::Failed(format!(
            "候选 {candidate_id} 已不在当前这轮的候选里（对方又发了新消息，候选被换掉了）。重新等一轮。"
        ));
    };
    // ⚠ 拿不到正文就**不注入**——见 [`deliver`] 里同一段的理由：
    // 往别人的聊天框里送一句占位假话，比不送严重得多。
    chatservant_capture::inject::fill_and_send(&text, hwnd)
}

/// **`Auto` 档的自动发送入口。给观察循环调用**（2026-10-15 新增）。
///
/// ## 为什么单独一个函数，而不是让循环去调 `dispatch`
///
/// `dispatch` 是 `#[tauri::command]`：它要求前端传 `action`，而自动发送
/// **没有人的点击**——那一层在这里凭空造一个 `"confirm"` 出来，
/// 就等于让「谁点的」这个问题有了一个假的答案。
///
/// 循环自己知道候选是谁、为什么发（护栏判据在 `watch.rs::AutoGate`），
/// 所以它直接调这个只做「送出去」的函数。
///
/// ## 护栏在哪
///
/// ⚠ **这里不做任何判断。** 「该不该自动发」全部在
/// [`crate::watch`] 的 [`AutoGate`] 里，纯函数 + 穷举测试。
/// 这里只负责「既然决定发了，那就发」——
/// 判断与执行分成两处，是为了**判断可以被测穷尽**，
/// 而执行不行（它要真按回车）。
pub fn auto_send(candidate_id: &str) -> Result<DeliveryReceipt, String> {
    // ⚠ 防重记录必须在这里更新，而不是只在循环里记：
    //   循环记的那份在 `Scratch` 里，而这个函数才是真正按下回车的地方。
    //   两处各记一份会漏——比如循环记了、这里发送前失败了。
    if let Ok(mut g) = STATE.lock()
        && let Some(s) = g.as_mut()
    {
        s.armed.disarm();
    }
    let Some(hwnd) = find_wechat() else {
        return Err("找不到微信窗口（已退出？换了进程名？）".into());
    };
    Ok(deliver_send(hwnd, candidate_id))
}

/// 回执 → 前端认得的字符串。
fn outcome_name(r: &DeliveryReceipt) -> &'static str {
    match r {
        DeliveryReceipt::NotSent => "NotSent",
        DeliveryReceipt::Sent => "Sent",
        DeliveryReceipt::Filled => "Filled",
        DeliveryReceipt::PossiblySent => "PossiblySent",
        DeliveryReceipt::Failed(_) => "Failed",
        DeliveryReceipt::TimedOut => "TimedOut",
    }
}

/// 回执 + 下一步 → 面板上那一句**给人看**的话。
///
/// ## 为什么停下时不说「下一步」
///
/// [`Next::Halt`] 的 `reason` **就是** `receipt.explain()` 本身——
/// `Dispatcher::next` 在「挡住自动重试」那条上把理由原样透传。
/// 于是 2026-10-15 之前那种
/// `format!("档位 {}｜{}｜下一步：{}", …, receipt.explain(), next.explain())`
/// 会把同一句话在界面上**念两遍**。真机量出来的就是：
///
/// ```text
/// 档位 仅填入｜已填进输入框并停在光标处——没有发出去，要发你自己按回车
/// ｜下一步：停下等人工：已填进输入框并停在光标处——没有发出去，要发你自己按回车
/// ```
///
/// 长句尤其明显：回执本来就该一屏读完，重复一遍直接把它顶出面板。
/// ⇒ 停下时说一次；只有在「接下来真的还有一步动作」时才接「下一步」。
fn receipt_detail(tier: Dispatch, receipt: &DeliveryReceipt, next: &Next) -> String {
    let why = receipt.explain();
    match next {
        Next::Halt { reason } if reason == why => format!("档位 {}｜{}", tier_name(tier), why),
        _ => format!(
            "档位 {}｜{}｜下一步：{}",
            tier_name(tier),
            why,
            next.explain()
        ),
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

/// 把某个隐藏的**辅助**窗口挪到屏幕内并显示。
///
/// ⚠ 现在只有 `debug` 一个调用方，注释里原来那句「三个窗口共用一条路径」
/// 已经不成立了——**因为剩下两个窗口的行为并不一样**：
///
/// - 主面板**不能挪**。它停在用户放的位置（`show_panel` 只 show + 聚焦）。
///   托盘菜单里点「打开设置」时如果把它 `set_position(120,120)`，
///   用户会发现面板凭空跳了个位置——打开设置这件事不该有副作用。
/// - 辅助窗口创建时就 park 在屏幕外（`-32000, -32000`），不给坐标就找不到它。
fn show_window(app: &AppHandle, label: &str) -> Result<(), String> {
    let w = app
        .get_webview_window(label)
        .ok_or_else(|| format!("{label} 窗口不存在"))?;
    w.set_position(PhysicalPosition::new(120.0, 120.0))
        .map_err(|e| e.to_string())?;
    w.show().map_err(|e| e.to_string())?;
    w.set_focus().map_err(|e| e.to_string())
}

/// 打开设置。**设置是主面板里的一屏，不是另一个窗口。**
///
/// 早先这里 `show_window(&app, "settings")`，弹的是 640 × 520 的独立窗口。
/// 现在那个窗口已经删掉了（`tauri.conf.json` 里不再有它），设置搬进了
/// 面板本身（`PanelSettings.svelte`）。所以这条命令改成两步：
/// **把面板叫到前面来，再告诉它「切到设置那一屏」**。
///
/// 两步都要：面板可能正收在托盘里，也可能停在主区。
/// 只 emit 不 show 的话，用户点了托盘菜单什么也看不见。
///
/// 用 `show_panel` 而不是 `show_window` —— 见上面那条注释：**面板不许挪位置**。
#[tauri::command]
pub fn open_settings(app: AppHandle) -> Result<(), String> {
    show_panel(app.clone())?;
    app.emit(crate::contract::events::MENU_SETTINGS, ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_debug(app: AppHandle) -> Result<(), String> {
    show_window(&app, "debug")
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// 读「关闭时」。**读不到就按默认的 `Exit`** —— 那是最不容易出事的那个。
#[tauri::command]
pub fn get_close_behavior(app: AppHandle) -> CloseBehavior {
    read_close_behavior(&app)
}

/// 写「关闭时」。前端设置页调它。
#[tauri::command]
pub fn set_close_behavior(app: AppHandle, behavior: CloseBehavior) -> Result<(), String> {
    // ⭐ **先验证落点存不存在，再写。**
    // 选了「收进托盘」却没有托盘 = 一个没有落点的动作 = 用户把面板
    // 收走之后它就变成了看不见的僵尸。所以这一档必须在托盘可用时才成立。
    if behavior == CloseBehavior::Tray && !tray_usable() {
        return Err("托盘不可用，不能设成「收进托盘」—— 那样关掉面板它就藏起来了。".into());
    }
    let path = prefs_dir(&app).join("close_behavior.txt");
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

/// **设置文件放在哪个目录**，不是哪个文件。
///
/// 早先这个函数的名字说的是「prefs（设置）」，实现给的却是某一个设置的
/// 那一个文件——`close_behavior.txt` 已经拼在函数体里了。加第二个设置时
/// 照抄它，唯一的改法就是改那个文件名：那是把特例伪装成通用入口，
/// 迟早会有人把两个设置写进同一个文件。现在返回目录，
/// 各设置自己 `join(自己的文件名)`。
///
/// 取不到 `app_config_dir()` 时回落当前目录，这个行为保持不变。
fn prefs_dir(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

fn read_close_behavior(app: &AppHandle) -> CloseBehavior {
    let Ok(s) = std::fs::read_to_string(prefs_dir(app).join("close_behavior.txt")) else {
        return CloseBehavior::default();
    };
    match s.trim() {
        "tray" if tray_usable() => CloseBehavior::Tray,
        // 「tray」但托盘没了 → 退回 Exit。**一个失效的设置比没有设置更危险**：
        // 用户以为它还在后台跑，其实它可能已经退出了。
        _ => CloseBehavior::Exit,
    }
}

/// 档位标记的形状。与前端 `types.ts` 的 `MarkShape` 镜像。
///
/// serde 的写法照抄同文件里的 `CloseBehavior`——它是 `#[tauri::command]` 的
/// **入参**，Tauri 要求命令参数类型实现 `CommandArg`，只 derive `Serialize`
/// 编译不过（2026-10-10 本机实测，见文件头那段注释）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkShape {
    /// 勾。**默认值**。
    Tick,
    /// 圈。
    Ring,
}

impl Default for MarkShape {
    /// **默认 `Tick`。**
    ///
    /// 这是个纯外观偏好，没有安全含义，所以「哪个都不报错」这一条不构成
    /// 理由；理由是勾是这两者里**信息量更大**的那个：它指着一个具体的
    /// 档位，而圈只标着「这里有个档」。将来真要显示档位差别时，
    /// 默认落点得是能分辨出区别的那个。
    fn default() -> Self {
        MarkShape::Tick
    }
}

/// 读「档位标记形状」。
///
/// ⚠ **这一条走落盘，绝不走 `watch.rs` 的 `TierCell`。**
/// 档位不落盘是产品硬约束（发送档默认最低、会话结束自动回落到 `Manual`）；
/// 标记形状是外观偏好，用户重启后期望它还在。两者性质不同，混在一起就
/// 会出现「档位的生命周期污染外观偏好」或者反过来。
#[tauri::command]
pub fn get_mark_shape(app: AppHandle) -> MarkShape {
    read_mark_shape(&app)
}

/// 写「档位标记形状」。
#[tauri::command]
pub fn set_mark_shape(app: AppHandle, shape: MarkShape) -> Result<(), String> {
    let dir = prefs_dir(&app);
    std::fs::create_dir_all(&dir).map_err(|e| format!("建目录失败：{e}"))?;
    std::fs::write(dir.join("mark_shape.txt"), mark_shape_str(shape))
        .map_err(|e| format!("写设置失败：{e}"))?;
    Ok(())
}

/// 落盘的那一行。小写，与 `close_behavior` 的 `"exit"` / `"tray"` 同风格。
fn mark_shape_str(s: MarkShape) -> &'static str {
    match s {
        MarkShape::Tick => "tick",
        MarkShape::Ring => "ring",
    }
}

fn read_mark_shape(app: &AppHandle) -> MarkShape {
    read_mark_shape_at(&prefs_dir(app).join("mark_shape.txt"))
}

/// **按路径读的内部入口。**
///
/// 拆出来是为了能单测：`AppHandle` 在测试里根本造不出来，而这里最要紧的
/// 恰恰是容错策略（读不到 / 认不出就回落默认值）——那部分必须能验。
fn read_mark_shape_at(path: &std::path::Path) -> MarkShape {
    let Ok(s) = std::fs::read_to_string(path) else {
        return MarkShape::default();
    };
    parse_mark_shape(&s)
}

fn parse_mark_shape(raw: &str) -> MarkShape {
    match raw.trim() {
        "ring" => MarkShape::Ring,
        // 认不出来的写法（空串、半截 JSON、被别的工具改坏了）一律落回
        // 默认值，**不报错也不崩**。与 `read_close_behavior` 同一条策略：
        // 一个失效的外观设置比没有设置更麻烦——面板会在每次读设置时
        // 拿到一个错误值，而用户根本没打算改它。
        _ => MarkShape::Tick,
    }
}

/// 托盘的状态。**由 `build_tray` 报结果，不是谁去查谁就知道。**
///
/// 实测（2026-10-10，本机 release + debug 各验）：`tauri.conf.json` 里
/// 定义的窗口在 `.setup()` **之前**就创建并开始加载，所以主面板会在
/// `build_tray` 跑之前发出它的第一个 IPC。日志实测顺序：
///
/// ```
/// 07:27:44.79x  托盘图标已就绪            <- build_tray
/// （更早，面板已拿到 closeBehavior=exit）  <- 面板的第一次提问
/// ```
///
/// 于是面板问的时候托盘「还没报」，两秒后设置视图再问就报 `tray`。
/// 同一个进程、同一个文件，答案不一样。
///
/// 关键是：**「还没报」和「报失败了」不是同一件事。**
/// 早先的实现把两者都当成 `false`（先是查 `tray_by_id()`，后来换成
/// 一个默认 `false` 的 AtomicBool），于是「还没报」被当成「用户其实
/// 没选托盘」，把用户的设置悄悄改掉了——这正是 `−` 永远不出现的原因。
/// 连跑 5 次，5 次都是先 exit 后 tray。
///
/// 只有**明确失败**才配降级：托盘真的建不起来时，一个失效的设置比没有
/// 设置更危险（用户以为它还在后台跑，其实它已经退出了）。
static TRAY_STATE: AtomicU8 = AtomicU8::new(TRAY_PENDING);

const TRAY_PENDING: u8 = 0;
const TRAY_READY: u8 = 1;
const TRAY_FAILED: u8 = 2;

/// `build_tray` 报结果。两个方向都要显式报——
/// 「建失败」不报的话，`Pending` 会被当成可用，设置就落在了一个
/// 没有落点的动作上。
pub fn set_tray_ready(ready: bool) {
    TRAY_STATE.store(
        if ready { TRAY_READY } else { TRAY_FAILED },
        Ordering::Relaxed,
    );
}

/// 「收进托盘」这一档现在能不能成立。
///
/// **`Pending` 算能成立**：`build_tray` 是 `.setup()` 的第一件事，
/// 它一定会跑完。押这一票不会把设置落在不存在的落点上——
/// 万一它真失败，`read_close_behavior` 下次读的时候会把这一档降回
/// `Exit`。这才是「失效比没有更危险」要防的方向。
fn tray_usable() -> bool {
    TRAY_STATE.load(Ordering::Relaxed) != TRAY_FAILED
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
/// ⚠ 不做二次确认：`Confirm` 档开着时退出可能留下半句没发出去。拦它的
/// 地方在**别处** —— 纸底标着当前档位，托盘图标在「会发出去」的档位下带红点，
/// 菜单首行写着「小周 · 确认后发」。用户是看得见的，不该在点下去
/// 的那一瞬间才拦住他。
///
/// ⚠ 2026-10-14 更正：这段原先通篇说的是 `Auto` 档（顶边 / 托盘红点 /
/// 「小周 · 自动发送」）。`Auto` 已于 2026-10-13 整档取消，档位也搬到了纸底，
/// 于是它举的三个例子里有两个指向的机制**当时就已经不对了**。
/// 保留「不做二次确认」这个结论，但理由要挂在**现在唯一会发出去的那档**上。
#[tauri::command]
pub fn quit_app(app: AppHandle) -> Result<(), String> {
    app.exit(0);
    Ok(())
}

/// 托盘图标跟着档位变。**Auto 档带红点** —— 面板看不见的时候，
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
    // 菜单里**没有文字行**了（见 `build_tray_menu`），档位信号走图标 +
    // tooltip。这两处都不影响菜单尺寸。
    if let Ok(icon) = tray_icon(tier_can_send()) {
        let _ = tray.set_icon(Some(icon));
    }
    let tip = tray_tooltip();
    let _ = tray.set_tooltip(Some(&tip));
    if let Ok(menu) = build_tray_menu(app) {
        let _ = tray.set_menu(Some(menu));
    }
}

/// 托盘 tooltip：应用名 + 当前档位。
///
/// ⚠ 「副驾 · chat-servant」这个前缀**只有这一处**。早先 `build_tray` 写死
/// 了一遍、`refresh_tray` 又用状态行整体盖掉一遍，于是名字在两处各活一份。
pub fn tray_tooltip() -> String {
    format!("{} · {}", APP_NAME, tray_status_line())
}

/// 应用名。tooltip 与窗口标题共用。
const APP_NAME: &str = "副驾 · chat-servant";

/// 托盘图标。`live` 为真时右下角盖一颗红点。
///
/// ⚠ **图标必须显式给**（实测 2026-10-10）：`TrayIconBuilder::with_id(...)`
/// 不像 `new(app)` 那样自动取应用图标，`bundle.icon` 只喂安装包不喂窗口，
/// 两头都没有 → 条目在、图标空，而日志照样打「托盘图标已就绪」。
/// 图编进二进制（`include_bytes!`），不依赖运行期资源路径。
///
/// ## `live` 的含义换过一档（2026-10-13 → 2026-10-15）
///
/// 早先是 `auto`，红点表示「现在是自动发送档」。
/// 2026-10-13 砍掉 `Auto` 之后红点改挂到 `Confirm` 上；
/// 2026-10-15 砍掉 `Confirm`、`Auto` 回来，红点又挂回 `Auto`。
/// 三次改动里**问题本身没变**：**面板看不见的时候，
/// 程序到底会不会把话发出去。**
///
/// - `Manual` → 无点。程序只填入，不会自己发。
/// - `Auto` → 红点。程序**会不问就发出去**。
///
/// ⚠ 两档之下红点不再有歧义：只有一个会发出去的档。
/// 三档时代 `Auto` 与 `Confirm` 都带点，看图分不出「要人确认」和「不问就发」。
pub fn tray_icon(live: bool) -> tauri::Result<tauri::image::Image<'static>> {
    use std::sync::OnceLock;
    // 底图只解一次。`refresh_tray` 在每次档位/会话变化时都会跑，
    // 每回重解一张 PNG 是白烧。
    static BASE: OnceLock<tauri::Result<(Vec<u8>, u32, u32)>> = OnceLock::new();
    let cell = BASE.get_or_init(|| {
        tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))
            .map(|i| (i.rgba().to_vec(), i.width(), i.height()))
    });
    let (src, w, h) = match cell {
        // `cell` 是 `&Result`，所以元组里每一项都是引用——直接透传会变成
        // `&u32` 传进要 `u32` 的地方，错误信息还指向调用点而不是这里。
        Ok((s, w, h)) => (s, *w, *h),
        Err(e) => return Err(tauri::Error::Anyhow(e.into())),
    };
    let mut px = src.clone();
    if live {
        stamp_live_dot(&mut px, w, h);
    }
    Ok(tauri::image::Image::new_owned(px, w, h))
}

/// 在 RGBA 缓冲右下角盖一颗实心红点（行主序，4 字节/像素）。
///
/// ⚠ 就地改，**不改尺寸**——托盘那边按图标原始尺寸建资源，
/// 尺寸一变就要重建整个托盘图标。
fn stamp_live_dot(px: &mut [u8], w: u32, h: u32) {
    // ⚠ 半径与圆心是**量出来的，不是随手取的**（2026-10-10 实测，
    // 2026-10-13 换图标后重量一次）。
    //
    // 【2026-10-10】旧图标（Tauri 脚手架默认的绿色气泡）里气泡占 x 9..22 / y 8..23。
    // 半径原先取边长 1/4、圆心 `(w-r-r/3, …)` = (22,22)，圆盘跨到 y=14 ——
    // 直接压在气泡上，而且红绿撞色，16px 下是一团脏东西。改成半径 1/6、圆心贴角。
    //
    // 【2026-10-13】图标换成「撕边纸 + 横跨的红勾」（`ui/scripts/icongen.py`）。
    // 半径公式没动，所以圆盘仍落在 x 21..31 / y 21..31；变的是**底下是什么**：
    //     红勾像素包围盒 x 5..26 / y 11..21，纸 x 7..24 / y 7..24
    //     圆盘 ∩ 红勾 = 0 像素；圆盘 ∩ 纸 = 8 像素（右下角一块空纸）
    // 也就是说红点仍然只落在「纸的空角」上——和旧图标落在气泡外的行为一致。
    // 16px 下红点直径约 5.3px，占画布 1/9，认得出也不糊。
    let r = (w.min(h) / 6).max(2) as i64;
    let cx = w as i64 - r - 1;
    let cy = h as i64 - r - 1;
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let dx = x - cx;
            let dy = y - cy;
            if dx * dx + dy * dy <= r * r {
                let i = ((y * w as i64 + x) * 4) as usize;
                if i + 3 < px.len() {
                    px[i] = 0xDC;
                    px[i + 1] = 0x26;
                    px[i + 2] = 0x26;
                    px[i + 3] = 0xFF;
                }
            }
        }
    }
}

/// 托盘状态：**只报档位**。
///
/// ⚠ 菜单里**不再摆这行字**了（用户 2026-10-10：「顶部非要显示个文字行？
/// 很好看？很有设计感？」）。一个灰的、点不动的首行是 Windows 托盘的老套路，
/// 不是设计——而且它让菜单凭空多高一行。它现在只喂 tooltip。
///
/// ⚠ **会话名不进托盘**（同上，用户判断 + 本机像素实测）。
///
/// - 用户看到托盘菜单时，**上下文里根本有那个会话**——菜单是从那个
///   聊天窗口旁边的托盘点开的。菜单上再写一遍名字不回答任何问题。
/// - 它曾是菜单里唯一由 **OCR 文本**驱动的字符串，认错字就把菜单撑宽
///   （实测 3 字名 → 181px，12 字名 → 230px）。
///   **一个识别错误不该改变界面布局。**
/// - 档位是**安全相关**的那一半：「自动发送」意味着程序会自己按下去。
///   它现在由托盘图标的红点承担。
///
/// 会话名仍然完整地显示在面板头部（`cs-peer`）和设置里——**托盘不复述，
/// 不是删掉**。
pub fn tray_status_line() -> String {
    tray_status_line_from(&current_tier_cn())
}

/// 状态行的纯逻辑，单独抽出来是为了能测——`STATE` 是进程级全局，
/// 测试去动它会跟别的用例互相踩。
///
/// ⚠ 空档位**不能变成空字符串**。宁可退回最低档 `Manual`。
fn tray_status_line_from(tier: &str) -> String {
    let t = tier.trim();
    if t.is_empty() {
        tier_name(Dispatch::Manual)
    } else {
        t.to_string()
    }
}

/// 托盘菜单。**构造在这里，不在 `main.rs`** —— 它要读档位和会话名，
/// 那是这个模块的事；`main.rs` 只负责把图标建出来。
pub fn build_tray_menu(app: &AppHandle) -> Result<tauri::menu::Menu<tauri::Wry>, tauri::Error> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    let mb = MenuBuilder::new(app);
    // ⚠ **任何一项建不起来就让整个菜单失败，不降级。**（2026-10-10 本机实测修正）
    //
    // 原来每项都写着 `.build().unwrap_or_else(|_| sep())`——
    // 建失败的菜单项被换成一条**分隔线**。那个降级方向是反的：
    // 这个菜单里唯一的出口就是末尾那颗「退出」，它一旦建失败就被抹成
    // 一条横线，于是用户把面板收进托盘之后，**程序又变成停不掉的那个**——
    // 正好是这次改动要消灭的东西——而且失败是静默的，界面上看不出来。
    //
    // 现在让 `?` 直接往上抛：`main.rs` 的 `build_tray` 会 `tracing::error!`
    // 记下来并**不建托盘**，于是设置里的「收进托盘」会被 `tray_available`
    // 拒掉，「关闭时 = 退出」照常工作。**降级要看得见。**
    //
    // `build(app)` 的 manager 参数是 Tauri 2.12 要求的——零参 `.build()`
    // 编译不过（E0061）；`PredefinedMenuItem::Separator` 更是 Tauri 1 的
    // 名字，2.x 里是 `PredefinedMenuItem::separator`。
    // 直接把 `MenuBuilder` 的结果交出去——`.build()` 本身就返回
    // `Result<Menu<Wry>, tauri::Error>`，套 `Ok(...?)` 是多余的
    // （clippy::needless_question_mark，2026-10-10 门禁实测）。
    mb
        // ⚠ **首行没有状态文字行**（2026-10-10，用户要求删掉）。
        //
        // 原来是 `MenuItemBuilder::with_id("status", …).enabled(false)`
        // —— 一行灰的、点不动的文字 + 一条分隔线。它是 Windows 托盘的老套路，
        // 不是设计，而且白占一行高度。档位信号改由**托盘图标的红点**
        // 和 **tooltip** 承担，见 `tray_icon` / `tray_tooltip`。
        .item(&MenuItemBuilder::with_id("show", "显示面板").build(app)?)
        .item(&MenuItemBuilder::with_id("relock", "锁定当前会话").build(app)?)
        .item(&MenuItemBuilder::with_id("settings", "打开设置").build(app)?)
        .item(&MenuItemBuilder::with_id("debug", "识别画布").build(app)?)
        .separator()
        // ⚠ **「退出」上没有加速键，这是量出来的，不是忘了**（2026-10-10 实测）。
        //
        // 原来挂着 `CmdOrCtrl+Q`。Win32 只要**任何一项**带加速键，就给整份
        // 菜单预留一整列右对齐的空白，所有项都得给它让位——本机像素实测：
        // 菜单宽 = 最长项文字 + 137px 固定开销，其中加速键列占掉 **73px**，
        // 整整三分之一，而那一列里除了「Ctrl+Q」五个字符全是空的。
        //
        // 换来的功能是「菜单已经用鼠标点开了，再按一下 Ctrl+Q」。
        // 这个菜单只能右键打开，键盘用户根本到不了这一步，
        // 那 73px 却要每一行、每一次弹出都付。撤掉。
        //
        // 全项目仅此一处加速键，没注册过任何全局快捷键插件（`rg` 实测），
        // 所以撤掉不会连带失去任何全局热键。要加回就一行。
        .item(&MenuItemBuilder::with_id("quit", "退出").build(app)?)
        .build()
}

/// 当前档位的中文名。
///
/// ⚠ **复用 `get_tier` 那条路径**（`with_state` + `tier_name`），不自己
/// 再解析一遍。读锁拿不到就按最低档 `Manual` —— **绝不能猜一个更宽松的值**。
fn current_tier_cn() -> String {
    tier_name(current_tier())
}

/// 当前档位。读锁拿不到就按最低档 `Manual` —— **绝不能猜一个更宽松的值**。
///
/// ⚠ `pub` 是给观察循环用的：`watch.rs` 的自动发送判据要读同一个格子，
///   两边各读一份就会分叉，而分叉方向永远是「多发」。
pub fn current_tier() -> Dispatch {
    with_state(|s| s.tier.get()).unwrap_or(Dispatch::Manual)
}

/// 当前这一档**会不会真的把消息发出去**。
///
/// ⚠ 名字里没有「自动」二字是有意的：这个函数回答的是
/// 「程序会不会把话发出去」，托盘红点靠它——
/// **面板看不见的时候，程序到底会不会替我说话。**
///
/// ⚠ 2026-10-15 之后，`Auto` 档**就是**「不问就发」的那一档
/// （`Confirm` 被砍掉，那道二次确认的闸随之拆掉）。
/// 所以这里两者合二为一：会发 = 会不问就发。
/// `Manual` 仍然只填入，红点不亮。
pub fn tier_can_send() -> bool {
    matches!(current_tier(), Dispatch::Auto)
}

/// 设置页能写的供应商。**「预设」那些。**
///
/// 换供应商只需在这里加一项，**不用重配整个应用**——设置页按这个列表
/// 渲染，每个供应商在**每一组**里的 key 独立存在凭据管理器里。
///
/// ⚠ `custom` 在里面，但**它不是一家供应商**：它是「自己填端点」那个动作。
/// 它收下的时候走的是 [`Route::custom`]（带校验），预设则走
/// [`Route::preset`]（端点定死，用户填什么都不采纳）——
/// 「给预设换一个地址」在语义上就是自定义，所以这里**不**允许。
///
/// ⚠ 这张表是**白名单**：`save_setting` / `forget_key` / `set_route` 都在
/// 它上面挡未知值。放行的后果是「拼错的名字看起来存上了，其实没有」。
pub const PROVIDERS: &[&str] = &[
    "openai",
    "anthropic",
    "minimax",
    "deepseek",
    "qwen",
    "zhipu",
    "ollama",
    "lmstudio",
    "custom",
];

/// 两组模型配置。与推理器**共用同一格**的句柄。
#[derive(Clone)]
struct ModelGroups {
    decision: chatservant_reason::ModelCell,
    llm: chatservant_reason::ModelCell,
}

impl ModelGroups {
    fn cell(&self, g: ModelGroup) -> &chatservant_reason::ModelCell {
        match g {
            ModelGroup::Decision => &self.decision,
            ModelGroup::Llm => &self.llm,
        }
    }

    /// 两组其实指向同一路？（决策组没单独配 key 时就是）。
    ///
    /// 界面上要**明说**这一点，否则用户改「决策模型」的端点时会发现
    /// 回复模型跟着变了，却没有任何地方解释。
    fn shared(&self) -> bool {
        self.decision.route() == self.llm.route() && self.decision.get() == self.llm.get()
    }
}

/// 设置页要读的状态。**这里没有任何 key。**
#[derive(Serialize)]
pub struct SettingsView {
    /// 两组，各自独立。
    pub groups: Vec<GroupView>,
    /// 代理。**单独一栏，两组都生效。**
    pub proxy: ProxyView,
    /// 「自定义端点」要选协议。**Rust 给什么前端就显示什么。**
    pub protocols: Vec<ChoiceView>,
}

/// 一个下拉项。**只有 id 与显示名**——值一律由 Rust 持有。
#[derive(Serialize)]
pub struct ChoiceView {
    pub id: String,
    pub label: String,
}

/// 一组模型配置。
#[derive(Serialize)]
pub struct GroupView {
    /// 稳定标识（凭据目标名也用它）。
    pub id: String,
    /// 界面上那一节的名称。
    pub label: String,
    /// 这一组这一段干什么。
    pub role: String,
    pub providers: Vec<ProviderView>,
    /// 当前生效的这一路。
    pub route: RouteView,
    /// 当前这一路能选的模型名（可能为空——本地与自定义）。
    pub models: Vec<String>,
    /// 当前选中的模型名（**真实生效值**，可能为空）。
    pub model: String,
    /// 关于上面那个模型名的**实话**：只列实测能通的，还是名字要你自己填。
    pub model_note: String,
    /// 这一组是否与另一组指向同一路。
    pub shared_with_other: bool,
}

/// 出站代理。
#[derive(Serialize)]
pub struct ProxyView {
    /// 空串 = 直连。⚠ 这是**地址**，界面上原样回显。
    pub url: String,
    /// 这条地址会不会被显示在日志里。恒为 true——脱敏由 `redact` 负责，
    /// 界面上要如实告诉用户「日志里会被抹掉凭据段」。
    pub redacted_in_logs: bool,
}

/// 当前生效的这一路。
#[derive(Serialize)]
pub struct RouteView {
    pub provider: String,
    pub provider_label: String,
    pub protocol: String,
    pub protocol_label: String,
    /// **归一化后的真实端点**（自定义时是用户填的那个）。
    pub base_url: String,
    /// 端点与模型名有没有在本机实测过。**false 就要在界面上说「未核实」。**
    pub measured: bool,
}

impl RouteView {
    fn of(r: &Route) -> Self {
        Self {
            provider: r.provider().key().to_string(),
            provider_label: r.provider().label().to_string(),
            protocol: r.protocol().key().to_string(),
            protocol_label: r.protocol().label().to_string(),
            base_url: r.base_url().to_string(),
            measured: r.measured(),
        }
    }
}

#[derive(Serialize)]
pub struct ProviderView {
    pub id: String,
    /// 界面上显示的名字（`id` 是稳定标识，不是显示名）。
    pub label: String,
    /// 已经配好了。
    pub configured: bool,
    /// 值来自环境变量——这种情况下输入框应只读。
    pub from_env: bool,
    /// 值来自**旧版那个不带组别的目标名**。界面上要说明连带删除的后果。
    pub legacy: bool,
    /// 掩码，仅供显示。
    pub masked: String,
    /// 没配时给的下一步指引。**不能是空字符串或静默失败。**
    pub hint: String,
}

/// 设置页那个模型选择器要的回包。
#[derive(Serialize)]
pub struct ModelView {
    /// 这一组是哪一组。
    pub group: String,
    pub provider: String,
    pub models: Vec<String>,
    pub model: String,
}

/// 切模型。**立即生效，不用重启**——推理器每一次发请求都重读那个格子。
///
/// ⚠ 拒收的口径分两种，见 [`Provider::accepts_model`]：实测过的几家
/// 拒收清单外的名字；没有核实过清单的那几家（本机与自定义）只挡空串。
#[tauri::command]
pub fn set_model(group: String, model: String) -> Result<ModelView, String> {
    let g = parse_group(&group)?;
    with_cell(g, |c| c.set(&model))??;
    with_cell(g, |c| model_view(g, c))
}

/// 「没有真实传输」时那句话。**统一一处**，免得三处各写一遍、
/// 然后慢慢说成三个意思。
fn no_transport_note() -> String {
    "还没接上真实传输（没有可用 key），所以现在没有模型可选".to_string()
}

/// 组别的短名进到这里。**认不出来一律拒**——静默兜一个默认值
/// 会把错一路带到凭据目标名和出网那一步。
fn parse_group(group: &str) -> Result<ModelGroup, String> {
    ModelGroup::from_key(group)
        .ok_or_else(|| format!("未知的组别 {group}。只有 decision 与 llm 两组"))
}

fn model_view(group: ModelGroup, cell: &chatservant_reason::ModelCell) -> ModelView {
    let route = cell.route();
    ModelView {
        group: group.key().to_string(),
        provider: route.provider().key().to_string(),
        models: route.models().iter().map(|m| (*m).to_string()).collect(),
        model: cell.get(),
    }
}

/// 在**共享的那一格**上跑一段闭包。
///
/// 拿不到格子（还没 `start()`，也就是没有可用 key）就报那句统一的话——
/// 与其在这里造一个假格子，不如让界面知道「现在没有真东西可改」。
fn with_cell<R>(
    group: ModelGroup,
    f: impl FnOnce(&chatservant_reason::ModelCell) -> R,
) -> Result<R, String> {
    let cell = with_state(|s| s.models.as_ref().map(|m| m.cell(group).clone())).flatten();
    cell.as_ref().map(f).ok_or_else(no_transport_note)
}

/// 读设置页要的那一屏。**两组各自独立 + 一栏共用代理。**
#[tauri::command]
pub fn get_settings() -> SettingsView {
    let models = with_state(|s| s.models.clone()).flatten();
    let proxy = with_state(|s| s.proxy.get()).flatten().unwrap_or_default();
    SettingsView {
        groups: ModelGroup::ALL
            .iter()
            .map(|g| group_view(*g, models.as_ref()))
            .collect(),
        proxy: ProxyView {
            url: proxy,
            redacted_in_logs: true,
        },
        protocols: [Protocol::OpenAiCompatible, Protocol::Anthropic]
            .iter()
            .map(|p| ChoiceView {
                id: p.key().to_string(),
                label: p.label().to_string(),
            })
            .collect(),
    }
}

fn group_view(g: ModelGroup, models: Option<&ModelGroups>) -> GroupView {
    // 没有真实传输时这一组也要渲染——**用户正是要在没配好的时候配置它**。
    // 这与「没 key 时不列模型」是两条不同的规则：模型清单要真实，
    // 而 key 输入框必须能显示（见 `.scratch/…/40-key-input-unreachable-when-unset.md`）。
    let (route, model, models_list, shared) = match models {
        None => (
            // 没有真实传输时也给一个**真值**（OpenAI 预设），
            // 而不是编一个假端点——这一格是回显「当前这一路」，
            // 编一个的话界面显示的地址从未生效。
            Route::preset(Provider::OpenAi).expect("OpenAI 有预设端点"),
            String::new(),
            Vec::new(),
            false,
        ),
        Some(m) => {
            let c = m.cell(g);
            (
                c.route(),
                c.get(),
                c.models().iter().map(|s| (*s).to_string()).collect(),
                m.shared(),
            )
        }
    };
    let model_note = if models_list.is_empty() {
        "这一路没有可选清单——模型名要你自己填。".to_string()
    } else if route.measured() {
        "只列本机实测能通的模型。".to_string()
    } else {
        "这一家的端点与模型名抄自官方文档，**未在本机实测**。".to_string()
    };
    GroupView {
        id: g.key().to_string(),
        label: g.label().to_string(),
        role: match g {
            ModelGroup::Decision => {
                "判断对方意图的那一段。输出只有几十个字，通常配一个便宜的模型就够。".into()
            }
            ModelGroup::Llm => "起草与排序那两段。图片（如果外发闸开着）跟着这一组走。".into(),
        },
        providers: PROVIDERS.iter().map(|p| provider_view(g, p)).collect(),
        route: RouteView::of(&route),
        models: models_list,
        model,
        model_note,
        shared_with_other: shared,
    }
}

fn provider_view(group: ModelGroup, id: &str) -> ProviderView {
    let s: KeyStatus = secret::status(group, id);
    ProviderView {
        id: id.to_string(),
        label: Provider::from_key(id)
            .map(|p| p.label())
            .unwrap_or(id)
            .to_string(),
        configured: s.configured,
        from_env: s.from_env,
        legacy: s.legacy,
        masked: s.masked,
        hint: if s.configured {
            String::new()
        } else {
            secret::missing_hint(group, id)
        },
    }
}

/// 换一组走哪一路（厂商 / 协议 / 自定义端点）。
///
/// **立即生效，不用重启**：这一格被推理器每次发请求重读。
///
/// ⚠ 预设**不接受**用户填的端点——「给预设换一个地址」在语义上就是自定义。
///   放行的话界面上会同时显示「厂商 = OpenAI」和一个完全不同的地址，
///   而那一行字就成了假话。
#[tauri::command]
pub fn set_route(
    group: String,
    provider: String,
    protocol: Option<String>,
    base_url: Option<String>,
) -> Result<GroupView, String> {
    let g = parse_group(&group)?;
    let route = route_from_input(&provider, protocol.as_deref(), base_url.as_deref())?;
    let models = with_state(|s| s.models.clone())
        .flatten()
        .ok_or_else(no_transport_note)?;
    models.cell(g).set_route(route);
    // ⚠ 回包是**读回来的真实状态**（含另一组是不是与我同路），
    // 而不是把传进去的三项原样弹回去——界面必须显示程序认定的那个。
    Ok(group_view(g, Some(&models)))
}

/// 由界面那三项（厂商 / 协议 / 端点）造出**一条真实生效的路**。
///
/// 认不出来的值一律拒，**不静默接受**——拼错的供应商名会让人以为配上了，
/// 其实没有。而自定义端点走 [`Route::custom`]，那里面有 scheme 与主机名的校验。
fn route_from_input(
    provider: &str,
    protocol: Option<&str>,
    base_url: Option<&str>,
) -> Result<Route, String> {
    let p = Provider::from_key(provider).ok_or_else(|| {
        format!(
            "未知的供应商 {provider}。支持的是：{}",
            PROVIDERS.join(" / ")
        )
    })?;
    // ⚠ `from_key` 认得的**别名**（`glm` → zhipu、`tongyi` → qwen）不在白名单里，
    //   所以在这里再核一次短名。别名的存在是为了命令行与日志可读，
    //   而配置这一格只认白名单上的那一个拼法。
    if !PROVIDERS.contains(&p.key()) {
        return Err(format!(
            "未知的供应商 {provider}。支持的是：{}",
            PROVIDERS.join(" / ")
        ));
    }
    if p != Provider::Custom {
        return Route::preset(p);
    }
    let pr = Protocol::from_key(protocol.unwrap_or_default())
        .ok_or_else(|| "自定义端点要选一种协议：OpenAI 兼容，还是 Anthropic 原生。".to_string())?;
    Route::custom(pr, base_url.unwrap_or_default())
}

/// 读代理。
#[tauri::command]
pub fn get_proxy() -> Result<ProxyView, String> {
    Ok(ProxyView {
        url: with_state(|s| s.proxy.get()).flatten().unwrap_or_default(),
        redacted_in_logs: true,
    })
}

/// 写代理。**空串 = 不走代理。**
///
/// ⚠ **一次设置，两组都生效**——代理是网络层的属性，不是任何一家的属性。
///   重复填两遍就迟早有两份不一样的，而「一组走代理、一组不走」是个
///   没人主动想要的状态。
#[tauri::command]
pub fn set_proxy(url: String) -> Result<ProxyView, String> {
    let applied = with_state(|s| s.proxy.set(&url))
        .ok_or_else(|| "观察循环还没起来，改不了设置".to_string())??;
    Ok(ProxyView {
        url: applied.unwrap_or_default(),
        redacted_in_logs: true,
    })
}

/// 提交设置。**立即生效，不用重启。**
///
/// `key` 为 `None` 或空白 = 保留原值。删 key 走 `forget_key`。
#[tauri::command]
pub fn save_setting(group: String, provider: String, key: Option<String>) -> Result<bool, String> {
    let g = parse_group(&group)?;
    if !PROVIDERS.contains(&provider.as_str()) {
        // 认不出来的 provider 一律拒绝，**不静默接受**——
        // 拼错的供应商名会让人以为 key 存上了，其实没有。
        return Err(format!(
            "未知的供应商 {provider}。支持的是：{}",
            PROVIDERS.join(" / ")
        ));
    }
    let wrote = secret::apply(g, &provider, key.as_deref())?;
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
    /// ⚠ **这个底账核实过没有。** 端点可换之后「第三方模型」这个词不再准确，
    ///   而没核实过的数字**必须能在界面上说出来**——否则用户会照着一个
    ///   编出来的数估自己的账单。
    pub provider_base_verified: bool,
    /// 当前这一组走的是哪家。
    pub provider: String,
}

/// 预算默认值。取标准档上限 1568 的约 3/4，够看清一张聊天图。
pub const BUDGET_DEFAULT: u32 = 1200;

fn budget_view(budget: u32, provider: &chatservant_reason::Provider) -> BudgetView {
    use chatservant_reason::Provider;
    // 底账是**随模型变**的：gpt-4o-mini 是 2833，Claude 的 patch 计费是 0。
    // 这里如实反映当前 provider，界面据此显示「预算不够」的警告。
    //
    // ⚠ **只有 OpenAI 那一项是核实过的数字。** 其余各家（MiniMax 之外
    // 新加的云端与本机、以及自定义端点）我们没有权威数字：端点可换之后，
    // 「第三方模型」这个词已经不准了，同一个端点可能是任何人的服务。
    // 拿不到数字时**不许编一个**——编出来的预算会让用户以为花了那个数。
    // 这里报 0 并把 `verified` 置 false，让界面说「成本未知」。
    let (provider_base, verified) = match provider {
        Provider::OpenAi => (2833, true),
        Provider::Anthropic => (0, true),
        _ => (0, false),
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
        provider_base_verified: verified,
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
/// ⚠ **这是个危险开关**，而且端点可换之后**更危险**：
///
/// 1. 对方发来的图片区域会被裁出、缩放、base64 编码，然后发出去。
///    媒体检测的准确率尚未在真实帧上完整验证，所以这里**只允许显式开启**，
///    没有「自动」档。
/// 2. 「发给第三方模型」这句话**已经不准了**：端点可以指向任何地址
///    ——用户自己填的自定义端点、本机 Ollama、或者一台内网机器。
///    ⇒ 界面上要显示**真实的目标地址**，而不是厂商名。
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
        Provider::OpenAi => 2833 + 5667,
        Provider::Anthropic => 1568,
        // 其余各家的视觉计费未核实（见 `budget_view` 的说明），
        // 所以给 Anthropic 同级的保守上界，**并让界面标「成本未知」**。
        _ => 1568,
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

/// 当前生效的供应商。**取「回复模型」那一组**——
///
/// 图片外发那一闸管的是**跟着起草走的那些图**，所以计费规则要看那一组，
/// 而不是决策组。**没配 key 时退回 OpenAI**——纯展示用的，
/// 不参与任何路由决策。
fn active_provider() -> chatservant_reason::Provider {
    with_cell(ModelGroup::Llm, |c| c.provider()).unwrap_or(chatservant_reason::Provider::OpenAi)
}

#[tauri::command]
pub fn forget_key(group: String, provider: String) -> Result<(), String> {
    let g = parse_group(&group)?;
    if !PROVIDERS.contains(&provider.as_str()) {
        return Err(format!("未知的供应商 {provider}"));
    }
    secret::erase(g, &provider)
}

/// 测连通性。**只回「通不通」**，不回任何响应体——
/// 上游的错误正文可能带 key。
///
/// ⚠ 测的是**这一组当前这一路**，包括用户自己填的端点。所以它同时
/// 是「自定义端点填对没有」的那一次验证——而界面上不会说它是另一样东西。
#[tauri::command]
pub fn probe_key(group: String) -> Result<bool, String> {
    let g = parse_group(&group)?;
    let route = with_cell(g, |c| c.route())?;
    let Some(key) = secret::load(g, route.provider().key()) else {
        return Err(secret::missing_hint(g, route.provider().key()));
    };
    let proxy = with_state(|s| s.proxy.get()).flatten();
    // ⚠ 探针走 `GET {base_url}models`，不调模型：不产生 token、不花钱。
    // 协议层归 genai 之后，这条 GET 是**唯一**一处没外包出去的网络调用，
    // 因为 genai 的 `WebClient` 是私有的、而 `models` 不是任何适配器的操作。
    // 代理与聊天那条路**同一个** `reqwest::Proxy::all`，别写第二套。
    match chatservant_reason::probe_route(
        &route,
        &key,
        std::time::Duration::from_secs(10),
        proxy.as_deref(),
    ) {
        Ok(()) => Ok(true),
        // 失败原因已经过 `redact`，可以安全显示。
        Err(e) => Err(e),
    }
}

/// 起观察循环。**只在启动时调一次。**
///
/// 有 key 就组装**真实传输**（genai 的 `HttpReasoner`）；
/// 没 key 就用**明确标注的预置传输**——界面上每条候选都带「预置」字样，
/// 不能让用户以为模型接通了。
pub fn start(app: &AppHandle) {
    let (cfg, models, proxy) = build_loop_config();
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
            models,
            proxy,
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
            // 去重指纹是 **(ticks, 阶段)**，不是单独的 ticks。
            //
            // ⚠ 少一半会出事：观察循环一轮里会推**两帧**——先「读到消息、
            // 正在起草」，再「起草完成」。这两帧的 `ticks` 相同，
            // 只比 `ticks` 的话第二帧（真正有候选的那帧）会被当成重复吃掉，
            // 界面永远停在「正在起草」。
            //
            // 两半缺一不可：`ticks` 覆盖「同一阶段内容变了」，
            // `phase` 覆盖「同一 ticks 里阶段变了」。
            let mut last: Option<(u64, &'static str)> = None;
            loop {
                if let Some(out) = with_state(|s| s.watcher.latest()).flatten()
                    && last != Some((out.ticks, out.phase.key()))
                {
                    last = Some((out.ticks, out.phase.key()));

                    // ⭐ **先把锁释放，再刷托盘。**
                    // `refresh_tray` 要读 `STATE` 拿档位和会话名，
                    // 在锁里调它就是自己等自己 —— 直接死锁。
                    let mut peer_changed = false;
                    if let Ok(mut g) = STATE.lock()
                        && let Some(s) = g.as_mut()
                    {
                        s.candidates = out.candidates().to_vec();
                        if s.peer != out.peer {
                            s.peer = out.peer.clone();
                            peer_changed = true;
                        }
                    }
                    if peer_changed {
                        // 托盘首行写的是「我到底在替谁说话」，
                        // 会话换了就得跟着换。
                        // 同模块内直接调——`commands::` 前缀在这里是
                        // 2015 edition 的写法，2018 之后不解析（E0433）。
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
                    if let Err(e) = app2.emit(crate::contract::events::READOUT, &out) {
                        tracing::error!(
                            target: "chat_servant::pump",
                            "readout 推不到界面：{e}（tick={}）",
                            out.ticks
                        );
                    }
                    tracing::info!(
                        target: "chat_servant::pump",
                        "emit readout tick={} phase={} peer={:?} messages={} candidates={}",
                        out.ticks,
                        out.phase.key(),
                        out.peer,
                        out.messages().len(),
                        out.candidates().len()
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
///
/// ## 两组各自装配
///
/// 判断那一段走**决策组**，起草与排序走**回复组**——这是用户要的两组
/// （票 43）。合并成一个推理器就表达不了「视觉/判断走 A、文字走 B」。
///
/// ⚠ **决策组没单独配 key 时，它与回复组是同一路。**
///   这是有意的默认：绝大多数用户只有一把 key，逼他配两把只会让人
///   第一步就卡住。界面上用 `shared_with_other` **明说**这件事，
///   而一旦在这一组填了自己的 key / 选了别的端点，两边立刻分开。
///
/// 第二个返回值是两组格子的句柄。**它们不在 `LoopConfig` 里**：
/// 循环自己不读这些格子，读它的是推理器内部——而推理器已经被
/// `Box<dyn Reasoner>` 吃掉了，从外面拿不回来。命令层要改模型与端点
/// 就必须拿到**同一个**格子，所以在这里一并交出去。
///
/// ⚠ **不落盘**：这两组配置与代理都只在内存里，与 `recent_n` / 预算
///   那一批设置同生命周期。重启后回到预设默认——凭据本身在凭据管理器里，
///   但「走哪一家」要重新选。持久化不在本票范围内。
fn build_loop_config() -> (
    LoopConfig,
    Option<ModelGroups>,
    chatservant_reason::ProxyCell,
) {
    use chatservant_reason::{HttpReasoner, ModelCell, ProxyCell};

    let proxy = ProxyCell::new();
    // 谁有 key 就用谁。**默认先试 OpenAI**——设置页上它是第一个选项。
    let order: [Provider; 3] = [Provider::OpenAi, Provider::Anthropic, Provider::MiniMax];

    let build = |group: ModelGroup| -> Option<(HttpReasoner, ModelCell)> {
        for provider in order {
            let Some(key) = secret::load(group, provider.key()) else {
                continue;
            };
            let Ok(route) = chatservant_reason::Route::preset(provider) else {
                continue;
            };
            // 模型格子**先建一份**，推理器和命令层各拿一个 clone ——
            // 两边指的是同一格，所以设置页改完下一次生成就用新的。
            let model = ModelCell::new(route);
            // ⚠ **起不起得来传输不是「配好了」的同义词。** genai 那条路要起一个
            // tokio runtime，那是真会失败的一步。失败就**退回预置传输**，
            // 不能让整个面板起不来——而预置候选每条都带自曝标记，
            // 用户不会把它当成模型输出。
            match HttpReasoner::with_config(model.clone(), key, proxy.clone()) {
                Ok(r) => return Some((r, model)),
                Err(_) => {
                    tracing::error!(
                        "provider={} 有 key，但装不起真实传输——这一组退回预置传输",
                        provider.key()
                    );
                    return None;
                }
            }
        }
        None
    };

    let llm = build(ModelGroup::Llm);
    // 决策组优先用自己的；没有就**沿用回复组那一路**（同一个格子）。
    let decision_owned = build(ModelGroup::Decision).or_else(|| llm.clone());
    let decision_cell = decision_owned
        .as_ref()
        .map(|(_, c)| c.clone())
        .unwrap_or_else(|| {
            ModelCell::new(
                chatservant_reason::Route::preset(Provider::OpenAi).expect("OpenAI 有预设端点"),
            )
        });
    let llm_cell = llm
        .as_ref()
        .map(|(_, c)| c.clone())
        .unwrap_or_else(|| decision_cell.clone());

    let transport_ready = llm.is_some();
    // ⚠ 计数要**覆盖三段全部**：「静默期零调用」那条硬约束断言的就是
    // `calls()`，所以任何一段走了预置传输都要被如实计入（它本来就不发请求）。
    let reasoner: Box<dyn chatservant_reason::Reasoner + Send> = match (&decision_owned, &llm) {
        (Some((d, _)), Some((l, _))) => Box::new(SplitReasoner {
            decision: d.clone(),
            llm: l.clone(),
        }),
        // ⚠ **至少要有回复组**，否则整个程序没有真实传输。
        (Some((d, _)), None) => Box::new(d.clone()),
        (None, Some((l, _))) => Box::new(l.clone()),
        (None, None) => Box::new(PresetReasoner),
    };

    let groups = ModelGroups {
        decision: decision_cell,
        llm: llm_cell,
    };
    let provider = groups.llm.provider();

    (
        LoopConfig {
            source: crate::watch::FrameSource::from_env(),
            ignore_top: IGNORE_TOP,
            reasoner,
            transport_ready,
            recent_n: RecentN::new(chatservant_reason::context::DEFAULT_RECENT),
            locked_peer: LOCKED_PEER.to_string(),
            token_budget: TokenBudget::new(BUDGET_DEFAULT),
            images_out: crate::watch::ImagesOut::off(),
            tier: crate::watch::TierCell::default_manual(),
            relock: crate::watch::Relock::new(),
            provider,
        },
        // ⚠ **没有真实传输就没有模型格子。** 返回 `None` 而不是造一个：
        // 造一个的话设置页会列出一堆「可选模型」，而点下去什么也不会发生
        // ——那正是这个项目一直在删的「调了没反应的控制项」。
        // ⚠ 但 key 输入框**仍然要显示**（见 `group_view`），
        //   因为没配 key 正是用户来这一页的原因。
        transport_ready.then_some(groups),
        proxy,
    )
}

/// **两段式路由**：判断走一组，起草与排序走另一组。
///
/// ## 为什么是「判断 / 起草+排序」这个分法
///
/// 票 43 说的是「决策模型」与「LLM」两组。当前三段式里，判断那一段是
/// 唯一一个「读上下文、输出几十个字」的，起草与排序是「照着口吻写」的那两段——
/// 前者换便宜的模型省一大截钱，后者换好的模型提质量。
///
/// ⚠ 图片跟着**起草**走（`ctx.images` 在 draft 那次请求里），所以
///   **图片外发走的是回复组**，而图片的视觉计费规则也按回复组取。
///
/// ## 计数怎么合
///
/// `calls` 是两边的**和**，而「静默期零调用」那条硬约束断言的正是这个数——
/// 所以它必须覆盖三段全部。分段计数逐项相加，不合并。
struct SplitReasoner {
    decision: chatservant_reason::HttpReasoner,
    llm: chatservant_reason::HttpReasoner,
}

impl chatservant_reason::Reasoner for SplitReasoner {
    fn judge(
        &mut self,
        ctx: &TurnContext,
    ) -> Result<chatservant_reason::Intent, chatservant_reason::ReasonError> {
        self.decision.judge(ctx)
    }

    fn draft(
        &mut self,
        ctx: &TurnContext,
        intent: &chatservant_reason::Intent,
    ) -> Result<chatservant_reason::DraftSet, chatservant_reason::ReasonError> {
        self.llm.draft(ctx, intent)
    }

    fn rank(
        &mut self,
        ctx: &TurnContext,
        draft: &chatservant_reason::DraftSet,
    ) -> Result<Vec<chatservant_reason::Candidate>, chatservant_reason::ReasonError> {
        self.llm.rank(ctx, draft)
    }

    fn calls(&self) -> u32 {
        self.decision.calls() + self.llm.calls()
    }

    fn stage_calls(&self) -> chatservant_reason::StageCalls {
        let d = self.decision.stage_calls();
        let l = self.llm.stage_calls();
        chatservant_reason::StageCalls {
            judge: d.judge + l.judge,
            draft: d.draft + l.draft,
            rank: d.rank + l.rank,
        }
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

    /// **同一条候选在同一轮里发不出去第二次。**
    ///
    /// `arm` 返回 `true` 的意思是「这条本轮已经发过了，**拒绝**」。
    /// 读反了就正好把防重变成鼓励重复发送——2026-10-09 出过一次同型的错：
    /// 判据写成 `tier == Confirm && action == "confirm"`，两次点击条件完全相同，
    /// 于是第二次又命中「装填」分支，**Confirm 档永远发不出去**，
    /// 而回执每次都写着「再点一次就会发」。那是一个明晃晃的假承诺。
    /// （`Confirm` 档本身已在 2026-10-15 砍掉，但**「返回值方向」这个坑**
    /// 跟那一档无关，现在这条守卫的是同一个语义。）
    ///
    /// 这条按用户的手来走：点一下（发出去）→ 再点同一条（被拒）。
    #[test]
    fn the_same_candidate_cannot_be_sent_twice_in_one_round() {
        let mut a = Arming::default();
        assert!(!a.arm("c0", 10), "本轮没发过 ⇒ 放行");
        assert!(a.arm("c0", 10), "本轮已经发过 ⇒ 必须拒绝");
        a.disarm();
        assert!(
            !a.arm("c0", 10),
            "解除防重之后（改档 / 换轮）才允许再发一次"
        );
    }

    /// **换了一条候选，不会把前一条的防重记录一起带走。**
    ///
    /// ⚠ 这条在 2026-10-15 之前是**不成立**的，而它曾经的写法（单槽）
    ///   恰好绕过了它：`id` 只存一条，发 c1 时顺手覆盖掉 c0，再点 c0
    ///   就当成没发过 ⇒ **同一条真消息发了第二遍**。
    ///   重复发撤不回来，所以这里守的是「记一整个列表」那个改动。
    #[test]
    fn dedupe_survives_switching_to_another_candidate() {
        let mut a = Arming::default();
        assert!(!a.arm("c0", 10), "c0 本轮没发过");
        assert!(!a.arm("c1", 10), "c1 是另一条，也该放行");
        assert!(
            a.arm("c0", 10),
            "点回 c0 必须仍然被拒——换了一条候选不该把 c0 的记录冲掉"
        );
        assert!(a.arm("c1", 10), "c1 自己也不许发第二次");
    }

    /// **改档必须清空防重记录。**（2026-10-14 补——这条规则此前只写在注释里）
    ///
    /// `Arming` 的文档写着「换轮了、**档位变了**，都要清空」，
    /// 而实现里换轮那半是 `arm` 自己做的，**改档那半压根没人做**：
    /// `set_tier` 从头到尾没碰过 `armed`。
    ///
    /// 漏出来的路当时长这样（`Confirm` 档，2026-10-15 已砍）：
    /// Auto 档发过候选 A → 切到 Manual → 在微信里改成自己想发的内容 →
    /// 切回 Auto → 再点 A，`arm("A", 同一 ticks)` 返回 true 判成
    /// 「已经发过」而拒绝。人以为程序坏了，其实记录早该在改档那下清掉。
    /// 今天它拦下的是一次重复发送——同一个洞，只是后果换了个方向。
    ///
    /// ⚠ 这条**直接调 `apply_tier`**——那正是 `set_tier` 跑的那段代码。
    ///   第一版的测试是在测试里**复刻**了判断，于是把 `set_tier` 里的
    ///   `disarm` 短路成 `if changed && false`，测试**照样 71 条全绿**——
    ///   **复印件不是守卫**。抽 `apply_tier` 就是为了让测试能碰到真代码。
    ///
    /// ⚠ 仍然只到「改档 → 记录作废」这一层：真机上连着 `STATE`、
    ///   `inject::fill_and_send` 的回执一起验，是票 20 剩的那一项。
    ///
    /// ⚠ 原先这里还有一条 `tier_changed_only_when_the_tier_actually_differs`，
    ///   测的是「`tier_changed(before, d)` 是否等于 `before != d`」——
    ///   而 `tier_changed` **随后被内联进 `apply_tier` 了**，测试就悬空成
    ///   「一个函数存在、但没有任何守卫」的假断言。它守的那件事
    ///   （只在真变更时清记录）**已由本条的正反两面覆盖**：
    ///   真变更 ⇒ 记录作废；同档再设 ⇒ 记录留着。
    ///   ⇒ 删掉它，而不是留着一条测「恒等式」的测试给人「有守卫」的错觉。
    #[test]
    fn changing_tier_disarms() {
        let tier = crate::watch::TierCell::default_manual();
        let mut armed = Arming::default();

        // 从默认的 Manual 升到 Auto —— 这是**一次真变更**。
        assert_eq!(apply_tier(&tier, &mut armed, Dispatch::Auto), "Auto");
        assert!(!armed.arm("c0", 10), "本轮没发过 ⇒ 放行");
        assert!(armed.arm("c0", 10), "已经发过 ⇒ 第二次点击被拒");

        // 切到 Manual —— 防重记录必须作废。
        assert_eq!(apply_tier(&tier, &mut armed, Dispatch::Manual), "Manual");
        assert!(
            !armed.arm("c0", 10),
            "改档之后那份记录必须作废，否则切回来再点一次就等于同一条发两遍"
        );

        // 切回 Auto —— 仍然作废，上一份**不许**被带回来。
        apply_tier(&tier, &mut armed, Dispatch::Auto);
        assert!(!armed.arm("c0", 10), "切回同一档不该把上一份记录复活");

        // ⚠ 反过来也要成立：**档位没真的变就不该作废**。
        //   否则每按一次「设为自动发送」就把本轮记录抹掉，
        //   同一条候选会被反复放行——那正好是这条测试要挡的重复发送。
        let mut b = Arming::default();
        apply_tier(&tier, &mut b, Dispatch::Auto);
        assert!(!b.arm("c1", 7));
        apply_tier(&tier, &mut b, Dispatch::Auto);
        assert!(b.arm("c1", 7), "档位没变时记录要留着");
    }

    /// **解析失败一律落到最低档，且不动防重记录。**
    ///
    /// ⚠ 这条不是废话：`set_tier` 是前端唯一能改档的入口，而前端传什么
    ///   本来就不可信。
    ///
    /// ⚠⚠ **本机 prefs 里很可能还存着 `"Confirm"`** —— 那是 2026-10-15
    ///   之前那一档的名字。它落进 `parse_tier` 的 `_` 分支 ⇒ 读成 **Manual**。
    ///   这个回落方向是**安全的那一侧**：宁可退回「只填入」，
    ///   也不要因为一个认不出的字符串就把程序静默升到「自动发送」。
    ///
    ///   若那时程序正停在 Manual，`apply_tier` 看到「没变」就**不清防重记录**
    ///   ——一次无效输入不该顺手把用户已有的状态抹掉。
    #[test]
    fn an_unparsable_tier_lands_on_manual_without_disarming() {
        assert_eq!(parse_tier("Confirm"), Dispatch::Manual);
        assert_eq!(parse_tier("nonsense"), Dispatch::Manual);
        assert_eq!(parse_tier(""), Dispatch::Manual);
        // ⚠ 而现役的两个 id 都必须解析得出来。
        assert_eq!(parse_tier("Auto"), Dispatch::Auto);
        assert_eq!(parse_tier("Manual"), Dispatch::Manual);

        let tier = crate::watch::TierCell::default_manual();
        let mut armed = Arming::default();
        assert!(!armed.arm("c0", 3));
        // 程序本来就在 Manual，输入 `"Confirm"` 也落到 Manual ⇒ 没变 ⇒ 记录留着。
        apply_tier(&tier, &mut armed, parse_tier("Confirm"));
        assert!(armed.arm("c0", 3), "无效输入不该抹掉已有的防重记录");
    }

    /// **穷举：除了「Auto 档 + confirm 动作」，没有第二条能发出去的路。**
    ///
    /// ⚠ 硬约束第 5 条的守门测试。2026-10-13 之前这个判断写死在 `dispatch`
    ///   的分支里，靠人读顺序保证；早先它就错过一次——`action` 是 `String`，
    ///   拼错的动作会静悄悄落进「只填入」那条路。
    ///
    /// ⚠⚠ **2026-10-15 条件从三个减到两个**：`Confirm` 档取消之后
    ///   「已装填过」不再是独立条件（`Auto` 一步到位，没有上一步）。
    ///   所以**能发出去的那个组合，恰恰就是「不问就发」的那个组合**——
    ///   这正是被砍掉的那道闸，见 `dispatch.rs` 里
    ///   `auto_is_the_one_tier_that_sends_without_asking`。
    #[test]
    fn only_the_auto_tier_clicks_send() {
        let tiers = [Dispatch::Manual, Dispatch::Auto];
        let acts = [Action::Fill, Action::Confirm];
        for tier in tiers {
            for act in acts {
                let expected = matches!((tier, act), (Dispatch::Auto, Action::Confirm));
                assert_eq!(sends_for_real(tier, act), expected, "{tier:?} + {act:?}");
                // 「不该发」的那些组合，一个都不许为真
                if !expected {
                    assert!(
                        !sends_for_real(tier, act),
                        "{tier:?} + {act:?} 不该发得出去"
                    );
                }
            }
        }
        // 钉死基线：能发出去的组合恰好一个
        let mut sending = 0;
        for tier in tiers {
            for act in acts {
                if sends_for_real(tier, act) {
                    sending += 1;
                }
            }
        }
        assert_eq!(sending, 1, "能发出去的组合必须恰好一个");
    }

    /// **停下时同一句话只说一遍**；真的还有下一步动作时才接「下一步」。
    ///
    /// ⚠ `Next::Halt { reason }` 的 `reason` 就是 `receipt.explain()` 本身
    /// （`Dispatcher::next` 原样透传），所以「回执 + 下一步」两段拼起来
    /// 在**大多数回执上**都会重复一次。真机量出来的是：
    /// 「…没有发出去，要发你自己按回车｜下一步：停下等人工：…没有发出去…」
    /// —— 长句被念两遍，直接顶出面板。
    #[test]
    fn a_halt_does_not_say_the_same_thing_twice() {
        let d = Dispatcher::new();

        // 最容易重复的一支：`Filled` 的停下理由就是它自己的 explain()。
        let r = DeliveryReceipt::Filled;
        let n = d.next(r.clone(), 0);
        let said = receipt_detail(Dispatch::Manual, &r, &n);
        assert_eq!(
            said.matches("没有发出去").count(),
            1,
            "同一句只该出现一次：{said}"
        );
        assert!(!said.contains("下一步"), "停下时不该再接「下一步」：{said}");

        // 而真的还有下一步动作时（确定没发出去 ⇒ 可重试），必须说出来。
        let r2 = DeliveryReceipt::Failed("x".into());
        let n2 = d.next(r2.clone(), 0);
        let said2 = receipt_detail(Dispatch::Manual, &r2, &n2);
        assert!(said2.contains("下一步"), "可重试时要说下一步：{said2}");

        // 两支都要带档位——用户得知道这句是在哪一档下发生的。
        assert!(said.contains("仅填入"), "缺档位：{said}");
        assert!(said2.contains("仅填入"), "缺档位：{said2}");
    }

    /// **候选换过之后，上一轮的防重记录立刻作废。**
    ///
    /// `ticks` 前进意味着观察循环出了新一轮 readout，候选集合已经不是
    /// 上一轮那一批了。留着旧记录就是把新一轮也挡掉——用户点了没反应。
    #[test]
    fn an_arming_dies_when_the_readout_advances() {
        let mut a = Arming::default();
        assert!(!a.arm("c0", 10), "第 10 轮没发过 ⇒ 放行");
        assert!(!a.arm("c0", 11), "换轮之后重新放行");
        assert!(a.arm("c0", 11), "第 11 轮里同样只许发一次");
    }

    /// **托盘状态行就是档位，一个字的修饰都不加。**
    ///
    /// 这条锁的是用户 2026-10-10 的判断：菜单是从那个聊天窗口旁边的
    /// 托盘点开的，**会话名就在用户眼前**，复述它不回答任何问题。
    /// 而且它曾是菜单里唯一由 OCR 文本驱动的字符串——一个识别错字
    /// 不该改变界面布局（实测 3 字名 → 菜单 181px，12 字名 → 230px）。
    ///
    /// 会话名仍然完整显示在面板头部；这里只是**不复述**。
    #[test]
    fn the_status_line_is_exactly_the_tier_and_nothing_else() {
        // ⚠ 只列**现役的两档**。「确认后发」是 2026-10-15 砍掉的那一档，
        //   留着它会让人以为面板上还有第三档。
        for tier in ["只填入", "自动发送"] {
            let line = tray_status_line_from(tier);
            assert_eq!(line, tier, "档位被改写了");
            assert!(
                !line.contains('·') && !line.contains(' '),
                "状态行带了修饰：{line:?} —— 会话名就是这么漏回来的"
            );
        }
    }

    /// **首行永远不为空。**
    ///
    /// tooltip 里一个空档位读起来就是「副驾 · chat-servant · 」。
    #[test]
    fn the_status_line_is_never_blank() {
        for tier in ["", "   ", "\t\n"] {
            let line = tray_status_line_from(tier);
            assert!(!line.trim().is_empty(), "空档位 {tier:?} 让状态空了");
            assert_eq!(line, tier_name(Dispatch::Manual));
        }
    }

    /// **`get_tier` / `set_tier` 必须回枚举 id，不能回中文名。**
    ///
    /// 这是 2026-10-10 实机验出来的缺陷：后端回「只填入」，前端按
    /// `type Tier = 'Manual' | 'Confirm'` 去索引 → `undefined`，
    /// 面板头部档位名整格空白，`SegSlider` 也定位不到滑块。
    ///
    /// 判据写死成**前端契约里的字面量**，不是「不是中文就行」——
    /// 前端要改 `Tier` 的话，这个测试就该红，那正是它该红的时候。
    #[test]
    fn the_ipc_tier_is_the_frontend_enum_id_not_a_display_name() {
        for (d, id) in [(Dispatch::Manual, "Manual"), (Dispatch::Auto, "Auto")] {
            assert_eq!(tier_id(d), id, "{d:?} 的 IPC id 与前端 Tier 契约不符");
            assert_eq!(parse_tier(id), d, "{id:?} 解析回来不是同一个档");
            // 显示名是另一条路。它一进来，滑块就又不动了。
            assert_ne!(tier_id(d), tier_name(d), "{d:?} 的 id 和显示名撞了");
        }
    }

    /// ⚠ **被砍掉的 `Confirm` 必须解析成最低档，而不是崩、也不是升档。**
    ///
    /// 2026-10-15 `Confirm` 整档取消（`Auto` 请回来），而**旧 prefs 与旧前端
    /// 还会发 `"Confirm"` 过来**（本机就极可能存着这个值）。
    /// 这时正确的回应是**落到 `Manual`**：用户原本处在「填入后还要再点一次」
    /// 那一档，程序一升级就变成「只填入」，这是**降权**，方向是对的。
    ///
    /// ⚠ 与 `"Auto"` 正好相反：`"Auto"` 现在是**现役**档，必须解析成 `Auto`
    ///   而不是回落——见 `parse_tier`。这条测试守的是「别把两个方向搞反」，
    ///   而搞反的后果正好是静默升档。
    #[test]
    fn a_retired_confirm_id_lands_on_the_lowest_tier() {
        assert_eq!(parse_tier("Confirm"), Dispatch::Manual);
        // 中文名同样不许蒙混过关——旧的「确认后发」也不再是合法输入。
        assert_eq!(parse_tier("确认后发"), Dispatch::Manual);
        // ⚠ 而现役的 `"Auto"` 必须**不能**回落。
        assert_eq!(parse_tier("Auto"), Dispatch::Auto);
    }

    /// **认不出来的档位一律落到最低档，不许升档。**
    ///
    /// 面板刚起来、状态还读不到时会传空串过来；那里不能报错弹红字，
    /// 更不能猜一个更宽松的值。
    ///
    /// ⚠ 列表里那些中文名（`只填入` / `自动发送`）**永远不该被解析成功**——
    ///   `parse_tier` 只认 IPC 契约里的枚举 id。中文是给人看的，
    ///   前端传过来就说明它拿错了字段，这时回落比猜一个更好。
    #[test]
    fn an_unparsable_tier_falls_back_to_manual_never_up() {
        for raw in [
            "",
            "   ",
            "auto",
            "AUTO",
            "自动发送",
            "仅填入",
            "只填入",
            "Manual ",
            "nope",
        ] {
            assert_eq!(parse_tier(raw), Dispatch::Manual, "{raw:?} 不该被升档");
        }
    }

    /// **tooltip 必须带应用名。**
    ///
    /// 早先 `build_tray` 写死一份「副驾 · chat-servant」、`refresh_tray`
    /// 又用状态行整体盖掉它，于是名字在两处各活一份。现在只有一处。
    #[test]
    fn the_tray_tooltip_keeps_the_app_name() {
        let tip = tray_tooltip();
        assert!(tip.starts_with(APP_NAME), "tooltip 丢了应用名：{tip:?}");
        assert!(tip.contains("·"), "tooltip 没有档位那半：{tip:?}");
        assert_eq!(
            tip,
            format!("{APP_NAME} · 仅填入"),
            "状态是全局的，别硬编码"
        );
    }

    /// **托盘红点必须落在角落，且绝不压到图标本体上。**
    ///
    /// ⚠ 红点现在表示「**这一档会发出去**」（`Confirm`）而不是旧的
    /// `Auto`。它要回答的问题是「面板看不见时程序会不会把话发出去」，
    /// 而这个问题在两档之下依然成立。
    ///
    /// 用**真实的底图**跑，不是造一张全零缓冲——图标本体在哪只有真图说了算。
    /// 判据是「红点盖到的像素」与「底图里的**笔迹**」不相交，
    /// 不是钉死某个坐标（圆心/半径是实现细节）。
    ///
    /// ⚠ **「笔迹」按当前图标定义成饱和红，不是「亮」（2026-10-13 换图标时改的）。**
    /// 旧判据是 `rgb 之和 > 150`，配的注释是「深色圆底不算 logo，绿色气泡算」
    /// ——那只对 Tauri 脚手架默认图标成立：那时**亮的是标记、暗的是底**。
    /// 换成「撕边纸 + 红勾」之后正好翻过来：亮的是**纸（本体）**，红的才是**勾（标记）**。
    /// 沿用旧判据会把红点压在纸角上的 8 个像素误判成「盖住了 logo」，
    /// 而实测那 8 个像素是 `(237,219,177)` / `(255,240,193)` 之类的奶白纸色，
    /// **一条红都没有**——红点与红勾的交集是 0 像素。
    /// 所以这里改的是**判据对「标记」的认定**，不是把红线放宽：红点真压到勾上照样红。
    #[test]
    fn the_live_dot_never_covers_the_logo() {
        let base = tray_icon(false).expect("底图解不出来");
        let (w, h) = (base.width(), base.height());
        let src = base.rgba().to_vec();
        let mut dotted = src.clone();
        stamp_live_dot(&mut dotted, w, h);

        assert_eq!(src.len(), dotted.len(), "红点改了缓冲长度");

        let px = |buf: &[u8], x: u32, y: u32| -> [u8; 4] {
            let i = ((y * w + x) * 4) as usize;
            [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
        };
        const RED: [u8; 4] = [0xDC, 0x26, 0x26, 0xFF];

        let mut red = Vec::new();
        let mut touched = 0usize;
        for y in 0..h {
            for x in 0..w {
                let before = px(&src, x, y);
                let after = px(&dotted, x, y);
                if before != after {
                    touched += 1;
                }
                if after == RED {
                    red.push((x, y, before));
                }
            }
        }

        assert!(!red.is_empty(), "红点一个像素都没画出来");
        assert_eq!(touched, red.len(), "除了红点还有别的东西被动过");
        assert!(
            red.len() < (w * h) as usize / 4,
            "红点占了 {} 个像素，缩到 16px 会糊成一坨",
            red.len()
        );

        // ⚠ 底图上这里本来就是**笔迹的一部分**，被红点盖掉就等于毁了图标。
        // 当前的标记是饱和红的勾；奶白纸与深色底都不算笔迹。
        for (x, y, before) in &red {
            let covers_ink = before[3] > 32
                && before[0] > 90
                && (before[0] as i32 - before[1] as i32) > 30
                && (before[0] as i32 - before[2] as i32) > 30;
            assert!(!covers_ink, "红点 ({x},{y}) 盖住了勾的 {before:?}");
            assert!(*x > w / 2 && *y > h / 2, "红点 ({x},{y}) 不在右下角");
        }
    }
}

/// 「档位标记形状」这个落盘设置自己的测试。
#[cfg(test)]
mod mark_shape_tests {
    use super::*;

    /// 一个测试独占的临时目录。
    ///
    /// 用 `std::env::temp_dir()` 而不是引 `tempfile`：落盘设置本来就要写
    /// 文件，为这几个测试加一个 dev-dependency 不值。名字里带进程号，
    /// 这样并发跑 `cargo test` 时不会互相踩。
    fn test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "chat-servant-mark-shape-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录失败");
        dir
    }

    /// **文件不存在时必须读回默认值，而不是报错。**
    ///
    /// 首次启动就是这条路径：设置文件根本还没有。面板在设置页打开之前
    /// 就会问一次标记形状，那里不能抛。
    #[test]
    fn a_missing_mark_shape_file_reads_back_the_default() {
        let dir = test_dir("missing");
        let path = dir.join("mark_shape.txt");
        assert_eq!(read_mark_shape_at(&path), MarkShape::default());
        // 判据写死成 `Tick`，别拿 `default()` 绕回来——那会让默认值和
        // 测试一起被改掉而仍然全绿。
        assert_eq!(read_mark_shape_at(&path), MarkShape::Tick);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **往返：写进去什么就读回什么。**
    ///
    /// 走的是「落盘那一行」那条真实路径（`mark_shape_str` + 文件），
    /// 因为这个设置的全部意义就是「重启后还在」——只在内存里往返等于没验。
    #[test]
    fn a_written_mark_shape_comes_back_as_itself() {
        let dir = test_dir("roundtrip");
        let path = dir.join("mark_shape.txt");
        std::fs::write(&path, mark_shape_str(MarkShape::Ring)).expect("写失败");
        assert_eq!(
            std::fs::read_to_string(&path).expect("回读失败"),
            "ring",
            "落盘的不是那两个小写字面量之一"
        );
        assert_eq!(read_mark_shape_at(&path), MarkShape::Ring);

        std::fs::write(&path, mark_shape_str(MarkShape::Tick)).expect("写失败");
        assert_eq!(read_mark_shape_at(&path), MarkShape::Tick);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **认不出来的内容一律落回 `Tick`，不 panic、不报错。**
    ///
    /// 与 `read_close_behavior` 同一条策略。这里没有托盘那样的「落点失效」
    /// 风险，所以判据是更朴素的一条：面板每次读设置都不该被脏数据卡住。
    #[test]
    fn an_unparsable_mark_shape_falls_back_to_tick() {
        let dir = test_dir("garbage");
        let path = dir.join("mark_shape.txt");
        for raw in [
            "",
            "   ",
            "TICK",
            "Tick",
            "勾",
            "r ing",
            "{\"shape\":\"ring\"}",
            "\u{0}\u{1}",
        ] {
            std::fs::write(&path, raw).expect("写失败");
            assert_eq!(
                read_mark_shape_at(&path),
                MarkShape::Tick,
                "{raw:?} 不该被当成一个形状"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **serde 的名字必须是 `"tick"` / `"ring"` 这两个字面量。**
    ///
    /// 判据钉死在字面量上，与落盘那一份是同一个约定——前端按这两个字符串
    /// 收发（`#[serde(rename_all = "lowercase")]`），改名会让两边同时失配，
    /// 而那种失配在界面上表现为「标记形状永远是默认值」。
    #[test]
    fn the_mark_shape_serde_names_are_lowercase_words() {
        assert_eq!(serde_json::to_string(&MarkShape::Tick).unwrap(), "\"tick\"");
        assert_eq!(serde_json::to_string(&MarkShape::Ring).unwrap(), "\"ring\"");
        // 命令入参走 `Deserialize`，所以反序列化方向也要验。
        assert_eq!(
            serde_json::from_str::<MarkShape>("\"ring\"").unwrap(),
            MarkShape::Ring
        );
        assert_eq!(
            serde_json::from_str::<MarkShape>("\"tick\"").unwrap(),
            MarkShape::Tick
        );
    }

    /// **默认值必须是 `Tick`。**
    ///
    /// 纯外观偏好，改默认值不涉及安全——但它会悄悄改变所有人第一眼看到的
    /// 面板。所以这一条写死，不许靠「重新选一个默认值」顺手改掉。
    #[test]
    fn the_mark_shape_default_is_tick() {
        assert_eq!(MarkShape::default(), MarkShape::Tick);
        // 连落盘那一行一起钉住，避免默认值与 `mark_shape_str` 同侧漂移。
        assert_eq!(mark_shape_str(MarkShape::default()), "tick");
    }
}
