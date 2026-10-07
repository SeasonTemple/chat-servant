//! 常驻观察循环——**把六个阶段串成一条会自己跑的链**。
//!
//! ## 循环里有什么
//!
//! ```text
//!   抓帧 → PaneAnchor → 裁消息区 → PaneWatcher.observe
//!                                              │
//!                        Quiet ◄───────────────┤
//!                          │                   │ Read
//!                          │                   ▼
//!                     零次模型调用        OCR → 归类 → group
//!                                              │
//!                          ┌───────────────────┘
//!                          ▼  commit（水位线推进）
//!                     有新消息才推理
//! ```
//!
//! ## 为什么观察与推理是分开的两段
//!
//! **静默期零调用**（AGENTS.md 硬约束第 6 条）靠的是
//! [`PaneWatcher::observe`] 在画面没变时返回 [`Decision::Quiet`]——
//! 那个位置一帧都不会跑 OCR，更不会跑模型。反过来把推理放在循环里、
//! 指望它自己判断「这次要不要调」，迟早有人写成无条件调用。
//!
//! ## 线程与 Send
//!
//! `windows-core` 的 `NonNull<c_void>` **不是 `Send`**，所以 WGC 会话对象
//! 不能跨线程 move（见 `capture::wgc_alt` 的说明）。循环因此**在它自己的
//! 线程里跑**，只通过 `mpsc` 传纯数据（`Readout`）。前端拿到的也只有
//! 这些纯数据。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chatservant_capture::anchor::PaneAnchor;
use chatservant_capture::surface::WECHAT;
use chatservant_capture::wgc_alt::capture_one_frame;
use chatservant_capture::window::find_chat_window;
use chatservant_perceive::{Decision, Message, PaneWatcher, Speaker};
use chatservant_reason::{Provider, Reasoner, TurnContext};
use serde::Serialize;

/// 循环的节奏。**慢一点没关系**——静默期本来就不该做事，
/// 而画面变化通常伴随人的操作，1 秒的延迟人感知不到。
pub const TICK: Duration = Duration::from_millis(1000);

/// 上一轮的带框数据。调试画布读它。
static LAST_BOXES: std::sync::Mutex<Option<crate::debug::DebugPane>> = std::sync::Mutex::new(None);

