//! 端到端：微信窗口开着 → 抓帧 → 锚定 → OCR → 判断 → 起草 → 填入 → **回读验证**。
//!
//! ## 这条命令治什么病
//!
//! 分层测试治不了**集成性故障**：WGC 在这台机器上的实际行为、webview 窗口层的
//! 问题、上游模型 API 契约的变化。所以它只在开发机上真跑，需要微信窗口开着，
//! **不进 CI**。
//!
//! ## 失败时必须指出是哪一段
//!
//! 「失败了」三个字没有任何诊断价值。所以每一段都有名字，失败信息里带着
//! 走过的段数与原因——运维第一反应是「哪一步」，不是「日志里翻」。
//!
//! ## 不写盘
//!
//! 全程帧只在内存里。唯一的例外是 `probe frame` 那种显式命令，
//! 而这条命令**不碰它**。

use std::fmt;
use std::time::{Duration, Instant};

use chatservant_capture::Frame;
use chatservant_capture::anchor::PaneAnchor;
use chatservant_capture::ocr::OcrText;
use chatservant_capture::surface::WECHAT;
use chatservant_capture::wgc_alt::capture_one_frame;
use chatservant_capture::window::find_chat_window;
use chatservant_perceive::{Attribution, Message, NoiseClass, Speaker, classify_all, group};
use chatservant_reason::{Candidate, Intent, ReasonError, Reasoner, TurnContext, turn};

/// 每一段的编号与名字。**报���时按它定位。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Locate = 1,
    Capture = 2,
    Anchor = 3,
    Ocr = 4,
    Classify = 5,
    Reason = 6,
    Inject = 7,
    Readback = 8,
}

impl Stage {
    pub const ALL: [Stage; 8] = [
        Stage::Locate,
        Stage::Capture,
        Stage::Anchor,
        Stage::Ocr,
        Stage::Classify,
        Stage::Reason,
        Stage::Inject,
        Stage::Readback,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Stage::Locate => "找窗口",
            Stage::Capture => "抓帧",
            Stage::Anchor => "锚定消息区",
            Stage::Ocr => "OCR",
            Stage::Classify => "归类",
            Stage::Reason => "判断+起草",
            Stage::Inject => "注入",
            Stage::Readback => "回读验证",
        }
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}段·{}", *self as u32, self.name())
    }
}

/// 哪一段失败 + 为什么。**带已走过的段数**——运维第一反应是「哪一步」。
#[derive(Debug)]
pub struct ChainError {
    pub stage: Stage,
    pub detail: String,
    /// 已经成功走过的段，顺序。
    pub passed: Vec<Stage>,
}

impl fmt::Display for ChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "❌ 第{}段「{}」失败：{}\n   已通过：{}",
            self.stage as u32,
            self.stage.name(),
            self.detail,
            if self.passed.is_empty() {
                "（无，失败在第一步）".to_string()
            } else {
                self.passed
                    .iter()
                    .map(|s| s.name())
                    .collect::<Vec<_>>()
                    .join(" → ")
            }
        )
    }
}

impl std::error::Error for ChainError {}

/// 中途产物。调试时看得见的东西。
pub struct ChainOutput {
    pub anchor: PaneAnchor,
    pub messages: Vec<Message>,
    pub attributions: Vec<Attribution>,
    pub intent: Intent,
    pub candidates: Vec<Candidate>,
    pub judged: bool,
    /// 注入用掉的文本。
    pub injected: String,
    /// **真正走到过的段**。dry-run 停在起草，最后两段**不在里面**。
    pub passed: Vec<Stage>,
    /// 是否真的注入并回读过。**没注入就不能说「回读相等」**。
    pub injected_and_read_back: bool,
    pub elapsed: Duration,
}

/// 跑完整条链。**每一步都可能被指名道姓地失败。**
///
/// `inject` 由调用方注入——因为真填输入框要动目标窗口的焦点，
/// 那不是这一层该做的决定。测试传一个假实现，`e2e` 命令传真实现。
pub trait Injector {
    fn fill(&mut self, hwnd: isize, text: &str) -> Result<(), String>;
}

/// 回读。**必须与注入时用的文本字节级相等**，否则整个投递不算数。
pub trait Readback {
    fn read(&mut self) -> Result<String, String>;
}

