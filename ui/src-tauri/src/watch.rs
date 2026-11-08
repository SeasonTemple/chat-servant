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
use chatservant_core::confidence::ConfidenceTier;
use chatservant_perceive::{Decision, Message, PaneWatcher, Speaker};
use chatservant_reason::{Provider, Reasoner, TurnContext};
use serde::Serialize;

use crate::contract::Phase;

/// 循环的节奏。**慢一点没关系**——静默期本来就不该做事，
/// 而画面变化通常伴随人的操作，1 秒的延迟人感知不到。
pub const TICK: Duration = Duration::from_millis(350);

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

/// 一次观察里各段各调了几次模型。**都要报，包括 0。**
///
/// 界面上要能看出「判断 2 · 起草 1 · 视觉 0」——
/// 只给一个总数的话，任何一段挂了都分不清是哪段。
/// **视觉恒为 0 是正常状态**（默认多模态配置下图片随文本进 B，不额外调），
/// 不报出来用户会以为没统计。
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct CallCounts {
    pub judge: u32,
    pub draft: u32,
    /// 额外视觉模型的调用次数。**只有 B 是纯文本且开了桥时才可能非 0。**
    pub vision: u32,
    // ⚠ **没有「排序」这一项了**（2026-10-25）：排序是**纯本地**的
    // （按 probability 降序），不发请求 ⇒ 没有次数可报。
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
    /// 这是一个**显式字段**，不是让界面去 `reason` 里找关键词。
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
    /// 顺带消掉了一类隐患：以前 `messages` / `candidates` 是 `Readout` 的
    /// 独立字段、`phase` 是另一个来源，两份数据要靠人去保证一致。
    /// 现在它们同属一个枚举变体，**不一致在类型上就构造不出来**。
    #[serde(flatten)]
    pub phase: Phase,
    /// 界面该显示哪句「正在干什么」。**只有起草中非空。**
    ///
    /// 它是 [`Phase::status_hint`] 的快照，**不是界面的第二份口径**。
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
}

/// 一条候选（给前端渲染用）。
///
/// 派生 `PartialEq` 是 [`crate::contract::Phase`] 需要它的连带结果：
/// `Phase` 派生了 `PartialEq`，而 `Drafted` 变体带着这个类型。
/// 这个孤儿模块直到 2026-10-13 才第一次进编译（`main.rs` 里补 `mod contract;`），
/// 于是这处缺失**潜伏了整个文件的存在期**而没人看见。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DraftCandidate {
    pub id: String,
    pub text: String,
    pub probability: f32,
    /// 置信度**档位**（高 / 中 / 低）——界面画笔迹轻重用的就是这个。
    ///
    /// ⚠ **分档在 Rust 侧算**（[`chatservant_core::confidence::ConfidenceTier`]），
    /// 阈值只有那一份 ⇒ 界面不自己按概率分档（前端自己算 ⇒ 阈值有第二份
    /// 拷贝 ⇒ 后端改阈值时界面不跟着改，且**没有任何编译器会告诉你**）。
    ///
    /// ⚠ 面板**不显示百分数**了（2026-10-25 用户定案：首选行只有勾，
    /// 备选行用一道**笔迹轻重**表示置信程度）⇒ 这个字段就是那一格的唯一内容。
    pub confidence: ConfidenceTier,
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
    /// **必须加**。夹具跑出来的界面上写着「文件传输助手」、消息和候选
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

/// 抓指定窗口句柄的一帧并做可读性检查。
fn capture_hwnd_frame(hwnd_raw: isize) -> Result<chatservant_capture::Frame, String> {
    if let Some(why) = chatservant_capture::region::why_wgc_unreadable(hwnd_raw) {
        return Err(why);
    }
    let frame =
        capture_one_frame(hwnd_raw).map_err(
            |e| match chatservant_capture::region::why_unreadable(hwnd_raw) {
                Some(why) => format!("抓帧失败：{e}。{why}"),
                None => format!("抓帧失败：{e}"),
            },
        )?;
    if frame.is_black() {
        return Err("抓到全黑帧——GPU 合成窗口的已知失败模式".into());
    }
    Ok(frame)
}

/// 读一帧。现场模式按 Z-Order 优先尝试前台独立小窗或主窗口；夹具模式只读文件。
fn load_frame(
    src: &FrameSource,
    last_hwnd: &mut Option<isize>,
) -> Result<chatservant_capture::Frame, String> {
    if let FrameSource::Fixture(path) = src {
        return chatservant_capture::frame_from_png(path)
            .map_err(|e| format!("读夹具 {} 失败：{e}", path.display()));
    }

    let cands = chatservant_capture::window::enumerate_for(&WECHAT).map_err(|e| {
        tracing::debug!(error = %e, "枚举微信窗口失败");
        "未检测到微信窗口".to_string()
    })?;
    let ordered = chatservant_capture::window::ordered_chat_candidates(&WECHAT, &cands);
    if ordered.is_empty() {
        let _ = find_chat_window(&WECHAT).map_err(|e| {
            tracing::debug!(error = %e, "找窗口失败");
        });
        return Err("未检测到微信窗口".to_string());
    }

    let mut last_err = String::from("未检测到微信窗口");
    for (idx, hit) in ordered.iter().enumerate() {
        match capture_hwnd_frame(hit.hwnd_raw) {
            Ok(frame) => {
                let is_main = Some(hit.title.as_str()) == WECHAT.main_title;
                if !is_main
                    && idx + 1 < ordered.len()
                    && chatservant_capture::anchor::derive(&frame).is_err()
                {
                    continue;
                }
                *last_hwnd = Some(hit.hwnd_raw);
                crate::commands::set_active_hwnd(hit.hwnd_raw);
                return Ok(frame);
            }
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
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
    /// 会话切换策略（锁定当前会话 / 自动跟随当前会话）。
    pub session_mode: SessionModeCell,
    /// 两组模型配置。**循环自己要用**——
    /// 传输变了要按**当前**格子重建（改了 key / 换了厂商要立刻生效）。
    pub groups: crate::commands::ModelGroups,
    /// 出站代理。重建传输时要用**同一个**代理格。
    pub proxy: chatservant_reason::ProxyCell,
    /// **启动时这条传输的指纹。**
    ///
    /// 循环拿它当「上一次重建的那一条」的基准——变了才重建。
    /// ⚠ 必须**随传输一起交出去**：循环线程起来时全局状态可能还没建好，
    ///   那时自己算会得到「没有」，第一圈就会误判成「传输变了」。
    pub transport_fp: u64,
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
    /// 后者是「现在该回什么」。混在一起两件事都会退化成同一份大杂烩：
    /// `history` 里两边的原话按时间混排，而口吻样本要的是**只属于我的那几句**。
    mine: Vec<String>,
    /// 最近一次识别出的消息流。**发给界面渲染用。**
    ///
    /// 它以前**根本不存在**：`step` 里算出 `messages` 只在这一轮里用，
    /// readout 里写死 `messages: Vec::new()`。于是界面上的消息流永远是空的
    /// ——即使识别完全正常、候选都出来了，聊天记录那一片永远是「还没有消息」。
    messages: Vec<Message>,
    /// **已处理过的消息账本**（保留重复正文）。
    ///
    /// ## 为什么不能直接用 [`Scratch::messages`]
    ///
    /// `messages` 走 `merge_into_stream`——那是**面板显示**语义，
    /// 它会把**重复正文**去掉 ⇒ 它的计数不反映「这句话处理过几次」。
    /// 而新颖度要的是**真实计数**（对方把同一句又说一遍 ⇒ 第二句是新的）。
    ///
    /// ## 它管什么
    ///
    /// 「哪几条是刚到的」的判据基准（见 [`absorb_round`]）。
    /// 用户**往上翻历史**时，画面里全是更早的消息 ——
    /// 若拿「最近几条」当基准，它们落在窗口外 ⇒ 被当成新的
    /// ⇒ **对旧内容触发推理**（Auto 档下还可能误发）。
    ///
    /// 换会话（重锁）时必须清空：那是另一段对话了。
    seen: Vec<Message>,
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
    /// **面板当前停在别的会话上**（指纹确实不同，不是「认不出头部」）。
    ///
    /// ## 它只管**显示**，不管状态
    ///
    /// 锁定会话的消息流留在 [`Scratch::messages`] 里，**切走时不清**——
    /// 切回来时要原样恢复（见 [`park_on_other_conversation`]）。
    /// 但停泊期间**面板不许显示它们**：屏幕上那块是别人的会话，
    /// 面板却挂着锁定会话的对话，那就是票 32 说的「停泊面板说假话」。
    ///
    /// ⇒ 「状态留着」与「显示清空」必须分开，否则只能二选一：
    /// 留着就骗人，清空就丢历史。
    parked_on_other: bool,
    /// 一帧从哪来。见 [`FrameSource`]。
    source: FrameSource,
    /// 现场模式下最近一次用到的窗口句柄。给日志与投递用。
    last_hwnd: Option<isize>,
    /// 上一帧验证过的消息区几何锚点（若本帧 4 条边界特征线未动则直接 `< 0.05ms` 复用）。
    last_anchor: Option<PaneAnchor>,
    /// `Auto` 档自动发送的护栏状态。见 [`AutoGate`]。
    auto_gate: AutoGate,
    /// 会话切换策略（锁定当前 / 自动跟随）。
    session_mode: SessionModeCell,
    /// 多会话分桶记忆（最多保留最近 16 个会话的独立上下文、已见账本与候选），
    /// 切到别的会话再切回时直接从桶中恢复，既不跨会话串味，也不丢失已聊历史。
    buckets: Vec<SessionBucket>,
}

const MAX_SESSION_BUCKETS: usize = 16;

#[derive(Clone)]
struct SessionBucket {
    print: chatservant_perceive::identity::HeaderPrint,
    locked_name: String,
    history: Vec<String>,
    mine: Vec<String>,
    messages: Vec<Message>,
    seen: Vec<Message>,
    candidates: Vec<DraftCandidate>,
    reason: String,
}

/// `Auto` 档的自动发送护栏。**2026-10-15 新增。**
///
/// ## 为什么需要它
///
/// `Auto` 档 = **不用点，程序自己填入并发送**。这在产品上是有效的，
/// 但直接实现会踩四个已知的坑（每一条都有依据，不是假想出来的风险）：
///
/// 1. **自己触发自己，无限循环。** 自动回复最经典的死法：我发的内容
///    再次命中触发条件，于是自己回自己，来回不停。**这一条是根因。**
/// 2. **对方连发 N 条 ⇒ 我回 N 条。** 看着像「很积极」，实际是刷屏，
///    而且每一条都撤不回来。
/// 3. **降级候选（`blind`）的概率不可信**，按它选「最高置信度」
///    等于按一个没有意义的数字做决定。
/// 4. **回得太规律 = 机器感。** 真人聊天有停顿。
///
/// ## 为什么做成一个结构而不是散在循环里
///
/// 「这一轮要不要自动发」是**决定一条消息会不会发进真人聊天窗口**的判断。
/// 它必须是**可穷举的测试**，写成散落在循环里的 `if` 读错一次
/// 就是发错一条真消息——本仓已经栽过（`Arming` 那个死胡同）。
#[derive(Debug, Default)]
struct AutoGate {
    /// 上一次**真的自动发送出去**的时刻。冷却期用它算。
    last_sent: Option<std::time::Instant>,
    /// 这一轮是否已经自动发过了。对方不说话就不重置——
    /// 于是「对方说了 N 条」也只会回一条。
    spent_this_round: bool,
}

/// 冷却时长。**30 秒**是自动回复的常见量级。
///
/// 为什么不是 0：冷却为 0 等于没有冷却，对方连发两条我就回两条。
const AUTO_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(10);

impl AutoGate {
    /// 这一轮**能不能自动发**。纯判定，不产生副作用。
    ///
    /// 逐条护栏：
    /// · `tier_is_auto`——只有 `Auto` 档才自动发；
    /// · 未发过本轮——对方的 N 条合成一条回复；
    /// · 冷却已过——挡连发，也留出反悔的余地。
    ///
    /// `blind` 不在这里判：那是**候选集的性质**，调用点才拿得到。
    ///   判据写在调用点，见 [`Scratch::step`] 里自动发送那段。
    fn may_auto_send(
        &self,
        tier_is_auto: bool,
        now: std::time::Instant,
    ) -> Result<(), &'static str> {
        if !tier_is_auto {
            return Err("不是自动发送档");
        }
        if self.spent_this_round {
            return Err("这一轮已经自动发过了");
        }
        if let Some(t) = self.last_sent
            && now.duration_since(t) < AUTO_COOLDOWN
        {
            return Err("冷却期内，等对方把话说完");
        }
        Ok(())
    }

    /// 记一次「真的发出去了」。
    fn record_sent(&mut self, now: std::time::Instant) {
        self.last_sent = Some(now);
        self.spent_this_round = true;
    }

    /// **对方又开口** ⇒ 进入新一轮，允许再自动发一次。
    ///
    /// 冷却**照旧生效**——「对方又说话」不是「冷却结束了」。
    ///   写成「一来消息就清冷却」等于没有冷却，这是最容易写错的一处。
    fn begin_new_round(&mut self) {
        self.spent_this_round = false;
    }
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
            seen: Vec::new(),
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
            parked_on_other: false,
            // 默认现场；`spawn` 会用 `LoopConfig` 里的那份覆盖。
            source: FrameSource::Live,
            last_hwnd: None,
            last_anchor: None,
            auto_gate: AutoGate::default(),
            session_mode: SessionModeCell::default_lock(),
            buckets: Vec::new(),
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
            1 => chatservant_core::dispatch::Dispatch::Auto,
            // **未知一律当 Manual。** 解析失败就升档是最危险的默认值。
            //
            // 编码 `2` 曾经是 2026-10-13 取消、2026-10-15 又请回来的第三档，
            //   它从来不是现行两档的任何一个，所以落进 `_` ⇒ 读成 **Manual（最低档）**，
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
pub struct Relock {
    flag: Arc<std::sync::atomic::AtomicBool>,
    wake: Arc<(std::sync::Mutex<()>, std::sync::Condvar)>,
}

impl Relock {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            wake: Arc::new((std::sync::Mutex::new(()), std::sync::Condvar::new())),
        }
    }

    /// 请求重锁并立即唤醒观察线程（零等待进入下一轮）。
    pub fn request(&self) {
        self.flag.store(true, Ordering::Relaxed);
        self.wake.1.notify_all();
    }

    /// 读一次就清掉——**重锁是一次性的**，不该每轮都重新认人。
    pub fn take(&self) -> bool {
        self.flag.swap(false, Ordering::Relaxed)
    }

    /// 在观察循环末尾等待 `timeout`，若中途收到 `request()` 则立即提前醒来。
    pub fn wait_timeout(&self, timeout: Duration) {
        if self.pending() {
            return;
        }
        let g = self.wake.0.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self
            .wake
            .1
            .wait_timeout_while(g, timeout, |_| !self.pending());
    }

    pub fn pending(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }
}

impl Default for Relock {
    fn default() -> Self {
        Self::new()
    }
}
/// 会话切换策略：锁定单个会话（`Lock`，默认保守值）或自动跟随微信当前激活会话（`Follow`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionMode {
    #[default]
    Lock = 0,
    Follow = 1,
}

impl SessionMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lock => "lock",
            Self::Follow => "follow",
        }
    }

    #[must_use]
    pub fn from_key(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "follow" => Self::Follow,
            _ => Self::Lock,
        }
    }
}

/// 跨命令层与观察循环共享的会话切换模式原子单元。
#[derive(Clone)]
pub struct SessionModeCell(Arc<std::sync::atomic::AtomicU8>);

impl SessionModeCell {
    #[must_use]
    pub fn default_lock() -> Self {
        Self(Arc::new(std::sync::atomic::AtomicU8::new(
            SessionMode::Lock as u8,
        )))
    }

    #[must_use]
    pub fn get(&self) -> SessionMode {
        match self.0.load(Ordering::Relaxed) {
            1 => SessionMode::Follow,
            _ => SessionMode::Lock,
        }
    }

    pub fn set(&self, mode: SessionMode) -> SessionMode {
        self.0.store(mode as u8, Ordering::Relaxed);
        mode
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
    /// 上一份快照与事件驱动唤醒条件变量，供 `cs-pump` 零轮询阻塞等待。
    latest: Arc<(std::sync::Mutex<Option<Readout>>, std::sync::Condvar)>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.latest.1.notify_all();
    }
}

