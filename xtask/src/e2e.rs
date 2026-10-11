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

/// 每一段的编号与名字。**报错时按它定位。**〔按上下文复原，非原文〕
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
    /// 注入之后在**输入框像素里**读到的东西。给界面/报告看。
    pub input_box_text: String,
    /// 是否真的注入并回读过。**没注入就不能说「回读相等」**。
    pub injected_and_read_back: bool,
    pub elapsed: Duration,
}

/// 注入之后从**输入框那块像素**里读到的结果。
///
/// ## 为什么不能只回读剪贴板
///
/// 剪贴板是我们自己写进去的。拿它跟自己比是**恒等**的：不管 Ctrl+V 有没有
/// 真的落进输入框，只要没有别的程序抢走剪贴板，它就等于我们写的原文。
/// 那种「证据」长得像证据、却证明不了任何事——比没有证据更糟。
///
/// 所以这里**再抓一帧**，OCR 输入框所在的那条带子。OCR 会认错字，
/// 所以判据是「期望文本的字符在读到的文字里占多大比例」，
/// 而不是字节级相等。
#[derive(Debug, Clone, PartialEq)]
pub enum InputVerdict {
    /// 读到了，且和期望文本足够像。
    Confirmed { got: String, ratio: f32 },
    /// 抓帧或 OCR 失败。**不算「没投递成功」**——是没证据。
    NoEvidence { why: String },
    /// 读到了东西，但和期望对不上。
    Mismatch { got: String, ratio: f32 },
}

impl InputVerdict {
    pub fn confirms(&self) -> bool {
        matches!(self, Self::Confirmed { .. })
    }

    pub fn explain(&self) -> String {
        match self {
            Self::Confirmed { got, ratio } => {
                format!(
                    "输入框像素里读到「{got}」（与候选相似度 {:.0}%）",
                    ratio * 100.0
                )
            }
            Self::NoEvidence { why } => {
                format!("**拿不到证据**（不是「投递失败」）：{why}")
            }
            Self::Mismatch { got, ratio } => format!(
                "输入框里读到的是「{got}」，与候选相似度只有 {:.0}%。\n\
                 \x20     Ctrl+V 很可能没落进输入框。",
                ratio * 100.0
            ),
        }
    }
}

/// 再抓一帧，OCR 输入框那条带子，判断候选是不是真的出现在输入框里。
fn read_input_box(hwnd: isize, anchor: PaneAnchor, expect: &str) -> InputVerdict {
    use chatservant_core::frame::Rect;

    let frame = match capture_one_frame(hwnd) {
        Ok(f) => f,
        Err(e) => {
            return InputVerdict::NoEvidence {
                why: format!("注入后重新抓帧失败：{e}"),
            };
        }
    };
    // 输入框在消息区**下方**，从 `chat` 的下沿一直到窗口底部。
    let band = Rect::new(
        anchor.chat.x0,
        anchor.chat.y1,
        anchor.chat.x1,
        frame.height() as i32,
    );
    let strip = match frame.crop(band) {
        Some(s) => s,
        None => {
            return InputVerdict::NoEvidence {
                why: "按锚点裁不出输入框那条带子".into(),
            };
        }
    };
    // 放大后再 OCR：与管线同一个理由（见 ADR-0002 补记）——小字识别器不收。
    let Some(view) = strip.upscaled_nearest(chatservant_perceive::OCR_UPSCALE) else {
        return InputVerdict::NoEvidence {
            why: "输入框条带放大失败".into(),
        };
    };
    let ocr = match chatservant_capture::ocr::recognize(&view) {
        Ok(o) => o,
        Err(e) => {
            return InputVerdict::NoEvidence {
                why: format!("输入框条带 OCR 失败：{e}"),
            };
        }
    };

    // 工具栏和「发送」按钮也会被读到。只看**候选文本本身**像不像。
    verdict_for(&ocr.text, expect)
}

/// 拿 OCR 到的输入框文字与候选比对，出判定。
///
/// 抽成独立函数是为了让测试**跑真代码**——抄一份判定逻辑进测试，
/// 测的就不是线上那一段了。
fn verdict_for(got_raw: &str, expect: &str) -> InputVerdict {
    let got = squash(got_raw);
    let want = squash(expect);
    if want.is_empty() {
        return InputVerdict::NoEvidence {
            why: "候选文本去掉空白后是空的，没法比对".into(),
        };
    }
    let ratio = similarity(&want, &got);
    if ratio >= SIMILARITY_OK {
        InputVerdict::Confirmed { got, ratio }
    } else {
        InputVerdict::Mismatch { got, ratio }
    }
}

/// 相似度下限。OCR 认错字是常态，不是失败；判据必须容忍它。
const SIMILARITY_OK: f32 = 0.6;