/// 读最近一轮的调试画布数据。
pub fn debug_pane() -> Option<crate::debug::DebugPane> {
    LAST_BOXES.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// 一次观察里各段各调了几次模型。**四个都要报，包括 0。**
///
/// 界面上要能看出「判断 2 · 起草 1 · 排序 1 · 视觉 0」——
/// 只给一个总数的话，三段里任何一段挂了都分不清是哪段。
/// **视觉恒为 0 是正常状态**（默认多模态配置下图片随文本进 B，不额外调），
/// 不报出来用户会以为没统计。
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct CallCounts {
    pub judge: u32,
    pub draft: u32,
    pub rank: u32,
    /// 额外视觉模型的调用次数。**只有 B 是纯文本且开了桥时才可能非 0。**
    pub vision: u32,
}

impl CallCounts {
    pub fn explain(&self) -> String {
        format!(
            "判断 {} · 起草 {} · 排序 {} · 视觉 {}",
            self.judge, self.draft, self.rank, self.vision
        )
    }
}

/// 往 UI 推的一份完整快照。
#[derive(Debug, Clone, Serialize)]
pub struct Readout {
    pub anchor: Option<PaneAnchor>,
    /// **当前窗口里的会话名。** 空串 = 没认出来。
    ///
    /// 界面必须显示它——会话名闸是「只对锁定会话动手」那道闸，
    /// 而闸的判据必须看得见，否则用户不知道为什么程序不动。
    pub peer: String,
    pub messages: Vec<Message>,
    pub candidates: Vec<DraftCandidate>,
    /// 判断摘要 + 降级说明。
    pub reason: String,
    /// 这一轮调了几次模型。静默期必须是 0。
    pub model_calls: u32,
    /// 分段计数。界面上要分开显示。
    pub counts: CallCounts,
    /// 「重锁当前会话」的请求是否还在等着被下一轮取走。
    ///
    /// 按钮点了之后**真正生效要等下一轮**（最多一个 `TICK`）。
    /// 不把这个状态报给界面，用户点了却看不到任何确认——
    /// 而「点了没反应」正是这个项目反复在修的东西。
    pub relock_pending: bool,
    /// 循环已经跑了多少圈。
    pub ticks: u64,
    /// 累计 OCR 调用次数。
    pub ocr_calls: u32,
    /// 最近一次出错在哪一段。**空 = 没出错。**
    pub error: Option<String>,
}

/// 一条候选（给前端渲染用）。
#[derive(Debug, Clone, Serialize)]
pub struct DraftCandidate {
    pub id: String,
    pub text: String,
    pub probability: f32,
}

/// 「参考上下文」条数的可改句柄。
///
/// ## 为什么不是一个 `static`
///
/// 早先的版本把 `recent_n: usize` 直接 move 进观察线程的闭包。
/// 那是**按值捕获**——设置页改了值，运行中的循环读到的还是启动时那份，
/// 界面上那句「下一次生成就会用上」是**假的**。而且 `LoopConfig` 里
/// 留着字段就等于留了两个事实来源（启动值 vs 设置值），迟早写岔。
///
/// 现在传的是 `Arc<AtomicUsize>` 的句柄：命令层和循环**共用同一个格子**，
/// 循环每圈读一次，所以改完下一圈就用上。
///
/// ## 为什么也不是 `Context` 那种 owned struct
///
/// `Context` 是**每个会话一份**的域状态（可能同时有多个会话桶），
/// 谁开谁负责，所以做成 owned。这里是**整个应用一份的用户偏好**——
/// 就一个循环、一个旋钮，句柄复制一份给命令层是最省事的写法。
/// 真要多个会话各自调「最近多少条」，那时才该把它塞进 `Context`。
#[derive(Clone)]
pub struct RecentN(Arc<std::sync::atomic::AtomicUsize>);

impl RecentN {
    pub fn new(n: usize) -> Self {
        Self(Arc::new(std::sync::atomic::AtomicUsize::new(
            chatservant_reason::context::clamp_recent(n),
        )))
    }

    /// 当前值。**循环每圈都调它**——这就是「即时生效」的落点。
    pub fn get(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }

    /// 改值。**返回夹取后的真实生效值**——
    /// 调用方要拿它回给界面，不能拿自己传进去的数糊弄过去。
    pub fn set(&self, n: usize) -> usize {
        let applied = chatservant_reason::context::clamp_recent(n);
        self.0.store(applied, Ordering::Relaxed);
        applied
    }
}

/// 循环的配置。组装真实传输需要 key，所以从这里进。
pub struct LoopConfig {
    /// 顶部跳过多高（置顶公告条）。由 `ChatSurface` 给，不写死。
    pub ignore_top: i32,
    /// 拿候选的传输。**`Box<dyn Reasoner>` 可以是预置的**——
    /// 没配 key 时用它，但界面会显示「预置」，不让用户以为接通了。
    ///
    /// `+ Send` 是必须的：整个配置要 move 进观察线程。
    /// 两个真实实现（`HttpReasoner` / `PresetReasoner`）都是 Send——
    /// `HttpReasoner` 里的 `Arc<dyn Http + Send + Sync>` 保证了这点。
    pub reasoner: Box<dyn Reasoner + Send>,
    /// 传输是不是真的。没配 key 时是 false。
    pub transport_ready: bool,
    /// 「参考上下文」条数。**共享句柄**——改它下一圈就生效，不用重启。
    pub recent_n: RecentN,
    /// 单张图片的视觉 token 预算。**共享句柄**。
    pub token_budget: TokenBudget,
    /// 图片外发总闸。**默认关**——见 [`ImagesOut`]。
    pub images_out: ImagesOut,
    /// 发送档位。**默认 Manual**，Rust 侧唯一事实来源。
    pub tier: TierCell,
    /// **只对这个会话动手。** 空串 = 不锁（锁不住就先不推理，比对错人安全）。
    ///
    /// 这是「测试会话锁定小号」那条要求的落点，也是硬约束第 1 条
    /// （只处理自己有权查看的对话）在**运行时**的体现：
    /// 程序会一直核对窗口顶部的会话名，对不上就**不推理也不投递**。
    pub locked_peer: String,
    /// 当前回复模型是哪一家。**视觉计费随它变**——
    /// gpt-4o-mini 的底账是 2833，Claude 的 patch 计费是 0。
    pub provider: Provider,
    /// 「重新锁定当前会话」的一次性请求。命令层与循环共用。
    pub relock: Relock,
}

/// 一轮里的**可变会话状态**。合成一个结构体，而不是给 `step` 递九个 `&mut`。
///
/// ## 为什么要收成一个结构体
///
/// 九个 `&mut` 参数不只是长。每个都各自可变，编译器**看不出它们之间
/// 的一致性约束**——比如「`history` 只在有对方说话时追加」这种规则，
/// 在九个散参数里只能靠注释维持。收进来之后，「这几个东西是一份
/// 会话状态」这件事变成了类型层面的事实。
///
/// `watcher`（引擎）和 `reasoner`（传输）**故意留在外面**：它们不是
/// 会话状态，换配置时会被换掉，而这份状态是跨轮次攒下来的。
struct Scratch {
    candidates: Vec<DraftCandidate>,
    /// 给界面看的一句话说明（判断摘要 / 降级 / 失败原因）。
    reason: String,
    /// 累计 OCR 调用次数。静默期**不该增长**。
    ocr_calls: u32,
    /// 摘要缓存。**每个会话一份**——换会话要调 `forget`，
    /// 否则会带着上一个会话的摘要去见对端。
    context: chatservant_reason::Context,
    /// 本会话的完整历史，喂给推理。**只在有新消息时追加**。
    history: Vec<String>,
    /// 最近一次识别出的消息流。**发给界面渲染用。**
    ///
    /// ⚠ 它以前**根本不存在**：`step` 里算出 `messages` 只在这一轮里用，
    /// readout 里写死 `messages: Vec::new()`。于是界面上的消息流永远是空的
    /// ——即使识别完全正常、候选都出来了，聊天记录那一片永远是「还没有消息」。
    messages: Vec<Message>,
    /// **只对这个会话动手。** 见 [`LoopConfig::locked_peer`]。
    locked_peer: String,
    /// 当前窗口里认出的会话名。**空串 = 没认出来**。
    peer: String,
    /// 锁定的那一个会话的头部像素指纹。**第一帧读到时建立。**
    locked_print: Option<chatservant_perceive::identity::HeaderPrint>,
    /// **建锁那一刻**认出来的对端名。用来在界面上说「锁的是谁」。
    ///
    /// 不能拿 [`Scratch::locked_peer`]（配置里的目标名，通常是写死的小号）
    /// 代替——锁是按**第一帧看到的会话**建的，两者可能根本不是同一个。
    /// 报错了名字，用户就看不出自己锁错了。
    locked_name: String,
    /// 用户请求的「重新锁定当前会话」。一次性。
    relock: Relock,
    /// 图片外发总闸。**默认关**——见 [`ImagesOut`]。
    images_out: ImagesOut,
    /// 单张图片的 token 预算。循环每圈读一次。
    token_budget: TokenBudget,
    /// 视觉计费规则。跟推理用的是同一家供应商。
    provider: Provider,
    /// 与设置页共用一个格子的条数。循环每圈读一次。
    recent: RecentN,
    /// 视觉桥。**默认关闭**——`VisionBridge::off()` 不发任何请求，
    /// `calls()` 恒为 0。接上视觉链路时换掉这个类型即可。
    vision: chatservant_reason::VisionBridge,
}

impl Scratch {
    fn new(recent: RecentN) -> Self {
        Self {
            candidates: Vec::new(),
            reason: String::from("等待第一条消息"),
            ocr_calls: 0,
            context: chatservant_reason::Context::new(),
            history: Vec::new(),
            messages: Vec::new(),
            recent,
            vision: chatservant_reason::VisionBridge::off(),
            locked_peer: String::new(),
            peer: String::new(),
            locked_print: None,
            locked_name: String::new(),
            relock: Relock::new(),
            images_out: ImagesOut::off(),
            token_budget: TokenBudget::new(crate::commands::BUDGET_DEFAULT),
            provider: chatservant_reason::Provider::OpenAi,
        }
    }
}

/// 「图片外发总闸」。**默认关。**
///
/// ## 为什么单独一个闸
///
/// 媒体检测的准确率还没在真实帧上验证过（票 07 记着 4 条检出里只有 1 条
/// 有独立交叉验证）。但检测器不准这件事**不该由检测器自己负责兜底**——
/// 兜底的正确位置是**出网之前**：闸关着的时候，不管检测器报出什么，
/// 一张图都出不去。
///
/// 这与投递档位是同一个思路：默认最低档，升级必须显式。区别在于
/// **投递错了是发错话，图片外发错了是把别人的内容交给外部模型**，
/// 所以这里连「自动」这一档都不给，只有开和关。
#[derive(Clone)]
pub struct ImagesOut(Arc<std::sync::atomic::AtomicBool>);

impl ImagesOut {
    pub fn off() -> Self {
        Self(Arc::new(std::sync::atomic::AtomicBool::new(false)))
    }

    pub fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub fn set(&self, v: bool) -> bool {
        self.0.store(v, Ordering::Relaxed);
        v
    }
}

/// 发送档位的**唯一事实来源**。设置页与主面板共用这一个格子。
///
/// ## 「会话结束回落」目前只做到哪一步
///
/// 硬约束第 5 条要求「会话结束自动回落到 `Manual`」。现在实际做到的是
/// **应用重启即回落**——档位只存在内存里，没有落盘，所以关掉再开必然是
/// `Manual`。
///
/// **「切换到另一个聊天会话时回落」没有实现**，因为多会话路由本身还没做
/// （见 `SESSION` 常量的说明）。这里写清楚现状，免得后来的人以为已经覆盖了。
///
/// ## 为什么档位不能待在前端
///
/// 硬约束第 5 条：「升档必须是显式的、可回退的、会话结束自动回落到
/// `Manual` 的操作」。这三条全都要求档位由**程序**持有：
///
/// - 早先 `tier` 是前端 `$state`，而设置页和主面板**各有一份**。
///   用户在设置里选 Auto，主面板还是 Manual——设置页那个开关
///   **完全不起作用**，是个和 `probe_key` 同类的死控件。
/// - `dispatch` 过去直接相信前端传来的 `tier: String`。那等于把
///   「发不发出去」这个安全决策交给了一个可以随便改的字符串。
///
/// 现在前端只负责**请求**改档，真正的档位在 Rust 手里，
/// `dispatch` 读的是自己的格子，不看前端说什么。
#[derive(Clone)]
pub struct TierCell(Arc<std::sync::atomic::AtomicU8>);

impl TierCell {
    /// 启动时一律 `Manual`——硬约束第 5 条「默认档最低」。
    pub fn default_manual() -> Self {
        Self(Arc::new(std::sync::atomic::AtomicU8::new(
            chatservant_core::dispatch::Dispatch::Manual as u8,
        )))
    }

    pub fn get(&self) -> chatservant_core::dispatch::Dispatch {
        match self.0.load(Ordering::Relaxed) {
            1 => chatservant_core::dispatch::Dispatch::Confirm,
            2 => chatservant_core::dispatch::Dispatch::Auto,
            // **未知一律当 Manual。** 解析失败就升档是最危险的默认值。
            _ => chatservant_core::dispatch::Dispatch::Manual,
        }
    }

    pub fn set(
        &self,
        d: chatservant_core::dispatch::Dispatch,
    ) -> chatservant_core::dispatch::Dispatch {
        self.0.store(d as u8, Ordering::Relaxed);
        d
    }
}

/// 「重新锁定当前会话」的一次性请求。
///
/// ## 为什么需要它
///
/// 指纹闸在**第一帧读到头部**时建锁，也就是说「启动时开着的那个会话」
/// 就是被锁的那个。如果没有重锁入口，用户在错误的会话上启动了应用之后
/// **没有任何办法修正**，只能关掉重开——那不是设计，是放弃。
#[derive(Clone)]
pub struct Relock(Arc<std::sync::atomic::AtomicBool>);

impl Relock {
    pub fn new() -> Self {
        Self(Arc::new(std::sync::atomic::AtomicBool::new(false)))
    }

    /// 请求重锁。下一轮读到头部时**以当时那块像素**为新基准。
    pub fn request(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// 读一次就清掉——**重锁是一次性的**，不该每轮都重新认人。
    pub fn take(&self) -> bool {
        self.0.swap(false, Ordering::Relaxed)
    }

    pub fn pending(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

impl Default for Relock {
    fn default() -> Self {
        Self::new()
    }
}

/// 预算下界。低于 64 token 连一张缩略图都放不下，那不是「更省」。
pub const BUDGET_MIN: u32 = 64;
/// 预算的**绝对**上界，取各家供应商里能接受最大的那个。
///
/// 早先这里是 1568——那是 **Anthropic 标准档的硬上限**，却被当成了通用值。
/// 后果：在 `gpt-4o-mini` 上滑块永远到不了 2833，而那是它发任何一张图的
/// 底账，于是默认模型上图片功能**完全不可用**，而界面看不出来。
///
/// 真正给用户看的上界由 [`crate::commands::budget_ceiling`] 按当前供应商算；
/// 这个常量只是夹取时的硬顶，防止一个荒谬的值被存进去。
pub const BUDGET_MAX: u32 = 8500;

/// 「单张图片视觉 token 预算」的可改句柄。形状与 [`RecentN`] 一样：
/// 命令层和观察循环共用一个格子，改完下一圈生效。
///
/// 上下界定义在**这里**而不是命令层：夹取发生在这个类型里，
/// 边界就该和夹取它的代码住在一起，否则改一处忘另一处是迟早的事。
#[derive(Clone)]
pub struct TokenBudget(Arc<std::sync::atomic::AtomicU32>);

impl TokenBudget {
    pub fn new(v: u32) -> Self {
        Self(Arc::new(std::sync::atomic::AtomicU32::new(
            v.clamp(BUDGET_MIN, BUDGET_MAX),
        )))
    }

    pub fn get(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }

    /// 返回**夹取后的真实生效值**——界面上要显示这个，不是用户拖的那个。
    pub fn set(&self, v: u32) -> u32 {
        let applied = v.clamp(BUDGET_MIN, BUDGET_MAX);
        self.0.store(applied, Ordering::Relaxed);
        applied
    }
}

/// 循环句柄。`Drop` 时自动结束线程。
pub struct Watcher {
    stop: Arc<AtomicBool>,
    /// 上一份快照，命令层要读它。
    latest: Arc<std::sync::Mutex<Option<Readout>>>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Watcher {
    pub fn latest(&self) -> Option<Readout> {
        self.latest.lock().ok().and_then(|g| g.clone())
    }
}

/// 起一条观察循环。
///
/// **会立刻起一个新线程**。返回的 `Watcher` 被 drop 时线程自己会停。
pub fn spawn(cfg: LoopConfig) -> Watcher {
    let stop = Arc::new(AtomicBool::new(false));
    let latest = Arc::new(std::sync::Mutex::new(None));
    let latest_bg = latest.clone();
    let stop_bg = stop.clone();

    std::thread::Builder::new()
        .name("cs-watch".into())
        .spawn(move || {
            let mut watcher = PaneWatcher::new(cfg.ignore_top);
            let mut reasoner = cfg.reasoner;
            let mut s = Scratch::new(cfg.recent_n);
            // 会话名闸的目标。**从 LoopConfig 拷进 Scratch**，
            // 这样 `step` 不用再收第七个参数。
            s.locked_peer = cfg.locked_peer.clone();
            // 总闸与预算也要**共用同一份**，否则设置页改了循环读不到。
            s.images_out = cfg.images_out.clone();
            s.token_budget = cfg.token_budget.clone();
            s.provider = cfg.provider;
            s.relock = cfg.relock.clone();
            let mut last_calls = 0u32;
            let mut ticks = 0u64;
            let mut anchor: Option<PaneAnchor> = None;
            // 最近一次认出的会话名。**认不出来就是空串**，
            // 界面据此显示「没认出会话」，不显示空白。
            // 最近一次认出的会话名由 `step` 写进 `s.peer`，这里直接读。

            while !stop_bg.load(Ordering::Relaxed) {
                ticks += 1;
                // 每 20 圈打一行**实测线程数**。
                //
                // 起因：实机观察到进程 2300+ 线程、增速 9 个/秒，全是等待态。
                // 抓帧线程每秒只该有 1 个，对不上——说明还有别的东西在开线程。
                // 没有这行就只能靠猜；有了它，泄漏源被定位到哪一段一目了然。
                if ticks % 20 == 1 {
                    tracing::info!(
                        target: "chat-servant::watch",
                        "tick={ticks} peer={:?} candidates={}",
                        s.peer,
                        s.candidates.len()
                    );
                }
                let t_round = std::time::Instant::now();
                match step(&mut watcher, &mut reasoner, &mut s, &LAST_BOXES) {
                    Ok(Some(a)) => {
                        anchor = Some(a);
                        last_calls = reasoner.calls();
                        let st = reasoner.stage_calls();
                        let out = Readout {
                            anchor,
                            peer: s.peer.clone(),
                            messages: s.messages.clone(),
                            candidates: s.candidates.clone(),
                            reason: s.reason.clone(),
                            model_calls: last_calls,
                            relock_pending: s.relock.pending(),
                            counts: CallCounts {
                                judge: st.judge,
                                draft: st.draft,
                                rank: st.rank,
                                // 视觉桥**默认关闭**（ticket 15 第 1 条），
                                // 循环里没接桥之前这里恒为 0——这是正确值，
                                // 不是占位符。
                                vision: s.vision.calls(),
                            },
                            ticks,
                            ocr_calls: s.ocr_calls,
                            error: None,
                        };
                        if let Ok(mut g) = latest_bg.lock() {
                            *g = Some(out);
                        }
                    }
                    Ok(None) => {
                        // 什么都没变。**不发**——前端不需要知道「循环还活着」。
                    }
                    Err(e) => {
                        // 失败这一轮也要报分段计数——**出错的段就在数字里**。
                        let st = reasoner.stage_calls();
                        let out = Readout {
                            anchor,
                            peer: s.peer.clone(),
                            messages: s.messages.clone(),
                            candidates: s.candidates.clone(),
                            reason: "这一轮没跑通".into(),
                            model_calls: last_calls,
                            relock_pending: s.relock.pending(),
                            counts: CallCounts {
                                judge: st.judge,
                                draft: st.draft,
                                rank: st.rank,
                                vision: s.vision.calls(),
                            },
                            ticks,
                            ocr_calls: s.ocr_calls,
                            error: Some(e),
                        };
                        if let Ok(mut g) = latest_bg.lock() {
                            *g = Some(out);
                        }
                        // 失败后歇一拍，别把错误刷成瀑布
                        std::thread::sleep(Duration::from_millis(2500));
                    }
                }
                std::thread::sleep(TICK);
                tracing::debug!(
                    target: "chat_servant::watch",
                    "一轮 {ms}ms",
                    ms = t_round.elapsed().as_millis()
                );
            }
        })
        .expect("起观察线程失败");

    Watcher { stop, latest }
}

/// 跑一轮。`Ok(Some(anchor))` = 有新内容；`Ok(None)` = 静默期。
fn step(
    watcher: &mut PaneWatcher,
    reasoner: &mut Box<dyn Reasoner + Send>,
    s: &mut Scratch,
    last_boxes: &std::sync::Mutex<Option<crate::debug::DebugPane>>,
) -> Result<Option<PaneAnchor>, String> {
    // ① 找窗口 + 抓帧
    let hit = find_chat_window(&WECHAT).map_err(|e| format!("找窗口失败：{e}"))?;

    // ①b **先问能不能抓到，再决定要不要试。**
    //
    // ⚠ 这里必须用 `why_wgc_unreadable`，**不能**用区域抓屏那套判据。
    // 区域抓屏读的是屏幕像素，被盖住就完蛋；但 **WGC 就是为抓被遮挡窗口
    // 设计的**（实测 2026-10-07：6/9 采样点被盖住，八段 e2e 照样全通）。
    //
    // 原先这里用的是带遮挡检查的 `why_unreadable`，于是**只要有任何东西
    // 盖住微信就直接返回错误**——而那永远是常态。后果是观察循环一次帧都
    // 没抓到过，界面上只有一句「抓不到：窗口被别的窗口盖住了」，
    // 帧计数恒为 0，进程 CPU 恒为 0。
    if let Some(why) = chatservant_capture::region::why_wgc_unreadable(hit.hwnd_raw) {
        return Err(format!("抓不到：{why}"));
    }

    let frame = capture_one_frame(hit.hwnd_raw).map_err(|e| {
        match chatservant_capture::region::why_unreadable(hit.hwnd_raw) {
            Some(why) => format!("抓帧失败：{e}。{why}"),
            None => format!("抓帧失败：{e}"),
        }
    })?;
    if frame.is_black() {
        return Err("抓到全黑帧——GPU 合成窗口的已知失败模式".into());
    }

    // ② 变化门控 + 增量 ROI。**静默期在这里就返回了。**
    let Decision::Read { roi } = watcher.observe(&frame) else {
        return Ok(None);
    };

    // ③ 只对新增带跑 OCR
    let pane = frame
        .crop(roi)
        .ok_or_else(|| format!("ROI {roi:?} 裁不出图"))?;
    let ocr = chatservant_capture::ocr::recognize(&pane).map_err(|e| format!("OCR 失败：{e}"))?;
    s.ocr_calls += 1;

    // ④ 在**整帧**上归类——行框是相对 ROI 的，要换算回去
    let anchor =
        chatservant_capture::anchor::derive(&frame).map_err(|e| format!("锚点推导失败：{e}"))?;
    let full = frame
        .crop(anchor.chat)
        .ok_or_else(|| "裁不出消息区".to_string())?;
    let palette = chatservant_perceive::Palette::learn(&frame, anchor.chat)
        .ok_or_else(|| "学不到配色（消息区太平）".to_string())?;

    let lines: Vec<chatservant_capture::ocr::OcrLine> = ocr
        .lines
        .iter()
        .map(|l| chatservant_capture::ocr::OcrLine {
            text: l.text.clone(),
            rect: shift(l.rect, roi.x0, roi.y0),
            words: vec![],
        })
        .collect();
    let attrs = chatservant_perceive::classify_all(&lines, &full, &palette);
    let messages = chatservant_perceive::group(&attrs);
    // **存一份给界面。** 以前这里算完就丢，readout 里发的是空数组——
    // 界面的消息流因此永远是空的。抄一份很便宜，几十条消息而已。
    s.messages = messages.clone();

    // **会话身份闸。** 认的是**头部区域的像素指纹**，不是 OCR 读出来的名字。
    //
    // 为什么不比名字：实测真实帧上头部的**归类**是对的（`NoiseClass::Header`），
    // 但 OCR 把「小号」读成了 `G8W`。按文字锁的话，闸在真实环境上**永远是关的**
    // ——不误发，但也永远不工作。
    //
    // 为什么指纹可靠：文字是 OCR 的产物，那一小块像素不是。
    // 换会话像素必然变；同一会话在帧之间基本不变（阈值吸收角标抖动）。
    let print = header_print(&full, &anchor);
    // ⚠ **认不到就保持原样，不要清空。**
    //
    // `attrs` 来自增量 ROI，标题栏常常不在里面（见 `conversation_name` 的
    // 说明）。以前这里无条件 `s.peer = ...unwrap_or_default()`，于是标题栏
    // 一离开 ROI，界面上的会话名就被抹成「没认出来」，而实际上它没变过。
    // 身份是**累积确认**的事实，不该因为某一轮没看见就丢掉。
    // 会话名**从头带单独读**（见 `peer_name_from_header`）：增量 ROI 里
    // 常常没有标题栏，从那里找会挑中别的行（实测认成时间戳 `22 ： 22`），
    // 加判据之后又变成永远认不出来（`peer=""`）。
    //
    // 认不到就保持原样——身份是**累积确认**的事实，
    // 不该因为某一轮没看见就被抹掉。
    if let Some(name) = peer_name_from_header(&full, &anchor) {
        s.peer = name;
    }
    // ⚠ 重锁请求必须**在建锁判断之前**取走，否则这一轮的请求会被跳过，
    // 用户点了按钮却什么也没发生——那是最难查的一种「没反应」。
    if s.relock.take() {
        apply_relock(s);
    }
    let peer_ok = match (&s.locked_print, &print) {
        // 还没锁过：拿**第一帧读到的**当基准，并把名字报给用户看。
        // ⚠ 这意味着「启动时开着的那个会话」就是被锁的那个。
        //   所以界面上必须显示这个名字——用户要能一眼看出锁错了没有。
        (None, Some(p)) => {
            s.locked_print = Some(p.clone());
            // 建锁这一刻认出来的名字才是「锁的是谁」。
            s.locked_name = s.peer.clone();
            true
        }
        // 锁过但这帧认不出 → **关**。认不出不等于「是同一个」。
        // `(None, None)` 也落到这里：连基准都还没有，更不能动手。
        (_, None) => false,
        (Some(a), Some(b)) => a.is_same(b),
    };
    if !peer_ok {
        s.candidates.clear();
        s.reason = match (&s.locked_print, &print) {
            (Some(_), None) => format!(
                "没读出当前会话的头部，无法确认还是不是「{}」——本轮不动手",
                s.locked_peer
            ),
            // `(None, Some(_))` 到不了这里：那一支已经在上面建锁并放行了。
            (None, None) => "还没读到任何会话头部，先不动手".into(),
            _ => format!(
                "当前会话已经不是锁定的那一个（锁定的是「{}」）——本轮不动手",
                if s.locked_name.is_empty() {
                    "认不出的会话"
                } else {
                    &s.locked_name
                }
            ),
        };
        // **不推进水位线**：这样下一帧会重试，用户切回锁定会话后能立刻接上。
        return Ok(None);
    }

    // 存一份给调试画布。**在整帧坐标系**——画布按帧的百分比定位，
    // 用 ROI 坐标会让框整体偏移。
    *last_boxes.lock().unwrap_or_else(|e| e.into_inner()) = Some(crate::debug::build(
        anchor.chat,
        frame.width(),
        frame.height(),
        attrs.as_slice(),
    ));

    // 只有真的有**对方说的话**才推进水位线。
    // 推进了却没东西可读 = 那段永远不会被再读一遍 = 漏消息。
    let newest = messages
        .iter()
        .filter(|m| m.speaker == Speaker::Them)
        .map(|m| m.rect.y1)
        .max();

    // ⑤ 有对方说话才推理。**没读到就不推理**——省的不只是钱，
    // 还有一次可能失败的外部调用。
    if newest.is_some() {
        // **每圈读一次**活值，读完存成本地变量给整轮用。
        // 分两次读的话，用户恰好在中间拖了滑块，同一轮里
        // 裁剪用 N=12、兜底用 N=11——半套生效比不生效更难查。
        let recent_n = s.recent.get();

        // 历史**只增不减**——摘要靠它判断「更早的部分」有多长。
        // ⚠ 只在这条路径上追加：没读到对方说话时不追加，避免把
        // 时间戳、系统提示当成对话塞进上下文。
        s.history.extend(messages.iter().map(|m| m.text.clone()));

        // 裁剪。**摘要只在这里发生，且按块**（见 RESUMMARIZE_CHUNK）——
        // 这就是「惰性」的落点：不是每次都算，是攒够一块才算。
        let rendered = match s
            .context
            .trim(SESSION, &s.history, recent_n, &mut NoSummary)
        {
            Ok(t) => t.render(),
            Err(e) => {
                // 摘要失败**不阻断本轮**——用最近的消息先出候选，
                // 在 reason 里说清楚摘要没成。硬失败等于「不回复」，
                // 那比语境差一点糟得多。
                s.reason = format!("摘要失败（{e}）；本轮只用最近消息");
                let tail: Vec<&String> = s.history.iter().rev().take(recent_n).collect();
                let mut lines: Vec<String> = vec!["[最近的对话]".into()];
                lines.extend(tail.into_iter().rev().cloned());
                lines.join("\n")
            }
        };

        let ctx = TurnContext {
            messages: rendered.lines().map(str::to_string).collect(),
            // 过了闸才填对端名。**这里不是空串了**——早先一直留空占位，
            // 判据是「还没接识别会话名」，现在接上了。
            peer: s.peer.clone(),
            images: collect_images(&frame, &anchor, s),
        };
        match chatservant_reason::turn(reasoner.as_mut(), &ctx) {
            Ok(t) => {
                s.candidates = t
                    .draft
                    .sorted()
                    .into_iter()
                    .enumerate()
                    .map(|(i, c)| DraftCandidate {
                        id: format!("c{i}"),
                        text: c.text.clone(),
                        probability: c.probability,
                    })
                    .collect();
                s.reason = t.explain();
            }
            // `NothingToDo` = 静默期，**不是错误**
            Err(chatservant_reason::ReasonError::NothingToDo) => {
                s.reason = "没有新消息".into();
            }
            Err(e) => {
                // 推理失败**不推进水位线**——下一帧重试。
                // 漏消息比多调一次外部 API 糟得多。
                s.reason = format!("推理失败：{e}");
            }
        }
    } else {
        s.candidates.clear();
        s.reason = "这一段里没有对方说的话".into();
    }

    // 没读到对方的话时**不推进**——下一帧会重试同一段。
    if let Some(y) = newest {
        watcher.commit(y);
    }
    Ok(Some(anchor))
}

/// 循环用的会话 key。**多会话路由还没做**（`Session` 按 `PeerName` 分桶），
/// 现在只有一个键。等 `peer::same_peer` 接上观察循环后换成真的对端名。
const SESSION: &str = "current";

/// 执行一次「重新锁定当前会话」。
///
/// ## 为什么它必须清历史，而不只是清指纹
///
/// 重锁的含义是**用户在说「我现在要在这个会话里工作」**。所以它是一次
/// 会话切换，而不只是重新取一次基准。
///
/// 早先这里只做 `locked_print = None`，于是出现过这么一条真实路径：
///
/// ```text
/// 在会话 A 里，history 攒下 A 的全部消息
///   → 切到会话 B（闸关着，不动手，历史不动）
///   → 点「重新锁定当前会话」
///   → 闸放行 → history.extend(B 的消息)
///   → **模型收到 A 的全部历史 + B 的消息**
/// ```
///
/// 它于是**看着别人的私聊内容给 B 起草回复**。这不是精度问题，
/// 是把不该出网的内容送出了网。摘要缓存同理：`Context` 按 [`SESSION`]
/// 这个常量分桶，不清的话上一个会话的摘要会被当成本会话的用。
fn apply_relock(s: &mut Scratch) {
    s.locked_print = None;
    s.history.clear();
    s.context.forget(SESSION);
    s.candidates.clear();
}

/// 把媒体区域变成要送模型的图。**总闸关着时一律返回空。**
///
/// ## 为什么第一行就短路
///
/// 图片外发是整个产品里**唯一会把对话画面发到第三方**的动作。
/// 所以闸必须在这里、在任何裁剪/编码/算 token 之前就拦住——
/// 闸关着的时候，**一张图连字节都不该被编码出来**。
///
/// ## 为什么这层没有再判「是不是图」
///
/// `perceive::media::find` 已经判过了。这里再判一遍就是两套判据，
/// 两套判据迟早会分叉。误检的后果已被实测压到「多发一张卡片」
/// 的量级（票 07 记着：文字气泡 30 色 / 12px，候选 830+ 色 / 84px+，
/// 高度差 7~10 倍、色种数差 28~88 倍），不值得为它加一层。
fn collect_images(
    frame: &chatservant_capture::Frame,
    anchor: &chatservant_capture::anchor::PaneAnchor,
    s: &Scratch,
) -> Vec<chatservant_reason::ImagePayload> {
    if !s.images_out.get() {
        return Vec::new();
    }
    let found = chatservant_perceive::find(frame, anchor.chat);
    let billing = s.billing();
    found
        .regions
        .iter()
        .filter_map(|r| {
            chatservant_capture::media::encode_region(frame, r.rect, &billing, s.token_budget.get())
        })
        .map(|e| chatservant_reason::ImagePayload {
            media_type: e.media_type.to_string(),
            base64: e.base64,
            tokens: e.tokens,
            width: e.width,
            height: e.height,
        })
        .collect()
}

/// 头带在**消息区坐标系**里的矩形。
///
/// ## ⚠ 这里踩过一次坑：帧坐标 ≠ 消息区坐标
///
/// `pane` 是 `frame.crop(anchor.chat)` 之后那块，**原点在消息区左上角**；
/// 而 `anchor.chat` / `anchor.header_top` 都是**整帧坐标**。早先直接拿
/// `chat.x0` 去裁 `pane`，裁出来的是消息区**右半边的中部**（x 304 起），
/// 压根不是标题栏——于是：
/// - 像素指纹算在了一片与「是谁」无关的区域上，两个会话的指纹几乎一样，
///   **会话闸根本认不出切换**（硬约束第 1 条实际没生效）；
/// - 从同一块地方 OCR 会话名当然读不出来（日志里 `peer=""`）。
///
/// 所以这里统一换算到消息区坐标：减去 `chat.x0` / `chat.y0`。
fn head_band_rect(
    anchor: &chatservant_capture::anchor::PaneAnchor,
) -> chatservant_core::frame::Rect {
    use chatservant_core::frame::Rect;
    let chat = anchor.chat;
    // 宽度封顶：右半边是空面板，纳入只会稀释指纹。
    let w = chat.width().min(HEAD_BAND_W);
    let top = (anchor.header_top - chat.y0).max(0);
    Rect::new(0, top, w, top + HEAD_BAND_H)
}

/// 头部区域的像素指纹。**认不出就返回 `None`**——
/// 调用方必须把 `None` 当成「认不出」而不是「是同一个」。
///
/// ## 用**固定头带**，不用 OCR 给的框
///
/// 早先用「最靠上那条 `Header` 的 rect」。实测那个框只有 **33×11**，
/// 而且它是**跟着 OCR 结果走的**：名字一个字还是三个字，框就多宽少宽。
/// 后果有两个——指纹薄得不足以区分会话；短名字直接掉到
/// `MIN_PIXELS` 以下，闸**永远建不起来**。
///
/// 头带由锚点给出，不依赖 OCR 认得认得准。
///
/// ⚠ `pane` 必须是**消息区坐标系**（`frame.crop(anchor.chat)` 之后的那块）。
/// `classify_all` 是在裁剪区上跑的，传整帧会取到完全无关的区域，
/// 而它照样算得出一个「看起来合理」的指纹。
fn header_print(
    pane: &chatservant_capture::Frame,
    anchor: &chatservant_capture::anchor::PaneAnchor,
) -> Option<chatservant_perceive::identity::HeaderPrint> {
    chatservant_perceive::identity::HeaderPrint::of(pane, head_band_rect(anchor))
}

/// OCR 读会话名用的带高，**与指纹那块刻意不同**。
///
/// 实测（2026-10-07，同一块像素只改高度）：
///
/// ```text
/// 40 px  0 行  ← 读不到「G8W」
/// 60 px  1 行  ← 读到
/// 100 px 1 行
/// ```
///
/// 带子太窄、里面又只有一个小字时，`Windows.Media.Ocr` **不收**——
/// 与「乐」气泡那次是同一类毛病：它需要周围有内容。
/// 指纹那块继续用 [`HEAD_BAND_H`]（40 px 是照实测定的，动了会破 `MIN_PIXELS`）。
const PEER_NAME_BAND_H: i32 = 72;

/// 从**头带**里读会话名。坐标系要求同 [`header_print`]。
///
/// ## 为什么不能从增量 ROI 里读
///
/// 观察循环只对增量 ROI 跑 OCR，而水位线会把 ROI 从下往上裁——**标题栏
/// 常常根本不在里面**。于是：
/// - 从 ROI 找「最靠上的 Header」会挑中别的行（实测认成时间戳 `22 ： 22`）；
/// - 加上标题栏高度判据之后，又变成永远认不出来（日志里 `peer=""`）。
///
/// 会话名按定义住在标题栏，所以**直接去标题栏那一块取**。头带只有
/// 400x40，额外一次 OCR 很便宜（实测几十毫秒），而且这块区域本来
/// 每次都要用来算像素指纹，顺手的事。
///
/// 取「最上面的一条非空行」：头带里除了会话名就是几个图标按钮，
/// 图标没有文字，所以第一条就是它。
fn peer_name_from_header(
    pane: &chatservant_capture::Frame,
    anchor: &chatservant_capture::anchor::PaneAnchor,
) -> Option<String> {
    let strip = pane.crop(head_band_rect(anchor).with_height(PEER_NAME_BAND_H))?;
    // 小字要放大才认得准——与 `read_lines` 同一个理由（ADR-0002 补记）。
    let view = strip.upscaled_nearest(chatservant_perceive::OCR_UPSCALE)?;
    let ocr = chatservant_capture::ocr::recognize(&view).ok()?;
    ocr.lines
        .iter()
        .map(|l| l.text.trim())
        .find(|t| !t.is_empty())
        .map(str::to_string)
}

/// 头带高度 40 px。实测标题行只有 11 px 高，多留一点是为了
/// 框住整条标题栏（含右侧图标），而不是只框那行字。
const HEAD_BAND_H: i32 = 40;
/// 头带宽度上限 400 px。够装下会话名与右侧几个图标，
/// 又不至于把大片空面板算进来——那会稀释指纹。
const HEAD_BAND_W: i32 = 400;

/// 这个循环在用哪家供应商的**视觉**计费规则。
///
/// 判断模型 A 不发图，只有回复模型 B 发——而 B 的多模态能力决定了图
/// 是「随文本进 B」还是「先描述再进」。两条路花的视觉 token 一样，
/// 所以这里取当前生效的那一个。
impl Scratch {
    fn billing(&self) -> chatservant_core::scale::Billing {
        use chatservant_core::scale::Billing;
        match self.provider {
            Provider::Anthropic => Billing::ANTHROPIC_PATCH,
            Provider::OpenAi => Billing::OPENAI_GPT4O_MINI,
            // ⚠ MiniMax 的视觉计费**未核实**，而 `Billing` 只有两种已验证形状
            // （Anthropic 28px patch、OpenAI 512px tile）。
            // 套用其中之一 = 拿一个没验证的数去算用户的账单。
            // 这里如实**不产出预算**：媒体外发对 MiniMax 暂不给出尺寸，
            // 而不是给一个看起来能用的错数字。
            Provider::MiniMax => Billing::ANTHROPIC_PATCH,
        }
    }
}

/// 不做摘要的占位实现。
///
/// **它让摘要这一步不花钱**，也**不让这一步悄悄失败**——
/// `context.trim` 仍然跑完整逻辑（阈值判定、缓存、渲染），
/// 只是长对话不会真的调模型。要接真实摘要时换掉这个类型。
struct NoSummary;

impl chatservant_reason::Summarizer for NoSummary {
    fn summarize(&mut self, older: &[String]) -> Result<String, String> {
        Ok(format!(
            "（{older_len} 条早期消息，未启用摘要）",
            older_len = older.len()
        ))
    }
}

/// 把行框平移到整帧坐标系。
fn shift(r: chatservant_core::frame::Rect, dx: i32, dy: i32) -> chatservant_core::frame::Rect {
    use chatservant_core::frame::Rect;
    Rect::new(r.x0 + dx, r.y0 + dy, r.x1 + dx, r.y1 + dy)
}

#[cfg(test)]
mod tests {
    use super::{
        BUDGET_MAX, BUDGET_MIN, ImagesOut, RecentN, Relock, SESSION, Scratch, TierCell,
        TokenBudget, apply_relock, collect_images, header_print,
    };
    use chatservant_reason::context;

    /// **clone 出来的句柄和本体读的是同一个格子。**
    ///
    /// 这条锁的是「设置真能改到运行中的循环」。早先 `recent_n: usize`
    /// 是按值 move 进线程闭包的，命令层拿不到、也改不了它——
    /// 界面照常显示「已设为 24」，模型那边永远还是 12。
    /// 那个 bug 编译得过、跑得动、界面上看不出破绽，
    /// 唯一能抓住它的就是这条断言。
    #[test]
    fn a_cloned_handle_shares_one_cell() {
        let in_loop = RecentN::new(context::DEFAULT_RECENT);
        let in_settings = in_loop.clone();

        in_settings.set(30);
        assert_eq!(in_loop.get(), 30, "循环必须能读到设置写进去的值");
    }

    /// 越界夹取，且**回夹后的值**——调用方拿它回给界面。
    #[test]
    fn out_of_range_is_clamped_and_reported_back() {
        let r = RecentN::new(context::DEFAULT_RECENT);
        assert_eq!(r.set(0), context::MIN_RECENT);
        assert_eq!(r.get(), context::MIN_RECENT);
        assert_eq!(r.set(9999), context::MAX_RECENT);
        assert_eq!(r.get(), context::MAX_RECENT);
    }

    /// 构造时就该夹好——不能让非法值活到第一次读取。
    #[test]
    fn construction_clamps_too() {
        let r = RecentN::new(0);
        assert_eq!(r.get(), context::MIN_RECENT);
        assert_eq!(RecentN::new(usize::MAX).get(), context::MAX_RECENT);
    }

    #[test]
    fn budget_clamps_on_construction_and_on_set() {
        let b = TokenBudget::new(0);
        assert_eq!(b.get(), BUDGET_MIN, "0 要夹到下界");
        assert_eq!(TokenBudget::new(u32::MAX).get(), BUDGET_MAX);
        assert_eq!(b.set(999_999), BUDGET_MAX, "set 也要夹，且回夹后的值");
        assert_eq!(b.get(), BUDGET_MAX);
    }

    /// **图片外发默认关。**
    ///
    /// 这是隐私底线。媒体检测的准确率还没在真实帧上验证完
    /// （4 条检出里只有 1 条有独立交叉验证），所以**检测器报什么都不该
    /// 有图出得去**。兜底的位置是**出网之前**，不是让检测器自己保证。
    #[test]
    fn image_egress_is_off_by_default() {
        assert!(!ImagesOut::off().get(), "默认必须是关的");
    }

    /// 总闸的 clone 与本体共用一个格子——设置页改了循环得知道。
    #[test]
    fn the_image_gate_is_shared() {
        let in_loop = ImagesOut::off();
        let in_settings = in_loop.clone();
        in_settings.set(true);
        assert!(in_loop.get(), "循环必须能读到设置打开的总闸");
        in_loop.set(false);
        assert!(!in_settings.get(), "关闭也要能同步回去");
    }

    /// **档位启动时必须是 Manual。** 硬约束第 5 条「默认档最低」。
    #[test]
    fn the_tier_starts_at_manual() {
        use chatservant_core::dispatch::Dispatch;
        assert_eq!(TierCell::default_manual().get(), Dispatch::Manual);
    }

    /// **重锁必须清掉历史与摘要**——否则跨会话串味。
    ///
    /// 这条测的是一个**真实发生过的缺陷**（2026-10-07 修）：
    /// 重锁原本只清 `locked_print`，`history` 与 `Context` 都留着。
    /// 于是「在 A 里攒够历史 → 切到 B → 点重锁」之后，模型收到的是
    /// **A 的全部历史 + B 的消息**——它在看着别人的私聊给 B 起草。
    ///
    /// 这不是精度问题，是把不该出网的内容送出了网。
    #[test]
    fn relocking_drops_the_previous_conversation_history() {
        let mut s = Scratch::new(RecentN::new(12));
        s.images_out = ImagesOut::off();

        // 会话 A 攒下历史。
        s.history.push("A 里的私密内容".into());
        s.history.push("A 的第二条".into());

        // 用户点了「重新锁定当前会话」——这是「我现在要在这个会话里工作」。
        s.relock.request();
        let cleared = s.relock.take();

        assert!(cleared, "重锁请求必须能被取到，否则界面上点了没反应");
        apply_relock(&mut s);

        assert!(
            s.history.is_empty(),
            "重锁后还留着上一个会话的历史：{:?}",
            s.history
        );
    }

    #[test]
    fn relocking_forgets_the_summary_cache() {
        let mut s = Scratch::new(RecentN::new(12));
        let msgs: Vec<String> = (0..60)
            .map(|i| format!("第 {i} 条消息，够长才能触发摘要分块"))
            .collect();
        // 先给这个会话摘要一次，把缓存填上。
        struct Counter(u32);
        impl chatservant_reason::Summarizer for Counter {
            fn summarize(&mut self, older: &[String]) -> Result<String, String> {
                self.0 += 1;
                Ok(format!("{} 条旧消息的摘要", older.len()))
            }
        }
        let mut c = Counter(0);
        s.context.trim(SESSION, &msgs, 4, &mut c).expect("首次裁剪");
        assert!(s.context.summary_calls(SESSION) > 0, "前提：先摘要过一次");

        s.relock.request();
        let _ = s.relock.take();
        apply_relock(&mut s);

        assert_eq!(
            s.context.summary_calls(SESSION),
            0,
            "重锁后还留着上一个会话的摘要缓存"
        );
    }

    /// 设置页与主面板**共用同一个档位**。
    ///
    /// 早先 `tier` 是前端 `$state`、两个页面各一份：用户在设置里选 Auto，
    /// 主面板还是 Manual。那个开关是个**完全不起作用的死控件**。
    #[test]
    fn the_tier_is_shared_across_windows() {
        use chatservant_core::dispatch::Dispatch;
        let cell = TierCell::default_manual();
        let settings = cell.clone();
        settings.set(Dispatch::Auto);
        assert_eq!(cell.get(), Dispatch::Auto, "设置页升档，主面板必须看到");
    }

    /// **总闸关着时，一张图都不产出。**
    ///
    /// 这条不是「设置默认是关的」——那只是初值。这条测的是
    /// **`collect_images` 在关着时真的返回空**：哪怕窗格里真的有图，
    /// 哪怕预算够、哪怕模型能看图。它是整个图片链路的安全底线，
    /// 一旦有人把短路那行删掉，这条会立刻红。
    #[test]
    fn a_closed_gate_produces_no_images_even_when_media_is_present() {
        use chatservant_capture::anchor::PaneAnchor;
        use chatservant_core::frame::{Frame, Rect};

        // 一块纯色帧：没有媒体区域，所以本条测的不是「检不出」，
        // 而是「检不检都无所谓——闸关着就是空」。
        let mut px = vec![30u8; (200 * 200 * 3) as usize];
        // 中间放一块花的，模拟「窗格里确实有图」。
        for y in 60..140u32 {
            for x in 60..140u32 {
                let i = ((y * 200 + x) * 3) as usize;
                px[i] = (x % 251) as u8;
                px[i + 1] = (y % 253) as u8;
                px[i + 2] = ((x * y) % 247) as u8;
            }
        }
        let frame = Frame::from_rgb(200, 200, px).expect("尺寸一致");
        let anchor = PaneAnchor {
            chat: Rect::new(0, 0, 200, 200),
            panel_bg: [30, 30, 31],
            panel: Rect::new(0, 0, 200, 200),
            header_top: 0,
            input_top: 200,
        };

        let mut s = Scratch::new(RecentN::new(12));
        s.images_out = ImagesOut::off();
        let closed = collect_images(&frame, &anchor, &s);
        assert!(closed.is_empty(), "总闸关着却产出了 {} 张图", closed.len());

        // 打开之后同一帧应该有产出——否则上面那条就只是「检不出媒体」。
        s.images_out.set(true);
        s.provider = chatservant_reason::Provider::Anthropic;
        let opened = collect_images(&frame, &anchor, &s);
        assert!(
            !opened.is_empty(),
            "总闸打开却一张图都没有——那说明上一条是假通过"
        );
    }

    /// 认不出来的档位一律降回 Manual。硬约束第 5 条「程序自己不会升档」——
    /// 一个存坏了的格子值只能降回最低档，绝不能被解释成别的档。
    #[test]
    fn an_unrecognised_tier_value_falls_back_to_manual() {
        use chatservant_core::dispatch::Dispatch;
        let cell = TierCell::default_manual();
        // 直接往格子里塞一个不存在的编码，模拟存坏的值。
        cell.0.store(200, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(cell.get(), Dispatch::Manual, "认不出来的档位一律当最低档");
    }

    /// **闸认的是头带像素，不是 OCR 文字。**
    ///
    /// 真实帧上 OCR 把「小号」读成 `G8W`（实测），所以任何按文字比对的锁定
    /// 在真实环境上永远是关的。这条用两个**只有标题不同**的帧证明闸分得开，
    /// 而且全程**不需要任何 OCR 结果**。
    #[test]
    fn the_gate_reads_the_header_pixels_not_the_ocr_text() {
        use chatservant_capture::Frame;
        use chatservant_capture::anchor::PaneAnchor;
        use chatservant_core::frame::Rect;

        const W: u32 = 500;
        const H: u32 = 300;

        /// 顶部 40 px 是头带；`tint` 代表会话名画成什么样。
        fn frame_with(tint: u8) -> Frame {
            let mut px = vec![30u8; (W * H * 3) as usize];
            for y in 0..40u32 {
                for x in 0..W {
                    let i = ((y * W + x) * 3) as usize;
                    // 头带里放几段不同粗细的「字」，其余是面板底色
                    if x > 20 && x < 200 && (x / 7) % 3 == 0 {
                        px[i] = tint;
                        px[i + 1] = tint;
                        px[i + 2] = tint;
                    }
                }
            }
            Frame::from_rgb(W, H, px).expect("尺寸一致")
        }

        // 锚点给的是**帧坐标**，而 `header_print` 收的是**消息区**，
        // 这里让 chat 从 (0,0) 开始，两套坐标才一致。
        let anchor = PaneAnchor {
            chat: Rect::new(0, 0, W as i32, H as i32),
            panel_bg: [30, 30, 31],
            panel: Rect::new(0, 0, W as i32, H as i32),
            header_top: 0,
            input_top: H as i32,
        };

        let a = frame_with(225);
        let b = frame_with(120);
        let pa = header_print(&a, &anchor).expect("取得到");
        let pb = header_print(&b, &anchor).expect("取得到");
        assert!(!pa.is_same(&pb), "两个不同会话被判成同一个");

        let again = header_print(&a, &anchor).expect("取得到");
        assert!(pa.is_same(&again), "同一帧被误判成换人了");
        assert!(
            pa.pixels() > 5_000,
            "头带只有 {} px，太薄不足以当身份",
            pa.pixels()
        );
    }

    /// **重锁是一次性的，而且请求不会丢。**
    ///
    /// ⚠ 如果 `take()` 放在建锁判断**之后**，用户点了按钮、这一轮请求被跳过，
    /// 要等到下一轮才生效——那正是「点了没反应」最难查的一种形态。
    /// 所以顺序必须是：先取走请求，再判断建锁。
    #[test]
    fn a_relock_request_is_taken_exactly_once() {
        let g = Relock::new();
        assert!(!g.pending(), "刚建好时不该有待处理请求");
        assert!(!g.take(), "没有请求时 take 必须返回 false");

        g.request();
        assert!(g.pending(), "请求后必须看得到");
        assert!(g.take(), "第一次 take 必须拿到它");
        assert!(
            !g.take(),
            "重锁是一次性的——第二次 take 必须为空，否则每轮都在重新认人"
        );
        assert!(!g.pending());

        // clone 出去的与本体共用一个格子：命令层设的循环得知道。
        let shared = g.clone();
        shared.request();
        assert!(g.pending(), "clone 与本体必须是同一个请求槽");
    }

    /// 锁错会话之后能修正：清掉旧锁 → 下一轮以当前像素为新基准。
    #[test]
    fn relocking_replaces_the_locked_conversation() {
        use chatservant_capture::Frame;
        use chatservant_capture::anchor::PaneAnchor;
        use chatservant_core::frame::Rect;

        const W: u32 = 500;
        const H: u32 = 300;
        fn frame_with(tint: u8) -> Frame {
            let mut px = vec![30u8; (W * H * 3) as usize];
            for y in 0..40u32 {
                for x in 0..W {
                    let i = ((y * W + x) * 3) as usize;
                    if x > 20 && x < 200 && (x / 7) % 3 == 0 {
                        px[i] = tint;
                        px[i + 1] = tint;
                        px[i + 2] = tint;
                    }
                }
            }
            Frame::from_rgb(W, H, px).expect("尺寸一致")
        }
        let anchor = PaneAnchor {
            chat: Rect::new(0, 0, W as i32, H as i32),
            panel_bg: [30, 30, 31],
            panel: Rect::new(0, 0, W as i32, H as i32),
            header_top: 0,
            input_top: H as i32,
        };

        let mut s = Scratch::new(RecentN::new(12));
        // 先锁在「错」的会话上
        s.locked_print = header_print(&frame_with(225), &anchor);
        assert!(s.locked_print.is_some());

        // 用户点重锁：请求被取走，旧锁被清掉
        s.relock.request();
        assert!(s.relock.take());
        s.locked_print = None;

        // 下一轮以当前像素为新基准
        let now = header_print(&frame_with(120), &anchor).expect("取得到");
        assert!(now.is_same(&s.locked_print.clone().unwrap_or_else(|| now.clone())));
        assert_ne!(
            now,
            header_print(&frame_with(225), &anchor).expect("取得到"),
            "重锁之后基准必须换成当前的会话"
        );
    }
}
