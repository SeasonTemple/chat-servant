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
use chatservant_reason::{Reasoner, TurnContext};
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

/// 往 UI 推的一份完整快照。
#[derive(Debug, Clone, Serialize)]
pub struct Readout {
    pub anchor: Option<PaneAnchor>,
    pub messages: Vec<Message>,
    pub candidates: Vec<DraftCandidate>,
    /// 判断摘要 + 降级说明。
    pub reason: String,
    /// 这一轮调了几次模型。静默期必须是 0。
    pub model_calls: u32,
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
    /// 与设置页共用一个格子的条数。循环每圈读一次。
    recent: RecentN,
}

impl Scratch {
    fn new(recent: RecentN) -> Self {
        Self {
            candidates: Vec::new(),
            reason: String::from("等待第一条消息"),
            ocr_calls: 0,
            context: chatservant_reason::Context::new(),
            history: Vec::new(),
            recent,
        }
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
            let mut last_calls = 0u32;
            let mut ticks = 0u64;
            let mut anchor: Option<PaneAnchor> = None;

            while !stop_bg.load(Ordering::Relaxed) {
                ticks += 1;
                match step(&mut watcher, &mut reasoner, &mut s, &LAST_BOXES) {
                    Ok(Some(a)) => {
                        anchor = Some(a);
                        last_calls = reasoner.calls();
                        let out = Readout {
                            anchor,
                            messages: Vec::new(),
                            candidates: s.candidates.clone(),
                            reason: s.reason.clone(),
                            model_calls: last_calls,
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
                        let out = Readout {
                            anchor,
                            messages: Vec::new(),
                            candidates: s.candidates.clone(),
                            reason: "这一轮没跑通".into(),
                            model_calls: last_calls,
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
    let frame = capture_one_frame(hit.hwnd_raw).map_err(|e| format!("抓帧失败：{e}"))?;
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

    // 存一份给调试画布。**在整帧坐标系**——画布按帧的百分比定位，
    // 用 ROI 坐标会让框整体偏移。
    *last_boxes.lock().unwrap_or_else(|e| e.into_inner()) = Some(crate::debug::build(
        anchor.chat,
        frame.width(),
        frame.height(),
        &attrs,
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

        let ctx = TurnContext::text(
            rendered.lines().map(str::to_string).collect(),
            String::new(),
        );
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
    use super::RecentN;
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
}