impl Watcher {
    pub fn latest(&self) -> Option<Readout> {
        self.latest
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    #[must_use]
    pub fn notified_handle(&self) -> Arc<(std::sync::Mutex<Option<Readout>>, std::sync::Condvar)> {
        self.latest.clone()
    }
}

fn init_scratch(cfg: &LoopConfig) -> Scratch {
    let mut s = Scratch::new(cfg.recent_n.clone());
    // 会话名闸的目标从 LoopConfig 拷进 Scratch，避免 step 额外收参数。
    s.locked_peer = cfg.locked_peer.clone();
    // 总闸与预算共用同一份句柄，确保设置页改动对观察循环立即可见。
    s.images_out = cfg.images_out.clone();
    s.token_budget = cfg.token_budget.clone();
    s.provider = cfg.provider;
    s.relock = cfg.relock.clone();
    s.session_mode = cfg.session_mode.clone();
    s.source = cfg.source.clone();
    s
}

fn push_readout(latest: &(std::sync::Mutex<Option<Readout>>, std::sync::Condvar), out: Readout) {
    let mut g = latest.0.lock().unwrap_or_else(|e| e.into_inner());
    *g = Some(out);
    drop(g);
    latest.1.notify_all();
}

fn log_watch_tick(ticks: u64, s: &Scratch) {
    if ticks % 20 == 1 {
        tracing::info!(
            target: "chat_servant::watch",
            "tick={ticks} peer={:?} candidates={}",
            s.peer,
            s.candidates.len()
        );
    }
}

fn maybe_rebuild_transport(
    transport_fp: &mut u64,
    groups: &mut crate::commands::ModelGroups,
    proxy: &chatservant_reason::ProxyCell,
    reasoner: &mut Box<dyn Reasoner + Send>,
    s: &mut Scratch,
) {
    // 判据是传输指纹是否变化：改了厂商或 key 下一次生成立即生效，未改则不重建 runtime。
    let fp = crate::commands::transport_fingerprint().unwrap_or(*transport_fp);
    if fp != *transport_fp {
        *transport_fp = fp;
        if let Some(latest_groups) = crate::commands::current_groups() {
            *groups = latest_groups;
        }
        let (new_reasoner, ready) = crate::commands::build_reasoner(groups, proxy);
        *reasoner = new_reasoner;
        s.provider = groups.llm.provider();
        tracing::info!(
            transport_ready = ready,
            decision = %groups.decision.provider().key(),
            llm = %groups.llm.provider().key(),
            "传输已按当前设置重建（key 或厂商变了）"
        );
    }
}

fn build_failed_readout(
    e: String,
    s: &Scratch,
    anchor: Option<PaneAnchor>,
    ticks: u64,
    last_calls: u32,
    st: chatservant_reason::StageCalls,
) -> Readout {
    // 失败原因必须进日志，且保留已读到的消息以免单次 OCR 抖动清空对话。
    tracing::warn!(target: "chat_servant::watch", "这一轮失败：{e}");
    let phase = Phase::Failed {
        messages: s.messages.clone(),
        error: e,
    };
    snapshot(s, anchor, phase, ticks, last_calls, st, "这一轮没跑通")
}

/// 起一条观察循环。
///
/// **会立刻起一个新线程**。返回的 `Watcher` 被 drop 时线程自己会停。
pub fn spawn(cfg: LoopConfig) -> Watcher {
    let stop = Arc::new(AtomicBool::new(false));
    let latest = Arc::new((std::sync::Mutex::new(None), std::sync::Condvar::new()));
    let latest_bg = latest.clone();
    let stop_bg = stop.clone();

    std::thread::Builder::new()
        .name("cs-watch".into())
        .spawn(move || {
            let mut s = init_scratch(&cfg);
            let mut watcher = PaneWatcher::new(cfg.ignore_top);
            let mut reasoner = cfg.reasoner;
            let mut groups = cfg.groups;
            let proxy = cfg.proxy;
            let mut transport_fp = cfg.transport_fp;
            let mut last_calls = 0u32;
            let mut ticks = 0u64;
            let mut anchor: Option<PaneAnchor> = None;
            let mut last_failed = false;

            while !stop_bg.load(Ordering::Relaxed) {
                ticks += 1;
                log_watch_tick(ticks, &s);
                maybe_rebuild_transport(
                    &mut transport_fp,
                    &mut groups,
                    &proxy,
                    &mut reasoner,
                    &mut s,
                );
                let t_round = std::time::Instant::now();
                let mut interim = |out: Readout| push_readout(&latest_bg, out);
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
                        let phase = current_phase(&s);
                        let out = snapshot(&s, anchor, phase, ticks, last_calls, st, &s.reason);
                        push_readout(&latest_bg, out);
                        last_failed = false;
                    }
                    Ok(None) => {
                        // 静默期不发，除非上一轮失败需要补一帧正常态让界面恢复。
                        if let Some(phase) = phase_to_push(&s, false, last_failed) {
                            let st = reasoner.stage_calls();
                            let out = snapshot(&s, anchor, phase, ticks, last_calls, st, &s.reason);
                            push_readout(&latest_bg, out);
                        }
                        last_failed = false;
                    }
                    Err(e) => {
                        let st = reasoner.stage_calls();
                        let out = build_failed_readout(e, &s, anchor, ticks, last_calls, st);
                        push_readout(&latest_bg, out);
                        std::thread::sleep(Duration::from_millis(2500));
                        last_failed = true;
                    }
                }
                s.relock.wait_timeout(TICK);
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
/// **为什么闸一关就要压过「正在起草」**：闸一关，候选就被清空了，
///   那一轮不会有任何产出。说「正在起草，通常几秒」是在骗用户等。
///
/// **为什么由 Rust 出这句话**：界面分不出「对方真没说话」与
///   「面板已停泊、没在读这个会话」——两者的 `messages` 都是空数组。
///   2026-10-14 真机实测：切到另一个会话后，面板正文显示的是
///   「对方说完话，这里才落笔。」，可那个会话对方明明说过话。
///   **那句话在停泊这条路径上是假的。**
///
/// ⇒ 与 `types.ts` 的纪律一致：`status_hint` 是 Rust 的口径，
///   **界面不许自己写死这句**。为什么要有下一半：
///   判据 12「空状态必须给一个明确的下一步动作」——
///   光说「不动手」用户不知道该干嘛，「切回来才继续」才是动作。
const GATE_CLOSED_HINT: &str = "锁着别的对话，切回来才继续";

/// **我方把着话头时**面板要说的话。
///
/// ## 它替掉的是一句假话（2026-10-25 真机，缺陷帧 `frame-1793475836.png`）
///
/// 那一帧：对方 1 条在最上面、我方 8 条在后面。闸开、这轮没起草、阶段是
/// `Read` ⇒ [`Phase::status_hint`] 返回 `None` ⇒ 前端回落到它自己写死的那句
/// 「对方说完了，这轮没起草」——而那一帧里**最后一句是我方说的**，
/// 那句话是假的。
///
/// ## 为什么前端堵不住
///
/// 它的判据是「最后一条**对方**消息存不存在」（`ui/src/lib/PaperPanel.svelte:249`
/// 的 `messages.filter(m => m.speaker === 'Them').at(-1)`），不是「最后一条消息
/// 是不是对方」——我方把着话头时它照样非空。文案归 Rust 出（`types.ts` 的纪律），
/// 所以判据也在这边，前端零改动。
///
/// 与 [`GATE_CLOSED_HINT`] 同风格：短句、只陈述现状、不下指令。
///
/// **一律不带句号**（2026-10-31 用户定案：「起草好了。为何要有句号？」）——
/// 状态句是纸上的**一行提示**，不是完整句子。这句话本身也顺势收短了：
/// 上一版是「你说了最后一句，等对方回话。」，用户要的是两个字的状态词。
const I_HAVE_THE_FLOOR_HINT: &str = "等待回复";

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
    // 必须在 `phase` 被移进结构体**之前**取：它的口径来自
    //   `Phase::status_hint`，而结构体字面量是按书写顺序求值的。
    // 闸关着时**改用** [`GATE_CLOSED_HINT`]，理由见那里那条注释。
    let status_hint = if s.gate_open {
        phase.status_hint().map(str::to_owned).or_else(|| {
            // 只有 `Read` 这一档才补：`Drafting` 有「正在起草」，`Failed` 故意
            // 不说话（靠顶部告警条），`Drafted` 本身就是产出——补一句会把真状态
            // 盖掉。
            match &phase {
                Phase::Read { messages }
                    if matches!(messages.last().map(|m| m.speaker), Some(Speaker::Me)) =>
                {
                    Some(I_HAVE_THE_FLOOR_HINT.to_owned())
                }
                _ => None,
            }
        })
    } else {
        Some(GATE_CLOSED_HINT.to_owned())
    };
    // ⚠⚠ **停泊在别的会话上时，消息列表恒为空**（2026-10-23 改）。
    //
    // `s.messages` 记的是**锁定会话**的历史，而停泊时屏幕上那块是
    // 别人的对话 ⇒ 原样推出去就是「停泊面板说假话」（票 32）。
    //
    // 历史**留在状态里**（`s.messages` 没被清过）⇒ 切回来原样恢复，
    // 不用靠整段重读把历史长出来（那会触发重复推理）。
    let phase = if s.parked_on_other {
        blank_messages(phase)
    } else {
        phase
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
            // 视觉桥**默认关闭**（ticket 15 第 1 条），循环里没接桥之前
            // 这里恒为 0——这是**正确值**，不是占位符。
            vision: s.vision.calls(),
        },
        ticks,
        ocr_calls: s.ocr_calls,
    }
}

/// 一轮「读」的结果。
struct Round {
    /// OCR 一共读到几行（**没按 ROI 过滤之前**）。诊断用。
    total_lines: usize,
    /// 落在本轮 ROI 里的行（**消息区坐标**）。
    lines: Vec<chatservant_capture::ocr::OcrLine>,
    /// 归类结果（行 → 对话人 / 噪声）。
    attrs: Vec<chatservant_perceive::Attribution>,
    /// 归并出来的消息。
    messages: Vec<Message>,
    /// 真正送进 OCR 的像素数。
    ocr_pixels: u64,
}

/// **一轮「读」：从帧里读出本轮该管的消息。**
///
/// ## 它为什么必须与探针走同一条路
///
/// 这就是**产品读帧路径**。它必须与 [`chatservant_perceive::read_lines`]
/// （端到端探针那条路）**逐字同一套配方**：OCR 看哪一块画面、放大几倍、
/// 行坐标怎么换算回消息区。
///
/// 曾经这份配方在这里**手抄了一遍**——「整帧 + `OCR_UPSCALE` 倍」，
/// 而那是 **WinRT** 实测要的那一组（2026-10-07）。默认引擎 PP-OCR 要的
/// 恰恰相反：**只给消息区 + 1 倍**（2026-10-18 实测）。
///
/// ⇒ 两边分叉，**而分叉不报错**：门禁全绿、探针全绿，
/// 只有真机上的消息数是 0（票 72）。
///
/// ## 为什么值得抽成一个函数
///
/// 配方写在 [`step`] 里面的时候，**没有任何机器判据能发现它写错了**——
/// 那段代码「看起来」完全合理。于是它变成一段只能靠真机发现的代码。
/// 抽出来之后：
/// · 配方由 [`chatservant_perceive::read_lines`] 提供，**只有一处**；
/// · 「产品路径 ≡ 探针路径」变成一条可测的判据（见
///   `the_product_read_path_agrees_with_the_probe_path`）；
/// · 「产品路径递进去的那张图对不对」也能测（见
///   `the_product_read_path_hands_ocr_the_view_the_engine_asks_for`）。
fn read_round(
    frame: &chatservant_capture::Frame,
    pane: &chatservant_capture::Frame,
    anchor: &PaneAnchor,
    roi: chatservant_core::frame::Rect,
    source: &impl chatservant_perceive::LineSource,
) -> Result<Round, String> {
    // 画面与倍数**由引擎决定**。这里不许再手抄一份——抄一份就会分叉，
    // 而分叉的后果是「看得见行、归成 0 条消息」，不报错。
    let ocr = chatservant_perceive::read_lines(frame, anchor.chat, source)
        .map_err(|e| format!("读行失败：{e}"))?;

    // 总行数要先取：**下面要把 `ocr.lines` 移走**。
    let total_lines = ocr.lines.len();

    // ROI 过滤：只留本轮该管的那些行。**ROI 与行框都是消息区坐标**
    // （`read_lines` 已经把行换算回消息区了），两边可以直接比。
    //
    // ⚠ 2026-10-23：ROI 现在覆盖**整块消息区**。
    // 原来它只取「水位线以下」那条带子，而**底部对齐**的聊天窗（微信）
    // 里新消息落在**旧水位线之上** ⇒ 那条带子结构性读不到它（票 83）。
    // 「哪几条是新的」改由**内容身份**判（`novel_messages`）。
    let lines: Vec<chatservant_capture::ocr::OcrLine> = ocr
        .lines
        .into_iter()
        .filter(|l| {
            l.rect.y1 > roi.y0 && l.rect.y0 < roi.y1 && l.rect.x1 > roi.x0 && l.rect.x0 < roi.x1
        })
        .collect();

    let palette = chatservant_perceive::Palette::learn(frame, anchor.chat)
        .ok_or_else(|| "学不到配色（消息区太平）".to_string())?;

    // ⚠ 归类采样要用的帧，**必须与行框同一坐标系**。
    // 行框是**消息区相对**的，所以这里给的是**消息区裁剪件**（`pane`），
    // 不是整帧。给整帧的话，外圈探针采到的是别处的像素，
    // 于是每行都被判成「不在气泡里」⇒ 消息数为 0。
    let attrs = chatservant_perceive::classify_all(&lines, pane, &palette);
    let messages = chatservant_perceive::group(&attrs);

    Ok(Round {
        total_lines,
        lines,
        attrs,
        messages,
        // 开销口径与真正递进去的那张图**逐字一致**，不在这里手算。
        ocr_pixels: chatservant_perceive::ocr_input_pixels(frame, anchor.chat, source),
    })
}

/// **已处理账本**留多少条。
///
/// ⚠ 这是残留的边界：**账本之外**的旧消息（用户往前翻出来的）
/// 身份不在账本里 ⇒ 仍可能被当成新的 ⇒ 触发一次对旧内容的推理。
///
/// 给 512 条（≈ 25 KB）：「往前翻 512 条消息」才可能出界 —— 真实使用里翻不到。
/// 计数判据本来就有「抵掉一个」的语义 ⇒ 账本越大越保守（越少误报）。
pub(crate) const SEEN_WINDOW: usize = 512;

/// **把我们自己说过的话，从「对方说的」改判回「我说的」。**
///
/// ## 为什么需要它（2026-10-24，用户真机报出）
///
/// 用户在 `Auto` 档点了一条候选 ⇒ 面板回执说「可能已经发出去了」
/// ⇒ 紧接着**又起草了一轮**。他怀疑「自己发出去的消息被当成了对方的新消息」。
///
/// 归类器靠**气泡底色的彩度**判说话人（饱和 = 我 / 近灰 = 对方）。
/// 真机帧实测它判对了（绿色气泡 `彩度=91` ⇒ 「我」✔），
/// 但那是**运气**：探针只要有一次采到气泡缝，彩度就掉到 0 ⇒ 判成「对方」。
/// ⇒ 一旦判错，那句话就是「对方的新消息」⇒ 触发一次完整推理
/// ⇒ `Auto` 档下还可能**再发一条** ⇒ **自问自答**。
///
/// ## 判据：**正文**，不是颜色、不是位置、不是时间
///
/// 「我们自己说过这句话」是**事实**，颜色只是**猜测**。
/// ⇒ 事实优先于猜测：正文对得上，就按我方算。
///
/// ## 改的是**说话人**，不是丢弃
///
/// 丢弃的话，面板上就看不到自己刚发的那条了；
/// 改判的话，它以「我」的身份进消息流 —— 这本来就是它应有的身份。
///
/// ## 残留
///
/// 对方在 10 分钟内说了**一模一样**的正文 ⇒ 那句会被算成我方
/// ⇒ 代价是**少起草一轮**（不是多发），方向是安全的。
fn relabel_our_own(messages: &mut [Message]) {
    for m in messages.iter_mut() {
        if m.speaker == Speaker::Them && crate::commands::sent_by_us(&m.text) {
            m.speaker = Speaker::Me;
            // `id` 是「说话人 + 正文」的口径（见 `perceive::message_id`），
            // ⇒ 改判之后必须同步，否则前端 keyed 渲染会认成另一条。
            m.id = format!("{speaker:?}:{text}", speaker = Speaker::Me, text = m.text);
        }
    }
}

/// **把本轮读到的消息并进消息流，并返回「这一轮新到的那些」。**
///
/// ## ⚠ 顺序是承重的：**先算新颖度，再并入流**（2026-10-23 修）
///
/// 先并后算的话，**本轮的消息已经在流里了** ⇒ 它们被判成「不新」
/// ⇒ 真正新到的对方消息**不会触发推理**。
///
/// 真机夹具实测（那个顺序下）：本轮读到 8 条、面板从 5 涨到 10，
/// 但「本轮新到」只有 **2** 条 —— 恰好是两条**旧**消息
/// （对方消息算错 ⇒ 推理被旧消息误触发）。
///
/// 合成帧那张夹具不会暴露这个：它每一轮都是**新画面**，
/// 并进去之后再算，与先算后并的结果一样。
///
/// ## 去重口径
///
/// `novel_messages` 与 `merge_into_stream` 用**同一套**判据
/// （说话人 + 正文，只比最近 `DEDUP_TAIL` 条）⇒
/// 「被判成新的」与「会被并进流」是同一批，不会出现两处口径打架。
fn absorb_round(
    seen: &mut Vec<Message>,
    display: &mut Vec<Message>,
    current: &[Message],
) -> Vec<Message> {
    // 找出本画面里「已经在账本 seen 里出现过」的最底部那条消息的 y1。
    // 微信是底部对齐列表：真正新到的消息一定出现在已见消息的下方（y1 >= max_known_y1 - 8）；
    // 而发送消息后画面整体上滚时，视口顶部（如 y=216）刚露出来或因滚边产生 1 字 OCR 抖动的
    // 历史旧消息，其 y1 远小于底部已见消息（如 216 << 560）。
    // 那类顶部历史行虽然要记入 seen 防重，但绝不能追加到 display 末尾冒充「最新一句」，
    // 更不能触发新一轮推理。
    let max_known_y1 = if seen.is_empty() {
        None
    } else {
        current
            .iter()
            .filter(|m| {
                seen.iter()
                    .any(|s| s.speaker == m.speaker && s.text == m.text)
            })
            .map(|m| m.rect.y1)
            .max()
    };

    let raw_novel = chatservant_perceive::novel_messages(seen, current);
    seen.extend(raw_novel.iter().cloned());
    if seen.len() > SEEN_WINDOW {
        seen.drain(..seen.len() - SEEN_WINDOW);
    }

    let mut novel = Vec::with_capacity(raw_novel.len());
    for m in raw_novel {
        if let Some(ky1) = max_known_y1
            && m.rect.y1 + 8 < ky1
        {
            // 位于已见消息上方的顶部旧消息：插在 display 头部，不篡改末尾最新引文，也不算新到消息
            display.insert(0, m);
        } else {
            display.push(m.clone());
            novel.push(m);
        }
    }
    if display.len() > chatservant_perceive::MESSAGE_WINDOW {
        let drop_n = display.len() - chatservant_perceive::MESSAGE_WINDOW;
        display.drain(..drop_n);
    }
    let mut used_ids = std::collections::HashSet::new();
    for m in display.iter_mut() {
        let base = format!("{:?}:{}", m.speaker, m.text);
        if used_ids.insert(base.clone()) {
            m.id = base;
        } else {
            for n in 2u32.. {
                let cand = format!("{base}#{n}");
                if used_ids.insert(cand.clone()) {
                    m.id = cand;
                    break;
                }
            }
        }
    }
    novel
}

/// 一行进 `history` 的样子：**带说话人**。
///
/// ## 为什么标签写在写入口，而不是渲染时
///
/// [`chatservant_reason::Context::trim`] 与 `Trimmed::render` 手上只有
/// `Vec<String>`，到那一步说话人已经丢了。标签在这里焊进正文之后，
/// 摘要那一路（同样只吃字符串）、摘要失败时的兜底那一路、以及将来任何读
/// `history` 的人都会自动带上——不用每处各记一条规则，而「每处各记一条」
/// 正是这个缺陷活下来的原因。
fn history_line(m: &Message) -> String {
    let who = match m.speaker {
        Speaker::Me => "我",
        Speaker::Them => "对方",
        // 既不是我也不是对方（系统提示之类）。**也要给个标签**——
        // 留一行光秃秃的原文，模型就又回到「这几行分不出谁说的」那个状态。
        Speaker::System => "系统",
    };
    format!("{who}：{}", m.text)
}

/// 「两个字被 rec 当成一个字」的诊断判据。返回 (类别, 宽, 高)。
///
/// ## 这里只做诊断，不改任何判读
///
/// 真机实测（票 96 缺陷 B）：一个 33×20 的框里是两个字，而 PP-OCRv6 small /
/// medium、v5 server、v4 server **四份权重**都读不对——整块读成 1 个码位
/// （p≈0.99）；拆成左右两半再喂，右半能读对，左半在三代模型上的最大概率都
/// ≤0.0024。⇒ 这是模型的 **prior**，不是分辨率不够；换权重、调采样、
/// 框内超采样六次都白费。
///
/// 本地渲染 + 形状匹配（`msyh.ttc` + 抹掉长宽比 + NCC）能认出来，但代价是
/// 引入字体光栅化器、依赖微软专有字体（**不能**打包进安装包）、且只验过
/// **一个**正例。所以先不修，先量频率——判据就是这里，落点是 `step` 里那行日志。
///
/// 两种形态各记一类，因为它们在真机上的样子不一样：
/// - `单码位宽框`：整块被读成**一个**汉字（`w >= 1.5*h`）。阈值来自同帧实测：
///   除目标行（1.62）外，其余「只认出 1 个 token」的行宽高比都在 0.88~1.33。
/// - `混进片假名`：读出来带**片假名**。中文对话里片假名几乎只有 OCR 才产生，
///   而那一块正是被读成「一个汉字 + 一个片假名」。
///
/// ⚠ 日志里**只许打几何，不许打正文**（硬约束 3）——那一个字就是聊天内容。
fn merged_pair_suspect(m: &Message) -> Option<(&'static str, i32, i32)> {
    let w = m.rect.width();
    let h = m.rect.height();
    if h <= 0 {
        return None;
    }
    let mut chars = m.text.chars();
    let only = chars.next()?;
    if chars.next().is_none()
        && is_cjk_ideograph(only)
        && w as f32 >= WIDE_SINGLE_CHAR_RATIO * h as f32
    {
        return Some(("单码位宽框", w, h));
    }
    if m.text.chars().any(is_katakana) && m.text.chars().any(is_cjk_ideograph) {
        return Some(("混进片假名", w, h));
    }
    None
}

/// 「单码位宽框」的判据下限，见 [`merged_pair_suspect`]。
const WIDE_SINGLE_CHAR_RATIO: f32 = 1.5;

/// 汉字：CJK 统一表意文字（常用区、扩展 A、兼容区）。
///
/// 判据是**码位区间**，不是「看起来像汉字」——emoji、全角标点、假名都该被排除，
/// 否则这一条诊断会被它们带偏（真机里那些行本来也窄，报出来只会稀释频率）。
fn is_cjk_ideograph(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF)
}

