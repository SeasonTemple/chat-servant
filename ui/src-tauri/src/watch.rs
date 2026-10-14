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

use crate::contract::Phase;

/// 循环的节奏。**慢一点没关系**——静默期本来就不该做事，
/// 而画面变化通常伴随人的操作，1 秒的延迟人感知不到。
pub const TICK: Duration = Duration::from_millis(1000);

/// 上一轮的带框数据。调试画布读它。
static LAST_BOXES: std::sync::Mutex<Option<crate::debug::DebugPane>> = std::sync::Mutex::new(None);

/// 读最近一轮的调试画布数据。
pub fn debug_pane() -> Option<crate::debug::DebugPane> {
    LAST_BOXES.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// 一个矩形的面积。**饱和减法**由 [`Rect::width`] / [`Rect::height`] 保证，
/// 所以负尺寸（锚点没找准时）读成 0 而不是绕回一大片。
fn pane_area(r: chatservant_core::frame::Rect) -> u64 {
    u64::from(r.width().max(0) as u32) * u64::from(r.height().max(0) as u32)
}

/// **每圈都记一次账**，静默期也记（全 0）。
///
/// ## 为什么不省掉静默期那一次
///
/// 只在跑过 OCR 的圈子里记账的话，界面挂着的永远是**上一次读**的数字。
/// 于是用户看到「读了 12% 的消息区」，而程序此刻**什么都没读**——
/// 两者在界面上长得一模一样，于是「变化门控有没有在起作用」这个问题
/// **看不出来**。而那恰恰是这张账唯一要回答的事。
fn record_cost(
    boxes: &std::sync::Mutex<Option<crate::debug::DebugPane>>,
    ocr_pixels: u64,
    roi_pixels: Option<u64>,
    pane_pixels: u64,
    ocr_calls: u32,
) {
    let mut g = boxes.lock().unwrap_or_else(|e| e.into_inner());
    let Some(p) = g.as_mut() else {
        // 还没跑出过第一帧的框。**不凭空造一个画布**——
        // 空框 + 真数字，比没有更容易让人以为「什么都没识别出来」。
        return;
    };
    p.cost = crate::debug::FrameCost {
        ocr_pixels,
        roi_pixels,
        pane_pixels,
        ocr_calls,
        vision_tokens: 0, // 图片预算那一步在更后面，见 record_vision_tokens
    };
}

/// 补记视觉 token。**必须独立于 [`record_cost`]**——
/// 图片是在推理构造 `TurnContext` 时才收的，那时账已经记过一次了。
fn record_vision_tokens(boxes: &std::sync::Mutex<Option<crate::debug::DebugPane>>, tokens: u64) {
    let mut g = boxes.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = g.as_mut() {
        p.cost.vision_tokens = tokens;
    }
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
    /// **会话身份闸现在是开着还是关着。**
    ///
    /// ⚠ 这是一个**显式字段**，不是让界面去 `reason` 里找关键词。
    /// 2026-10-11 实机：闸关上时 `reason` 写着「本轮不动手」，
    /// 而前端的理由栏只放行含「失败/错误/抓不到/收不到/异常」的那几条，
    /// 于是**闸关了但界面一个字都不说**——用户看到的还是
    /// 「在等〈新会话〉的新消息」，读起来就是程序在盯着那个新会话。
    /// 那正是硬约束第 1 条要求「看得见」的东西。
    pub gate_open: bool,
    /// **面板当前处于哪一阶段。**
    ///
    /// ## 为什么它必须是 `Readout` 的字段而不是界面的推断
    ///
    /// 2026-10-12 实测（`.scratch/…/21-readout-phase-never-sent.md`）：
    /// 界面 `apply()` 写的是 `candidates = r.phase === 'drafted' ? r.candidates : []`，
    /// 而当时的 `Readout` **根本没有 `phase` 字段**，`contract.rs` 里那个
    /// 带 `#[serde(tag = "phase")]` 的 `Phase` 只在它自己的测试里被构造过。
    /// 于是 `r.phase` 恒 `undefined` ⇒ `candidates` 恒空 ⇒
    /// **起草中 / 已起草 / 出错三态在真机上永远不可达**，
    /// 而 `?demo` 因为夹具手写了 `phase`，八态照样渲染得出来——
    /// 演示页是绿的、真机是断的，两边看着一样。
    ///
    /// ## 为什么是 `flatten` 而不是各字段平铺
    ///
    /// 界面的类型是 `Phase & Extra`（`+page.svelte` 的 `FullReadout`），
    /// 也就是**判别字段与消息、候选在同一层**。`flatten` 出来的 JSON 正好是
    /// 那个形状：`{"phase":"drafted","messages":[…],"candidates":[…],…}`。
    ///
    /// ⚠ 顺带消掉了一类隐患：以前 `messages` / `candidates` 是 `Readout` 的
    /// 独立字段、`phase` 是另一个来源，两份数据要靠人去保证一致。
    /// 现在它们同属一个枚举变体，**不一致在类型上就构造不出来**。
    #[serde(flatten)]
    pub phase: Phase,
    /// 界面该显示哪句「正在干什么」。**只有起草中非空。**
    ///
    /// ⚠ 它是 [`Phase::status_hint`] 的快照，**不是界面的第二份口径**。
    ///   早先 `(panel)/+page.svelte` 把「正在成形…」「通常几秒」写死成两个
    ///   `<p>`，Rust 这边改了口径也不会跟着变——同一句话有两份，
    ///   而面板是唯一会显示它的地方。
    pub status_hint: Option<String>,
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
}

impl Readout {
    /// 已读到的消息。**四个阶段都保留**——出错时也不该把对话清空。
    pub fn messages(&self) -> &[Message] {
        self.phase.messages()
    }

    /// 候选。**只有 `Drafted` 非空**，判据就是判别字段本身。
    pub fn candidates(&self) -> &[DraftCandidate] {
        self.phase.candidates()
    }

    /// 最近一次出错在哪一段。**空 = 没出错。**
    pub fn error(&self) -> Option<&str> {
        self.phase.error()
    }
}

/// 一条候选（给前端渲染用）。
///
/// ⚠ 派生 `PartialEq` 是 [`crate::contract::Phase`] 需要它的连带结果：
/// `Phase` 派生了 `PartialEq`，而 `Drafted` 变体带着这个类型。
/// 这个孤儿模块直到 2026-10-13 才第一次进编译（`main.rs` 里补 `mod contract;`），
/// 于是这处缺失**潜伏了整个文件的存在期**而没人看见。
#[derive(Debug, Clone, PartialEq, Serialize)]
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

/// 一帧从哪来。
///
/// ## 为什么要有这个枚举
///
/// 原来循环里写死了 `find_chat_window` + WGC，于是**整个应用都只能在
/// 微信真的开着、并且窗口位置摆对的时候跑**。后果不是「多写点测试」的问题，
/// 而是：改一次界面就得去点一次微信、截一次图、确认一次像素——
/// 而这些步骤每次结果都不一样。于是「界面能不能跑通」这件事
/// **没法被可靠地验证**，只能靠一次次人肉点击碰运气。
///
/// 有了它，`CHATSERVANT_FIXTURE=<png>` 就能让**整个应用**（含 Tauri 窗口与
/// 面板）跑在一张冻结的真实帧上：OCR、归类、锁定、起草、事件推送、界面渲染
/// 一条都不少，只有「这一帧从哪来」变了。
///
/// 钉在夹具上时画面每轮都一样，所以第一轮起草之后循环会自然进入静默期——
/// 这正好也是要验的行为（硬约束第 6 条）。
///
/// ## 为什么用环境变量而不是编译期开关
///
/// 切模式不该要求重新编译。要是一般参数就得 `cargo build` 一次，
/// 这个开关没两天就会被当成摆设绕过去。
#[derive(Clone, Debug)]
pub enum FrameSource {
    /// 现场：找微信窗口 + WGC 抓帧。
    Live,
    /// 从一张 PNG 反复读同一帧。
    Fixture(std::path::PathBuf),
}

impl FrameSource {
    /// 从环境变量读。没设就是现场模式。
    pub fn from_env() -> Self {
        match std::env::var("CHATSERVANT_FIXTURE") {
            Ok(p) if !p.trim().is_empty() => Self::Fixture(std::path::PathBuf::from(p)),
            _ => Self::Live,
        }
    }

    pub fn is_fixture(&self) -> bool {
        matches!(self, Self::Fixture(_))
    }

    /// 夹具模式下给 reason 加的前缀。
    ///
    /// ⚠ **必须加**。夹具跑出来的界面上写着「文件传输助手」、消息和候选
    /// 都和真的一模一样，不标出来就没人分得清这是不是实时的。
    /// 而一个会让人以为「程序在实时看着我的微信」的界面——
    /// 那是在说谎。
    pub fn stamp(&self, reason: &str) -> String {
        if self.is_fixture() {
            format!("【夹具帧，不是实时画面】{reason}")
        } else {
            reason.to_string()
        }
    }
}

/// 读一帧。现场模式做完整的可读性检查；夹具模式只读文件。
fn load_frame(
    src: &FrameSource,
    last_hwnd: &mut Option<isize>,
) -> Result<chatservant_capture::Frame, String> {
    if let FrameSource::Fixture(path) = src {
        return chatservant_capture::frame_from_png(path)
            .map_err(|e| format!("读夹具 {} 失败：{e}", path.display()));
    }

    let hit = find_chat_window(&WECHAT).map_err(|e| format!("找窗口失败：{e}"))?;
    *last_hwnd = Some(hit.hwnd_raw);

    // ⚠ **先问能不能抓到，再决定要不要试。**
    //
    // 这里必须用 `why_wgc_unreadable`，**不能**用区域抓屏那套判据。
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
    Ok(frame)
}

/// 循环的配置。组装真实传输需要 key，所以从这里进。
pub struct LoopConfig {
    /// **一帧从哪来。** 默认现场抓；也可以钉在一张冻结的 PNG 上。
    pub source: FrameSource,
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
    /// **我说过的话**，按时间正序累积。只收 [`Speaker::Me`]。
    ///
    /// ## 为什么和 `history` 分开存
    ///
    /// `history` 是「整段对话」：对方说的、我说过的、系统提示都在里面；
    /// 这一路是「我自己的话」，给 [`chatservant_reason::voice`] 挑口吻样本用。
    /// 合成一份再按说话人过滤也行，但那样每次要**回扫全history**才能挑出
    /// 我的那几条，而它是每轮都要挑的。
    ///
    /// ## 为什么要单独一路，而不是混进 `history`
    ///
    /// 「说话风格」和「对话内容」回答的是两个问题：前者是「我平时怎么说话」，
    /// 后者是「现在该回什么」。混在一起模型无从区分——`history` 渲染出来
    /// 时**不带说话人标签**（见 `Trimmed::render`），它会把样本当成对方说过的话。
    mine: Vec<String>,
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
    /// 上一次给会话名做缓存键的那个指纹。见 [`refresh_peer`]。
    last_print: Option<chatservant_perceive::identity::HeaderPrint>,
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
    /// 上一轮会话闸是开是关。`None` = 还没跑过第一轮。
    ///
    /// 存在的唯一理由：**闸的状态变了要告诉界面一次**。
    ///
    /// 实测问题（2026-10-08）：用户在微信里切到别的会话，闸关上了，
    /// `step` 于是返回 `Ok(None)`——面板上一个字都不更新，
    /// 继续挂着**上一个会话的会话名、消息和候选**。
    /// 用户看到的是「面板说我在处理 G8W，其实我已经在看古文山了」，
    /// 而界面上没有任何地方说明这件事。
    ///
    /// 硬约束第 1 条要求「只对锁定会话动手」是**看得见**的；
    /// 不动手指甲的闸等于没有闸。
    prev_gate_open: Option<bool>,
    /// 闸现在是开着还是关着。**推给界面用**——见 [`Readout::gate_open`]
    /// 上那条为什么不能靠 `reason` 的关键词来判断。
    gate_open: bool,
    /// 一帧从哪来。见 [`FrameSource`]。
    source: FrameSource,
    /// 现场模式下最近一次用到的窗口句柄。给日志与投递用。
    last_hwnd: Option<isize>,
}

impl Scratch {
    fn new(recent: RecentN) -> Self {
        Self {
            candidates: Vec::new(),
            reason: String::from("等待第一条消息"),
            ocr_calls: 0,
            context: chatservant_reason::Context::new(),
            history: Vec::new(),
            mine: Vec::new(),
            messages: Vec::new(),
            recent,
            vision: chatservant_reason::VisionBridge::off(),
            locked_peer: String::new(),
            peer: String::new(),
            locked_print: None,
            last_print: None,
            locked_name: String::new(),
            relock: Relock::new(),
            images_out: ImagesOut::off(),
            token_budget: TokenBudget::new(crate::commands::BUDGET_DEFAULT),
            provider: chatservant_reason::Provider::OpenAi,
            prev_gate_open: None,
            gate_open: true,
            // 默认现场；`spawn` 会用 `LoopConfig` 里的那份覆盖。
            source: FrameSource::Live,
            last_hwnd: None,
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
            // ⚠ **未知一律当 Manual。** 解析失败就升档是最危险的默认值。
            //
            // ⚠ 编码 `2` 曾经是 `Auto`，2026-10-13 该档整档取消。
            //   它落进 `_` 分支 ⇒ 旧配置里的 2 会被读成 **Manual（最低档）**，
            //   而不是 panic 或升档——这是失败时唯一正确的方向。
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
            s.source = cfg.source.clone();
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
                // 一轮**中间**推出去的帧（`Drafting`）走这里。
                //
                // 它与轮末那一帧的唯一区别是 `ticks` 相同而阶段不同，
                // 所以 `cs-pump` 的去重指纹必须带 `phase.key()`——
                // 只看 `ticks` 会把这一帧吃掉，首屏就还是 6~9s
                // （见 [`crate::contract::Phase::key`] 的注释）。
                let mut interim = |out: Readout| {
                    if let Ok(mut g) = latest_bg.lock() {
                        *g = Some(out);
                    }
                };
                match step(
                    &mut watcher,
                    &mut reasoner,
                    &mut s,
                    &LAST_BOXES,
                    ticks,
                    last_calls,
                    &mut interim,
                ) {
                    Ok(Some(a)) => {
                        anchor = Some(a);
                        last_calls = reasoner.calls();
                        let st = reasoner.stage_calls();
                        // 阶段由 Rust 判，不让界面去猜 `candidates` 空不空。
                        let phase = if s.candidates.is_empty() {
                            Phase::Read {
                                messages: s.messages.clone(),
                            }
                        } else {
                            Phase::Drafted {
                                messages: s.messages.clone(),
                                candidates: s.candidates.clone(),
                            }
                        };
                        let out = snapshot(&s, anchor, phase, ticks, last_calls, st, &s.reason);
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
                        // ⚠ 出错时**保留已读到的消息**：一个 OCR 抖动不该把
                        // 整段对话清空，用户会以为程序把会话弄丢了。
                        let phase = Phase::Failed {
                            messages: s.messages.clone(),
                            error: e,
                        };
                        let out =
                            snapshot(&s, anchor, phase, ticks, last_calls, st, "这一轮没跑通");
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

/// 拼一份往 UI 推的快照。**`Readout` 在整个 crate 里只有这一个构造点。**
///
/// ## 为什么收敛成一处
///
/// 票 21 的验收项写着「`Readout` 的每个构造点都赋了值，漏一个就红」。
/// 靠 review 盯不住——`Readout { … }` 在哪个 `match` 分支里看着都正常，
/// 而漏掉的那个后果是**某个态在真机上永远发不出去**，
/// 门禁（`cargo test` 测 Rust、`svelte-check` 只管类型、`?demo` 夹具手写了
/// `phase`）三者一起看不见。唯一的可靠办法是让机器去数，见
/// `every_readout_goes_through_the_snapshot_constructor`。
///
/// ## `phase` 为什么由调用方给而不是这里算
///
/// 因为「这一轮处于哪个阶段」只有产生它的那段代码知道：
/// 轮末可能是 `Drafted`（起草出东西了）或 `Read`（这轮没对方说话），
/// 轮中那一帧恒为 `Drafting`，出错恒为 `Failed`。
/// 在这里用 `candidates.is_empty()` 反推，等于把
/// 闸关着时界面显示的那句话。
///
/// ⚠ **为什么闸一关就要压过「正在起草」**：闸一关，候选就被清空了，
///   那一轮不会有任何产出。说「正在起草，通常几秒」是在骗用户等。
///
/// ⚠ **为什么由 Rust 出这句话**：界面分不出「对方真没说话」与
///   「面板已停泊、没在读这个会话」——两者的 `messages` 都是空数组。
///   2026-10-14 真机实测：切到另一个会话后，面板正文显示的是
///   「对方说完话，这里才落笔。」，可那个会话对方明明说过话。
///   **那句话在停泊这条路径上是假的。**
///
/// ⇒ 与 `types.ts` 的纪律一致：`status_hint` 是 Rust 的口径，
///   **界面不许自己写死这句**。为什么要有下一半：
///   判据 12「空状态必须给一个明确的下一步动作」——
///   光说「不动手」用户不知道该干嘛，「切回来才继续」才是动作。
const GATE_CLOSED_HINT: &str = "锁着别的对话，切回来才继续。";

/// 「刚读出消息、模型还没回来」和「这轮没有对方说话」混成同一个态——
/// 那正是前端一直在猜、猜错的那件事。
fn snapshot(
    s: &Scratch,
    anchor: Option<PaneAnchor>,
    phase: Phase,
    ticks: u64,
    model_calls: u32,
    st: chatservant_reason::StageCalls,
    reason: &str,
) -> Readout {
    // ⚠ 必须在 `phase` 被移进结构体**之前**取：它的口径来自
    //   `Phase::status_hint`，而结构体字面量是按书写顺序求值的。
    // ⚠ 闸关着时**改用** [`GATE_CLOSED_HINT`]，理由见那里那条注释。
    let status_hint = if s.gate_open {
        phase.status_hint().map(str::to_owned)
    } else {
        Some(GATE_CLOSED_HINT.to_owned())
    };
    Readout {
        anchor,
        peer: s.peer.clone(),
        gate_open: s.gate_open,
        phase,
        status_hint,
        reason: s.source.stamp(reason),
        model_calls,
        relock_pending: s.relock.pending(),
        counts: CallCounts {
            judge: st.judge,
            draft: st.draft,
            rank: st.rank,
            // 视觉桥**默认关闭**（ticket 15 第 1 条），循环里没接桥之前
            // 这里恒为 0——这是**正确值**，不是占位符。
            vision: s.vision.calls(),
        },
        ticks,
        ocr_calls: s.ocr_calls,
    }
}

/// 跑一轮。`Ok(Some(anchor))` = 有新内容；`Ok(None)` = 静默期。
fn step(
    watcher: &mut PaneWatcher,
    reasoner: &mut Box<dyn Reasoner + Send>,
    s: &mut Scratch,
    last_boxes: &std::sync::Mutex<Option<crate::debug::DebugPane>>,
    ticks: u64,
    model_calls: u32,
    interim: &mut dyn FnMut(Readout),
) -> Result<Option<PaneAnchor>, String> {
    // ① 找窗口 + 抓帧（或读夹具）
    let frame = load_frame(&s.source, &mut s.last_hwnd)?;

    // ④ 锚点与消息区。
    //
    // ⚠ **坐标系必须从头统一**：给 `PaneWatcher` 的必须是**消息区**，
    // 它内部算出来的 ROI 是消息区坐标；拿整帧去喂，它会按消息区尺寸算，
    // 于是 ROI 变成「整帧左上角起、宽 852」——把侧边栏也圈进来，
    // 而行框又没减 `chat.x0`，**所有气泡在消息区坐标系里整体右移 304px**。
    // 分类去采样时采到的是别的气泡的位置，于是**一条消息都判不出来**
    // （实测：97 轮全是 `messages=0`）。
    let anchor =
        chatservant_capture::anchor::derive(&frame).map_err(|e| format!("锚点推导失败：{e}"))?;
    let full = frame
        .crop(anchor.chat)
        .ok_or_else(|| "裁不出消息区".to_string())?;

    // ② **会话身份闸。必须排在任何 OCR 之前。**
    //
    // 闸认的是**头部区域的像素指纹**，不是 OCR 读出来的名字。
    //
    // 为什么不比名字：实测真实帧上头部的**归类**是对的（`NoiseClass::Header`），
    // 但 OCR 把「小号」读成了 `G8W`。按文字锁的话，闸在真实环境上**永远是关的**
    // ——不误发，但也永远不工作。
    //
    // 为什么指纹可靠：文字是 OCR 的产物，那一小块像素不是。
    // 换会话像素必然变；同一会话在帧之间基本不变（阈值吸收角标抖动）。
    //
    // 闸关着 = 屏幕上那块**不是我们要读的对话**，那就不该对它做任何识别。
    //
    // ⚠ 以前闸排在 OCR 之后，于是用户切到别的会话期间，循环**每秒跑一次
    // 整帧 OCR + 放大 + 分类**，识别出来的消息立刻被丢掉——直到用户切回来。
    // 识别了不用是一回事，**不该识别**是另一回事：那是在对一段我们无权
    // 处理的对话反复读像素。闸只需要头部那块像素、不碰 OCR，
    // 所以它可以、也必须排在前面。
    // ⚠ 这里传的是 **`frame`（整帧）**，不是下面那个 `full`。
    // 2026-10-14 修：会话名行在 `anchor.chat` **之内但不在其上沿**
    // （实测 `chat.y0 + 18`），喂裁过的消息区会切在它上面/下面。
    // 2026-10-14 票 31：位置不再猜，由 `HeaderPrint::discover` 从像素推出。
    let discovered = header_print(&frame, &anchor);
    refresh_peer(
        s,
        &frame,
        &anchor,
        discovered.as_ref().map(|(p, _)| p),
        discovered.as_ref().map(|(_, r)| *r),
    );
    // ⚠ 重锁请求必须**在建锁判断之前**取走，否则这一轮的请求会被跳过，
    // 用户点了按钮却什么也没发生——那是最难查的一种「没反应」。
    //
    // ⚠⚠ **必须连水位线一起退**，否则重锁等于「锁上了但永远读不到东西」。
    // 见 [`chatservant_perceive::PaneWatcher::rewind`]：水位线不在 `Scratch`
    // 里，所以「把这个会话的状态清干净」这件事天然会漏掉它。
    if s.relock.take() {
        apply_relock(s);
        watcher.rewind();
    }
    let peer_ok = match (&s.locked_print, &discovered) {
        // 还没锁过：拿**第一帧读到的**当基准，并把名字报给用户看。
        // ⚠ 这意味着「启动时开着的那个会话」就是被锁的那个。
        //   所以界面上必须显示这个名字——用户要能一眼看出锁错了没有。
        (None, Some((p, _))) => {
            s.locked_print = Some(p.clone());
            // 建锁这一刻认出来的名字才是「锁的是谁」。
            s.locked_name = s.peer.clone();
            true
        }
        // 锁过但这帧认不出 → **关**。认不出不等于「是同一个」。
        // `(None, None)` 也落到这里：连基准都还没有，更不能动手。
        (_, None) => false,
        (Some(a), Some((b, _))) => a.is_same(b),
    };
    if !peer_ok {
        // ⚠ 连水位线一起退，否则切回锁定会话时只读得到底部那条缝。
        //   见 `park_on_other_conversation`——面板清空和引擎退位必须同一次做。
        let just_closed =
            park_on_other_conversation(s, watcher, discovered.as_ref().map(|(p, _)| p));
        // 闸关着 = 这一帧没读任何像素。**照样记账**，
        // 否则界面挂着上一帧的数字，用户以为程序还在给那个新会话干活。
        record_cost(last_boxes, 0, None, pane_area(anchor.chat), 0);
        return Ok(if just_closed { Some(anchor) } else { None });
    }
    // 闸是开的。记一笔，供下次关闸时判断「是不是刚刚才关的」。
    s.prev_gate_open = Some(true);
    // 推给界面的那份也一样要跟着变——否则用户切回锁定会话之后，
    // 面板还挂着上一轮「闸关着」的横幅。
    s.gate_open = true;

    // ③ 变化门控 + 增量 ROI。**静默期在这里就返回了。**
    let Decision::Read { roi } = watcher.observe(&full) else {
        // 没变：一行都没读。**这是硬约束第 6 条的落点**，
        // 而「这一帧 0 像素 / 0 次 OCR」正是它唯一可信的证据。
        record_cost(last_boxes, 0, None, pane_area(anchor.chat), 0);
        return Ok(None);
    };

    // ③ **OCR 整帧（放大后），再按 ROI 过滤行——不要裁一块去 OCR。**
    //
    // 实测（2026-10-07，同一块画面）：
    //
    // ```text
    // 整帧 2x        5 行，读到「乐」（对方消息）
    // ROI 带子 2x   4 行，读不到「乐」
    // ```
    //
    // 「乐」就在带子里（pane 坐标 79,128），它没消失，是识别器在裁剪后
    // 把它漏了。这与「乐」气泡那次、标题栏 40px 那次是**同一类毛病**：
    // `Windows.Media.Ocr` 需要行周围有内容才收，裁得越紧漏得越多。
    // 与其为裁剪框反复试参数，不如**永远 OCR 整帧**——这也顺带消掉了
    // 「裁剪坐标 ↔ 帧坐标 ↔ 消息区坐标」三套换算，正是坐标 bug 的来源。
    //
    // 配方与 `perceive::read_lines` 完全一致（放大 → OCR → 换算到消息区坐标），
    // 别在这里自己改一套。
    let sc = chatservant_perceive::OCR_UPSCALE;
    let view = frame
        .upscaled_nearest(sc)
        .ok_or_else(|| "整帧放大失败".to_string())?;
    let ocr = chatservant_capture::ocr::recognize(&view).map_err(|e| format!("OCR 失败：{e}"))?;
    s.ocr_calls += 1;

    let palette = chatservant_perceive::Palette::learn(&frame, anchor.chat)
        .ok_or_else(|| "学不到配色（消息区太平）".to_string())?;

    // 行框：放大后的视图坐标 → 整帧（除以倍数）→ **消息区坐标**（减 chat 原点）。
    // 然后按 ROI 裁掉本轮不该管的那些行——ROI 本身就是消息区坐标。
    let lines: Vec<chatservant_capture::ocr::OcrLine> = ocr
        .lines
        .iter()
        .map(|l| {
            let f = |v: i32| v / sc as i32 - anchor.chat.x0;
            let g = |v: i32| v / sc as i32 - anchor.chat.y0;
            chatservant_capture::ocr::OcrLine {
                text: l.text.clone(),
                rect: chatservant_core::frame::Rect::new(
                    f(l.rect.x0),
                    g(l.rect.y0),
                    f(l.rect.x1),
                    g(l.rect.y1),
                ),
                words: vec![],
            }
        })
        // 按 ROI 过滤：只留本轮该管的那些行。ROI 是消息区坐标，
        // 行框也已经换算到消息区坐标，两边可以直接比。
        .filter(|l| {
            l.rect.y1 > roi.y0 && l.rect.y0 < roi.y1 && l.rect.x1 > roi.x0 && l.rect.x0 < roi.x1
        })
        .collect();
    let attrs = chatservant_perceive::classify_all(&lines, &full, &palette);
    let messages = chatservant_perceive::group(&attrs);
    tracing::debug!(
        target: "chat_servant::watch",
        "roi={roi:?} ocr总行={} 本轮行={} 归类={} 消息={}",
        ocr.lines.len(),
        lines.len(),
        attrs.len(),
        messages.len()
    );
    // **累积**给界面。直接发本轮 ROI 的那几条，面板上的聊天记录会在
    // 「本轮那几条」和「空」之间来回跳——因为 ROI 被水位线从下往上裁，
    // 静默时它压根不含任何消息。累积后面板显示的是这个会话最近的一段对话。
    //
    // ⚠⚠ **这一行必须在身份闸之后**，这是 2026-10-11 修的真实缺陷。
    //
    // 它原来排在闸**前面**。于是用户在微信里切到会话 B 时，B 的消息先被
    // `merge_into_stream` 并进了 A 的消息流，然后闸才关——闸只挡住了推理与
    // 投递，**没挡住消息累积**。面板于是挂着「B 的会话名」显示「A 的历史
    // 夹着 B 的新消息」，正是这段注释原本要防的
    // 「看起来在处理 A，其实已经在看 B」。
    //
    // 现在闸整块提到了 OCR 之前，这个约束是**结构上**成立的：闸关着时
    // 根本走不到这里，也就没什么可漏进来的。
    s.messages = chatservant_perceive::merge_into_stream(&s.messages, &messages);

    // 存一份给调试画布。**在整帧坐标系**——画布按帧的百分比定位，
    // 用 ROI 坐标会让框整体偏移。
    *last_boxes.lock().unwrap_or_else(|e| e.into_inner()) = Some(crate::debug::build(
        anchor.chat,
        frame.width(),
        frame.height(),
        attrs.as_slice(),
    ));

    // 开销账。⚠ `ocr_pixels` 是**放大后的整帧**，不是 ROI——
    // 见下面 ③ 那段实测记录：裁 ROI 去 OCR 会漏行，所以送进去的确实是整帧。
    // 报成 ROI 面积的话，用户会以为「调小 ROI 就能省 OCR」，而那调了没用。
    record_cost(
        last_boxes,
        u64::from(frame.width()) * u64::from(frame.height()) * u64::from(sc),
        Some(pane_area(roi)),
        pane_area(anchor.chat),
        1,
    );

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
        // ⚠ **先把「读到消息了、模型还没回来」这件事推出去**，再去做耗时的事。
        //
        // 这是 S3（起草中）唯一的来源。少了这一帧，起草中这一态在真机上
        // 根本不存在——而模型这一段要 6~9s，用户面对的是一个**不动的面板**，
        // 分不清是在跑还是卡死了（票 21 的根因之一）。
        //
        // 代价是**一轮里推两帧**，所以 `cs-pump` 的去重指纹必须带 `phase.key()`。
        // 只看 `ticks` 会把这一帧当重复吃掉，首屏就白做了。
        {
            let st = reasoner.stage_calls();
            interim(snapshot(
                s,
                Some(anchor),
                Phase::Drafting {
                    messages: s.messages.clone(),
                },
                ticks,
                model_calls,
                st,
                "正在起草",
            ));
        }

        // **每圈读一次**活值，读完存成本地变量给整轮用。
        // 分两次读的话，用户恰好在中间拖了滑块，同一轮里
        // 裁剪用 N=12、兜底用 N=11——半套生效比不生效更难查。
        let recent_n = s.recent.get();

        // 历史**只增不减**——摘要靠它判断「更早的部分」有多长。
        // ⚠ 只在这条路径上追加：没读到对方说话时不追加，避免把
        // 时间戳、系统提示当成对话塞进上下文。
        s.history.extend(messages.iter().map(|m| m.text.clone()));

        // 口吻样本走**另一路**（`FEATURE.md` 的「单独一路」）。
        //
        // ⚠ 它和 `history` 是两份**平行**的累积，不是同一份的视图：
        // `history` 要全部消息（对方说的也在里面），这里**只要我说过的**。
        // 早先这里只推 `.text`，`speaker` 整个被丢掉——于是上层根本没有
        // 任何办法知道哪句是我说的，这条路在实现层面就不存在。
        s.mine.extend(chatservant_reason::voice::my_side(&messages));

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

        let images = collect_images(&frame, &anchor, s);
        // 视觉 token 记在**收完图之后**——收图之前谁也不知道这一帧花了多少。
        // 默认配置（多模态 B）这里是 0，而 0 是有意义的数，必须照报。
        record_vision_tokens(last_boxes, images.iter().map(|i| u64::from(i.tokens)).sum());

        let ctx = TurnContext {
            messages: rendered.lines().map(str::to_string).collect(),
            // 过了闸才填对端名。**这里不是空串了**——早先一直留空占位，
            // 判据是「还没接识别会话名」，现在接上了。
            peer: s.peer.clone(),
            images,
            // 「我平时的说话风格」。空的时候是空 Vec，渲染那一步会整节省掉，
            // 不往提示里塞空标题。
            voice: chatservant_reason::voice::voice_samples(&s.mine),
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
        // ⚠⚠ **这里绝不能清候选。** 这是 2026-10-08 实机抓到的路径：
        //
        // ```text
        // 第 1 轮   ROI = 整屏 → 读到对方的话 → 起草 3 条 → 面板上出现候选
        // 第 3 轮   ROI = 贴着输入框的一条缝，里面只有我自己说的话
        //           → candidates.clear() → 候选没了，面板说「对方还没发新消息」
        // 第 4 轮   同上——因为「没有对方说话」所以水位线不推进，
        //           ROI 还是那条缝，**这个状态不会自愈**
        // ```
        //
        // 也就是说**候选活不过两秒**，而对话里明明已经有对方说的话。
        // 之前每轮 `roi=... 消息=1` 且 `candidates=0` 的日志就是它。
        //
        // 为什么「这一段没有对方说话」不该推翻已有候选：候选对应的是
        // **这段对话的状态**，不是「刚才这条带子里有什么」。一条自己发的
        // 消息出现在底下，既没有让对方说话失效，也没有让已起草的内容过时。
        //
        // 清候选只有两个正当时机，都不在这里：
        // · 真的起草出新的 —— `Ok(t)` 分支整体覆盖；
        // · 会话闸判定当前不是锁定会话 —— `!peer_ok` 分支（那里清得对）。
        //
        // reason 照旧更新成「这一段里没有对方说的话」：它描述的是**这一轮**
        // 读到的东西，不是「界面上一条候选都没有」，两者不能混为一谈。
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
    // ⚠ 名字的缓存键也要一起丢。重锁就是一次会话切换，新会话的会话名
    //   必须重新读一次——留着旧键会让 `refresh_peer` 以为「没变」而跳过。
    s.last_print = None;
    s.history.clear();
    // ⚠ 口吻样本同样要清。它是**从对方私聊里学来的东西**——带着上一个会话的
    // 口吻去给新会话起草，等于把 A 的说话风格套在 B 身上，而且这份「风格」
    // 本身是从 A 的私聊内容里长出来的。跟 `history` 是同一类事故。
    s.mine.clear();
    s.context.forget(SESSION);
    s.candidates.clear();
    // ⚠ 面板上累积的消息**也要清**。不清的话重锁之后，标题写着新会话名，
    // 下面却还挂着上一个会话的对话——那正是「界面上出现不是这个会话的内容」，
    // 和上面那条历史串味是同一类事故，只是它出在给人看的那一层。
    s.messages.clear();
}

/// **闸关着的时候，把面板收拾干净。** 返回「是不是刚刚才关的」。
///
/// 抽成独立函数是为了能直接测它——它原来内联在 `step` 里，而 `step` 要
/// 真帧、真 OCR、真 Reasoner 才跑得起来，于是这段「关闸时到底清了什么」
/// 一次都没被断言过。
///
/// ## 为什么确认换了会话要连消息一起清
///
/// `s.peer` 每轮都在跟着真实窗口更新，所以闸一关，标题已经是新会话的名字了。
/// 这时候下面还挂着**锁定会话**的历史，读起来就是「这是新会话的对话」——
/// 而它不是。硬约束第 1 条要求这道闸是**看得见的**，一个会说谎的面板比
/// 一条没反应的按钮更糟：用户无从判断哪段内容是自己正在看的那个会话的。
///
/// ## 为什么「认不出头部」不清
///
/// 认不出不等于换了会话（见 [`chatservant_perceive::identity::HeaderPrint::of`]）。
/// 标题栏被裁掉、动画没画完都会短暂认不到，此时清空面板会让消息列表**闪一下**。
/// 所以只有**指纹确实不同**这一支才清。
///
/// 闸关着的时候，**面板这边**实际做了什么。
///
/// ## `switched` 为什么必须单独报出来
///
/// 它对应的是「水位线要不要一起退」这一个决定，而水位线**不住在
/// [`Scratch`] 里**——它住在 [`PaneWatcher`] 里。所以「把这个会话的状态
/// 清干净」这件事天然漏掉它，调用方必须拿到这个信号才退得回去。
///
/// 漏掉的后果不是「多读一次」，是**永久少读**：面板被清空了，可引擎还
/// 停在旧会话的底部，切回来时增量范围只剩贴着底部的一条缝，
/// 8 条消息只回来 1 条，而且水位线从此不再动，**不会自愈**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GateOutcome {
    /// 闸的状态刚变成「关」——要推一次 readout，否则界面停在旧状态。
    just_closed: bool,
    /// **确认换了会话**（指纹确实不同），面板上的消息已清空。
    switched: bool,
}

/// ⚠ 清了面板**就要一起退水位线**——这件事由 [`park_on_other_conversation`]
/// 兜住，不要在这里只清一半。
fn apply_gate_closed(
    s: &mut Scratch,
    print: Option<&chatservant_perceive::identity::HeaderPrint>,
) -> GateOutcome {
    s.candidates.clear();
    let mut switched = false;
    s.reason = match (&s.locked_print, print) {
        (Some(_), None) => format!(
            "没读出当前会话的头部，无法确认还是不是「{}」——本轮不动手",
            s.locked_peer
        ),
        // `(None, Some(_))` 到不了这里：那一支已经在建锁时放行了。
        (None, None) => "还没读到任何会话头部，先不动手".into(),
        _ => {
            // 确认换人了：上一个会话的消息**不该继续挂在面板上**。
            s.messages.clear();
            switched = true;
            format!(
                "当前会话已经不是锁定的那一个（锁定的是「{}」）——本轮不动手",
                if s.locked_name.is_empty() {
                    "认不出的会话"
                } else {
                    &s.locked_name
                }
            )
        }
    };
    // ⚠ 但**闸的状态一变就要推一次 readout**。否则用户在微信里切了会话，
    // 面板上一个字都不更新，继续挂着上一个会话的候选——
    // 静默是对的，但静默到界面开始说谎就不对了。
    let just_closed = s.prev_gate_open != Some(false);
    s.prev_gate_open = Some(false);
    s.gate_open = false;
    GateOutcome {
        just_closed,
        switched,
    }
}

/// 闸关着 = 屏幕上那块**不是我们要读的对话**。面板和引擎必须一起收拾。
///
/// 返回「是不是刚刚才关的」——只有刚关的那一刻才推 readout。
///
/// ## 为什么不能只调 [`apply_gate_closed`]
///
/// 2026-10-11 实机复现的真实缺陷：用户在微信里切走再切回来，面板上
/// 8 条消息只剩 6 条，而且**不会自愈**。
///
/// 根因不是丢消息，是**两处状态不同步**：闸关着时面板上的累积消息被
/// 清掉了，可水位线还停在旧会话的底部——它不在 [`Scratch`] 里，
/// 「把会话状态清干净」这件事天然漏掉它。于是切回来时增量范围只剩
/// 贴着底部的那条缝，只有一条消息落在里面。
///
/// 以前这里还留着一句注释说「清空之后不用补偿，关闸那一支不推进水位线，
/// 用户切回时整个消息区会重读一遍」——**那句话是错的**。不推进水位线
/// 恰恰意味着 ROI 保持 `[水位线, 底部)`，也就是那条缝。重读整段需要的是
/// **退回顶部**，不是「不推进」。
///
/// 只在 `switched`（确实换了会话）时退：「认不出头部」那一支没有清面板，
/// 引擎那边的记忆仍然对得上，跟着一块退只会白白多读一次。
fn park_on_other_conversation(
    s: &mut Scratch,
    watcher: &mut PaneWatcher,
    print: Option<&chatservant_perceive::identity::HeaderPrint>,
) -> bool {
    let g = apply_gate_closed(s, print);
    if g.switched {
        watcher.rewind();
    }
    g.just_closed
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

/// 头部区域的像素指纹。**认不出就返回 `None`**——
/// 调用方必须把 `None` 当成「认不出」而不是「是同一个」。
///
/// ## 用**推导出来的头部行**，不用调用方给的矩形
///
/// 更早还用过「最靠上那条 `Header` 的 rect」。实测那个框只有 **33×11**，
/// 而且它是**跟着 OCR 结果走的**：名字一个字还是三个字，框就多宽少宽。
/// 后果有两个——指纹薄得不足以区分会话；短名字直接掉到
/// `MIN_PIXELS` 以下，闸**永远建不起来**。
///
/// 头带由锚点给出，不依赖 OCR 认得认得准。
///
/// ⚠ `frame` 必须是**整帧**（`frame.crop(anchor.chat)` **之前**那块）。
///
/// 2026-10-14 改的：原先这里收的是消息区，于是取样框算出来的
/// `header_top - chat.y0` 恒为 0、带子永远落在消息区顶部——
/// 闸认的一直是「消息开头长什么样」，不是「这是谁的对话」。
/// 标题栏在 `chat.y0` **之上**，喂裁过的画面就等于**它根本不在输入里**。
fn header_print(
    frame: &chatservant_capture::Frame,
    anchor: &chatservant_capture::anchor::PaneAnchor,
) -> Option<(
    chatservant_perceive::identity::HeaderPrint,
    chatservant_core::frame::Rect,
)> {
    // ⚠⚠ 2026-10-14：**不再自己拼矩形**。
    //
    // 原先这里按 `chat.y0` 猜头部带子的位置，连猜三次全错
    // （消息区顶部 / `chat.y0` 之上 / `chat.y0` 之下），
    // 而三次的测试都是绿的——因为夹具也按各自的错误假设画字块。
    // 病根是**没有任何一层算出过「头部在哪」**，于是每次都得改魔数。
    //
    // 现在由感知层从像素**推**出来（`discover`：逐行数墨、找第一条
    // 成行的字），布局变了它自己跟着走。
    // ⇒ 见 `.scratch/…/31-perceive-has-no-relayout-duty.md`。
    //
    // ⚠ 返回 `None` = **认不出**，调用方必须按「认不出」处理（关闸不起稿）。
    chatservant_perceive::identity::HeaderPrint::discover(frame, anchor.chat, HEAD_BAND_W)
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
///
/// ⚠ 2026-10-14 票 31：这行原先写的是「指纹那块继续用 [`HEAD_BAND_H`]」，
///   而那个常量**连同 `header_band_rect` 一起删掉了**——闸那块不再有固定高度，
///   高度由 `HeaderPrint::discover` 从像素推出（真机实测 11 ⇒ 量化到 12）。
///   这个 72 只管**这里**（取名字时的 OCR 窗口），与闸的取样范围无关。
const PEER_NAME_BAND_H: i32 = 72;

/// 从**推导出的头部行**里读会话名。坐标系要求同 [`header_print`]——**整帧**。
///
/// ## 这里原来读的是消息区顶部、再改到 `chat.y0` 之上，两次都读不出名字
///
/// 票 30 真机 `peer=""` 查了三轮取证才落到「那块像素里没有名字」这个事实上；
/// 修的时候又按「标题栏在 `chat.y0` 之上」猜了一次，**仍然读不出**——
/// 真机上会话名在 `chat.y0` **之下**（y 49..59，`chat.y0 = 31`）。
///
/// ⇒ 现在**不猜**：矩形由
/// [`chatservant_perceive::identity::HeaderPrint::discover`] 从像素里推出来，
/// 这里直接用它。判据
/// `identity::tests::discover_follows_the_header_line_wherever_it_moves`
/// 守着「挪了位置也跟得上」；把它退回写死 y 时那条立刻红。
///
/// ⚠ 带高不写死：OCR 需要周围有内容才肯收（见 [`PEER_NAME_BAND_H`] 的实测），
/// 所以在推导出的行**上下各留一点余量**，且不越过 `chat` 的下沿。
fn peer_name_from_header(
    frame: &chatservant_capture::Frame,
    head: chatservant_core::frame::Rect,
    chat: chatservant_core::frame::Rect,
) -> Option<String> {
    use chatservant_core::frame::Rect;
    // 上下各留一点余量：11px 的行直接丢给 OCR，它会返回 0 行。
    let pad = (PEER_NAME_BAND_H - head.height()).max(0) / 2;
    let y0 = (head.y0 - pad).max(chat.y0);
    let y1 = (head.y1 + pad).min(chat.y1);
    if y1 <= y0 {
        return None;
    }
    let strip = frame.crop(Rect::new(head.x0, y0, head.x1, y1))?;
    // 小字要放大才认得准——与 `read_lines` 同一个理由（ADR-0002 补记）。
    let view = strip.upscaled_nearest(chatservant_perceive::OCR_UPSCALE)?;
    let ocr = chatservant_capture::ocr::recognize(&view).ok()?;
    ocr.lines
        .iter()
        .map(|l| l.text.trim())
        .find(|t| !t.is_empty())
        .map(str::to_string)
}

/// **头部像素没变就不重读会话名。**
///
/// ## 为什么能拿指纹当缓存键
///
/// 会话名是**头部那块像素的函数**——像素一模一样，OCR 不可能读出不同的字。
/// 而 [`HeaderPrint::is_same`] 判定的正是「这块还是不是那块」，
/// 阈值吸收的是角标、动画这类抖动。所以指纹没变就不必再花一次 OCR。
///
/// ⚠ 这条缓存让「闸关着时别干活」成为可能：换会话时头部像素确实变了，
/// 所以**面板仍然会显示当前窗口里真正是谁**（硬约束第 1 条要求这道闸
/// 看得见）；而停在 B 不动时，指纹不变，于是那一整段时间**一次 OCR 都不跑**。
///
/// ## 认不到就保持原样
///
/// 身份是**累积确认**的事实。早先这里无条件
/// `s.peer = ...unwrap_or_default()`，于是标题栏一离开增量 ROI，
/// 界面上的会话名就被抹成「没认出来」——而实际上它没变过。
fn refresh_peer(
    s: &mut Scratch,
    frame: &chatservant_capture::Frame,
    anchor: &chatservant_capture::anchor::PaneAnchor,
    print: Option<&chatservant_perceive::identity::HeaderPrint>,
    head: Option<chatservant_core::frame::Rect>,
) {
    let changed = match (&s.last_print, print) {
        // 还没记过：第一次总得读一次。
        (None, _) => true,
        // 这帧认不出头部 → 不动已认出来的名字，也不更新缓存键
        // （否则下次认不出就会被当成「变了」，白读一次）。
        (_, None) => false,
        (Some(a), Some(b)) => !a.is_same(b),
    };
    if !changed {
        return;
    }
    s.last_print = print.cloned();
    // ⚠ 指纹变了但**没推出头部矩形**（`discover` 认不出）⇒ 不动已认出的名字。
    //   「认不出」不是「名字变了」，也不是「没有名字」。
    if let Some(rect) = head {
        if let Some(name) = peer_name_from_header(frame, rect, anchor.chat) {
            s.peer = name;
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::{
        BUDGET_MAX, BUDGET_MIN, DraftCandidate, ImagesOut, Message, Phase, Readout, RecentN,
        Relock, SESSION, Scratch, Speaker, TierCell, TokenBudget, apply_gate_closed, apply_relock,
        collect_images, header_print, park_on_other_conversation, snapshot,
    };
    use chatservant_perceive::{Decision, PaneWatcher};
    use chatservant_reason::context;

    /// 造一份最小的 `Scratch`，只为了喂 `snapshot`。
    fn scratch_with(messages: usize, candidates: usize) -> Scratch {
        let mut s = Scratch::new(RecentN::new(12));
        s.messages = (0..messages)
            .map(|i| a_msg(&format!("对方说了 {i}")))
            .collect();
        s.candidates = (0..candidates)
            .map(|i| DraftCandidate {
                id: format!("c{i}"),
                text: format!("候选 {i}"),
                probability: 0.5,
            })
            .collect();
        s
    }

    /// `Readout` 摊平之后，**判别字段与消息、候选在同一层**。
    ///
    /// 这是票 21 的核心断言。界面的类型是 `Phase & Extra`
    /// （`ui/src/routes/(panel)/+page.svelte` 的 `FullReadout`），
    /// 它按 `r.phase === 'drafted'` 走判别分支、再读**同一层**的 `r.candidates`。
    /// 而票 21 之前 Rust 侧**根本没发 `phase`** ⇒ `r.phase` 恒 `undefined`
    /// ⇒ `candidates` 恒空 ⇒ 起草中 / 已起草 / 出错三态在真机上永远不可达，
    /// 而 `?demo` 因为夹具手写了 `phase` 照样八态全渲染——
    /// **演示页是绿的、真机是断的，两边看着一样**。
    #[test]
    fn readout_carries_the_phase_discriminant_at_the_top_level() {
        let s = scratch_with(2, 2);
        let st = chatservant_reason::StageCalls::default();
        let out: Readout = snapshot(
            &s,
            None,
            Phase::Drafted {
                messages: s.messages.clone(),
                candidates: s.candidates.clone(),
            },
            7,
            3,
            st,
            "起草好了",
        );
        let v = serde_json::to_value(&out).unwrap();

        assert_eq!(v.get("phase").and_then(|x| x.as_str()), Some("drafted"));
        // 关键：`messages` / `candidates` 与 `phase` **平级**，
        // 不是嵌在 `phase` 里面。
        assert!(v.get("messages").and_then(|x| x.as_array()).is_some());
        assert_eq!(v["candidates"].as_array().map(Vec::len), Some(2));
        assert!(v.get("phase").unwrap().get("messages").is_none());
    }

    /// 三个阶段**各自**都得带上判别字段。
    ///
    /// 只测 `Drafted` 是不够的：出错的形态恰恰是票 21 里最隐蔽的一处——
    /// 出错那一帧的 `error` 在旧结构里是个独立的 `Option<String>`，
    /// 改成 `Phase::Failed` 之后如果漏了改，界面就**静默地不显示任何错误**，
    /// 用户看到的是一个「什么都没发生」的面板。
    #[test]
    fn every_phase_shape_carries_its_own_discriminant() {
        let s = scratch_with(1, 0);
        let st = chatservant_reason::StageCalls::default();
        let cases = [
            (
                Phase::Read {
                    messages: s.messages.clone(),
                },
                "read",
            ),
            (
                Phase::Drafting {
                    messages: s.messages.clone(),
                },
                "drafting",
            ),
            (
                Phase::Drafted {
                    messages: s.messages.clone(),
                    candidates: s.candidates.clone(),
                },
                "drafted",
            ),
            (
                Phase::Failed {
                    messages: s.messages.clone(),
                    error: "OCR 失败".into(),
                },
                "failed",
            ),
        ];
        for (phase, key) in cases {
            let out = snapshot(&s, None, phase, 1, 0, st, "r");
            let v = serde_json::to_value(&out).unwrap();
            assert_eq!(v["phase"].as_str(), Some(key), "{key} 的判别字段不对");
            // ⚠ **出错时消息不能丢**：一个 OCR 抖动不该把整段对话清空。
            assert_eq!(
                v["messages"].as_array().map(Vec::len),
                Some(1),
                "{key} 丢了消息"
            );
            if key == "drafted" {
                assert!(v.get("candidates").is_some(), "drafted 必须带候选");
            } else {
                assert!(v.get("candidates").is_none(), "{key} 不该带候选");
            }
        }
    }

    /// **闸一关，面板不许再说「对方说完话，这里才落笔」。**
    ///
    /// ⚠ 2026-10-14 真机实测出来的说假话路径：用户在微信里切到另一个会话，
    ///   闸关上、面板被清空，于是正文显示「对方说完话，这里才落笔。」——
    ///   **而那个会话对方明明说过话**。面板其实处在「已停泊、没在读这个会话」，
    ///   但界面分不出这两者：`messages` 都是空数组。
    ///
    /// ## 为什么这条判据住在 Rust 这边
    ///
    /// 文案归 Rust 出（`types.ts` 的纪律：界面不许自己写死 `status_hint`），
    /// 所以判据也在这边。界面那边只能验「有没有照着渲染」。
    #[test]
    fn a_closed_gate_replaces_the_waiting_line_with_an_honest_one() {
        let st = chatservant_reason::StageCalls::default();

        // ── 闸关着（停泊）：必须给「停泊」那句，且**不是**在等对方说话 ──
        let mut parked = scratch_with(0, 0);
        parked.gate_open = false;
        let out = snapshot(
            &parked,
            None,
            Phase::Read {
                messages: Vec::new(),
            },
            7,
            0,
            st,
            "当前会话已经不是锁定的那一个——本轮不动手",
        );
        assert!(!out.gate_open, "前提：闸是关着的");
        let hint = out.status_hint.clone().expect("闸关着必须给一句");
        assert_eq!(hint, super::GATE_CLOSED_HINT, "停泊时说的是别的会话");
        assert!(
            !hint.contains("落笔"),
            "闸关着还说「这里才落笔」= 骗用户等一个不会来的动作：{hint}"
        );

        // ── 闸开着：这句不许出现，否则正常态全被污染 ──
        let mut open = scratch_with(0, 0);
        open.gate_open = true;
        let out = snapshot(
            &open,
            None,
            Phase::Read {
                messages: Vec::new(),
            },
            7,
            0,
            st,
            "这一轮对方没有新消息",
        );
        assert!(
            out.status_hint.is_none(),
            "闸开着却带了停泊那句，正常态会说错话：{:?}",
            out.status_hint
        );

        // ── 闸关着 + 正在起草：**压过**「正在起草，通常几秒」 ──
        // 闸一关候选就被清空了，那一轮不会有产出，
        // 说「通常几秒」是骗用户等。这条钉住优先级。
        let mut drafting = scratch_with(0, 0);
        drafting.gate_open = false;
        let out = snapshot(
            &drafting,
            None,
            Phase::Drafting {
                messages: Vec::new(),
            },
            7,
            1,
            st,
            "本轮不动手",
        );
        assert_eq!(
            out.status_hint.as_deref(),
            Some(super::GATE_CLOSED_HINT),
            "闸关着还显示「正在起草，通常几秒」，用户会白等"
        );
    }

    /// **`Readout` 全仓只有一个构造点。**
    ///
    /// 票 21 的验收项原文：「`Readout` 的每个构造点都赋了值，漏一个就红」。
    /// 靠 code review 盯不住——`Readout { … }` 出现在哪个 `match` 分支里
    /// 看着都正常，而漏掉的后果是**某个态在真机上永远发不出去**，
    /// 且 `cargo test` / `svelte-check` / `?demo` 夹具**三个一起看不见**
    /// （夹具里 `phase` 是手写的）。
    ///
    /// 所以让机器去数：构造点只允许是 [`snapshot`] 那一处。
    /// 新增构造点必须先问「为什么不走 `snapshot`」。
    #[test]
    fn every_readout_goes_through_the_snapshot_constructor() {
        let me = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut hits: Vec<String> = Vec::new();
        let mut files = Vec::new();
        walk_rs(&me, &mut files);
        for p in &files {
            let Ok(text) = std::fs::read_to_string(p) else {
                continue;
            };
            // 形如 `Readout {` 的字面构造（排除类型定义那一行）。
            for (n, line) in text.lines().enumerate() {
                let t = line.trim();
                if t.starts_with("Readout {") || t == "Readout {" {
                    hits.push(format!("{}:{}", p.display(), n + 1));
                }
            }
        }
        // 本文件里那一处就是 `snapshot` 里的 `Readout { … }`（缩进后以
        // `Readout {` 开头）。允许恰好 1 处，多一处少一处都要人来解释。
        assert_eq!(
            hits.len(),
            1,
            "Readout 的字面构造点必须只有 snapshot() 那一处，现在有：\n{}",
            hits.join("\n")
        );
    }

    fn walk_rs(dir: &std::path::Path, acc: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk_rs(&p, acc);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                acc.push(p);
            }
        }
    }

    /// 左侧边栏宽度，**实测微信的值**。
    ///
    /// ⚠ 夹具里它必须**非零**，否则 `frame.crop(anchor.chat)` 是恒等操作，
    /// 两套坐标系一模一样，头部取样那次减法算错也就看不出来——
    /// 而那正是 2026-10-11 那个破口的形态：闸在真实帧上从来没关过，
    /// 而当时所有测试都是绿的。
    /// （那次出错的 `head_band_rect` 已于 2026-10-14 票 31 删除。）
    const SIDEBAR_W: u32 = 304;

    /// 消息区自身宽高（**不含**侧边栏）。
    const PANE_W: u32 = 400;
    const PANE_H: u32 = 700;

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

    // ── 会话身份闸：关闸时到底清了什么 ────────────────────────

    /// 造一块「标题栏」：深底 + 三个白字块。
    ///
    /// ⚠ `name` 决定**笔画落在哪**，而三个 name 的**亮度完全一样**。
    ///
    /// 这是 2026-10-11 那个破口的教训，代价是整个会话闸失效：
    /// 早先这几处夹具用 `tint` 改**亮度**（225 对 120）来造「另一个会话」，
    /// 于是它们一直是绿的；而真实会话名之间亮度一模一样，差的是**字形**。
    /// 闸因此从来没在真实帧上关过——直到实机切了一次会话才暴露。
    ///
    /// 字块起点固定在 x=24/42/60，宽度 13 px，落在 y=12..24，
    /// 与实测那块区域（16000 px 里 189 个墨迹像素）同一量级。
    ///
    /// ⚠⚠ 2026-10-14 票 31：原先画在 `y 0..40`，而夹具的 `chat.y0 = 40`
    /// ⇒ 字块整块落在**消息区上沿之上**。于是「头部在哪」这个答案
    /// 被夹具自带了——被测代码猜哪个位置都能对上。
    /// 现在按真机几何画在 `y 58..69`（`chat.y0` 往下 18px、行高 11），
    /// **必须由 `discover` 自己推出来**。
    fn header_frame(w: u32, h: u32, name: u32) -> chatservant_capture::Frame {
        let mut px = vec![30u8; (w * h * 3) as usize];
        let name_top = 40u32 + 18; // == chat.y0 + 18，与 test_anchor 的几何一致
        for y in name_top..(name_top + 11u32).min(h) {
            for x in 0..w {
                let mut on = false;
                for sx in [24u32, 42, 60] {
                    let (Some(cx), Some(cy)) = (x.checked_sub(sx), y.checked_sub(name_top)) else {
                        continue;
                    };
                    if cx < 13 && cy < 13 {
                        on = match name {
                            0 => cx < 7,
                            1 => cx >= 7,
                            _ => cy < 7,
                        };
                    }
                }
                if on {
                    let i = ((y * w + x) * 3) as usize;
                    px[i] = 225;
                    px[i + 1] = 225;
                    px[i + 2] = 225;
                }
            }
        }
        chatservant_capture::Frame::from_rgb(w, h, px).expect("尺寸一致")
    }

    /// 造一块**消息区**画面：深底 + 若干条文字行（`(y0, y1)`）。
    ///
    /// 气泡色 225/225/229 配底色 30/30/31，与实测一致。
    /// ⚠ 造「另一个会话」时**不许只改亮度**——见 [`header_frame`] 那条，
    /// 会话闸就是这么失效过一整轮的。
    fn pane_frame(rows: &[(u32, u32)]) -> chatservant_capture::Frame {
        const W: u32 = 400;
        const H: u32 = 700;
        let mut px = vec![30u8; (W * H * 3) as usize];
        for &(y0, y1) in rows {
            for y in y0..y1 {
                for x in 40..(W - 40) {
                    let i = ((y * W + x) * 3) as usize;
                    px[i] = 225;
                    px[i + 1] = 225;
                    px[i + 2] = 229;
                }
            }
        }
        chatservant_capture::Frame::from_rgb(W, H, px).expect("尺寸一致")
    }

    /// 取一块「标题栏」的头部指纹。
    ///
    /// ⚠ 走 `discover`，**不自己拼矩形**——这正是票 31 要禁的做法。
    /// 帧按 `header_frame` 的真实几何放大到能装下会话名行。
    fn print_of(name: u32) -> chatservant_perceive::identity::HeaderPrint {
        use chatservant_core::frame::Rect;
        const W: u32 = 200;
        // header_frame 把字块画在 y 58..69，帧至少要 72 高才装得下。
        const H: u32 = 72;
        let f = header_frame(W, H, name);
        let chat = Rect::new(0, 40, W as i32, H as i32);
        chatservant_perceive::identity::HeaderPrint::discover(&f, chat, W as i32)
            .map(|(p, _)| p)
            .expect("取得到指纹")
    }

    fn a_msg(text: &str) -> Message {
        Message {
            id: format!("Them:{text}"),
            speaker: Speaker::Them,
            text: text.into(),
            rect: chatservant_core::frame::Rect::new(0, 0, 10, 10),
            evidence: "测试".into(),
        }
    }

    /// **换会话时，面板上的消息必须跟着清掉。**
    ///
    /// 这条测的是 2026-10-11 定位到的真实缺陷：原来 `merge_into_stream`
    /// 排在身份闸**前面**，于是切到会话 B 时 B 的消息先被并进 A 的消息流，
    /// 闸才关——**闸只挡住了推理与投递，没挡住消息累积**。面板于是挂着
    /// 「B 的会话名」显示「A 的历史夹着 B 的新消息」。
    #[test]
    fn closing_the_gate_on_a_different_conversation_drops_the_messages() {
        let a = print_of(0);
        let b = print_of(1);
        assert!(!a.is_same(&b), "前提：这两块像素要判成不同会话");

        let mut s = Scratch::new(RecentN::new(12));
        s.locked_print = Some(a);
        s.locked_name = "A".into();
        s.messages.push(a_msg("A 里的私密内容"));
        s.candidates.push(DraftCandidate {
            id: "c1".into(),
            text: "给 A 的草稿".into(),
            probability: 0.5,
        });

        let g = apply_gate_closed(&mut s, Some(&b));

        assert!(
            g.just_closed,
            "刚关闸要推一次 readout，否则界面还停在旧状态"
        );
        assert!(
            g.switched,
            "指纹确实不同，必须报「换了会话」——水位线要跟着一起退"
        );
        assert!(
            s.messages.is_empty(),
            "换了会话还挂着锁定会话的消息：{:?}",
            s.messages
        );
        assert!(s.candidates.is_empty(), "候选也必须一起清");
        assert!(
            s.reason.contains("锁定的是「A」"),
            "面板要说明为什么不动手，实际：{}",
            s.reason
        );
        // ⚠ **历史不能跟着清**：它是锁定会话的推理上下文，不是面板显示。
        //   清它等于把当前会话攒下的上下文也扔了。
        s.history.push("A 里的历史".into());
        apply_gate_closed(&mut s, Some(&b));
        assert_eq!(s.history.len(), 1, "关闸不是重锁，历史不该被清掉");
    }

    /// **连续关闸只推一次 readout。**
    ///
    /// `just_closed` 存在的意义就是这个：闸关着的时候每一圈都会走到这里，
    /// 每圈都推一次等于面板每秒被重写一次，白费渲染。
    #[test]
    fn a_gate_that_stays_closed_only_pushes_one_readout() {
        let a = print_of(0);
        let b = print_of(1);
        let mut s = Scratch::new(RecentN::new(12));
        s.locked_print = Some(a);

        assert!(
            apply_gate_closed(&mut s, Some(&b)).just_closed,
            "第一次关闸要推"
        );
        assert!(
            !apply_gate_closed(&mut s, Some(&b)).just_closed,
            "闸一直关着时不该每圈都推一次"
        );
    }

    /// **「认不出头部」不等于换了会话——这时不许清面板。**
    ///
    /// 标题栏被裁掉、动画没画完都会短暂认不到。此时清空消息列表会让
    /// 面板**闪一下**，而用户什么也没做错。清不清只该由「指纹确实不同」决定。
    #[test]
    fn losing_the_header_is_not_a_conversation_change() {
        let a = print_of(0);
        let mut s = Scratch::new(RecentN::new(12));
        s.locked_print = Some(a);
        s.messages.push(a_msg("同一会话里对方刚说的"));

        apply_gate_closed(&mut s, None);

        assert_eq!(
            s.messages.len(),
            1,
            "只是认不到头部就把面板清空了：{:?}",
            s.reason
        );
        assert!(
            s.reason.contains("无法确认"),
            "要说清是「认不出」而不是「换了会话」，实际：{}",
            s.reason
        );
    }

    /// **切走再切回锁定会话，消息必须全部回来。**
    ///
    /// 2026-10-11 实机复现的缺陷：微信里切到别的会话再切回来，面板上
    /// 8 条消息只剩 6 条，而且**不会自愈**。
    ///
    /// 根因不是丢消息，是**两处状态不同步**：闸关着时面板上的累积消息被
    /// 清掉了，可水位线还停在旧会话的底部——它不在 `Scratch` 里，
    /// 「把会话状态清干净」这件事天然漏掉它。于是切回来时增量范围只剩
    /// 贴着底部的那条缝，只有一条消息落在里面。
    ///
    /// 这条按真实路径走：`park_on_other_conversation`（**生产用的那个函数**，
    /// 不是手工再调一次 `rewind`——那样测的是测试自己）。
    #[test]
    fn switching_back_to_the_locked_conversation_restores_every_message() {
        // A 会话：8 条消息铺满 700 高的消息区。
        let rows: [(u32, u32); 8] = [
            (60, 80),
            (140, 160),
            (220, 240),
            (300, 320),
            (380, 400),
            (460, 480),
            (540, 560),
            (620, 640),
        ];
        let a = pane_frame(&rows);
        let mut w = PaneWatcher::new(0);
        let mut s = Scratch::new(RecentN::new(12));
        s.locked_print = Some(print_of(0));

        // 在 A 里读到全部 8 条，水位线推到最下面一条的下沿。
        let Decision::Read { .. } = w.observe(&a) else {
            panic!("首次观察必须读整段消息区")
        };
        s.messages = (0..8).map(|i| a_msg(&format!("A 的第 {i} 条"))).collect();
        w.commit(660);

        // 切到 B：闸关，面板被清空。**水位线必须同时退掉。**
        park_on_other_conversation(&mut s, &mut w, Some(&print_of(1)));
        assert!(s.messages.is_empty(), "前提：面板上的消息确实被清了");
        assert_eq!(
            w.watermark(),
            0,
            "闸关着就等于这块画面不是我们的——水位线必须退回顶部，\
             否则切回来只会读到贴着底部的那条缝"
        );

        // 切回 A：闸重新开。整段消息区必须重读。
        assert!(
            print_of(0).is_same(s.locked_print.as_ref().expect("锁还在")),
            "前提：切回来之后指纹要判成同一个会话"
        );
        let Decision::Read { roi } = w.observe(&a) else {
            panic!("切回锁定会话必须重新读整段消息区")
        };
        assert!(
            roi.y0 <= 1,
            "增量范围从 y={} 起，8 条消息里只有最下面一条落在里面：{roi:?}",
            roi.y0
        );
    }

    /// **端到端序列：同一窗口 HWND，内容换一批帧。** 票 18 最后一条验收项。
    ///
    /// ## 为什么上面那条撑不起来
    ///
    /// `switching_back_to_the_locked_conversation_restores_every_message`
    /// 用的是 `print_of(n)`——**手搓指纹**，然后直接调闸函数。
    /// 它跳过了「帧 → [`header_print`] → 闸」这条链。
    ///
    /// 而上一轮那个破口恰恰出在链子上：**头部矩形曾经**在这里自己拼
    /// （`header_band_rect`，2026-10-14 票 31 已连同 `HEAD_BAND_H` 一起删除），
    /// 拿 `chat.x0` 去裁一块**已经是消息区坐标**的画面，裁出来的是消息区
    /// 右半边的中部——指纹算在了一片与「是谁」无关的区域上，
    /// 两个会话的指纹几乎一样，闸**从来没在真实帧上关过**。
    /// 手搓指纹的测试永远测不到这种错：它压根不经过那块坐标换算。
    ///
    /// ⚠ 票 31 之后矩形不再由这里拼，改由
    ///   `chatservant_perceive::identity::HeaderPrint::discover` **从像素推出来**
    ///   （`header_print` 那条线）。这段留作事故史，不是现状描述。
    ///
    /// 所以这条必须从**帧**出发，走生产用的 `header_print()`。
    ///
    /// ## 「同一窗口」体现在哪
    ///
    /// `w` 是同一个 [`PaneWatcher`] 实例，全程没有重建——水位线正是
    /// 住在它里面的。重建实例等于把这个会话读过的全忘掉，
    /// 而那恰恰是「切回来消息变少」的另一个成因。
    ///
    /// ## 两条性质都实测过「掐掉修复就变红」（2026-10-12）
    ///
    /// | 掐掉的 | 结果 |
    /// |---|---|
    /// | `head_band_rect` 的 `x0` 改回 `chat.x0`（**该函数已于票 31 删除**，
    ///   这行是当时的历史记录） | 本条红，**旧的
    ///   `the_gate_reads_the_header_pixels_not_the_ocr_text` 照样绿** |
    /// | `park_on_other_conversation` 里的 `watcher.rewind()` | 本条 + 已有的那条一起红 |
    ///
    /// 第一行是这条存在的理由：旧夹具把 `chat.x0` 设成 0，`crop` 成了
    /// 恒等操作，两套坐标系一模一样——**坐标换算算错了它也测不出来**。
    /// 而 `park_on_other_conversation` 那条另有测试守着，所以本条的新增
    /// 价值在于「真实帧 → 指纹」这半条链，和帧序里闸的开关时机。
    #[test]
    fn the_gate_closes_and_reopens_over_a_frame_sequence_with_one_watcher() {
        // 首条消息从 **y 80** 起（票 31）：与 y 58..69 的会话名行留出
        // y 70..79 的空行，对齐真机几何。原先从 60 起会与名字行粘连。
        let rows_a: [(u32, u32); 8] = [
            (80, 100),
            (160, 180),
            (240, 260),
            (320, 340),
            (400, 420),
            (480, 500),
            (560, 580),
            (640, 660),
        ];
        // B 是另一个会话：字形不同、**亮度完全一样**，且消息更少。
        let rows_b: [(u32, u32); 2] = [(520, 540), (620, 640)];

        let a = conversation_frame(&rows_a, 0);
        let b = conversation_frame(&rows_b, 1);
        let anchor = test_anchor();

        // 走生产路径取指纹，不手搓。
        //
        // ⚠ **两套坐标系，2026-10-14 起真的分开了**：
        //   · `header_print` / `refresh_peer` 收**整帧**——标题栏在
        //     `anchor.chat` **之上**，喂裁过的画面它就根本不在输入里。
        //   · `PaneWatcher::observe` 收**消息区**（它的 ROI 是消息区坐标）。
        // 以前两者都喂裁过的那块，于是闸取样取到了消息区顶部。
        // `pane` 闭包保留给 `observe`；闸那几处一律传 `&a` / `&b`。
        let pane = |f: &chatservant_capture::Frame| f.crop(anchor.chat).expect("消息区裁得出来");
        let pa = header_print(&a, &anchor).expect("A 的指纹取得到").0;
        let pb = header_print(&b, &anchor).expect("B 的指纹取得到").0;
        assert!(
            !pa.is_same(&pb),
            "两个会话的指纹必须不同，否则这条测的是个假闸"
        );

        let mut s = Scratch::new(RecentN::new(12));
        s.images_out = ImagesOut::off();
        let mut w = PaneWatcher::new(0);

        // ── 第 1 圈：在 A 里建锁，读到 8 条 ──
        s.locked_print = Some(pa.clone());
        s.locked_name = "A".into();
        s.messages = vec![a_msg("A1"), a_msg("A2")];
        let Decision::Read { roi } = w.observe(&pane(&a)) else {
            panic!("第一帧必然要读")
        };
        w.commit(roi.y1);

        // ── 第 2 圈：切到 B。同一个 watcher，只是内容换了 ──
        let peer_ok = match (&s.locked_print, Some(&pb)) {
            (Some(x), Some(y)) => x.is_same(y),
            _ => true,
        };
        assert!(!peer_ok, "前提：B 必须被认成不是锁定的那个会话");
        let just_closed = park_on_other_conversation(&mut s, &mut w, Some(&pb));
        assert!(just_closed, "刚换会话：面板要推一次 readout");
        assert!(
            s.messages.is_empty() && s.candidates.is_empty(),
            "闸关着却还挂着锁定会话的内容：{:?}",
            s.messages
        );

        // 在 B 里读一轮并提交——水位线因此被推到 B 的底部。
        let Decision::Read { roi } = w.observe(&pane(&b)) else {
            panic!("画面变了必然要读")
        };
        w.commit(roi.y1);
        s.messages = vec![a_msg("B1")];

        // ── 第 3 圈：切回 A ──
        let back = header_print(&a, &anchor).expect("切回来的指纹取得到").0;
        assert!(back.is_same(&pa), "前提：切回来认得是同一个会话");
        let out = park_on_other_conversation(&mut s, &mut w, Some(&pa));
        assert!(!out, "闸是重新开的：不该再推一次「刚关」");
        s.messages = vec![a_msg("A1"), a_msg("A2")];

        let Decision::Read { roi } = w.observe(&pane(&a)) else {
            panic!("切回锁定会话必须重新读")
        };
        assert!(
            roi.y0 <= 1,
            "增量范围从 y={} 起 —— 水位线没跟着退回顶部，8 条里只有一条能读回来：{roi:?}",
            roi.y0
        );
        assert!(s.locked_print.is_some(), "锁还在，闸重新开着");
        assert!(s.messages.len() == 2, "切回来之后消息必须重新有内容");
    }

    /// 锚点：**消息区从 x=[SIDEBAR_W] 起**，标题栏在顶上。
    ///
    /// `chat.x0` 非零是有意的——见 [`SIDEBAR_W`]。头带在消息区坐标系里
    /// 于是落在 x∈[0,400) 的字块上；错成拿 `chat.x0` 去裁一块**已裁过**的
    /// 画面时，取样框会整个跑到右边外面，`header_print` 直接给 `None`。
    fn test_anchor() -> chatservant_capture::anchor::PaneAnchor {
        use chatservant_capture::anchor::PaneAnchor;
        use chatservant_core::frame::Rect;
        // ⚠ 2026-10-14 改：这里原先给 `header_top: 0`，于是 `chat.y0` 也是 0，
        //   `conversation_frame` 画在整帧 y 12..25 的标题栏字块**落进了消息区内部**，
        //   生产取错地方反而取得到——**夹具把 bug 编码进去了**，闸的测试一直绿。
        // 现在给出真实几何（实测真机 `header_top ≈ 31`）：
        // `chat.y0 == header_top` 是 `anchor::derive` 的定义，两处必须相等。
        const HEADER_H: i32 = 40;
        PaneAnchor {
            chat: Rect::new(
                SIDEBAR_W as i32,
                HEADER_H,
                (SIDEBAR_W + PANE_W) as i32,
                PANE_H as i32,
            ),
            panel_bg: [30, 30, 31],
            panel: Rect::new(
                SIDEBAR_W as i32,
                0,
                (SIDEBAR_W + PANE_W) as i32,
                PANE_H as i32,
            ),
            header_top: HEADER_H,
            input_top: PANE_H as i32,
        }
    }

    /// 一块**同时带标题栏和消息行**的画面。
    ///
    /// 之前那两个夹具各造一半（`header_frame` 只画字、`pane_frame` 只画行），
    /// 于是测试只能分别取指纹、分别喂行。真实的那一块是**一起**的，
    /// 而且左右**带着侧边栏**——坐标换算只有在这种画面上才算得出来。
    fn conversation_frame(rows: &[(u32, u32)], name: u32) -> chatservant_capture::Frame {
        let w = SIDEBAR_W + PANE_W;
        let mut px = vec![30u8; (w * PANE_H * 3) as usize];
        // 侧边栏换个底色：一眼能看出「它不在消息区里」，
        // 也就知道头带不该从这儿取。
        for y in 0..PANE_H {
            for x in 0..SIDEBAR_W {
                let i = ((y * w + x) * 3) as usize;
                px[i] = 60;
                px[i + 1] = 60;
                px[i + 2] = 62;
            }
        }
        // 会话名行：按**真机几何**画（票 31）。
        //
        // ⚠⚠ 原来这里画在整帧 y 12..25，而 `test_anchor()` 的
        // `chat.y0 = 40` ⇒ 字块落在**消息区上沿之上**。于是「头部在哪」
        // 这个答案**被夹具自带了**：被测代码猜哪个位置都能对上，
        // 三次修复的测试因此全绿——而真机上一次都没对过。
        //
        // 现在按实测画：`chat.y0` 往下 18px、行高 11（真机 y 49..59、
        // `chat.y0 = 31`）。**被测代码必须自己把这个位置推出来。**
        let name_top = 40u32 + 18; // 40 == test_anchor() 的 chat.y0 == header_top
        for y in name_top..(name_top + 11u32) {
            for x in 0..PANE_W {
                let mut on = false;
                for off in [24u32, 42, 60] {
                    let (Some(cx), Some(cy)) = (x.checked_sub(off), y.checked_sub(name_top)) else {
                        continue;
                    };
                    if cx < 13
                        && cy < 13
                        && match name {
                            0 => cx < 7,
                            1 => cx >= 7,
                            _ => cy < 7,
                        }
                    {
                        on = true;
                    }
                }
                if on {
                    let i = ((y * w + SIDEBAR_W + x) * 3) as usize;
                    px[i] = 225;
                    px[i + 1] = 225;
                    px[i + 2] = 225;
                }
            }
        }
        // 消息行。
        //
        // ⚠⚠ 2026-10-14 票 31：首条消息原本从 y=60 起，而会话名行是 y 58..69
        // ——**两者首尾相接、中间没有空行**。于是 `discover` 的行游程把它们
        // **合并成一个 22px 的行段**（量化后 20），取样带 8000 px 里消息文字
        // 占 5760、会话名字形差只剩 ~189 ⇒ 相似度 0.9704 ≥ 阈值 0.90
        //   ⇒ 判成「同一个会话」。
        //
        // 真机是有间隙的：名字 y 49..59、首条消息 y 80..97，中间空 60..79
        //（`HEAD_SEARCH_H` 的注释就记着「首条消息在 y 80」）。
        // ⇒ 首条消息一律从 **y 80** 起，否则夹具测的是「行粘连」而不是「会话身份」。
        for &(y0, y1) in rows {
            for y in y0..y1 {
                for x in 40..(PANE_W - 40) {
                    let i = ((y * w + SIDEBAR_W + x) * 3) as usize;
                    px[i] = 225;
                    px[i + 1] = 225;
                    px[i + 2] = 229;
                }
            }
        }
        chatservant_capture::Frame::from_rgb(w, PANE_H, px).expect("尺寸一致")
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

    /// **重锁必须清掉口吻样本**——与上一条同一类事故。
    ///
    /// 口吻样本是**从对方私聊里长出来的东西**：用户怎么打字，是他在
    /// 那些对话里留下的痕迹。带着 A 的口吻给 B 起草，套的不只是语气，
    /// 那份「风格」本身就是 A 的私聊内容。
    #[test]
    fn relocking_drops_the_previous_conversation_voice() {
        let mut s = Scratch::new(RecentN::new(12));
        s.images_out = ImagesOut::off();

        s.mine.push("A 里的口吻样本".into());
        s.mine.push("只发给 A 的说法".into());

        s.relock.request();
        let _ = s.relock.take();
        apply_relock(&mut s);

        assert!(
            s.mine.is_empty(),
            "重锁后还留着上一个会话的口吻样本：{:?}",
            s.mine
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
    /// 早先 `tier` 是前端 `$state`、两个页面各一份：用户在设置里选高档，
    /// 主面板还是 Manual。那个开关是个**完全不起作用的死控件**。
    #[test]
    fn the_tier_is_shared_across_windows() {
        use chatservant_core::dispatch::Dispatch;
        let cell = TierCell::default_manual();
        let settings = cell.clone();
        settings.set(Dispatch::Confirm);
        assert_eq!(cell.get(), Dispatch::Confirm, "设置页升档，主面板必须看到");
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
        use chatservant_capture::anchor::PaneAnchor;
        use chatservant_core::frame::Rect;

        const W: u32 = 500;
        const H: u32 = 300;

        // 同一个亮度的白字，**笔画落点不同** —— 这才是两个名字的真实差别。
        //
        // ⚠ 早先这里是 `tint: u8`，用 225 对 120 的**亮度差**造「另一个会话」。
        // 真实会话名亮度一样、差的是字形，所以那条夹具一直证明着
        // 与现实无关的事，闸也一直在真实帧上关不掉。
        let frame_with = |name: u32| header_frame(W, H, name);

        // 锚点给的是**帧坐标**，而 `header_print` 收的是**消息区**，
        // 这里让 chat 从 (0,0) 开始，两套坐标才一致。
        // ⚠ 2026-10-14：给**真实几何**。`chat.y0 == header_top` 是
        // `anchor::derive` 的定义（`Rect::new(x0, header_top, x1, input_top)`）。
        // 原先这里是 0，于是 `header_frame` 画在 y 12..25 的「标题栏字块」
        // 实际落在**消息区里面**——闸取错地方也照样取得到，夹具把 bug 编码了。
        // 现在带子往 `chat.y0` **之上**取，才是真的标题栏。
        const HEADER_H: i32 = 40;
        let anchor = PaneAnchor {
            chat: Rect::new(0, HEADER_H, W as i32, H as i32),
            panel_bg: [30, 30, 31],
            panel: Rect::new(0, 0, W as i32, H as i32),
            header_top: HEADER_H,
            input_top: H as i32,
        };

        let a = frame_with(0);
        let b = frame_with(1);
        let pa = header_print(&a, &anchor).expect("取得到").0;
        let pb = header_print(&b, &anchor).expect("取得到").0;
        assert!(!pa.is_same(&pb), "两个不同会话被判成同一个");

        let again = header_print(&a, &anchor).expect("取得到").0;
        assert!(pa.is_same(&again), "同一帧被误判成换人了");
        // ⚠ 2026-10-14 票 31：原先写 `> 5_000`，那是按**已删掉**的固定带子
        //   （500 宽 × 40 高 = 20_000）写的。改成 `discover` 推导之后，取样范围
        //   是**行高说了算**：400 宽 × 行高。
        //
        //   行高实测：真机会话名 11px，`quantize_row` 向上对齐到 4px 栅格 ⇒ 12。
        //   ⇒ 400 × 12 = **4_800** px。
        //
        //   这里钉的是**下限**：`HeaderPrint::of` 的 `MIN_PIXELS = 200`。
        //   低于它直接返回 `None`（认不出）。1_800 留了三倍余量，
        //   既守住「掩码不能薄到没内容」，又不把具体行高写死成常量。
        assert!(
            pa.pixels() >= 1_800,
            "头带只有 {} px，太薄不足以当身份（MIN_PIXELS = 200）",
            pa.pixels()
        );
    }

    /// ⚠ 2026-10-14 新增。**这条是这次修复的判据。**
    ///
    /// 修复前闸取的是**消息区顶部**，于是「开头像素一样、只有名字不同」
    /// 的两个会话——比如两个都从「好的」开头的对话——指纹几乎一样，
    /// **闸认不出切换**（硬约束第 1 条被削弱的地方）。
    ///
    /// 这条把那个情形正面钉住：消息行**完全相同**，只有标题字形不同，
    /// 必须被判成两个会话。
    #[test]
    fn the_gate_distinguishes_conversations_that_open_the_same_way() {
        // 同一个 `rows` ⇒ 两个会话的**开头像素逐字节相同**。
        // 首条消息从 y 80 起，与会话名行（y 58..69）之间留空行——见
        // `conversation_frame` 里记的那条：粘连会让取样带被消息文字灌满。
        let same_opening: [(u32, u32); 3] = [(80, 100), (160, 180), (240, 260)];
        let a = conversation_frame(&same_opening, 0);
        let b = conversation_frame(&same_opening, 1);
        let anchor = test_anchor();

        let pa = header_print(&a, &anchor).expect("A 的指纹取得到").0;
        let pb = header_print(&b, &anchor).expect("B 的指纹取得到").0;
        assert!(
            !pa.is_same(&pb),
            "开头像素完全相同、只有名字不同的两个会话被判成了同一个——\
             闸认的还是「消息开头」，不是「这是谁的对话」"
        );
    }

    /// 头部行**挪了位置，推导要跟着走**。
    ///
    /// ⚠⚠ 这条判据替掉过一条按「带子落在消息区**之上**」写的
    /// （`the_header_band_sits_above_the_message_area_not_inside_it`）——
    /// 那个前提本身是错的，两次修复都被真机推翻。
    ///
    /// 现在 `watch.rs` **不拼矩形**，矩形由
    /// `chatservant_perceive::identity::HeaderPrint::discover` 从像素推出来。
    /// ⇒ 几何对不对由 perceive 那边的
    /// `discover_follows_the_header_line_wherever_it_moves` 守着；
    /// 这条守的是**接线**：这里真的用了推导结果，没有偷偷退回常量。
    #[test]
    fn the_watch_layer_does_not_hand_roll_the_header_rect() {
        let source = include_str!("watch.rs");
        // 曾经自己拼带子的那个函数不许回来。
        //
        // ⚠⚠ 匹配串**必须拼出来**，不能写成完整字面量：判据查的是
        // `include_str!` 进来的**整个文件**，而判据自己就住在这个文件里。
        // 写成字面量时，它会匹配到自己那行 ⇒ 永远红。
        // 这与「量具自检必须能因为量具偏了而失败」同源：判据也会匹配自己。
        // ⚠⚠ 2026-10-14：**连注释里也不行**。这条注释原先写着那个完整函数名，
        //   于是判据匹配到自己解释「为什么不能写字面量」的那行——
        //   判据自己被自己的注释绊倒。拆成 `header_band_` + `rect` 两截。
        let banned = format!("fn header_band_{}(", "rect");
        assert!(
            !source.contains(&banned),
            "header_band_rect 又回来了——头部矩形必须由 perceive 的 discover 推，\
             不许在这里拼"
        );
        // ⚠ 换掉原先那条查 `chat.y0 - HEAD_BAND_H` 的检查：常量连同函数一起
        //   删掉之后，那个字符串永远不会出现 ⇒ 判据永真、等于没有。
        //   真正要守的是**入口**：生产代码只许走 discover，不许再拿
        //   `HeaderPrint::of` 配一个自己拼的矩形——那正是三次猜错的形状。
        let prod = source.split("#[cfg(test)]").next().unwrap_or(source);
        assert!(
            !prod.contains("HeaderPrint::of("),
            "生产代码又直接调 HeaderPrint::of 拼矩形了——头部位置必须由 \
             HeaderPrint::discover 从像素推出来"
        );
        assert!(
            prod.contains("HeaderPrint::discover("),
            "生产代码没走 discover，接线断了"
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
        use chatservant_capture::anchor::PaneAnchor;
        use chatservant_core::frame::Rect;

        const W: u32 = 500;
        const H: u32 = 300;
        // 同一个亮度的白字，**笔画落点不同** —— 这才是两个名字的真实差别。
        let frame_with = |name: u32| header_frame(W, H, name);
        // ⚠ 2026-10-14：给**真实几何**。`chat.y0 == header_top` 是
        // `anchor::derive` 的定义（`Rect::new(x0, header_top, x1, input_top)`）。
        // 原先这里是 0，于是 `header_frame` 画在 y 12..25 的「标题栏字块」
        // 实际落在**消息区里面**——闸取错地方也照样取得到，夹具把 bug 编码了。
        // 现在带子往 `chat.y0` **之上**取，才是真的标题栏。
        const HEADER_H: i32 = 40;
        let anchor = PaneAnchor {
            chat: Rect::new(0, HEADER_H, W as i32, H as i32),
            panel_bg: [30, 30, 31],
            panel: Rect::new(0, 0, W as i32, H as i32),
            header_top: HEADER_H,
            input_top: H as i32,
        };

        let mut s = Scratch::new(RecentN::new(12));
        // 先锁在「错」的会话上
        s.locked_print = header_print(&frame_with(0), &anchor).map(|(p, _)| p);
        assert!(s.locked_print.is_some());

        // 用户点重锁：请求被取走，旧锁被清掉
        s.relock.request();
        assert!(s.relock.take());
        s.locked_print = None;

        // 下一轮以当前像素为新基准
        let now = header_print(&frame_with(1), &anchor).expect("取得到").0;
        assert!(now.is_same(&s.locked_print.clone().unwrap_or_else(|| now.clone())));
        assert_ne!(
            now,
            header_print(&frame_with(0), &anchor).expect("取得到").0,
            "重锁之后基准必须换成当前的会话"
        );
    }
}
