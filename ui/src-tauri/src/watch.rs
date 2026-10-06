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
            let mut last_calls = 0u32;
            let mut ticks = 0u64;
            let mut ocr_calls = 0u32;
            let mut candidates: Vec<DraftCandidate> = Vec::new();
            let mut reason = String::from("等待第一条消息");
            let mut anchor: Option<PaneAnchor> = None;

            while !stop_bg.load(Ordering::Relaxed) {
                ticks += 1;
                match step(
                    &mut watcher,
                    &mut reasoner,
                    &mut candidates,
                    &mut reason,
                    &mut ocr_calls,
                    &LAST_BOXES,
                ) {
                    Ok(Some(a)) => {
                        anchor = Some(a);
                        last_calls = reasoner.calls();
                        let out = Readout {
                            anchor,
                            messages: Vec::new(),
                            candidates: candidates.clone(),
                            reason: reason.clone(),
                            model_calls: last_calls,
                            ticks,
                            ocr_calls,
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
                            candidates: candidates.clone(),
                            reason: "这一轮没跑通".into(),
                            model_calls: last_calls,
                            ticks,
                            ocr_calls,
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
    candidates: &mut Vec<DraftCandidate>,
    reason_out: &mut String,
    ocr_calls: &mut u32,
    // `Mutex` 自带内部可变性，**共享引用就够了**——要 `&mut` 反而
    // 得给静态量绑一层 `mut` 局部名，白绕一道。
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
    *ocr_calls += 1;

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
        let ctx = TurnContext {
            messages: messages.iter().map(|m| m.text.clone()).collect(),
            // 对端名要 OCR 头部才知道，ticket 11 接。现在留空——
            // 空名只影响模型的话术，不影响链路。
            peer: String::new(),
        };
        match chatservant_reason::turn(reasoner.as_mut(), &ctx) {
            Ok(t) => {
                *candidates = t
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
                *reason_out = t.explain();
            }
            // `NothingToDo` = 静默期，**不是错误**
            Err(chatservant_reason::ReasonError::NothingToDo) => {
                *reason_out = "没有新消息".into();
            }
            Err(e) => {
                // 推理失败**不推进水位线**——下一帧重试。
                // 漏消息比多调一次外部 API 糟得多。
                *reason_out = format!("推理失败：{e}");
            }
        }
    } else {
        candidates.clear();
        *reason_out = "这一段里没有对方说的话".into();
    }

    // 没读到对方的话时**不推进**——下一帧会重试同一段。
    if let Some(y) = newest {
        watcher.commit(y);
    }
    Ok(Some(anchor))
}

/// 把行框平移到整帧坐标系。
fn shift(r: chatservant_core::frame::Rect, dx: i32, dy: i32) -> chatservant_core::frame::Rect {
    use chatservant_core::frame::Rect;
    Rect::new(r.x0 + dx, r.y0 + dy, r.x1 + dx, r.y1 + dy)
}