/// 片假名（含半角）。见 [`merged_pair_suspect`] 的第二类。
fn is_katakana(c: char) -> bool {
    matches!(c as u32, 0x30A1..=0x30FA | 0x30FC..=0x30FF | 0xFF66..=0xFF9D)
}

/// 这一轮该不该起草。**这是唯一的判据入口**，`step` 只调它。
///
/// ## 为什么要「最后一句是对方说的」这半个条件
///
/// 2026-10-25 真机：对方只有 1 条（在很上面），用户自己 8 条在后面。
/// 模型看到最后一段全是「我」，于是顺着用户自己的话往下写——面板给出的
/// 候选是在接用户的话，而不是回复对方。
///
/// 用户刚说完、对方还没回时本来就没有可回复的对象，所以这一条同时挡住
/// 「自问自答」（`Auto` 档下多发一条的入口）。
///
/// `peer_spoke_now` 那半是硬约束第 6 条：没有新的对方消息就一次都不调，
/// 最后一句是不是对方说的都不例外。
fn should_draft_now(peer_spoke_now: bool, visible: &[Message]) -> bool {
    peer_spoke_now && matches!(visible.last().map(|m| m.speaker), Some(Speaker::Them))
}

/// 当前这一刻的阶段：**有没有候选**决定是 `Read` 还是 `Drafted`。
///
/// 阶段由 Rust 判，不让界面去猜 `candidates` 空不空。
fn current_phase(s: &Scratch) -> Phase {
    if s.candidates.is_empty() {
        Phase::Read {
            messages: s.messages.clone(),
        }
    } else {
        Phase::Drafted {
            messages: s.messages.clone(),
            candidates: s.candidates.clone(),
        }
    }
}

/// 这一轮要不要给界面推一帧，推的是什么阶段。
///
/// - `changed`：这一轮有内容变化（`step` 返回 `Ok(Some)`）。
/// - `last_failed`：上一轮推的是不是 `Failed`。
///
/// ## 「失败之后补一帧」为什么必须在这儿
///
/// 真机（2026-10-20）：微信窗口被最小化 ⇒ 采集失败 ⇒ 界面挂上
/// 「没写成：抓不到：窗口已最小化——还原它才能抓到」。
/// 用户还原窗口 ⇒ 采集恢复 ⇒ **恢复后的第一轮「什么都没变」**，
/// 于是那一支**不发** ⇒ **界面永远挂着那句话**——
/// 用户明明已经还原了，界面还在催他还原。
///
/// ⇒ 失败之后的**第一轮成功**必须补一帧正常态。
///
/// ⚠ 另一半也必须成立：`last_failed == false` 时**不许推**
/// ——那是「静默期零调用」（硬约束第 6 条）：每一圈都推一帧，
/// 前端就被白刷一遍，界面上的「安静」也没了。
fn phase_to_push(s: &Scratch, changed: bool, last_failed: bool) -> Option<Phase> {
    if changed || last_failed {
        Some(current_phase(s))
    } else {
        None
    }
}

/// **闸刚刚打开了吗？** 顺便把闸的状态记成「开着」，把停泊标记清掉。
///
/// 返回 `true` = 刚从关变成开 ⇒ **必须推一帧**。
///
/// ## 为什么需要它
///
/// 停泊期间面板显示的是「锁着别的对话，切回来才继续」，
/// 而切回来那一轮**很可能什么都没变**（没有新消息）⇒
/// `observe` 返回 `Quiet` ⇒ 本来不会推帧 ⇒ 面板就一直停在停泊态，
/// **明明已经切回来了**。
///
/// ⇒ 闸从关变成开，要推一帧把锁定会话的历史恢复出来。
fn gate_opened(s: &mut Scratch) -> bool {
    let just_opened = s.prev_gate_open == Some(false);
    s.prev_gate_open = Some(true);
    // 面板不再是停泊态：锁定会话的消息与候选可以显示了。
    s.parked_on_other = false;
    if just_opened {
        crate::commands::sync_candidates(&s.candidates);
    }
    just_opened
}

/// 跑一轮。`Ok(Some(anchor))` = 有新内容；`Ok(None)` = 静默期。
fn try_auto_send_pending(s: &mut Scratch) -> bool {
    if s.candidates.is_empty() {
        return false;
    }
    let now = std::time::Instant::now();
    let tier_is_auto =
        crate::commands::current_tier() == chatservant_core::dispatch::Dispatch::Auto;
    if let Err(why) = s.auto_gate.may_auto_send(tier_is_auto, now) {
        if tier_is_auto && !s.auto_gate.spent_this_round {
            tracing::info!(target: "chat_servant::watch", "自动发送等待冷却：{why}");
        }
        return false;
    }
    if matches!(s.source, FrameSource::Live)
        && let Some(idle_ms) = chatservant_capture::inject::user_idle_ms()
        && idle_ms < 1200
    {
        tracing::debug!(
            target: "chat_servant::watch",
            idle_ms,
            "用户正在操作键鼠（idle < 1200ms），暂缓本圈自动发送"
        );
        return false;
    }
    let Some(top) = s.candidates.first().cloned() else {
        return false;
    };
    crate::commands::sync_candidates(&s.candidates);
    match crate::commands::auto_send(&top.id) {
        Ok(receipt) => {
            tracing::info!(
                target: "chat_servant::watch",
                "自动发送回执：{}",
                receipt.explain()
            );
            if !matches!(
                receipt,
                chatservant_core::dispatch::DeliveryReceipt::NotSent
                    | chatservant_core::dispatch::DeliveryReceipt::Failed(_)
            ) {
                s.auto_gate.record_sent(now);
                s.candidates.clear();
                crate::commands::sync_candidates(&s.candidates);
                s.reason = format!(
                    "{}。已按最高置信度自动发送：{}",
                    s.reason,
                    receipt.explain()
                );
                true
            } else {
                s.reason = format!("{}。自动发送未成：{}", s.reason, receipt.explain());
                false
            }
        }
        Err(e) => {
            s.reason = format!("{}。自动发送失败：{e}", s.reason);
            false
        }
    }
}

fn take_pending_relock(watcher: &mut PaneWatcher, s: &mut Scratch) {
    // 重锁请求必须在建锁判断之前取走，并连同引擎水位线一起退回。
    if s.relock.take() {
        tracing::info!(
            target: "chat_servant::watch",
            "重锁：清空账本并退水位线（清之前 seen={} messages={}）",
            s.seen.len(),
            s.messages.len()
        );
        stash_current_bucket(s);
        apply_relock(s);
        watcher.rewind();
    }
}

fn evaluate_session_gate(
    watcher: &mut PaneWatcher,
    s: &mut Scratch,
    discovered: &Option<(
        chatservant_perceive::identity::HeaderPrint,
        chatservant_core::frame::Rect,
    )>,
) -> (bool, bool) {
    let mut just_switched = false;
    let peer_ok = match (&s.locked_print.clone(), discovered) {
        (None, Some((p, _))) => {
            just_switched = restore_or_bind_bucket(s, p);
            true
        }
        (_, None) => false,
        (Some(a), Some((b, _))) => {
            let same_name =
                s.locked_name.is_empty() || s.peer.is_empty() || s.locked_name == s.peer;
            if a.is_same(b) && same_name {
                if !s.locked_name.is_empty() && s.peer != s.locked_name {
                    s.peer = s.locked_name.clone();
                } else if s.locked_name.is_empty() && !s.peer.is_empty() {
                    s.locked_name = s.peer.clone();
                }
                true
            } else if s.session_mode.get() == SessionMode::Follow {
                stash_current_bucket(s);
                apply_relock(s);
                let _ = restore_or_bind_bucket(s, b);
                watcher.rewind();
                just_switched = true;
                true
            } else {
                false
            }
        }
    };
    (peer_ok, just_switched)
}

struct AbsorbedRound {
    messages: Vec<Message>,
    novel: Vec<Message>,
    seen_snap: Vec<Message>,
    msgs_snap: Vec<Message>,
    newest: Option<i32>,
}

fn read_and_absorb_round(
    frame: &chatservant_capture::Frame,
    full: &chatservant_capture::Frame,
    anchor: &PaneAnchor,
    roi: chatservant_core::frame::Rect,
    s: &mut Scratch,
    last_boxes: &std::sync::Mutex<Option<crate::debug::DebugPane>>,
) -> Result<AbsorbedRound, String> {
    let Round {
        total_lines,
        lines,
        attrs,
        messages,
        ocr_pixels,
    } = read_round(frame, full, anchor, roi, &chatservant_perceive::SystemOcr)?;
    s.ocr_calls += 1;

    *last_boxes.lock().unwrap_or_else(|e| e.into_inner()) = Some(crate::debug::build(
        anchor.chat,
        frame.width(),
        frame.height(),
        attrs.as_slice(),
    ));
    record_cost(
        last_boxes,
        ocr_pixels,
        Some(pane_area(roi)),
        pane_area(anchor.chat),
        1,
    );

    // 先把我们自己发出的消息认回来，再按内容身份计算新颖度并合流。
    let mut messages = messages;
    relabel_our_own(&mut messages);
    let seen_snap = s.seen.clone();
    let msgs_snap = s.messages.clone();
    let novel = absorb_round(&mut s.seen, &mut s.messages, &messages);
    let newest = novel
        .iter()
        .filter(|m| m.speaker == Speaker::Them)
        .map(|m| m.rect.y1)
        .max();

    tracing::debug!(
        target: "chat_servant::watch",
        "roi={roi:?} ocr总行={} 本轮行={} 归类={} 消息={} 本轮新到={}（对方={} 我方={}） 本轮记到y={:?}",
        total_lines,
        lines.len(),
        attrs.len(),
        messages.len(),
        novel.len(),
        novel.iter().filter(|m| m.speaker == Speaker::Them).count(),
        novel.iter().filter(|m| m.speaker == Speaker::Me).count(),
        newest,
    );
    for (kind, w, h) in novel.iter().filter_map(merged_pair_suspect) {
        tracing::debug!(
            target: "chat_servant::watch",
            "窄框诊断：{kind} {w}x{h}",
        );
    }

    Ok(AbsorbedRound {
        messages,
        novel,
        seen_snap,
        msgs_snap,
        newest,
    })
}

fn render_trimmed_history(s: &mut Scratch, recent_n: usize) -> String {
    match s
        .context
        .trim(SESSION, &s.history, recent_n, &mut NoSummary)
    {
        Ok(t) => t.render(),
        Err(e) => {
            s.reason = format!("摘要失败（{e}）；本轮只用最近消息");
            let tail: Vec<&String> = s.history.iter().rev().take(recent_n).collect();
            let mut lines: Vec<String> = vec!["[最近的对话]".into()];
            lines.extend(tail.into_iter().rev().cloned());
            lines.join("\n")
        }
    }
}

fn run_draft_turn(
    reasoner: &mut dyn Reasoner,
    frame: &chatservant_capture::Frame,
    anchor: &PaneAnchor,
    s: &mut Scratch,
    absorbed: &AbsorbedRound,
    last_boxes: &std::sync::Mutex<Option<crate::debug::DebugPane>>,
) -> Result<(), String> {
    let recent_n = s.recent.get();
    let hist_len_before = s.history.len();
    let mine_len_before = s.mine.len();
    s.history.extend(absorbed.novel.iter().map(history_line));
    s.mine.extend(
        chatservant_reason::voice::my_side(&absorbed.novel)
            .into_iter()
            .filter(|t| !crate::commands::sent_by_us(t)),
    );

    let rendered = render_trimmed_history(s, recent_n);
    let images = collect_images(frame, anchor, s);
    record_vision_tokens(last_boxes, images.iter().map(|i| u64::from(i.tokens)).sum());

    let ctx = TurnContext {
        messages: rendered.lines().map(str::to_string).collect(),
        peer: s.peer.clone(),
        images,
        voice: chatservant_reason::voice::voice_samples(&s.mine),
    };
    match chatservant_reason::turn(reasoner, &ctx) {
        Ok(t) => {
            s.candidates = t
                .draft
                .display_order()
                .into_iter()
                .enumerate()
                .map(|(i, c)| DraftCandidate {
                    id: format!("c{i}"),
                    text: c.text.clone(),
                    probability: c.probability,
                    confidence: ConfidenceTier::from_probability(c.probability),
                })
                .collect();
            s.reason = t.explain();
            s.auto_gate.begin_new_round();
            try_auto_send_pending(s);
            Ok(())
        }
        Err(chatservant_reason::ReasonError::NothingToDo) => {
            s.reason = "没有新消息".into();
            Ok(())
        }
        Err(e) => {
            // 推理失败必须回滚本轮入账，且不调 watcher.commit，让下一圈重试。
            s.seen.clone_from(&absorbed.seen_snap);
            s.messages.clone_from(&absorbed.msgs_snap);
            s.history.truncate(hist_len_before);
            s.mine.truncate(mine_len_before);
            s.reason = format!("推理失败：{e}");
            tracing::warn!(target: "chat_servant::watch", "推理失败（下一圈自动重试）：{e}");
            Err(format!("推理失败，正在重试：{e}"))
        }
    }
}

fn step(
    watcher: &mut PaneWatcher,
    reasoner: &mut Box<dyn Reasoner + Send>,
    s: &mut Scratch,
    last_boxes: &std::sync::Mutex<Option<crate::debug::DebugPane>>,
    ticks: u64,
    model_calls: u32,
    interim: &mut dyn FnMut(Readout),
) -> Result<Option<PaneAnchor>, String> {
    let frame = load_frame(&s.source, &mut s.last_hwnd)?;
    let anchor = chatservant_capture::anchor::derive_or_reuse(&frame, s.last_anchor)
        .map_err(|e| format!("锚点推导失败：{e}"))?;
    s.last_anchor = Some(anchor);

    // 会话身份闸必须排在任何消息区裁剪与 OCR 之前（认头部像素指纹，停泊态零裁剪零分配）。
    let discovered = header_print(&frame, &anchor);
    refresh_peer(
        s,
        &frame,
        &anchor,
        discovered.as_ref().map(|(p, _)| p),
        discovered.as_ref().map(|(_, r)| *r),
    );
    take_pending_relock(watcher, s);
    let (peer_ok, just_switched) = evaluate_session_gate(watcher, s, &discovered);
    if !peer_ok {
        let just_closed = park_on_other_conversation(s, discovered.as_ref().map(|(p, _)| p));
        record_cost(last_boxes, 0, None, pane_area(anchor.chat), 0);
        return Ok(if just_closed { Some(anchor) } else { None });
    }
    let just_opened = gate_opened(s);
    s.gate_open = true;

    let full = frame
        .crop(anchor.chat)
        .ok_or_else(|| "裁不出消息区".to_string())?;

    // 变化门控 + 增量 ROI（静默期直接返回，硬约束第 6 条）。
    let Decision::Read { roi } = watcher.observe(&full) else {
        record_cost(last_boxes, 0, None, pane_area(anchor.chat), 0);
        let auto_fired = try_auto_send_pending(s);
        return Ok(if just_opened || just_switched || auto_fired {
            Some(anchor)
        } else {
            None
        });
    };

    let absorbed = read_and_absorb_round(&frame, &full, &anchor, roi, s, last_boxes)?;
    if should_draft_now(absorbed.newest.is_some(), &absorbed.messages) {
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
        run_draft_turn(reasoner.as_mut(), &frame, &anchor, s, &absorbed, last_boxes)?;
    } else {
        // 候选对应整段对话状态，本轮无对方新话时不清空已有候选。
        s.reason = "这一段里没有对方说的话".into();
    }

    match absorbed.newest {
        Some(y) => watcher.commit(y),
        None => watcher.complete(),
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
    // 名字的缓存键也要一起丢。重锁就是一次会话切换，新会话的会话名
    //   必须重新读一次——留着旧键会让 `refresh_peer` 以为「没变」而跳过。
    s.last_print = None;
    s.history.clear();
    // 口吻样本同样要清。它是**从对方私聊里学来的东西**——带着上一个会话的
    // 口吻去给新会话起草，等于把 A 的说话风格套在 B 身上，而且这份「风格」
    // 本身是从 A 的私聊内容里长出来的。跟 `history` 是同一类事故。
    s.mine.clear();
    s.context.forget(SESSION);
    s.candidates.clear();
    // 面板上累积的消息**也要清**。不清的话重锁之后，标题写着新会话名，
    // 下面却还挂着上一个会话的对话——那正是「界面上出现不是这个会话的内容」，
    // 和上面那条历史串味是同一类事故，只是它出在给人看的那一层。
    s.messages.clear();
    // 账本同样要清：那是**上一个会话**处理过的消息。
    // 留着它 ⇒ 新会话里说的同一句话会被当成「已经见过」⇒ 不回复。
    s.seen.clear();
}

/// 将当前已锁定会话的上下文、已见账本与候选存入 `s.buckets`（按像素指纹去重更新，上限 16 个桶）。
fn stash_current_bucket(s: &mut Scratch) {
    let Some(ref print) = s.locked_print else {
        return;
    };
    if s.history.is_empty() && s.messages.is_empty() && s.seen.is_empty() {
        return;
    }
    if let Some(idx) = s.buckets.iter().position(|b| {
        b.print.is_same(print)
            && (b.locked_name.is_empty()
                || s.locked_name.is_empty()
                || b.locked_name == s.locked_name)
    }) {
        s.buckets.remove(idx);
    }
    if s.buckets.len() >= MAX_SESSION_BUCKETS {
        s.buckets.remove(0);
    }
    s.buckets.push(SessionBucket {
        print: print.clone(),
        locked_name: s.locked_name.clone(),
        history: s.history.clone(),
        mine: s.mine.clone(),
        messages: s.messages.clone(),
        seen: s.seen.clone(),
        candidates: s.candidates.clone(),
        reason: s.reason.clone(),
    });
}

/// 绑定到新会话头部指纹 `p`：若 `s.buckets` 中已有该会话的分桶记忆则原样恢复并返回 `true`。
fn restore_or_bind_bucket(
    s: &mut Scratch,
    p: &chatservant_perceive::identity::HeaderPrint,
) -> bool {
    s.locked_print = Some(p.clone());
    s.parked_on_other = false;
    s.gate_open = true;
    if let Some(idx) = s.buckets.iter().position(|b| {
        b.print.is_same(p)
            && (b.locked_name.is_empty() || s.peer.is_empty() || b.locked_name == s.peer)
    }) {
        let bucket = s.buckets.remove(idx);
        s.locked_name = if s.peer.is_empty() {
            bucket.locked_name
        } else {
            s.peer.clone()
        };
        s.history = bucket.history;
        s.mine = bucket.mine;
        s.messages = bucket.messages;
        s.seen = bucket.seen;
        s.candidates = bucket.candidates;
        s.reason = bucket.reason;
        crate::commands::sync_candidates(&s.candidates);
        true
    } else {
        s.locked_name = s.peer.clone();
        false
    }
}
/// **闸关着的时候，把面板收拾干净。** 返回「是不是刚刚才关的」。
///
/// 抽成独立函数是为了能直接测它——它原来内联在 `step` 里，而 `step` 要
/// 真帧、真 OCR、真 Reasoner 才跑得起来，于是这段「关闸时到底清了什么」
/// 一次都没被断言过。
///
/// ## 「确认换了会话」时清的是**显示**，不是**状态**
///
/// `s.peer` 每轮都在跟着真实窗口更新，所以闸一关，标题已经是新会话的名字了。
/// 这时候下面还挂着**锁定会话**的历史，读起来就是「这是新会话的对话」——
/// 而它不是。硬约束第 1 条要求这道闸是**看得见的**，一个会说谎的面板比
/// 一条没反应的按钮更糟：用户无从判断哪段内容是自己正在看的那个会话的。
///
/// ⇒ 显示清空（[`Scratch::parked_on_other`]），**状态留着**
/// （`s.messages` 不动）。2026-10-23 改：原来两件事一起做（清消息 +
/// 退水位线），代价是切回来必须**整段重读**，于是旧消息被当成新消息
/// 再推理一次——那正是「切回会话慢 7 秒」的主因。
///
/// ## 为什么「认不出头部」不清
///
/// 认不出不等于换了会话（见 [`chatservant_perceive::identity::HeaderPrint::of`]）。
/// 标题栏被裁掉、动画没画完都会短暂认不到，此时清空面板会让消息列表**闪一下**。
/// 所以只有**指纹确实不同**这一支才清。
///
/// ## 显示清空由谁做
///
/// 见 [`blank_messages`]：停泊期间推给界面的消息列表恒为空。
///
/// ## 「确认换了会话」这件事记在哪
///
/// 记在 [`Scratch::parked_on_other`]（**状态**），不另设一个
/// 「本轮刚换了会话」的标志——两处真相迟早分叉：
/// 一个是「现在是停泊态」，另一个是「这一轮刚好检测到切换」。
/// 真正需要的「刚关闸」信号由 `just_closed` 表达。
///
/// 漏掉的后果不是「多读一次」，是**重复推理**：面板被清空了，
/// 引擎也只能整段重读才能把历史找回来，于是**早已看过的旧消息**
/// 被当成新消息再推一次推理（2026-10-23 实测：切回会话要 7 秒，
/// 其中 6~9 秒是那次重复推理）。
///
/// 现在历史留在状态里，切回来不用重读 ⇒ 零 OCR、零推理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GateOutcome {
    /// 闸的状态刚变成「关」——要推一次 readout，否则界面停在旧状态。
    just_closed: bool,
}