/// 跑一条完整链。
pub fn run<R: Reasoner + ?Sized, I: Injector, B: Readback>(
    reasoner: &mut R,
    injector: &mut I,
    back: &mut B,
    inject: bool,
) -> Result<ChainOutput, ChainError> {
    let t0 = Instant::now();
    let mut passed: Vec<Stage> = Vec::new();

    // ── 1 找窗口 ──
    let hit = find_chat_window(&WECHAT).map_err(|e| ChainError {
        stage: Stage::Locate,
        detail: format!("{e}（微信开着的吗？换了进程名？）"),
        passed: passed.clone(),
    })?;
    passed.push(Stage::Locate);

    // ── 2 抓帧 ──
    let frame: Frame = capture_one_frame(hit.hwnd_raw).map_err(|e| ChainError {
        stage: Stage::Capture,
        detail: format!("{e}"),
        passed: passed.clone(),
    })?;
    // 帧非黑：WGC 用错路径会**稳定**返回全黑帧，
    // 把它当成「抓到了」会让后面每一步都在错误的输入上工作。
    if frame.is_black() {
        return Err(ChainError {
            stage: Stage::Capture,
            detail: "抓到的是全黑帧——GPU 合成窗口用错捕获路径的已知失败模式".into(),
            passed: passed.clone(),
        });
    }
    passed.push(Stage::Capture);

    // ── 3 锚定 ──
    let anchor = pane_anchor_of(&frame).map_err(|e| ChainError {
        stage: Stage::Anchor,
        detail: e,
        passed: passed.clone(),
    })?;
    // 矩形必须在窗口内，且高度过下限——太矮说明锚到了标题栏或输入框
    if anchor.chat.height() < 60 {
        return Err(ChainError {
            stage: Stage::Anchor,
            detail: format!(
                "消息区只有 {} px 高，锚歪了（锚到标题栏或输入框了）",
                anchor.chat.height()
            ),
            passed: passed.clone(),
        });
    }
    passed.push(Stage::Anchor);

    // ── 4 OCR ──
    // ⚠ **必须走 `read_lines`，不能自己裁一块再调 OCR。**
    // 这里原本复制了一份「裁剪 + OCR」，产品管线改了它不跟着改，
    // 于是探针测的还是已经不存在的旧路——诊断命令证明不了任何事。
    // 和 `probe frame` 默认路径指向坏实现是同一类病。
    let pane = frame.crop(anchor.chat).ok_or_else(|| ChainError {
        stage: Stage::Ocr,
        detail: "按锚点裁不出消息区".into(),
        passed: passed.clone(),
    })?;
    let ocr: OcrText =
        chatservant_perceive::read_lines(&frame, anchor.chat, &chatservant_perceive::SystemOcr)
            .map_err(|e| ChainError {
                stage: Stage::Ocr,
                detail: format!("{e}"),
                passed: passed.clone(),
            })?;
    passed.push(Stage::Ocr);

    // ── 5 归类 ──
    let palette =
        chatservant_perceive::Palette::learn(&frame, anchor.chat).ok_or_else(|| ChainError {
            stage: Stage::Classify,
            detail: "学不到配色（消息区太平，可能是空会话）".into(),
            passed: passed.clone(),
        })?;
    let attributions = classify_all(&ocr.lines, &pane, &palette);
    let messages = group(&attributions);
    // 至少要有一条对方说的话——一条都没有说明根本没读懂
    if !attributions
        .iter()
        .any(|a| a.speaker == Speaker::Them && a.noise == NoiseClass::Text)
    {
        return Err(ChainError {
            stage: Stage::Classify,
            detail: format!(
                "OCR 读到 {} 行，但没有任何一条被判成「对方正文」。\n\
                 \x20     注意：这**不能**直接判定为归类阈值失效——也可能是 OCR \
                 压根没读到对方那几条。\n\
                 \x20     先用 `cargo xtask ocr <帧.png> --scale {}` 确认那一行在不在 \
                 OCR 结果里，再谈阈值。",
                ocr.lines.len(),
                chatservant_perceive::OCR_UPSCALE
            ),
            passed: passed.clone(),
        });
    }
    passed.push(Stage::Classify);

    // ── 6 判断 + 起草 ──
    let ctx = TurnContext::text(
        messages.iter().map(|m| m.text.clone()).collect(),
        hit.title.clone(),
    );
    let turn = match turn(reasoner, &ctx) {
        Ok(t) => t,
        // 静默期不是失败——这一轮没新消息而已
        Err(ReasonError::NothingToDo) => {
            return Err(ChainError {
                stage: Stage::Reason,
                detail: "消息区里没有可送审的消息（可能被判定成噪声了）".into(),
                passed: passed.clone(),
            });
        }
        Err(e) => {
            return Err(ChainError {
                stage: Stage::Reason,
                detail: format!("{e}"),
                passed: passed.clone(),
            });
        }
    };
    let judged = turn.judged;
    let intent = turn.intent.clone();
    let candidates: Vec<Candidate> = turn.draft.candidates.clone();
    if candidates.is_empty() {
        return Err(ChainError {
            stage: Stage::Reason,
            detail: "起草返回了 0 条候选".into(),
            passed: passed.clone(),
        });
    }
    passed.push(Stage::Reason);

    let recommended = candidates
        .iter()
        .max_by(|a, b| {
            a.probability
                .partial_cmp(&b.probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .cloned()
        .expect("上面已断言非空");
    let injected = recommended.text.clone();

    if !inject {
        // 只跑到起草。`--dry-run` 用它：不碰用户的输入框。
        // ⚠ 此刻**没有注入、也没有回读**——报告里不许出现「回读相等」。
        return Ok(ChainOutput {
            anchor,
            messages,
            attributions,
            intent,
            candidates,
            judged,
            injected,
            passed,
            injected_and_read_back: false,
            elapsed: t0.elapsed(),
        });
    }

    // ── 7 注入 ──
    injector
        .fill(hit.hwnd_raw, &injected)
        .map_err(|e| ChainError {
            stage: Stage::Inject,
            detail: e,
            passed: passed.clone(),
        })?;
    passed.push(Stage::Inject);

    // ── 8 回读验证 ──
    // **字节级相等是唯一判据。** 返回成功而没有回读等于没有证据。
    let got = back.read().map_err(|e| ChainError {
        stage: Stage::Readback,
        detail: e,
        passed: passed.clone(),
    })?;
    if got != injected {
        return Err(ChainError {
            stage: Stage::Readback,
            detail: format!(
                "回读与注入内容不等：注入 {} 字节，回读 {} 字节",
                injected.len(),
                got.len()
            ),
            passed: passed.clone(),
        });
    }
    passed.push(Stage::Readback);

    Ok(ChainOutput {
        anchor,
        messages,
        attributions,
        intent,
        candidates,
        judged,
        injected,
        passed,
        injected_and_read_back: true,
        elapsed: t0.elapsed(),
    })
}

fn pane_anchor_of(frame: &Frame) -> Result<PaneAnchor, String> {
    chatservant_capture::anchor::derive(frame).map_err(|e| e.to_string())
}

/// 一条被判定为「对方正文」的行，测试用。
#[allow(dead_code)]
fn is_them_text(a: &Attribution) -> bool {
    a.speaker == Speaker::Them && a.noise == NoiseClass::Text
}

/// **明确标注的预置传输**。
///
/// 真实传输（HTTP + key）还没接。所以端到端命令跑出来的候选是**假的**——
/// 命令因此**必须失败**，不能让用户以为整条链通了。
/// 它存在的意义是：把「抓帧→锚定→OCR→归类」四段真跑一遍，
/// 那四段不依赖任何模型。
pub struct PresetReasoner {
    calls: u32,
}

impl PresetReasoner {
    pub fn new() -> Self {
        Self { calls: 0 }
    }
}

impl Default for PresetReasoner {
    fn default() -> Self {
        Self::new()
    }
}

impl Reasoner for PresetReasoner {
    fn judge(&mut self, ctx: &TurnContext) -> Result<Intent, ReasonError> {
        self.calls += 1;
        Ok(Intent {
            intent: format!(
                "回应「{}」（预置，非真实模型）",
                ctx.messages.last().unwrap_or(&String::new())
            ),
            need: "占位".into(),
            action: "preset".into(),
            tension: 0.0,
        })
    }

    fn draft(
        &mut self,
        _ctx: &TurnContext,
        intent: &Intent,
    ) -> Result<chatservant_reason::DraftSet, ReasonError> {
        self.calls += 1;
        Ok(chatservant_reason::DraftSet {
            candidates: vec![Candidate {
                text: format!("【预置候选，未接真实模型】{}", intent.intent),
                probability: 1.0,
            }],
            blind: intent.action == "blind",
            calls: self.calls,
        })
    }

    fn rank(
        &mut self,
        _ctx: &TurnContext,
        d: &chatservant_reason::DraftSet,
    ) -> Result<Vec<Candidate>, ReasonError> {
        self.calls += 1;
        Ok(d.candidates.clone())
    }
}

impl ChainOutput {
    /// 候选按胜出概率降序，供打印。
    pub fn draft_view(&self) -> Vec<(usize, &Candidate)> {
        let mut v: Vec<(usize, &Candidate)> = self.candidates.iter().enumerate().collect();
        v.sort_by(|a, b| {
            b.1.probability
                .partial_cmp(&a.1.probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        v
    }
}