/// 去掉所有空白与标点，只留可比对的字。
///
/// OCR 常在字里插空格（真实帧上「跑 了 S00 积 分 了」就是这么出来的），
/// 不去掉就没法比。标点同理：`is_ascii_punctuation` **不管全角标点**，
/// 所以按 `is_alphanumeric` 取补集——汉字属 Unicode 的 Alphabetic 属性，
/// 会被保留，而 `，。！？` 会被去掉。
fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 候选文本有多少比例**出现在**读到的文字里。取值 0.0 ~ 1.0。
///
/// ## 为什么用字符重合率，不用编辑距离
///
/// 候选通常一二十个字，编辑距离对「个别字认错」过于敏感——差一个字
/// 就把结果推到 0.8 以下，于是**注入成功反而报失败**。
/// 字符重合率对个别错字宽容，对「完全是另一句话」严格。
///
/// ## 方向是单向的：只问「候选有没有出现」
///
/// 不反向计算，否则输入框里混进工具栏和「发送」两个字就会被算成
/// 半个证据。中文二十个字里错一两个字，重合率仍在 0.9 以上；
/// 而输入框空的时候，比值接近 0——两种结局分得很开。
fn similarity(needle: &str, haystack: &str) -> f32 {
    if needle.is_empty() || haystack.is_empty() {
        return 0.0;
    }
    let h: Vec<char> = haystack.chars().collect();
    let hit = needle.chars().filter(|c| h.contains(c)).count();
    hit as f32 / needle.chars().count() as f32
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
            input_box_text: String::new(),
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
    // **剪贴板相等只证明「剪贴板还是我们写的那份」，不证明粘贴落进了输入框。**
    // 那份剪贴板是上面 `fill` 自己写的，拿它跟自己比是**恒等**的——
    // 它证明不了的东西却长得像证据。所以这里做两件事：
    //   ① 剪贴板回读：确认注入之后没有别的程序把它换掉；
    //   ② **再抓一帧 OCR 输入框那块**：这才是「字真的出现在输入框里」的证据。
    let got = back.read().map_err(|e| ChainError {
        stage: Stage::Readback,
        detail: e,
        passed: passed.clone(),
    })?;
    if got != injected {
        return Err(ChainError {
            stage: Stage::Readback,
            detail: format!(
                "注入后剪贴板被别的程序换掉了：注入 {} 字节，回读 {} 字节",
                injected.len(),
                got.len()
            ),
            passed: passed.clone(),
        });
    }

    let verdict = read_input_box(hit.hwnd_raw, anchor, &injected);
    let input_box_text = match &verdict {
        InputVerdict::Confirmed { got, .. } | InputVerdict::Mismatch { got, .. } => got.clone(),
        InputVerdict::NoEvidence { .. } => String::new(),
    };
    if !verdict.confirms() {
        return Err(ChainError {
            stage: Stage::Readback,
            detail: verdict.explain(),
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
        input_box_text,
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

#[cfg(test)]
mod tests {
    use super::*;

    const CANDIDATE: &str = "看起来你已经很努力了，加油，相信你会做得更好的！";

    /// OCR 会在字里插空格。真实帧上「跑 了 S00 积 分 了」就是这么出来的，
    /// 不去掉就没法比。
    #[test]
    fn squash_drops_whitespace_and_punctuation() {
        assert_eq!(squash("跑 了 S00 积 分 了，"), "跑了s00积分了");
        assert_eq!(squash("a b-c_d"), "abcd");
    }

    #[test]
    fn identical_text_scores_one() {
        assert!((similarity(&squash(CANDIDATE), &squash(CANDIDATE)) - 1.0).abs() < 1e-6);
    }

    /// 认错一两个字不该让注入成功被误判成失败——那会逼人去查一个没坏的东西。
    #[test]
    fn a_wrong_character_still_confirms() {
        let typo = "看起来你已经很努力了，加油，相信你会做得更好啊！";
        let r = similarity(&squash(CANDIDATE), &squash(typo));
        assert!(r >= SIMILARITY_OK, "错一个字就只有 {r:.2}，会误报失败");
    }

    /// ⚠ **这条是整段回读逻辑的底线**：空输入框不能被判成「投递成功」。
    ///
    /// 实测（2026-10-07）：微信输入框为空时，OCR 只读出 `0` 和「发送」。
    /// 若判据写成「回读剪贴板 == 注入内容」就**恒等**，空输入框照样通过——
    /// 那种证据证明不了任何事。
    #[test]
    fn an_empty_input_box_never_confirms() {
        let got = squash("0 发 送");
        let r = similarity(&squash(CANDIDATE), &got);
        assert!(
            !verdict_for(&got, CANDIDATE).confirms(),
            "空输入框居然判成投递成功了（相似度 {r:.2}）"
        );
    }

    /// 输入框里有别人的字时也不能判成功。
    #[test]
    fn unrelated_text_in_the_box_never_confirms() {
        let got = squash("明天记得开会");
        assert!(!verdict_for(&got, CANDIDATE).confirms());
    }

    #[test]
    fn a_matching_box_confirms() {
        assert!(verdict_for(&squash(CANDIDATE), CANDIDATE).confirms());
    }

    /// 候选全是空白时不许判成功——那会变成「随便什么都行」。
    #[test]
    fn a_blank_candidate_is_no_evidence_not_success() {
        assert!(!verdict_for(&squash(CANDIDATE), "  \n ").confirms());
    }
}