/// 闸关着时，**面板不许显示锁定会话的消息**。
///
/// 屏幕上那块是别人的对话，而 `s.messages` 记的是**锁定会话**的历史
/// （累积只发生在闸开着的时候）。原样推出去 ⇒ 面板顶着别人的会话名
/// 显示锁定会话的对话 —— 票 32 说的「停泊面板说假话」。
///
/// ⇒ 停泊期间推的消息列表恒为空。**历史留在状态里**，
/// 切回来时原样恢复（`s.messages` 没被清过，见 [`apply_gate_closed`]）。
fn blank_messages(phase: Phase) -> Phase {
    match phase {
        Phase::Read { .. } | Phase::Drafted { .. } => Phase::Read {
            messages: Vec::new(),
        },
        Phase::Drafting { .. } => Phase::Drafting {
            messages: Vec::new(),
        },
        Phase::Failed { error, .. } => Phase::Failed {
            messages: Vec::new(),
            error,
        },
    }
}

fn apply_gate_closed(
    s: &mut Scratch,
    print: Option<&chatservant_perceive::identity::HeaderPrint>,
) -> GateOutcome {
    crate::commands::sync_candidates(&[]);
    s.reason = match (&s.locked_print, print) {
        (Some(_), None) => format!(
            "没读出当前会话的头部，无法确认还是不是「{}」——本轮不动手",
            s.locked_peer
        ),
        // `(None, Some(_))` 到不了这里：那一支已经在建锁时放行了。
        (None, None) => "还没读到任何会话头部，先不动手".into(),
        _ => {
            // 确认换人了：上一个会话的消息**不该继续挂在面板上**。
            //
            // ⚠⚠ **但状态里要留着**（2026-10-23 改）。
            //
            // `s.messages` 记的是**锁定会话**的历史——累积只发生在闸开着
            // 的时候（`step` 里那句 `merge_into_stream` 在身份闸之后），
            // 所以它**永远**是锁定会话的，不会混进别人的话。
            //
            // 原来这里把它清掉了，于是切回来时面板是空的，得靠**整段重读**
            // 才能重新长出来——而整段重读会把**早已看过的旧消息**当成新消息
            // 再推一次推理（绕过硬约束第 6 条，还多一次自动发送的风险）。
            //
            // ⇒ 现在：**状态留着，显示清空**。
            // 显示清空由 [`Scratch::parked_on_other`] 负责（见 [`snapshot`]），
            // 切回来时面板原样恢复，不用重读。
            s.parked_on_other = true;
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
    // 但**闸的状态一变就要推一次 readout**。否则用户在微信里切了会话，
    // 面板上一个字都不更新，继续挂着上一个会话的候选——
    // 静默是对的，但静默到界面开始说谎就不对了。
    let just_closed = s.prev_gate_open != Some(false);
    s.prev_gate_open = Some(false);
    s.gate_open = false;
    GateOutcome { just_closed }
}

/// 闸关着 = 屏幕上那块**不是我们要读的对话**。面板和引擎必须一起收拾。
///
/// 返回「是不是刚刚才关的」——只有刚关的那一刻才推 readout。
///
/// ## ⚠⚠ 停泊**不许退参照系**（2026-10-23 改，这是「切回会话慢 7 秒」的主因）
///
/// ### 参照系属于**锁定会话**，不属于「屏幕上那块」
///
/// [`PaneWatcher`] 这个实例**整个生命周期只服务锁定会话**：
/// `observe` 只在闸开着的时候被调用（见 [`step`]），也就是只在锁定会话上。
/// ⇒ 屏幕上看的是谁，跟引擎的参照系**毫无关系**。
///
/// ### 原来退参照系，代价是什么
///
/// 2026-10-11 修的那个缺陷（「切走再切回，面板 8 条消息只剩 6 条」）的
/// 根因是**两处状态不同步**：面板上的累积消息被清掉了，可引擎还停在
/// 旧会话的底部。于是切回来时增量范围只剩贴着底部的那条缝。
///
/// 当时的修法是「一起退」⇒ 切回来时**整段重读**。
/// 而整段重读会把**早已看过的旧消息**当成新消息：
/// `newest` 取的是**本轮 ROI 读到的**对方消息，完全不去重
/// ⇒ 触发一次**完整模型推理**（6~9 秒）。
///
/// 实测（2026-10-23 真机）：切回会话一轮 3.28 秒 + 一次完整推理 6~9 秒
/// ⇒ 「切换会话 7s+」。而且这一下还**绕过了硬约束第 6 条**
/// （静默期零调用），并让 `auto_gate.begin_new_round()` 被调用
/// ⇒ **有为旧消息自动发送的风险**。
///
/// ### 现在怎么办
///
/// **状态留着，显示清空**：
/// · 锁定会话的消息流不清（[`Scratch::messages`]）⇒ 切回来面板原样恢复；
/// · 停泊期间面板显示的消息列表恒为空（[`blank_messages`]）⇒ 不骗人；
/// · **引擎的参照系不动** ⇒ 切回来若画面逐字节相同，`observe` 判 `Quiet`
///   ⇒ 消息区零 OCR、零推理，只推一帧把历史恢复出来。
///
/// ⚠ 2026-10-23 补：**「参照系」这个说法本身也过期了**——
/// `9003291` 把「增量读哪几行」的判据从**像素水位线**换成了**内容身份**
/// （见 [`chatservant_perceive::novel_messages`]），引擎里已经**没有水位线**。
/// ⇒ 现在挡住「重复推理」的是**两道独立的闸**，不是一道：
/// ① 画面没变 ⇒ `Quiet` ⇒ 根本不读消息区；
/// ② 画面变了（真机上光标闪烁、滚动条抖动都会让它变）⇒ 读出来的消息
///    与**已处理账本** [`Scratch::seen`] 里的身份**逐条计数比对**
///    ⇒ 旧消息**不算新** ⇒ `newest` 为空 ⇒ 依然零推理。
/// 第 ② 条是 `a1f5a1a` 之后才有的**承重**部分：
/// 夹具实验里画面逐字节相同，走的是第 ① 条；
/// 真机上画面常常有细微变化，走的才是第 ② 条。
///
/// ⇒ 2026-10-11 那个缺陷也不会回来：它要的就是「历史能恢复」，
/// 而现在历史本来就在状态里，不需要靠重读长出来。
///
/// ### 实测（2026-10-23 08:51，当前代码，夹具帧源，真 OCR + 生产接缝）
///
/// | 动作 | 端到端延迟 | 那一轮做了什么 | 模型调用 |
/// |---|---|---|---|
/// | 切走（→ 文件传输助手） | **1389 ms** | 窄带 OCR 认名字（~0.75 s）+ 闸关 | **0** |
/// | 切回（→ 小号） | **890 ms** | 窄带 OCR 认名字（~0.74 s）+ 逐字节比对 ⇒ `Quiet` + 推一帧 | **0** |
/// | 静默期一轮 | ~1.17 s（1 s tick + 抓帧） | 只有抓帧与像素比对 | **0** |
///
/// ⇒ 「切换会话 7s+」现在是 **1.4 s / 0.9 s**。
/// 剩下的大头是 **1 秒的 tick**（[`TICK`]）与**一次窄带 OCR**（~0.74 s）。
///
/// ⚠ 那一整场里 `roi=` 行**只有启动那一次** ⇒ 切走切回全程
/// **一条消息都没读**、**一次模型都没调**（`phase=drafting` 只出现 1 次，
/// 是启动首读那一次）。
fn park_on_other_conversation(
    s: &mut Scratch,
    print: Option<&chatservant_perceive::identity::HeaderPrint>,
) -> bool {
    let g = apply_gate_closed(s, print);
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
/// `frame` 必须是**整帧**（`frame.crop(anchor.chat)` **之前**那块）。
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
    //2026-10-14：**不再自己拼矩形**。
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
    // 返回 `None` = **认不出**，调用方必须按「认不出」处理（关闸不起稿）。
    chatservant_perceive::identity::HeaderPrint::discover(frame, anchor.chat, HEAD_BAND_W)
}

/// **头部像素没变就不重读会话名。**
///
/// ## 为什么能拿指纹当缓存键
///
/// 会话名是**头部那块像素的函数**——像素一模一样，OCR 不可能读出不同的字。
/// 而 [`HeaderPrint::is_same`] 判定的正是「这块还是不是那块」，
/// 阈值吸收的是角标、动画这类抖动。所以指纹没变就不必再花一次 OCR。
///
/// 这条缓存让「闸关着时别干活」成为可能：换会话时头部像素确实变了，
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
    // 指纹变了但**没推出头部矩形**（`discover` 认不出）⇒ 不动已认出的名字。
    //   「认不出」不是「名字变了」，也不是「没有名字」。
    if let (Some(rect), Some(print)) = (head, print) {
        if let Some(name) = chatservant_perceive::read_peer_name(frame, print, rect, anchor.chat) {
            s.peer = name;
        } else if let Some(ref locked) = s.locked_print
            && locked.is_same(print)
            && !s.locked_name.is_empty()
        {
            s.peer = s.locked_name.clone();
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
            // **其余各家的视觉计费没有核实过**，而 `Billing` 只有两种
            // 已验证形状（Anthropic 28px patch、OpenAI 512px tile）。
            // 套用其中之一 = 拿一个没验证的数去算用户的账单。
            //
            // 2026-10-15（票 43）这条 arm 从「只有 MiniMax」扩成了「除
            // 上面那两家之外的一切」：端点可换之后，这一格可能是任何人的服务。
            //
            //**下面这个 `_` 不是「不产出预算」，它退回 Anthropic 的
            // 28px patch。** `Billing` 只有 `Patch` 与 `Tile` 两个变体，
            // **没有「未知」这一档**，所以这里无法表达「这家没核实过」——
            // 函数签名强迫它给一个数，而任何数对没核实过的家都是编的。
            // 这不是本轮引入的：MiniMax 早就在这条 arm 上拿同一个数。
            // 本轮只是把命中面从「一家」扩大到「除两家之外的一切」。
            //
            // ⇒ 真正的修法是给 `Billing` 加一档 `Unknown`，让调用方
            // **拒绝给图片定尺寸**（宁可不出图，也不出一个错尺寸）。
            // 那是改 `core::scale` 公开枚举的事，会牵动
            // `encode_region` 与预算滑块，**不在票 43 范围内**。
            // 记在这里，别让下一个人以为这一档是「如实不产出」。
            _ => Billing::ANTHROPIC_PATCH,
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
        AUTO_COOLDOWN, AutoGate, BUDGET_MAX, BUDGET_MIN, CallCounts, DraftCandidate, ImagesOut,
        Message, NoSummary, PaneAnchor, Phase, Readout, RecentN, Relock, SEEN_WINDOW, SESSION,
        Scratch, SessionMode, SessionModeCell, Speaker, TierCell, TokenBudget, absorb_round,
        apply_gate_closed, apply_relock, collect_images, current_phase, gate_opened, header_print,
        history_line, merged_pair_suspect, park_on_other_conversation, phase_to_push, read_round,
        relabel_our_own, restore_or_bind_bucket, should_draft_now, snapshot, stash_current_bucket,
    };
    use chatservant_capture::Frame;
    use chatservant_capture::ocr::{OcrError, OcrLine, OcrText};
    use chatservant_core::confidence::ConfidenceTier;
    use chatservant_core::frame::Rect;
    use chatservant_perceive::{Decision, LineSource, PaneWatcher, ViewPlan, ViewPolicy};
    use chatservant_reason::context;

    // ── `Auto` 档自动发送的护栏 ──────────────────────────────────────
    //
    // 这些是**决定消息会不会发进真人聊天窗口**的判据，
    //   所以逐条都要能自证会响。删测试比没有测试更糟——
    //   它会让下一个人以为这里没有判断。

    /// 只填入档**永远不许自动发**。哪怕所有条件都满足。
    ///
    /// 这是整个护栏的地基：`Manual` 是默认档，程序在默认状态下
    /// 绝不能替人说话。
    #[test]
    fn the_fill_only_tier_never_auto_sends() {
        let g = AutoGate::default();
        for waited in [0, 1, 10, 1_000, 100_000] {
            let now = std::time::Instant::now() + std::time::Duration::from_secs(waited);
            assert!(
                g.may_auto_send(false, now).is_err(),
                "只填入档在等了 {waited} 秒之后仍然自动发了"
            );
        }
    }

    /// **对方连发 N 条 ⇒ 只回一条。**
    ///
    /// 这是本护栏存在的首要理由：不挡这一条，`Auto` 档就是刷屏，
    /// 而发出去的每一条都撤不回来。
    #[test]
    fn several_messages_in_one_round_produce_one_auto_send() {
        let now = std::time::Instant::now();
        let mut g = AutoGate::default();

        assert!(g.may_auto_send(true, now).is_ok(), "第一轮该能发");
        g.record_sent(now);

        // 对方接着又说了 5 句，**同一轮**（水位线没推进成新一轮）
        for i in 0..5 {
            assert!(
                g.may_auto_send(true, now).is_err(),
                "第 {i} 句之后又自动发了一次 ⇒ 对方说 N 句我回 N 句"
            );
        }
    }

    /// **冷却期内一条都不许发**，即使已经进入了新一轮。
    ///
    /// 这一条最容易写错：`begin_new_round` 只清 `spent_this_round`，
    ///   **不许碰 `last_sent`**。若有人图省事在 `begin_new_round` 里
    ///   一并把冷却清了，这条会红——那正是它该做的事。
    #[test]
    fn the_cooldown_survives_a_new_round() {
        let t0 = std::time::Instant::now();
        let mut g = AutoGate::default();
        g.record_sent(t0);

        // 对方很快又开口 ⇒ 新一轮（spent 清了），但冷却还在
        g.begin_new_round();
        let shortly_after = t0 + AUTO_COOLDOWN / 2;
        assert!(
            g.may_auto_send(true, shortly_after).is_err(),
            "冷却期内就发了 ⇒ 「一来消息就清冷却」那条错误写法溜过去了"
        );

        // 冷却走完 ⇒ 允许
        let after = t0 + AUTO_COOLDOWN + std::time::Duration::from_secs(1);
        assert!(
            g.may_auto_send(true, after).is_ok(),
            "冷却都过了还不许发，那这个档就没有自动的意义了"
        );
    }

    /// 防重自证：`record_sent` 之后立刻重复调用**不会**放行两次。
    ///
    /// 把 `AUTO_COOLDOWN` 调成 0 会让上面几条失去意义——
    ///   这条测试就是挡住那种改动的。
    #[test]
    fn recording_a_send_immediately_blocks_a_second_one() {
        let now = std::time::Instant::now();
        let mut g = AutoGate::default();
        assert!(g.may_auto_send(true, now).is_ok());
        g.record_sent(now);
        assert!(
            g.may_auto_send(true, now).is_err(),
            "同一条立刻重发 ⇒ 防重失效"
        );
        // 而冷却本身也不该是 0，否则上面几条护栏全被架空
        assert!(
            AUTO_COOLDOWN.as_secs() >= 10,
            "冷却被调到 {} 秒 —— 连发就会变成 N 句回 N 句",
            AUTO_COOLDOWN.as_secs()
        );
    }

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
                confidence: ConfidenceTier::from_probability(0.5),
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
            // **出错时消息不能丢**：一个 OCR 抖动不该把整段对话清空。
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
    /// 2026-10-14 真机实测出来的说假话路径：用户在微信里切到另一个会话，
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

    /// **我方把着话头时，面板不许说「对方说完了」。**
    ///
    /// ## 2026-10-25 真机（缺陷帧 `frame-1793475836.png`）
    ///
    /// 对方 1 条在最上面、我方 8 条在后面。闸开、这轮没起草、阶段是 `Read`
    /// ⇒ `Phase::status_hint` 返回 `None` ⇒ 前端回落到它自己写死的那句
    /// 「对方说完了，这轮没起草」——而那一帧里**最后一句是我方说的**。
    ///
    /// 前端判据（`ui/src/lib/PaperPanel.svelte:249`）是「最后一条**对方**消息」，
    /// 不是「最后一条消息是不是对方」⇒ 我方把着话头时它照样非空 ⇒ 前端堵不住。
    fn assert_non_read_phases_keep_status_hints(s: &Scratch, st: chatservant_reason::StageCalls) {
        let drafting = snapshot(
            s,
            None,
            Phase::Drafting {
                messages: vec![mine_msg("x")],
            },
            1,
            1,
            st,
            "起草中",
        );
        assert_eq!(
            drafting.status_hint.as_deref(),
            Some("正在起草，通常几秒"),
            "起草中说成了「等待回复」"
        );
        let drafted = snapshot(
            s,
            None,
            Phase::Drafted {
                messages: vec![mine_msg("x")],
                candidates: Vec::new(),
            },
            1,
            1,
            st,
            "有候选",
        );
        assert!(
            drafted.status_hint.is_none(),
            "drafted 被塞了这句：{:?}",
            drafted.status_hint
        );
        let failed = snapshot(
            s,
            None,
            Phase::Failed {
                messages: vec![mine_msg("x")],
                error: "OCR 失败".into(),
            },
            1,
            0,
            st,
            "这一轮没跑通",
        );
        assert!(
            failed.status_hint.is_none(),
            "failed 被塞了这句：{:?}",
            failed.status_hint
        );
    }

    #[test]
    fn when_i_have_the_floor_the_panel_says_so() {
        let st = chatservant_reason::StageCalls::default();
        let mut s = scratch_with(0, 0);
        s.gate_open = true;

        let mut messages = vec![a_msg("对方最上面那一句")];
        messages.extend((0..8).map(|i| mine_msg(&format!("我自己的第 {i} 句"))));
        let out = snapshot(
            &s,
            None,
            Phase::Read { messages },
            1,
            0,
            st,
            "这一轮对方没有新消息",
        );
        assert_eq!(
            out.status_hint.as_deref(),
            Some(super::I_HAVE_THE_FLOOR_HINT),
            "最后一句是我方却不说这句 ⇒ 前端回落到「对方说完了」那句假话"
        );
        assert_ne!(
            out.status_hint.as_deref(),
            Some(super::GATE_CLOSED_HINT),
            "闸开着我方把着话头，不能说成「锁着别的对话」"
        );
        assert_ne!(
            super::I_HAVE_THE_FLOOR_HINT,
            super::GATE_CLOSED_HINT,
            "两句必须不同，否则上面那条断言测不出东西"
        );

        // 一条消息都没有 ⇒ 保持现状，不凭空说「你说了最后一句」。
        let empty = snapshot(
            &s,
            None,
            Phase::Read {
                messages: Vec::new(),
            },
            1,
            0,
            st,
            "还没读到",
        );
        assert!(
            empty.status_hint.is_none(),
            "没有消息却说「等待回复」：{:?}",
            empty.status_hint
        );

        // 最后一句是对方 ⇒ 这句也不许出现（那时该等的是起草，不是等对方）。
        let theirs = snapshot(
            &s,
            None,
            Phase::Read {
                messages: vec![mine_msg("我先说"), a_msg("对方回我")],
            },
            1,
            0,
            st,
            "对方刚说完",
        );
        assert!(
            theirs.status_hint.is_none(),
            "最后一句是对方却说「等待回复」：{:?}",
            theirs.status_hint
        );

        // 另外三档不受影响：补一句会盖掉它们各自的话（或故意不说话）。
        assert_non_read_phases_keep_status_hints(&s, st);
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

    /// 砍掉第一个 `#[cfg(test)]` 之后的内容。
    ///
    /// 「只在测试里被构造过」正是票 21 的病根，所以这一组数的是
    /// **运行时代码**里的构造点，测试里的不算。
    fn strip_test_modules(text: &str) -> &str {
        match text.find("#[cfg(test)]") {
            Some(i) => &text[..i],
            None => text,
        }
    }

    /// **面板能渲染的每一态，都必须有运行时代码在产出它。**
    ///
    /// 票 21 的病就是这个形状：`Phase` 在它自己的测试里被构造过，
    /// 而 `contract.rs` 在 2026-10-13 补进 `main.rs` 之前**一次都没被
    /// 编译过**——于是「面板画得出来的三态」在真机上压根没有生产者，
    /// 而 `?demo` 夹具手写了 `phase`，**两边看着一样**。
    ///
    /// 类型层已经保证「构造不出没有 `phase` 的 `Readout`」
    /// （`#[serde(flatten)]` + 单一构造点）。这一条补的是上一层：
    /// **枚举变体本身**必须有人在运行时真的造它。
    /// 只在测试里构造出来的变体 = 真机上不可达的态，而门禁全绿。
    #[test]
    fn every_phase_the_panel_can_render_is_produced_by_runtime_code() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        walk_rs(&src, &mut files);

        // 变体 → 判别键。键名与 `Phase::key()` 一一对应（下面先对齐再数）。
        let variants = [
            ("Phase::Read {", "read"),
            ("Phase::Drafting {", "drafting"),
            ("Phase::Drafted {", "drafted"),
            ("Phase::Failed {", "failed"),
        ];
        let real: Vec<&str> = vec![
            Phase::Read { messages: vec![] }.key(),
            Phase::Drafting { messages: vec![] }.key(),
            Phase::Drafted {
                messages: vec![],
                candidates: vec![],
            }
            .key(),
            Phase::Failed {
                messages: vec![],
                error: "x".into(),
            }
            .key(),
        ];
        let listed: Vec<&str> = variants.iter().map(|(_, k)| *k).collect();
        assert_eq!(
            listed, real,
            "这张表必须与 `Phase::key()` 一一对应：加了变体却忘了往里加，\
             就会有一个态谁都不检查"
        );

        for (needle, key) in variants {
            let mut hits = Vec::new();
            for p in &files {
                let Ok(text) = std::fs::read_to_string(p) else {
                    continue;
                };
                for (n, line) in strip_test_modules(&text).lines().enumerate() {
                    let t = line.trim();
                    if t.starts_with("//") {
                        continue;
                    }
                    if t.contains(needle) {
                        hits.push(format!("{}:{}", p.display(), n + 1));
                    }
                }
            }
            assert!(
                !hits.is_empty(),
                "面板能渲染 `{key}`，而 Rust 的**运行时代码**里没有任何地方产出它：\
                 这一态在真机上不可达，而这正是票 21 的原症状。\
                 确认它是不是只活在 `#[cfg(test)]` 里就没再搬出来。"
            );
        }
    }

    /// **demo 夹具能画的态，恰好是 Rust 会发的态。**
    ///
    /// 票 21 写下的病是「演示页是绿的、真机是断的，两边看着一样」。
    /// 这里**双向**锁：
    /// · 夹具里多一个 Rust 永远不会发的 `phase` ⇒ 演示页在骗人
    ///   （改界面时照着它调，真机上没有这一态）；
    /// · Rust 多发一个夹具里没有的 `phase` ⇒ 真机上有一态从来没被
    ///   人看过渲染。
    #[test]
    fn the_demo_fixtures_show_exactly_the_phases_the_machine_can_produce() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../static/demo");
        let rd = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("读不到夹具目录 {}：{e}", dir.display()));
        let mut seen = std::collections::BTreeSet::new();
        let mut count = 0usize;
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let text = std::fs::read_to_string(&p).expect("夹具读不出来");
            let v: serde_json::Value = serde_json::from_str(&text)
                .unwrap_or_else(|err| panic!("{} 不是合法 JSON：{err}", p.display()));
            let phase = v["phase"].as_str().unwrap_or_else(|| {
                panic!(
                    "{} 没有 `phase` 字段——面板会把它读成 undefined，\
                     而这正是票 21 的症状在夹具上的复现",
                    p.display()
                )
            });
            seen.insert(phase.to_string());
            count += 1;
        }
        assert!(count > 0, "夹具目录是空的，等于一个态都没验");
        let real: std::collections::BTreeSet<String> = [
            Phase::Read { messages: vec![] }.key(),
            Phase::Drafting { messages: vec![] }.key(),
            Phase::Drafted {
                messages: vec![],
                candidates: vec![],
            }
            .key(),
            Phase::Failed {
                messages: vec![],
                error: "x".into(),
            }
            .key(),
        ]
        .iter()
        .map(|k| (*k).to_string())
        .collect();
        assert_eq!(
            seen, real,
            "夹具的 `phase` 集合与 Rust 会发的对不上。\
             多出来的那个在真机上不存在（演示页在骗人），\
             少掉的那个没人验过渲染。"
        );
    }

    /// **demo 夹具的调用计数必须与后端契约逐字对得上。**
    ///
    /// ⚠ **2026-10-25 的病**：第三段（排序）取消之后，后端的 [`CallCounts`]
    /// 只剩 `judge / draft / vision` 三项，而 12 份 demo 夹具**还写着**
    /// `"rank": 1` 与 `"model_calls": 3`
    /// ⇒ **夹具比契约多一个键、总数也多一次**
    /// ⇒ 演示页展示的调用数与真机对不上（真机现在只有两次）。
    ///
    /// ⚠ 为什么之前没人发现：JSON 是**运行时**读的，TS 类型里没有 `rank`
    /// ⇒ 编译器不管；而那 12 个键**谁都没在看**。
    /// ⇒ 这条守卫把**键集合直接从 [`CallCounts`] 序列化出来取**（不写死字段名）
    /// ⇒ 后端加一项、删一项，这里自动跟着变。
    #[test]
    fn the_demo_fixtures_count_the_same_calls_the_backend_does() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../static/demo");
        let rd = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("读不到夹具目录 {}：{e}", dir.display()));
        // 契约的键集合：**从后端自己序列化出来**，不写死字段名。
        let contract: std::collections::BTreeSet<String> =
            serde_json::to_value(CallCounts::default())
                .expect("CallCounts 序列化不了")
                .as_object()
                .expect("CallCounts 不是对象")
                .keys()
                .cloned()
                .collect();
        let mut checked = 0usize;
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let text = std::fs::read_to_string(&p).expect("夹具读不出来");
            let v: serde_json::Value = serde_json::from_str(&text)
                .unwrap_or_else(|err| panic!("{} 不是合法 JSON：{err}", p.display()));
            let counts = v["counts"]
                .as_object()
                .unwrap_or_else(|| panic!("{} 没有 `counts` 对象", p.display()));
            let keys: std::collections::BTreeSet<String> = counts.keys().cloned().collect();
            assert_eq!(
                keys,
                contract,
                "{} 的 `counts` 键与后端契约对不上：\
                 多出来的那个是后端**已经不报**的那一段（演示页在骗人），\
                 少掉的那个是后端**正在报**的那一段（真机上没人看过它渲染）。",
                p.display()
            );
            let num = |k: &str| -> u64 {
                counts
                    .get(k)
                    .and_then(|x| x.as_u64())
                    .unwrap_or_else(|| panic!("{} 的 counts.{k} 不是整数", p.display()))
            };
            // ⚠ 视觉桥有**自己那份**计数（`s.vision.calls()`），默认关闭；
            //   `model_calls` 是 `reasoner.calls()` ⇒ 只含判断 + 起草。
            let total = v["model_calls"]
                .as_u64()
                .unwrap_or_else(|| panic!("{} 没有 `model_calls`", p.display()));
            assert_eq!(
                total,
                num("judge") + num("draft"),
                "{} 的 `model_calls` 与「判断 + 起草」对不上。",
                p.display()
            );
            checked += 1;
        }
        assert!(checked > 0, "夹具目录是空的，等于一份都没验");
    }

    /// **演示夹具里的候选，带的键与后端契约一模一样；置信度档位与概率对得上。**
    ///
    /// ## 为什么又要一条守卫（2026-10-25）
    ///
    /// 夹具是**运行时**读的 JSON ⇒ 编译器不管它；TS 类型里也没有 `confidence`
    /// ⇒ 类型系统也不管它。⇒ 后端加一个字段，夹具不会跟着加，
    /// **演示页就会拿旧结构去渲染**——而它照样画得出来，只是画的是错的。
    ///
    /// ⚠ 判据的期望值**从契约本身取**（[`DraftCandidate`] 序列化出来的键集合），
    /// 不写死字段名 ⇒ 后端加一项、删一项，这里自动跟着变。
    /// ⚠ 上一轮就踩过同族：夹具里写着 `"rank": 1` 与 `"model_calls": 3`，
    /// 而后端早就不报这两样了 ⇒ 演示页展示的数字与真机对不上。
    #[test]
    fn the_demo_fixtures_carry_the_same_candidate_shape_the_backend_sends() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../static/demo");
        let rd = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("读不到夹具目录 {}：{e}", dir.display()));
        // 契约的键集合：**从后端自己序列化出来**，不写死字段名。
        let contract: std::collections::BTreeSet<String> = serde_json::to_value(DraftCandidate {
            id: String::new(),
            text: String::new(),
            probability: 0.0,
            confidence: ConfidenceTier::Low,
        })
        .expect("DraftCandidate 序列化不了")
        .as_object()
        .expect("DraftCandidate 不是对象")
        .keys()
        .cloned()
        .collect();
        let mut checked = 0usize;
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let text = std::fs::read_to_string(&p).expect("夹具读不出来");
            let v: serde_json::Value = serde_json::from_str(&text)
                .unwrap_or_else(|err| panic!("{} 不是合法 JSON：{err}", p.display()));
            let arr = match v["candidates"].as_array() {
                Some(a) if !a.is_empty() => a,
                _ => continue, // 没有候选的夹具（fresh / silent 等）不参与
            };
            for (i, c) in arr.iter().enumerate() {
                let keys: std::collections::BTreeSet<String> = c
                    .as_object()
                    .expect("候选不是对象")
                    .keys()
                    .cloned()
                    .collect();
                assert_eq!(
                    keys,
                    contract,
                    "{} 的第 {i} 条候选键与后端契约对不上：\
                     少掉的那个是后端**正在下发**的（演示页渲染不出来），\
                     多出来的那个是后端**已经不发了**的（演示页在骗人）。",
                    p.display()
                );
                let prob = c["probability"]
                    .as_f64()
                    .unwrap_or_else(|| panic!("{} 的第 {i} 条候选没有 `probability`", p.display()));
                assert_eq!(
                    c["confidence"].as_str(),
                    Some(ConfidenceTier::from_probability(prob as f32).as_str()),
                    "{} 的第 {i} 条候选：置信度档位与概率对不上。\
                     演示页会照着这个档位画笔迹轻重 ⇒ 画错了就是在骗人。",
                    p.display()
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "一条候选都没验到，等于没验");
    }

    /// **候选正文逐字进 JSON。**
    ///
    /// 票 21 剩下的验收项是「正文里真的出现那句回复」。这句话能被界面
    /// 看到，中间要过一整段：候选 `text` → `snapshot` → `Readout`
    /// → serde 摊平。前两段都有人守，最后这段（序列化）只在
    /// `every_phase_shape_carries_its_own_discriminant` 里数了**条数**。
    ///
    /// ⚠ 判据钉的是「逐字到达」，不是某个字面量——起草器换模型、
    /// 清洗规则改字，都不该让这里红。
    #[test]
    fn the_candidate_text_reaches_the_wire_verbatim() {
        let s = scratch_with(1, 1);
        let st = chatservant_reason::StageCalls::default();
        let out = snapshot(
            &s,
            None,
            Phase::Drafted {
                messages: s.messages.clone(),
                candidates: s.candidates.clone(),
            },
            4,
            1,
            st,
            "起草好了",
        );
        let v = serde_json::to_value(&out).unwrap();
        let first = &v["candidates"][0];
        assert_eq!(
            first["text"].as_str(),
            Some(s.candidates[0].text.as_str()),
            "候选正文没有逐字到达界面（会被当成空消息发出去的那一类）"
        );
        assert_eq!(
            first["id"].as_str(),
            Some(s.candidates[0].id.as_str()),
            "候选 id 变了，防重与点击都对不上"
        );
        assert!(
            first["probability"].is_number(),
            "概率必须带过去：界面按它排序，而 Auto 档挑的是第一条"
        );
    }

    /// **置信度档位必须带过去，且与概率对得上**（2026-10-25）。
    ///
    /// ⚠ 面板那一格画的是**笔迹轻重**，档位就是那一格的**全部内容**
    /// ⇒ 档位错了 = 画错了，而且**界面看不出错**（它只管照着画）。
    ///
    /// ⚠ 这条守卫量的是**事实来源**：它比对的是「后端算出来的档位」
    /// 与「按同一阈值从概率重新算一遍的档位」，
    /// 而不是写死一个字符串 ⇒ 阈值改了这里自动跟着改。
    #[test]
    fn the_confidence_tier_reaches_the_wire_and_matches_the_probability() {
        let mut s = scratch_with(1, 3);
        // 三档各来一条，让守卫真的遍历到不同的档位。
        for (i, p) in [0.7_f32, 0.2, 0.1].into_iter().enumerate() {
            s.candidates[i].probability = p;
            s.candidates[i].confidence = ConfidenceTier::from_probability(p);
        }
        let st = chatservant_reason::StageCalls::default();
        let out = snapshot(
            &s,
            None,
            Phase::Drafted {
                messages: s.messages.clone(),
                candidates: s.candidates.clone(),
            },
            4,
            1,
            st,
            "起草好了",
        );
        let v = serde_json::to_value(&out).unwrap();
        let arr = v["candidates"].as_array().expect("候选不是数组");
        assert_eq!(arr.len(), 3, "三条候选都在");
        for (i, c) in arr.iter().enumerate() {
            let p = c["probability"].as_f64().expect("概率没带上");
            assert_eq!(
                c["confidence"].as_str(),
                Some(ConfidenceTier::from_probability(p as f32).as_str()),
                "第 {i} 条的置信度档位与它的概率对不上：\
                 界面画的是笔迹轻重，档位错了就是画错了"
            );
        }
        // 三档都得真的出现过——不然这条守卫只覆盖了一种情况。
        let tiers: std::collections::BTreeSet<&str> = arr
            .iter()
            .filter_map(|c| c["confidence"].as_str())
            .collect();
        assert_eq!(
            tiers,
            ["high", "low", "mid"]
                .into_iter()
                .collect::<std::collections::BTreeSet<&str>>(),
            "三档没都出现：守卫只覆盖了一种档位就等于没覆盖"
        );
    }

    /// 左侧边栏宽度，**实测微信的值**。
    ///
    /// 夹具里它必须**非零**，否则 `frame.crop(anchor.chat)` 是恒等操作，
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
    /// `name` 决定**笔画落在哪**，而三个 name 的**亮度完全一样**。
    ///
    /// 这是 2026-10-11 那个破口的教训，代价是整个会话闸失效：
    /// 早先这几处夹具用 `tint` 改**亮度**（225 对 120）来造「另一个会话」，
    /// 于是它们一直是绿的；而真实会话名之间亮度一模一样，差的是**字形**。
    /// 闸因此从来没在真实帧上关过——直到实机切了一次会话才暴露。
    ///
    /// 字块起点固定在 x=24/42/60，宽度 13 px，落在 y=12..24，
    /// 与实测那块区域（16000 px 里 189 个墨迹像素）同一量级。
    ///
    ///2026-10-14 票 31：原先画在 `y 0..40`，而夹具的 `chat.y0 = 40`
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
    /// 造「另一个会话」时**不许只改亮度**——见 [`header_frame`] 那条，
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
    /// 走 `discover`，**不自己拼矩形**——这正是票 31 要禁的做法。
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

    /// **「新到的」必须拿「并入本轮之前」的消息流去比。**
    ///
    /// ## 这条守的是什么（2026-10-23 修的一个真缺陷）
    ///
    /// 原来的顺序是：**先并入流，再算新颖度**。于是算新颖度时
    /// **本轮的消息已经在流里了** ⇒ 它们被判成「不新」
    /// ⇒ **真正新到的对方消息不会触发推理**。
    ///
    /// 真机夹具实测（那个顺序下）：本轮读到 8 条、面板从 5 涨到 10，
    /// 但「本轮新到」只有 **2** 条 —— 恰好是两条**旧**消息
    /// （对方消息算错 ⇒ 推理被旧消息误触发）。
    ///
    /// ⚠ **合成帧夹具暴露不了这个**：每一轮都是新画面 ⇒ 两种顺序结果一样
    /// ⇒ 假绿。只有「本轮的消息在上一轮之后已经在流里了」才分岔。
    #[test]
    fn novelty_is_computed_against_the_ledger_before_this_round() {
        // 上一轮：两条旧消息进了账本与面板。
        let mut seen: Vec<Message> = Vec::new();
        let mut display: Vec<Message> = Vec::new();
        let first = absorb_round(&mut seen, &mut display, &[a_msg("旧的甲"), a_msg("旧的乙")]);
        assert_eq!(first.len(), 2, "前提：第一轮两条都该算新的");
        assert_eq!(seen.len(), 2, "前提：第一轮之后账本里有两条");
        assert_eq!(display.len(), 2, "前提：第一轮之后面板里有两条");

        // 这一轮：一条**旧的**（又读到了）+ 一条**真新的**。
        let second = absorb_round(&mut seen, &mut display, &[a_msg("旧的乙"), a_msg("刚到的")]);
        let texts: Vec<&str> = second.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["刚到的"],
            "本轮只该报出真正新到的那条；实际 {texts:?} —— \
             若把「并入」放在「算新颖度」之前，本轮消息已在账本里 ⇒ 全被判成旧的"
        );
        assert_eq!(seen.len(), 3, "账本里应是两条旧的 + 一条新的");
        assert_eq!(display.len(), 3, "面板里应是两条旧的 + 一条新的");
    }

    /// **账本要留得住很多条**——「往前翻」不至于出界。
    ///
    /// 账本容量是残留的边界：账本之外的旧消息身份不在账本里
    /// ⇒ 仍可能被当成新的 ⇒ 对旧内容触发推理。
    /// 512 条意味着真实使用里翻不到那个边界。
    #[test]
    fn the_ledger_keeps_enough_for_a_long_scroll_back() {
        let mut seen: Vec<Message> = Vec::new();
        let mut display: Vec<Message> = Vec::new();

        // 读过 600 条（超过账本容量）。
        for i in 0..600 {
            let batch = vec![a_msg(&format!("第 {i} 条"))];
            absorb_round(&mut seen, &mut display, &batch);
        }
        assert!(
            seen.len() <= SEEN_WINDOW,
            "账本不该超过容量：{} > {}",
            seen.len(),
            SEEN_WINDOW
        );
        assert!(
            seen.len() >= 256,
            "账本留得太少（{} 条）——往前翻几屏就出界 ⇒ 旧消息被当成新的",
            seen.len()
        );

        // 往前翻：画面里是**更早**的消息（它们在账本里）⇒ 不该算新。
        let scrolled: Vec<Message> = (400..410).map(|i| a_msg(&format!("第 {i} 条"))).collect();
        let novel = absorb_round(&mut seen, &mut display, &scrolled);
        assert!(
            novel.is_empty(),
            "往前翻出来的旧消息被当成了新的：{:?}",
            novel.iter().map(|m| &m.text).collect::<Vec<_>>()
        );
    }

    /// **用户往上翻历史 ⇒ 画面里的旧消息不算新的。**
    ///
    /// ## 这条守的是什么（2026-10-23 修的最后一个洞）
    ///
    /// 新颖度原来拿「**最近几条**」当基准（`DEDUP_TAIL = 6`）。
    /// 可**用户往上翻历史**时，画面里全是**更早**的消息 ——
    /// 它们落在那个窗口之外 ⇒ 被当成新的 ⇒ **对旧内容触发一次推理**
    /// （Auto 档下还可能误发）。
    ///
    /// ## 为什么用**计数**（多重集）而不是集合
    ///
    /// 对方把同一句又说一遍（「在吗」…「在吗」）是合法对话。
    /// 集合判 ⇒ 第二句被当成「已经见过」⇒ **不回复**。
    /// 计数判 ⇒ 第一句抵掉、第二句抵不动 ⇒ 第二句算新的 ✔
    ///
    /// 这两条一起才完整：**旧消息不误报**，**新消息不漏报**。
    #[test]
    fn scrolled_up_history_is_not_counted_as_new_but_a_repeat_is() {
        // 账本里已有 8 条（模拟读过一段历史）。
        let mut seen: Vec<Message> = Vec::new();
        let mut display: Vec<Message> = Vec::new();
        let history: Vec<Message> = (0..8).map(|i| a_msg(&format!("旧消息 {i}"))).collect();
        absorb_round(&mut seen, &mut display, &history);
        assert_eq!(seen.len(), 8, "前提：八条都进了账本");

        // ── 情景一：用户往上翻，画面里是**更早**的几条（窗口之外）──
        let scrolled: Vec<Message> = (0..4).map(|i| a_msg(&format!("旧消息 {i}"))).collect();
        let novel = absorb_round(&mut seen, &mut display, &scrolled);
        assert!(
            novel.is_empty(),
            "往上翻历史被当成了新消息：{:?}",
            novel.iter().map(|m| &m.text).collect::<Vec<_>>()
        );

        // ── 情景二：对方把**同一句**又说了一遍（旧的那条还在画面里）──
        let mut seen2: Vec<Message> = Vec::new();
        let mut display2: Vec<Message> = Vec::new();
        absorb_round(&mut seen2, &mut display2, &[a_msg("在吗")]);
        let repeated = absorb_round(&mut seen2, &mut display2, &[a_msg("在吗"), a_msg("在吗")]);
        let texts: Vec<&str> = repeated.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["在吗"],
            "对方把同一句又说一遍 ⇒ 第二句是新的；实际 {texts:?} —— \
             用集合判会把第二句吃掉 ⇒ 不回复"
        );
    }
    /// **同一会话里对方又发了消息 ⇒ 那几条被算成「新到的」。**
    ///
    /// ## 这条守的是什么（2026-10-23 的真缺陷）
    ///
    /// 微信的消息列表是**底部对齐**的：对话变长时**视口整体上移**，
    /// 最新的那条始终贴在输入框上方。⇒ 对方又发了消息之后，
    /// **新消息落在旧水位线之上**。
    ///
    /// 真机实测（同一会话的两张真机帧，夹具换文件模拟「新消息到达」）：
    ///
    /// ```text
    /// 第 1 圈  roi y0=60   本轮行=14  消息=5   ← 首读，水位线推到 569
    /// 换帧后  roi y0=569  本轮行=0   消息=0   ← 新增带是空的：一条都没进来
    /// ```
    ///
    /// 而换帧后**新增的那 5 条**在 y = 130..567 ⇒ **全在 569 之上**。
    ///
    /// ⇒ 判据不能是「在第几行」，只能是「**这条消息之前有没有出现过**」。
    /// 这条钉的就是那个新判据：**内容身份**。
    #[test]
    fn a_new_peer_message_is_counted_as_new() {
        // 已知：A 会话里已经有 3 条对方消息。
        let known: Vec<Message> = (0..3).map(|i| a_msg(&format!("旧消息 {i}"))).collect();

        // 本轮画面里：两条旧的 + 一条新的（正文从没出现过）。
        let current: Vec<Message> = vec![a_msg("旧消息 0"), a_msg("旧消息 1"), a_msg("刚到的")];

        let novel = chatservant_perceive::novel_messages(&known, &current);
        let texts: Vec<&str> = novel.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["刚到的"],
            "只有没出现过的那条才算新到的；实际 {texts:?}"
        );
    }

    /// **诊断只报那两种形态，其余一律不报。**
    ///
    /// 这条守的是「诊断自己别乱报」：它要是把普通单字行也报出来，
    /// 频率统计就没有分母意义了 —— 真机上每屏都有单字行。
    ///
    /// 两种形态都是真机实测来的（票 96 缺陷 B 那一帧）：
    /// 一个是 33×20 的框里两个汉字被读成 1 个码位；另一个是同一帧在别的
    /// 判读下变成「一个汉字 + 一个片假名」。
    #[test]
    fn merged_pair_suspect_flags_exactly_the_two_shapes_seen_on_a_real_screen() {
        let mut wide = a_msg("好");
        wide.rect = chatservant_core::frame::Rect::new(381, 214, 415, 235);
        assert_eq!(
            merged_pair_suspect(&wide),
            Some(("单码位宽框", 34, 21)),
            "34×21 的单码位框应当命中"
        );

        // 同一块被读成「一个汉字 + 一个片假名」——框不宽也要报，这是第二类。
        let mut kana = a_msg("好テ");
        kana.rect = chatservant_core::frame::Rect::new(381, 214, 415, 235);
        assert_eq!(
            merged_pair_suspect(&kana).map(|(k, _, _)| k),
            Some("混进片假名")
        );

        let mut normal = a_msg("好");
        normal.rect = chatservant_core::frame::Rect::new(77, 184, 99, 204);
        assert!(
            merged_pair_suspect(&normal).is_none(),
            "近似方框的单码位行不该报"
        );

        let mut two = a_msg("好的");
        two.rect = chatservant_core::frame::Rect::new(381, 214, 415, 235);
        assert!(
            merged_pair_suspect(&two).is_none(),
            "两个汉字（都读对了）不是这个形态"
        );

        let mut latin = a_msg("ok");
        latin.rect = chatservant_core::frame::Rect::new(381, 214, 700, 235);
        assert!(merged_pair_suspect(&latin).is_none(), "非汉字不参与");
    }

    /// **旧消息重新出现在画面里（滚动翻出）⇒ 不算新的。**
    ///
    /// 这是新判据的另一半：底部对齐的窗里，视口一动整段内容就换位置，
    /// 于是**读到的行里绝大多数是早就看过的旧消息**。
    /// 若按位置判，它们会被当成新消息 ⇒ 每次滚动都触发一次完整推理
    /// ⇒ 既烧钱又可能**对旧消息自动发送**。
    #[test]
    fn an_old_message_re_read_is_not_counted_as_new() {
        let known: Vec<Message> = (0..3).map(|i| a_msg(&format!("旧消息 {i}"))).collect();

        // 视口上移：读到的全是旧消息（顺序还变了）。
        let current: Vec<Message> = vec![a_msg("旧消息 2"), a_msg("旧消息 0"), a_msg("旧消息 1")];

        let novel = chatservant_perceive::novel_messages(&known, &current);
        assert!(
            novel.is_empty(),
            "旧消息被当成新消息了：{:?}",
            novel.iter().map(|m| &m.text).collect::<Vec<_>>()
        );
    }

    /// **正文相同但说话人不同 ⇒ 是两条不同的消息。**
    ///
    /// 身份是「说话人 + 正文」。只看正文的话，「我也说了一句『好的』」
    /// 会被当成对方说的 ⇒ 面板左右不分 ⇒ 推理上下文里我方的话被当对方的话。
    #[test]
    fn the_same_text_from_the_other_side_is_a_different_message() {
        let mut mine = a_msg("好的");
        mine.speaker = Speaker::Me;
        let known: Vec<Message> = vec![mine];

        let current: Vec<Message> = vec![a_msg("好的")]; // 对方说的同一句

        let novel = chatservant_perceive::novel_messages(&known, &current);
        assert_eq!(
            novel.len(),
            1,
            "对方说的同一句话被我方那条吃掉了：{:?}",
            novel.iter().map(|m| &m.text).collect::<Vec<_>>()
        );
    }
    /// **我们自己发出去的那句话，不能被当成对方的新消息。**
    ///
    /// ## 这条守的是什么（2026-10-24，用户真机报出）
    ///
    /// 用户在 `Auto` 档点了一条候选 ⇒ 面板回执说「可能已经发出去了」
    /// ⇒ 紧接着**又起草了一轮**。他怀疑「自己发出去的消息被当成了对方的新消息」。
    ///
    /// ## 为什么这条不能靠「气泡颜色判据」保证
    ///
    /// 归类器靠**气泡底色的彩度**判说话人（饱和 = 我 / 近灰 = 对方）。
    /// 真机帧实测它判对了（绿色气泡 `彩度=91` ⇒ 「我」✔），
    /// 但探针只要有一次采到气泡缝，彩度就掉到 0 ⇒ 判成「对方」
    /// ⇒ 一句话成了「对方的新消息」⇒ 触发一次完整推理
    /// ⇒ `Auto` 档下还可能**再发一条** ⇒ **自问自答**。
    ///
    /// ⇒ 判据必须是**事实**（「我们自己说过这句话」），不是**猜测**（颜色）。
    ///
    /// ## 这条走的是**生产路径**
    ///
    /// 断言的是 `relabel_our_own`（`step()` 里改判的那个函数），
    /// 而它读的 `commands::sent_by_us` 也走生产路径（`note_sent` 在
    /// 装填/发送那一刻记下正文）⇒ 把 `note_sent` 拆掉，这条测试**也该红**。
    #[test]
    fn our_own_sent_line_is_not_counted_as_a_new_peer_message() {
        // ⚠ 正文必须**唯一**：那份账本是进程级的全局，
        // 而测试是并发的 ⇒ 用一句别的测试不会用到的话。
        let text = "我们自己发出去的那一句（守卫测试专用，别与真会话重样）";
        crate::commands::note_sent(text);

        // 模拟「下一轮读回来，探针这次采歪了 ⇒ 判成了对方」。
        let mut msgs = vec![Message {
            id: format!("Them:{text}"),
            speaker: Speaker::Them,
            text: text.into(),
            rect: chatservant_core::frame::Rect::new(0, 0, 10, 10),
            evidence: "测试".into(),
        }];

        relabel_our_own(&mut msgs);

        assert_eq!(
            msgs[0].speaker,
            Speaker::Me,
            "我们自己发出去的话被算成了对方说的"
        );
        assert_eq!(
            msgs[0].id,
            format!("Me:{text}"),
            "改判之后 `id` 必须跟着说话人走（前端按 `id` 做 keyed 渲染）"
        );
        // 判据链的第二段：改判之后，「本轮新到里有没有**对方**的话」必须为空。
        // 这一段才是**真正**的守——推理只由对方的新消息触发（见 `step()`）。
        let peer_news = msgs.iter().filter(|m| m.speaker == Speaker::Them).count();
        assert_eq!(
            peer_news, 0,
            "改判之后仍被算成对方的新消息 ⇒ 会白起草一轮（还可能再发一条）"
        );
    }

    /// **没发过的话不许被改判。**
    ///
    /// 这条守的是护栏的**另一半**：改判判据是「我们自己说过这句话」，
    /// 而不是「这句看起来像从右边来的」⇒
    /// 对方说的、我们没说过的话，必须原样保持「对方」。
    #[test]
    fn a_peer_line_we_never_sent_is_left_alone() {
        let text = "对方说的一句我们从没发过的话（守卫测试专用）";
        let mut msgs = vec![Message {
            id: format!("Them:{text}"),
            speaker: Speaker::Them,
            text: text.into(),
            rect: chatservant_core::frame::Rect::new(0, 0, 10, 10),
            evidence: "测试".into(),
        }];

        relabel_our_own(&mut msgs);

        assert_eq!(
            msgs[0].speaker,
            Speaker::Them,
            "对方的话被误改判成我方 ⇒ 对方的消息不再触发推理"
        );
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

    fn mine_msg(text: &str) -> Message {
        Message {
            id: format!("Me:{text}"),
            speaker: Speaker::Me,
            text: text.into(),
            rect: chatservant_core::frame::Rect::new(0, 0, 10, 10),
            evidence: "测试".into(),
        }
    }

    /// **用户刚说完、对方还没回 ⇒ 不起草。**
    ///
    /// ## 这条守的是什么（2026-10-25 用户真机报出）
    ///
    /// 真机现场：对方只有 1 条（很靠上），用户自己 8 条在后面。
    /// 起草照样触发 ⇒ 模型顺着**用户自己的话**往下写 ⇒ 面板给出的候选
    /// 是在接用户的话，而不是回复对方。
    ///
    /// ## 判据就是分支条件本身
    ///
    /// `should_draft_now` 是 `step()` 里那个 `if` 的**唯一**条件，而起草那一段
    /// （本轮唯一会调模型的出口）在它里面 ⇒ 它返回 `false` 就等于**零模型调用**。
    #[test]
    fn the_drafting_gate_shuts_while_i_have_the_floor() {
        let mut visible = vec![a_msg("对方最上面那一句")];
        visible.extend((0..8).map(|i| mine_msg(&format!("我自己的第 {i} 句"))));
        assert!(
            !should_draft_now(true, &visible),
            "最后一句是我方却仍进起草 ⇒ 模型只能接着用户自己的话往下写"
        );

        visible.push(a_msg("对方回的最后一句"));
        assert!(should_draft_now(true, &visible), "最后一句是对方 ⇒ 该起草");

        // 硬约束第 6 条：没有新的对方消息时，最后一句是谁说的都不许调模型。
        assert!(
            !should_draft_now(false, &visible),
            "没有新的对方消息却要起草 ⇒ 静默期零调用当场作废"
        );
        assert!(
            !should_draft_now(true, &[]),
            "一条消息都没读到 ⇒ 没有可回复的对象"
        );
    }

    /// **送进模型的每一行都带说话人**（同上那次修的另一半）。
    ///
    /// 走的是生产那一路：`history` → `Context::trim` → `Trimmed::render`。
    /// 少了这一半，模型看到的是几行光秃秃的原文，分不出谁说的——
    /// 那正是「接用户的话」的另一半原因。
    #[test]
    fn every_line_the_model_sees_names_its_speaker() {
        let recent = [
            a_msg("对方先开口"),
            mine_msg("我回了一句"),
            a_msg("对方又来一句"),
        ];
        let history: Vec<String> = recent.iter().map(history_line).collect();

        let mut c = NoSummary;
        let rendered = context::Context::new()
            .trim(SESSION, &history, 12, &mut c)
            .expect("裁剪")
            .render();

        // 第一行是 `[最近的对话]` 标题，不是消息正文。
        let lines: Vec<&str> = rendered.lines().skip(1).collect();
        assert_eq!(lines.len(), 3, "最近三条要原样送：{rendered}");
        for line in &lines {
            assert!(
                line.starts_with("对方：") || line.starts_with("我："),
                "这一行没有说话人标记 ⇒ 模型分不出谁说的：{line}"
            );
        }
        assert!(
            lines[0].ends_with("对方先开口"),
            "正文被改写了：{}",
            lines[0]
        );
        assert!(
            lines[1].starts_with("我："),
            "我方那句被标成了别人：{}",
            lines[1]
        );
        assert!(
            lines[2].starts_with("对方："),
            "对方那句被标成了我方：{}",
            lines[2]
        );
    }

    /// **换会话时：状态留着，面板显示清空。**
    ///
    /// ## 这条守的是 2026-10-23 那个改动的两侧
    ///
    /// 原来关闸时做两件事：**清消息** + **退水位线**。两件事都为了同一个
    /// 目标——「面板不许显示别的会话的内容」——代价却是切回来必须**整段重读**，
    /// 于是早已看过的旧消息被当成新消息再推理一次。
    ///
    /// 现在拆成两件：**状态留着**（切回来原样恢复）、**显示清空**
    /// （停泊期间推给界面的消息列表恒为空）。
    ///
    /// ⇒ 这条同时钉住两侧：状态侧不许被清，显示侧不许漏。
    #[test]
    fn closing_the_gate_on_a_different_conversation_keeps_the_history_but_blanks_the_panel() {
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
            confidence: ConfidenceTier::from_probability(0.5),
        });

        let g = apply_gate_closed(&mut s, Some(&b));

        assert!(
            g.just_closed,
            "刚关闸要推一次 readout，否则界面还停在旧状态"
        );
        assert!(
            s.parked_on_other,
            "指纹确实不同 ⇒ 面板必须进入「停泊在别的会话上」状态"
        );
        // ⚠ **状态里不许清**：锁定会话的历史留在 `s.messages` 里，
        //   切回来时要原样恢复（不用靠整段重读把历史长出来）。
        assert_eq!(
            s.messages.len(),
            1,
            "锁定会话的历史被清掉了：{:?}",
            s.messages
        );
        assert_eq!(
            s.candidates.len(),
            1,
            "锁定会话的候选在状态里也必须留着，切回时原样恢复"
        );
        assert!(
            s.reason.contains("锁定的是「A」"),
            "面板要说明为什么不动手，实际：{}",
            s.reason
        );

        // ⚠⚠ **但推给界面的那份必须是空的**：屏幕上那块是别人的会话，
        //   面板却挂着锁定会话的对话或候选 = 停泊面板说假话（票 32）。
        let out = snapshot(
            &s,
            None,
            current_phase(&s),
            1,
            0,
            chatservant_reason::StageCalls::default(),
            &s.reason,
        );
        assert!(
            out.phase.messages().is_empty(),
            "停泊期间推给界面的消息列表必须为空，实际：{:?}",
            out.phase.messages()
        );
        assert!(
            out.phase.candidates().is_empty(),
            "停泊期间推给界面的候选列表必须为空，实际：{:?}",
            out.phase.candidates()
        );
        // **历史不能跟着清**：它是锁定会话的推理上下文，不是面板显示。
        //   清它等于把当前会话攒下的上下文也扔了。
        s.history.push("A 里的历史".into());
        apply_gate_closed(&mut s, Some(&b));
        assert_eq!(s.history.len(), 1, "关闸不是重锁，历史不该被清掉");
    }

    /// **闸重新打开时必须推一帧**——面板还停在「锁着别的对话，切回来才继续」
    ///
    /// 切回来那一轮**很可能什么都没变**（没有新消息）⇒ `observe` 返回
    /// `Quiet` ⇒ 本来不会推帧 ⇒ 面板就一直停在停泊态，
    /// **明明已经切回来了**。
    #[test]
    fn returning_to_the_locked_conversation_pushes_a_frame() {
        let mut s = Scratch::new(RecentN::new(12));
        s.locked_print = Some(print_of(0));
        s.messages.push(a_msg("A 里的历史"));

        // 停泊：闸关着，面板处于停泊态。
        s.prev_gate_open = Some(false);
        s.parked_on_other = true;

        // 切回来：闸重新开。
        let just_opened = gate_opened(&mut s);

        assert!(
            just_opened,
            "闸从关变成开，必须推一帧把面板恢复出来 —— \
             否则面板一直停在「切回来才继续」，而人已经切回来了"
        );
        assert!(
            !s.parked_on_other,
            "切回来之后面板不再是停泊态，锁定会话的消息要能显示"
        );
        assert_eq!(
            s.messages.len(),
            1,
            "切回来时锁定会话的历史必须原样恢复：{:?}",
            s.messages
        );
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

    /// **停泊不许丢锁定会话的状态——切回时不得重复推理。**
    ///
    /// ## 为什么这条是承重的
    ///
    /// 停泊（闸关着）时锁定会话的**消息流必须留着**：
    /// 丢了它 ⇒ 切回来时整段重读 ⇒ **早已看过的旧消息被当成新消息**
    /// ⇒ 触发一次完整模型推理（6~9 秒）。
    ///
    /// 实测形态（2026-10-23 真机）：切回会话一轮 3.28 秒 + 重复推理 6~9 秒
    /// ⇒ 「切换会话 7s+」。而且它**绕过了硬约束第 6 条**（静默期零调用），
    /// 还让 `auto_gate.begin_new_round()` 被调用 ⇒ **有为旧消息自动发送的风险**。
    ///
    /// ## 这条按真实路径走
    ///
    /// `park_on_other_conversation`（**生产用的那个函数**），
    /// 不是手工再调一次 `rewind`——那样测的是测试自己。
    #[test]
    fn parking_keeps_the_locked_conversation_state() {
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

        // 在 A 里读到全部 8 条。
        let Decision::Read { .. } = w.observe(&a) else {
            panic!("首次观察必须读整段消息区")
        };
        s.messages = (0..8).map(|i| a_msg(&format!("A 的第 {i} 条"))).collect();
        w.complete();

        // 切到 B：闸关。**锁定会话的历史不许清**（那是切回来时要恢复的东西）。
        park_on_other_conversation(&mut s, Some(&print_of(1)));
        assert_eq!(
            s.messages.len(),
            8,
            "锁定会话的历史被清了：{:?}",
            s.messages
        );

        // 切回 A：闸重新开。**画面没变 ⇒ 零 OCR、零推理。**
        assert!(
            print_of(0).is_same(s.locked_print.as_ref().expect("锁还在")),
            "前提：切回来之后指纹要判成同一个会话"
        );
        assert!(
            matches!(w.observe(&a), Decision::Quiet),
            "切回锁定会话时画面没变 ⇒ 必须是静默（零 OCR）；\
             读一遍就意味着旧消息被当成新消息再推理一次"
        );
        assert_eq!(w.calls(), 1, "整场只该读过一次（A 里的那一次）");
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
    /// 票 31 之后矩形不再由这里拼，改由
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
    fn assert_parked_snapshot_hides_retained_history(
        s: &Scratch,
        anchor: chatservant_capture::anchor::PaneAnchor,
    ) {
        assert_eq!(
            s.messages.len(),
            2,
            "闸关着的时候锁定会话的消息流变了 —— 切回来会读到错的区间，\
             而旧消息会被当成新消息再推理一次"
        );
        let parked = snapshot(
            s,
            Some(anchor),
            current_phase(s),
            2,
            0,
            chatservant_reason::StageCalls::default(),
            &s.reason,
        );
        assert!(
            parked.phase.messages().is_empty(),
            "闸关着却还挂着锁定会话的内容：{:?}",
            parked.phase.messages()
        );
        assert!(
            parked.phase.candidates().is_empty(),
            "闸关着却还向界面推候选：{:?}",
            parked.phase.candidates()
        );
        assert_eq!(
            s.messages.len(),
            2,
            "锁定会话的历史被清掉了：{:?}",
            s.messages
        );
    }

    #[test]
    fn the_gate_closes_and_reopens_over_a_frame_sequence_with_one_watcher() {
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
        let rows_b: [(u32, u32); 2] = [(520, 540), (620, 640)];

        let a = conversation_frame(&rows_a, 0);
        let b = conversation_frame(&rows_b, 1);
        let anchor = test_anchor();
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
        let Decision::Read { .. } = w.observe(&pane(&a)) else {
            panic!("第一帧必然要读")
        };
        w.complete();

        // ── 第 2 圈：切到 B。同一个 watcher，只是内容换了 ──
        let peer_ok = match (&s.locked_print, Some(&pb)) {
            (Some(x), Some(y)) => x.is_same(y),
            _ => true,
        };
        assert!(!peer_ok, "前提：B 必须被认成不是锁定的那个会话");
        let just_closed = park_on_other_conversation(&mut s, Some(&pb));
        assert!(just_closed, "刚换会话：面板要推一次 readout");
        assert_parked_snapshot_hides_retained_history(&s, anchor);

        // ── 第 3 圈：切回 A（闸重新打开）──
        let back = header_print(&a, &anchor).expect("切回来的指纹取得到").0;
        assert!(back.is_same(&pa), "前提：切回来认得是同一个会话");
        let just_opened = gate_opened(&mut s);
        assert!(just_opened, "切回来：闸重新打开，要推一帧");
        assert!(!s.parked_on_other, "切回来之后面板不再是停泊态");
        assert!(
            matches!(w.observe(&pane(&a)), Decision::Quiet),
            "切回锁定会话时画面没变 ⇒ 必须是静默（零 OCR）；\
             读一遍就意味着旧消息被当成新消息再推理一次"
        );
        assert_eq!(w.calls(), 1, "整场只该读过一次（A 里的那一次）");

        let restored = snapshot(
            &s,
            Some(anchor),
            current_phase(&s),
            3,
            0,
            chatservant_reason::StageCalls::default(),
            &s.reason,
        );
        assert_eq!(
            restored.phase.messages().len(),
            2,
            "切回来之后面板必须恢复锁定会话的历史：{:?}",
            restored.phase.messages()
        );
        assert!(s.locked_print.is_some(), "锁还在，闸重新开着");
    }

    /// 锚点：**消息区从 x=[SIDEBAR_W] 起**，标题栏在顶上。
    ///
    /// `chat.x0` 非零是有意的——见 [`SIDEBAR_W`]。头带在消息区坐标系里
    /// 于是落在 x∈[0,400) 的字块上；错成拿 `chat.x0` 去裁一块**已裁过**的
    /// 画面时，取样框会整个跑到右边外面，`header_print` 直接给 `None`。
    fn test_anchor() -> chatservant_capture::anchor::PaneAnchor {
        use chatservant_capture::anchor::PaneAnchor;
        use chatservant_core::frame::Rect;
        // 2026-10-14 改：这里原先给 `header_top: 0`，于是 `chat.y0` 也是 0，
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
    fn is_header_glyph_pixel(x: u32, y: u32, name_top: u32, name: u32) -> bool {
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
        on
    }

    fn conversation_frame(rows: &[(u32, u32)], name: u32) -> chatservant_capture::Frame {
        let w = SIDEBAR_W + PANE_W;
        let mut px = vec![30u8; (w * PANE_H * 3) as usize];
        // 侧边栏换个底色：一眼能看出「它不在消息区里」，头带不该从这儿取。
        for y in 0..PANE_H {
            for x in 0..SIDEBAR_W {
                let i = ((y * w + x) * 3) as usize;
                px[i] = 60;
                px[i + 1] = 60;
                px[i + 2] = 62;
            }
        }
        // 会话名行：按真机几何画在 chat.y0 往下 18px、行高 11。
        let name_top = 40u32 + 18;
        for y in name_top..(name_top + 11u32) {
            for x in 0..PANE_W {
                if is_header_glyph_pixel(x, y, name_top, name) {
                    let i = ((y * w + SIDEBAR_W + x) * 3) as usize;
                    px[i] = 225;
                    px[i + 1] = 225;
                    px[i + 2] = 225;
                }
            }
        }
        // 消息行从 y=80 起，与 y 58..69 的会话名行留出空隙。
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
        use chatservant_capture::anchor::PaneAnchor;
        use chatservant_core::frame::Rect;

        const W: u32 = 500;
        const H: u32 = 300;

        // 同一个亮度的白字，**笔画落点不同** —— 这才是两个名字的真实差别。
        //
        // 早先这里是 `tint: u8`，用 225 对 120 的**亮度差**造「另一个会话」。
        // 真实会话名亮度一样、差的是字形，所以那条夹具一直证明着
        // 与现实无关的事，闸也一直在真实帧上关不掉。
        let frame_with = |name: u32| header_frame(W, H, name);

        // 锚点给的是**帧坐标**，而 `header_print` 收的是**消息区**，
        // 这里让 chat 从 (0,0) 开始，两套坐标才一致。
        // 2026-10-14：给**真实几何**。`chat.y0 == header_top` 是
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
        // 2026-10-14 票 31：原先写 `> 5_000`，那是按**已删掉**的固定带子
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

    /// 2026-10-14 新增。**这条是这次修复的判据。**
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
    ///这条判据替掉过一条按「带子落在消息区**之上**」写的
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
        //匹配串**必须拼出来**，不能写成完整字面量：判据查的是
        // `include_str!` 进来的**整个文件**，而判据自己就住在这个文件里。
        // 写成字面量时，它会匹配到自己那行 ⇒ 永远红。
        // 这与「量具自检必须能因为量具偏了而失败」同源：判据也会匹配自己。
        //2026-10-14：**连注释里也不行**。这条注释原先写着那个完整函数名，
        //   于是判据匹配到自己解释「为什么不能写字面量」的那行——
        //   判据自己被自己的注释绊倒。拆成 `header_band_` + `rect` 两截。
        let banned = format!("fn header_band_{}(", "rect");
        assert!(
            !source.contains(&banned),
            "header_band_rect 又回来了——头部矩形必须由 perceive 的 discover 推，\
             不许在这里拼"
        );
        // 换掉原先那条查 `chat.y0 - HEAD_BAND_H` 的检查：常量连同函数一起
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
    /// 如果 `take()` 放在建锁判断**之后**，用户点了按钮、这一轮请求被跳过，
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

    /// **重锁分支必须留下日志**（2026-11-01 加）。
    ///
    /// 这条守的不是行为，是**可诊断性**：真机上出现过「同一批消息被推理两次」
    /// （同一帧两轮 `本轮新到` 逐字相同、两次 Judge+Draft），而清账本的全仓只有
    /// `apply_relock` 一处。它当时**一行 tracing 都没有** ⇒ 日志既不能证实也不能
    /// 否证「是不是重锁了」，只能靠排除法猜。
    ///
    /// ⇒ 判据是「分支体里有没有 `tracing::`」，不是「日志写了什么」：
    /// 措辞可以改，**静默地删掉**才是要拦的形态。
    /// ⚠ 窗口只取 `if s.relock.take() {` 到 `apply_relock(s);` 之间，
    /// 而 `prod` 已经切掉了测试段 ⇒ 判据不会匹配到自己。
    #[test]
    fn the_relock_branch_says_so_in_the_log() {
        let source = include_str!("watch.rs");
        let prod = source.split("#[cfg(test)]").next().unwrap_or(source);
        let marker = format!("if s.{}.take() {{", "relock");
        let branch = prod
            .split(&marker)
            .nth(1)
            .and_then(|s| s.split("apply_relock(s);").next())
            .unwrap_or("");
        assert!(!branch.is_empty(), "重锁分支不见了——判据悬空等于没有");
        assert!(
            branch.contains("tracing::"),
            "重锁分支又变成静默的了——它清空账本并退水位线，\
             没有这行日志就分不清「重锁」与「有别的路径在清账本」"
        );
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
        // 2026-10-14：给**真实几何**。`chat.y0 == header_top` 是
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

    // ─────────────────────────────────────────────────────────────────
    // 票 72：产品读帧路径的 OCR 配方
    //
    // 病根：产品路径（`watch.rs`）手抄了一份 OCR 配方，而抄的那份是
    // **WinRT** 的（整帧 + 2 倍），默认引擎 **PP-OCR** 要的是
    // **只给消息区 + 1 倍**。两边分叉**不报错**——门禁全绿、探针全绿，
    // 只有真机上的消息数是 0。
    //
    // 下面这几条把它钉成机器判据。
    // ─────────────────────────────────────────────────────────────────

    /// 合成一张「微信式」的帧，**带真墨色**，并给出它的锚点。
    ///
    /// ## 为什么不用仓里的脱敏夹具
    ///
    /// 脱敏会把文字刷成底色，于是 `probe_bubble_bg` 量到的 `ink`/`contrast`
    /// 恒为零，分类判据无从测量（见 `perceive::pipeline` 的模块注释）。
    /// ⇒ 分类这一层**必须**用带墨色的合成帧，而合成帧**不含任何真人数据**。
    ///
    /// ## 几何与 `perceive` 的测试帧一致
    ///
    /// 侧边栏 192 宽、标题栏 27 高、输入框上沿在 426——这样
    /// `anchor::derive` 推出来的消息区是 `(192,27)-(800,426)`，
    /// 本模块才能拿 `perceive::read`（**探针那条路**）当参照。
    fn wechat_frame_with_ink() -> (Frame, PaneAnchor) {
        const W: u32 = 800;
        const H: u32 = 600;
        const SIDEBAR_W: u32 = 192;
        const HEADER_H: u32 = 27;
        const INPUT_TOP: u32 = 426;
        const PANEL: [u8; 3] = [30, 30, 31];
        const CHROME: [u8; 3] = [51, 51, 51];
        const THEIR_BUBBLE: [u8; 3] = [47, 47, 48];
        const THEIR_INK: [u8; 3] = [225, 225, 229];
        const GRAY_INK: [u8; 3] = [123, 123, 128];

        /// 侧边栏画成**横条纹**，不是一块纯色。
        ///
        /// ## 为什么不能是纯色
        ///
        /// 真机上那一列是**满的**——联系人名、头像、消息预览，颜色不统一。
        /// 采样落错地方（拿整帧去采消息区坐标的行框）时，外圈纯度会掉到
        /// `RING_PURITY_MIN` 以下 ⇒ 那一行被判成「图里的字」⇒
        /// **一条消息都读不出来**——这正是真机上的症状。
        ///
        /// 夹具若把侧边栏画成纯色，采错地方也一样「干净」⇒ 守卫永远绿。
        /// ⇒ 这条纹不是装饰，是**让错误看得见**。
        const SIDEBAR_STRIPES: [[u8; 3]; 4] =
            [[34, 34, 51], [62, 48, 96], [40, 74, 58], [96, 52, 64]];

        let mut px = vec![0u8; (W * H * 3) as usize];
        for y in 0..H {
            for x in 0..W {
                let c = if x < SIDEBAR_W {
                    SIDEBAR_STRIPES[((y / 4) as usize) % SIDEBAR_STRIPES.len()]
                } else if y < HEADER_H || y == INPUT_TOP {
                    CHROME
                } else {
                    PANEL
                };
                let i = ((y * W + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&c);
            }
        }
        // 对方气泡（近灰底 + 亮字）⇒ 归类为「对方说的」。
        fill_rect(&mut px, W, Rect::new(224, 156, 416, 184), THEIR_BUBBLE);
        stroke_rect(&mut px, W, Rect::new(240, 164, 400, 176), THEIR_INK);
        // 面板上的一行灰字（时间戳）⇒ 归类为噪声，**不许**进消息流。
        stroke_rect(&mut px, W, Rect::new(400, 250, 500, 262), GRAY_INK);

        let f = Frame::from_rgb(W, H, px).expect("合成帧");
        let anchor = chatservant_capture::anchor::derive(&f).expect("锚点推得出来");
        (f, anchor)
    }

    /// **底部对齐的第二帧**：视口整体上移 ⇒ 旧气泡上移，**新气泡落在旧水位线之上**。
    ///
    /// 与 [`wechat_frame_with_ink`] 同几何（侧栏条纹、标题栏、输入框上沿），
    /// 差别只在消息区里的气泡：旧的挪到 126..154，**新的那条在 96..124**
    /// —— 即新消息的下沿 124 **小于**第一轮那条的下沿 156（消息区相对 129）。
    ///
    /// ⚠ 这就是微信的真相：最新的那条始终贴在输入框上方，
    /// 视口一变，**整段内容的位置都变了**。
    fn wechat_frame_bottom_anchored() -> (Frame, PaneAnchor) {
        const W: u32 = 800;
        const H: u32 = 600;
        const SIDEBAR_W: u32 = 192;
        const HEADER_H: u32 = 27;
        const INPUT_TOP: u32 = 426;
        const PANEL: [u8; 3] = [30, 30, 31];
        const CHROME: [u8; 3] = [51, 51, 51];
        const THEIR_BUBBLE: [u8; 3] = [47, 47, 48];
        const THEIR_INK: [u8; 3] = [225, 225, 229];
        const SIDEBAR_STRIPES: [[u8; 3]; 4] =
            [[34, 34, 51], [62, 48, 96], [40, 74, 58], [96, 52, 64]];

        let mut px = vec![0u8; (W * H * 3) as usize];
        for y in 0..H {
            for x in 0..W {
                let c = if x < SIDEBAR_W {
                    SIDEBAR_STRIPES[((y / 4) as usize) % SIDEBAR_STRIPES.len()]
                } else if y < HEADER_H || y == INPUT_TOP {
                    CHROME
                } else {
                    PANEL
                };
                let i = ((y * W + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&c);
            }
        }
        // 视口上移 30px：旧气泡 156..184 → 126..154。
        fill_rect(&mut px, W, Rect::new(224, 126, 416, 154), THEIR_BUBBLE);
        stroke_rect(&mut px, W, Rect::new(240, 134, 400, 146), THEIR_INK);
        // **新到的那条**：在 96..124 ⇒ 它的下沿 124 在旧那条的下沿 156 **之上**。
        fill_rect(&mut px, W, Rect::new(224, 96, 416, 124), THEIR_BUBBLE);
        stroke_rect(&mut px, W, Rect::new(240, 104, 400, 116), THEIR_INK);

        let f = Frame::from_rgb(W, H, px).expect("合成帧");
        let anchor = chatservant_capture::anchor::derive(&f).expect("锚点推得出来");
        (f, anchor)
    }

    fn fill_rect(px: &mut [u8], w: u32, r: Rect, c: [u8; 3]) {
        for y in r.y0.max(0) as u32..r.y1.max(0) as u32 {
            for x in r.x0.max(0) as u32..r.x1.max(0) as u32 {
                let i = ((y * w + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&c);
            }
        }
    }

    /// 在框内画两道 2 px 横笔——真实字形就这个粗细。
    ///
    /// 画成实心块的话外圈众数会被拉偏，测的就不是真实情形了。
    fn stroke_rect(px: &mut [u8], w: u32, r: Rect, c: [u8; 3]) {
        for y in [r.y0 + 3, r.y0 + 7] {
            for dy in 0..2i32 {
                for x in (r.x0 + 4)..(r.x1 - 4) {
                    let i = (((y + dy) as u32 * w + x as u32) * 3) as usize;
                    px[i..i + 3].copy_from_slice(&c);
                }
            }
        }
    }

    /// 一个**看得见自己收到的是哪张图**的假引擎。
    ///
    /// 真引擎对画面是有脾气的：PP-OCR 收整帧会漏行、WinRT 收裁剪件会漏行
    /// （`perceive::pipeline` 里两组实测）。这个假引擎把那种脾气做成可测的：
    /// **只有收到配方规定的那张图，它才给出行**；收到别的就一行不给。
    ///
    /// 票 72 的病正是这个形状：产品路径把 **WinRT 的配方**喂给了 **PP-OCR**，
    /// 真机上的症状是「OCR 总行 56、本轮行 10、**归类 10、消息 0**」。
    struct PickyEngine {
        plan: ViewPlan,
        /// 消息区（换算行框要用）。
        chat: Rect,
        /// 配方规定的那张图的尺寸。收到别的就装死。
        expect_view: (u32, u32),
        /// 行框写在**帧坐标**里；源按配方换算成视图坐标。
        lines: Vec<(String, Rect)>,
        /// 实际收到的那张图的尺寸（诊断用）。
        seen: std::cell::RefCell<Option<(u32, u32)>>,
    }

    impl PickyEngine {
        fn new(plan: ViewPlan, chat: Rect, frame: &Frame) -> Self {
            let expect_view = match plan.policy {
                ViewPolicy::WholeFrame => {
                    (frame.width() * plan.upscale, frame.height() * plan.upscale)
                }
                ViewPolicy::ChatPane => (chat.width() as u32, chat.height() as u32),
            };
            PickyEngine {
                plan,
                chat,
                expect_view,
                // 对方那条 + 面板上的一行灰字（时间戳）。
                lines: vec![
                    ("对方那条".to_string(), Rect::new(240, 164, 400, 176)),
                    ("时间戳".to_string(), Rect::new(400, 250, 500, 262)),
                ],
                seen: std::cell::RefCell::new(None),
            }
        }
    }

    impl LineSource for PickyEngine {
        fn read(&self, view: &Frame) -> Result<OcrText, OcrError> {
            *self.seen.borrow_mut() = Some((view.width(), view.height()));
            if (view.width(), view.height()) != self.expect_view {
                // 收到的不是配方规定的那张图 ⇒ 真引擎在这里会**读不到东西**。
                return Ok(OcrText {
                    text: String::new(),
                    lines: vec![],
                });
            }
            let s = self.plan.upscale as i32;
            let lines = self
                .lines
                .iter()
                .map(|(t, r)| {
                    let rect = match self.plan.policy {
                        // 整帧视图：视图坐标 = 帧坐标 × 倍数。
                        ViewPolicy::WholeFrame => Rect::new(r.x0 * s, r.y0 * s, r.x1 * s, r.y1 * s),
                        // 裁剪视图：视图坐标已经就是消息区相对坐标。
                        ViewPolicy::ChatPane => Rect::new(
                            r.x0 - self.chat.x0,
                            r.y0 - self.chat.y0,
                            r.x1 - self.chat.x0,
                            r.y1 - self.chat.y0,
                        ),
                    };
                    OcrLine {
                        text: t.clone(),
                        rect,
                        words: vec![],
                    }
                })
                .collect();
            Ok(OcrText {
                text: String::new(),
                lines,
            })
        }

        fn view_plan(&self) -> ViewPlan {
            self.plan
        }
    }

    /// **产品路径递进 OCR 的那张图，必须是引擎要的那一张。**
    ///
    /// 判据有两条：① 引擎**收到**的尺寸 = 配方规定的尺寸；
    /// ② 按配方喂进去之后，**对方那条消息真的被读出来了**。
    ///
    /// 第二条是重点：配方错了的时候症状是「看得见行、归成 0 条消息」，
    /// 那正是真机上票 72 的样子——**不报错、不告警，只是读不到**。
    #[test]
    fn the_product_read_path_hands_ocr_the_view_the_engine_asks_for() {
        let (frame, anchor) = wechat_frame_with_ink();
        let pane = frame.crop(anchor.chat).expect("裁得出消息区");
        let roi = Rect::new(0, 0, anchor.chat.width(), anchor.chat.height());

        for plan in [ViewPlan::PPOCR, ViewPlan::WINRT] {
            let src = PickyEngine::new(plan, anchor.chat, &frame);
            let expect_view = src.expect_view;
            let round = read_round(&frame, &pane, &anchor, roi, &src).expect("读帧成功");
            assert_eq!(
                *src.seen.borrow(),
                Some(expect_view),
                "配方 {plan:?}：引擎收到的那张图不是它要的那一张"
            );
            let theirs: Vec<_> = round
                .messages
                .iter()
                .filter(|m| m.speaker == Speaker::Them)
                .collect();
            assert_eq!(
                theirs.len(),
                1,
                "配方 {plan:?}：对方那条没被读出来——{:?}",
                round
                    .messages
                    .iter()
                    .map(|m| (&m.text, m.speaker))
                    .collect::<Vec<_>>()
            );
            assert!(theirs[0].text.contains("对方那条"));
            // 面板上的灰字（时间戳）不许进消息流。
            assert!(
                !round.messages.iter().any(|m| m.text.contains("时间戳")),
                "灰字混进消息流了：{:?}",
                round.messages.iter().map(|m| &m.text).collect::<Vec<_>>()
            );
        }
    }

    /// **底部对齐的窗里，对方来了新消息 ⇒ 产品路径必须读到它。**
    ///
    /// ## 这条守的是什么（2026-10-23 的真缺陷，端到端形态）
    ///
    /// 微信的消息列表是**底部对齐**的：对话变长时**视口整体上移**，
    /// 最新的那条始终贴在输入框上方。⇒ 对方又发了消息之后，
    /// **新消息落在旧水位线之上**。
    ///
    /// 真机实测（同一会话的两张真机帧，夹具换文件模拟「新消息到达」）：
    ///
    /// ```text
    /// 第 1 圈  roi y0=60   本轮行=14  消息=5   ← 首读，水位线推到 569
    /// 换帧后  roi y0=569  本轮行=0   消息=0   ← 新增带是空的：一条都没进来
    /// ```
    ///
    /// 而换帧后**新增的那 5 条**在 y = 130..567 ⇒ **全在 569 之上**。
    ///
    /// ## 为什么这条要落在**产品路径**上（而不是引擎层）
    ///
    /// 旧形态横跨两个 crate：
    /// 引擎按**像素水位线**给出 ROI（`perceive::stream`）
    /// → `read_round` 按 ROI 过滤行（这里）
    /// → 新消息被滤掉。
    ///
    /// 只在引擎层测**抓不住它**（实测：把引擎改回旧判据，引擎层的守卫不红）——
    /// 因为拆掉水位线之后，引擎那一层压根不知道「水位线以上」是什么。
    /// ⇒ 守卫必须走**产品路径**：`read_round` 拿到的 roi 覆盖不到新消息时，
    /// 那条消息就丢了。
    ///
    /// ## 判据分两半
    ///
    /// ① **旧形态的 ROI 确实读不到它**（前提）——证明这条测的不是假闸；
    /// ② **产品路径的 ROI 读得到它**（结论）。
    #[test]
    fn a_bottom_anchored_new_message_survives_the_product_read_path() {
        let (frame_a, anchor) = wechat_frame_with_ink();
        let (frame_b, _) = wechat_frame_bottom_anchored();
        let pane_a = frame_a.crop(anchor.chat).expect("裁得出消息区");
        let pane_b = frame_b.crop(anchor.chat).expect("裁得出消息区");

        // 行框写在**帧坐标**里。
        let old_line = Rect::new(240, 164, 400, 176);
        let moved_line = Rect::new(240, 134, 400, 146);
        let fresh_line = Rect::new(240, 104, 400, 116);

        // ── 第一轮：读到旧消息，水位线 = 它的下沿 ──
        let mut src_a = PickyEngine::new(ViewPlan::PPOCR, anchor.chat, &frame_a);
        src_a.lines = vec![("旧的那条".to_string(), old_line)];
        let roi_a = Rect::new(0, 0, anchor.chat.width(), anchor.chat.height());
        let round_a =
            read_round(&frame_a, &pane_a, &anchor, roi_a, &src_a).expect("第一轮读帧成功");
        assert_eq!(
            round_a.messages.len(),
            1,
            "前提：第一轮必须读到旧的那条 —— 否则这条测的是个假闸"
        );
        let watermark = round_a.messages[0].rect.y1;
        assert!(
            watermark > 0,
            "前提：第一轮那条的下沿必须为正 —— 否则水位线无从谈起"
        );

        // ── 第二轮：视口上移，新消息落在旧水位线之上 ──
        let mut src_b = PickyEngine::new(ViewPlan::PPOCR, anchor.chat, &frame_b);
        src_b.lines = vec![
            ("旧的那条".to_string(), moved_line),
            ("刚到的".to_string(), fresh_line),
        ];

        // **旧形态的 ROI**：像素水位线以下。
        let old_roi = Rect::new(0, watermark, anchor.chat.width(), anchor.chat.height());
        let round_old =
            read_round(&frame_b, &pane_b, &anchor, old_roi, &src_b).expect("第二轮读帧成功");
        assert!(
            !round_old.messages.iter().any(|m| m.text.contains("刚到的")),
            "前提不成立：旧 ROI 竟然读到了新消息 —— 这条守卫会假绿"
        );

        // ── 产品路径的 ROI：它必须覆盖到新消息 ──
        let mut w = chatservant_perceive::PaneWatcher::new(0);
        assert!(matches!(
            w.observe(&pane_a),
            chatservant_perceive::Decision::Read { .. }
        ));
        // ⚠ **必须推进水位线**（`step()` 的真实行为：读到对方消息就 `commit(最大 y1)`）。
        // 不推进的话，旧机制下 `top.max(watermark=0)` = top ⇒ 整块
        // ⇒ **这条守卫会假绿**（实测：那样写，反向自证抓不住它）。
        w.commit(watermark);
        let chatservant_perceive::Decision::Read { roi } = w.observe(&pane_b) else {
            panic!("画面变了必须读 —— 底部对齐的窗里视口一上移，整段像素都变了")
        };

        let round_new =
            read_round(&frame_b, &pane_b, &anchor, roi, &src_b).expect("第二轮读帧成功");
        let texts: Vec<&str> = round_new.messages.iter().map(|m| m.text.as_str()).collect();
        assert!(
            texts.iter().any(|t| t.contains("刚到的")),
            "产品路径没读到新到的那条：{texts:?} —— \
             底部对齐的窗里新消息落在旧水位线之上，ROI 覆盖不到它"
        );
    }
    /// **产品路径与探针路径必须给出同一个答案。**
    ///
    /// 探针那条路是 `chatservant_perceive::read`（`xtask perceive` 走它）。
    /// 两条路读同一张帧、用同一个引擎，答案必须一致——
    /// **不一致就说明产品路径自己抄了一份配方**，而分叉的后果是
    /// 「探针全绿、真机消息为 0」（票 72）。
    ///
    /// ⚠ 比较时给产品路径的 ROI 覆盖**整个消息区**：探针那条路没有 ROI
    /// 这一层，两条路只在「本轮该管哪些行」上不同。
    #[test]
    fn the_product_read_path_agrees_with_the_probe_path() {
        let (frame, anchor) = wechat_frame_with_ink();
        let pane = frame.crop(anchor.chat).expect("裁得出消息区");
        let roi = Rect::new(0, 0, anchor.chat.width(), anchor.chat.height());

        for plan in [ViewPlan::PPOCR, ViewPlan::WINRT] {
            let product = read_round(
                &frame,
                &pane,
                &anchor,
                roi,
                &PickyEngine::new(plan, anchor.chat, &frame),
            )
            .expect("产品路径读帧成功");
            let probe =
                chatservant_perceive::read(&frame, &PickyEngine::new(plan, anchor.chat, &frame))
                    .expect("探针路径读帧成功");

            assert_eq!(
                product.messages.len(),
                probe.messages.len(),
                "配方 {plan:?}：两条路读出的条数不同——产品路径抄了自己的配方？"
            );
            for (a, b) in product.messages.iter().zip(probe.messages.iter()) {
                assert_eq!(a.speaker, b.speaker, "配方 {plan:?}：说话人不同");
                assert_eq!(a.text, b.text, "配方 {plan:?}：正文不同");
                assert_eq!(a.rect, b.rect, "配方 {plan:?}：位置不同");
            }
        }
    }

    /// **失败之后必须补一帧正常态**——采集恢复了，界面不能一直挂着错误。
    ///
    /// 真机（2026-10-20）：微信窗口被最小化 ⇒ 采集失败 ⇒ 界面挂上
    /// 「没写成：抓不到：窗口已最小化——还原它才能抓到」；
    /// 用户还原窗口 ⇒ 采集恢复 ⇒ 但**恢复后的第一轮「什么都没变」**，
    /// 那一支**不发** ⇒ 界面永远挂着那句话——
    /// 用户明明已经还原了，界面还在催他还原。
    #[test]
    fn a_failure_is_followed_by_a_normal_frame_once_the_capture_recovers() {
        let s = Scratch::new(RecentN::new(12));
        // 静默期：**不许**推（硬约束第 6 条——静默期零调用、零推送）。
        assert!(
            phase_to_push(&s, false, false).is_none(),
            "静默期不许推帧——每一圈都推会把界面刷成瀑布"
        );
        // 失败过之后，即使这一轮「什么都没变」，也必须补一帧。
        let pushed = phase_to_push(&s, false, true).expect("失败之后必须补一帧");
        assert!(
            !matches!(pushed, Phase::Failed { .. }),
            "补的那一帧不能又是 Failed"
        );
        // 有变化那一支照旧推。
        assert!(phase_to_push(&s, true, false).is_some(), "有变化必须推帧");

        // 接线：**函数测了但忘了调**就是空守卫。
        let source = include_str!("watch.rs");
        let prod = source.split("#[cfg(test)]").next().unwrap_or(source);
        assert!(
            prod.contains("phase_to_push(&s, false,"),
            "「什么都没变」那一支没调 phase_to_push——恢复后补帧那条判据没接上"
        );
    }

    /// **生产代码不许自己拼 OCR 配方。**
    ///
    /// 这条是**接线**层面的守：配方由
    /// `chatservant_perceive::read_lines` 提供，调用方只许调它。
    /// 手抄一份的代价，票 72 已经付过一次了。
    #[test]
    fn the_watch_layer_does_not_hand_roll_the_ocr_recipe() {
        let source = include_str!("watch.rs");
        let prod = source.split("#[cfg(test)]").next().unwrap_or(source);
        assert!(
            prod.contains("chatservant_perceive::read_lines("),
            "产品路径没走 `perceive::read_lines`——接线断了"
        );
        // `peer_name_from_header` 那一处放大是**另一件事**（读会话名，
        // 不是读消息），所以不能一刀切禁掉放大。
        // 真正要禁的是**消息读帧路径**里出现它。
        let read_round_body = prod
            .split("fn read_round(")
            .nth(1)
            .and_then(|s| s.split("\n}\n").next())
            .unwrap_or("");
        assert!(
            !read_round_body.contains("upscaled_nearest"),
            "读帧路径里又自己放大画面了——配方必须由引擎决定"
        );
        // ⚠ **双线性同样不许出现在读帧路径里**（2026-10-21 加）。
        // 重采样方式也是**引擎的属性**（`ocr::Engine::resample`）：
        // WinRT 那条路要双线性、PP-OCR 那条路是 1× 不放大。
        // 谁在这里手写一份，谁就在两条路之间造出了第三个配方。
        assert!(
            !read_round_body.contains("upscaled_bilinear"),
            "读帧路径里又自己挑重采样方式了——配方必须由引擎决定"
        );
        assert!(
            !read_round_body.contains("Resample::"),
            "读帧路径里又写死了重采样方式——配方必须由引擎决定"
        );
        assert!(
            !read_round_body.contains("OCR_UPSCALE"),
            "读帧路径里又写死放大倍数了——配方必须由引擎决定"
        );

        // ⚠⚠ **读会话名那条路已经搬走了**（2026-10-21，票 77）：
        // 它原先在本文件里（自己裁带子、自己放大、自己喂 OCR），
        // 而 `xtask perceive` 那条**探针**另写了一套 ⇒ **两条不同的路**，
        // 于是探针那行「OCR 读到的会话名」**证明不了产品**。
        // ⇒ 现在实现住在 `chatservant_perceive::peer`，产品与探针**共用一份**，
        // 配方守卫也跟着搬到那边（`perceive::peer` 的
        // `the_name_path_does_not_hand_roll_the_ocr_recipe`）。
        //
        // 这里只守**接线**：产品必须调那一份，且**不许**在本文件里
        // 又长出一份读名字的实现。
        assert!(
            prod.contains("chatservant_perceive::read_peer_name("),
            "产品路径没走 `perceive::read_peer_name`——接线断了"
        );
        assert!(
            !prod.contains("fn peer_name_from_header"),
            "本文件里又长出了一份读会话名的实现——必须只有 perceive 那一份"
        );
    }

    #[test]
    fn session_buckets_isolate_and_restore_conversations_across_switches() {
        let mut s = Scratch::new(RecentN::new(12));
        let pa = print_of(1);
        let pb = print_of(2);

        // 1. 绑定会话 A 并累积历史与已见账本
        assert!(!restore_or_bind_bucket(&mut s, &pa));
        s.locked_name = "会话A".into();
        s.history.push("对方: A的第一句".into());
        s.mine.push("我对A说的话".into());
        s.seen.push(Message {
            id: "Them:A的第一句".into(),
            speaker: Speaker::Them,
            text: "A的第一句".into(),
            rect: chatservant_core::frame::Rect::new(0, 0, 10, 10),
            evidence: String::new(),
        });

        // 2. 切换到会话 B（先将 A 存入分桶，再清空并绑定 B）
        stash_current_bucket(&mut s);
        apply_relock(&mut s);
        assert!(!restore_or_bind_bucket(&mut s, &pb));
        assert!(s.history.is_empty(), "切到新会话 B 时不应带入 A 的历史");
        assert!(s.mine.is_empty(), "切到新会话 B 时不应带入 A 的口吻");
        s.locked_name = "会话B".into();
        s.history.push("对方: B的第一句".into());

        // 3. 从会话 B 切回会话 A：B 进桶，A 从桶中完整恢复
        stash_current_bucket(&mut s);
        apply_relock(&mut s);
        assert!(
            restore_or_bind_bucket(&mut s, &pa),
            "切回已见过的会话 A 应命中分桶"
        );
        assert_eq!(s.history, vec!["对方: A的第一句"]);
        assert_eq!(s.mine, vec!["我对A说的话"]);
        assert_eq!(s.seen.len(), 1);

        // 4. 验证 SessionModeCell 的读写往返
        let cell = SessionModeCell::default_lock();
        assert_eq!(cell.get(), SessionMode::Lock);
        assert_eq!(cell.set(SessionMode::Follow), SessionMode::Follow);
        assert_eq!(cell.get().as_str(), "follow");
    }
}
