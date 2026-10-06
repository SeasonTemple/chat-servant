//! 推理层的数据类型。**纯数据 + 校验，不含任何网络代码。**
//!
//! 这一层是**唯一**允许出网做模型调用的阶段（见 CONTEXT.md 的系统边界）。
//! 但「唯一允许」不等于「必须出网」——所以传输是 trait，测试用假实现，
//! 于是整个推理接缝的测试**不联网、不花钱、可确定性重跑**。

use serde::{Deserialize, Serialize};

/// 决策模型的判断结果。**它是一段小抄，不是最终回复。**
///
/// 四个字段都是**面向人可读**的，不是黑盒标签：
/// 界面上要直接显示 `need` 与 `action`，用户才能判断「它是不是听懂了我」。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Intent {
    /// 对方想干什么。一句话，不超过 20 字。
    pub intent: String,
    /// 对方需要什么。可能是「只是分享，不需要回复」。
    pub need: String,
    /// **建议动作**：`reply` / `wait` / `react`。不是给模型看���内部标签，
    /// 是给用户看的一句话建议。
    pub action: String,
    /// 紧张度 0..=1。对方的语气压力。
    pub tension: f32,
}

impl Intent {
    /// 判断失败时的**降级判断**：不假装判断过。
    ///
    /// 这一条很重要——降级路径若返回「看起来很正常」的假判断，
    /// 用户会以为程序理解了对话，而它其实什么都没读到。
    /// `known: false` 让界面能明确显示「这次没判断，只盲拟」。
    pub fn unknown() -> Self {
        Self {
            intent: String::new(),
            need: String::new(),
            action: "blind".into(),
            tension: 0.0,
        }
    }

    /// 判断摘要，供界面显示。**空的时候要说「没判断」，不能显示空白。**
    pub fn summary(&self) -> String {
        if self.action == "blind" {
            return "判断服务没响应，这次是盲拟（没读懂对话）".into();
        }
        format!(
            "意图：{}｜需要：{}｜建议：{}｜紧张度 {:.2}",
            self.intent, self.need, self.action, self.tension
        )
    }
}

/// 一条候选回复。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub text: String,
    /// 胜出概率 0..=1。界面按它排序，**不按生成顺序**。
    pub probability: f32,
}

/// 一组候选。**至少一条**——空集是错误，不是「没有结果」。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DraftSet {
    pub candidates: Vec<Candidate>,
    /// 判断失败时为 `true`。界面据此显示降级提示。
    pub blind: bool,
    /// 生成这条 DraftSet 时模型调用了几次。测试用它断言「静默期零调用」。
    pub calls: u32,
}

impl DraftSet {
    /// 按胜出概率降序，**推荐条在第一条**。
    ///
    /// 稳定排序：概率相同时保持原顺序——不然每次渲染推荐条都可能换人，
    /// 用户按下去才发现不是刚才看的那条。
    pub fn sorted(&self) -> Vec<&Candidate> {
        let mut v: Vec<&Candidate> = self.candidates.iter().collect();
        v.sort_by(|a, b| {
            b.probability
                .partial_cmp(&a.probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        v
    }

    pub fn recommended(&self) -> Option<&Candidate> {
        self.sorted().first().copied()
    }
}

/// 三段式的每一段。**分开是为了能分别降级。**
///
/// 排序失败 ≠ 判断失败 ≠ 起草失败：它们的降级路径完全不同，
/// 混成一个 `generate()` 就没法单独测「某一段挂了会怎样」。
///
/// ⚠ 要能装进 `Box<dyn Reasoner>`（换真实传输时用得上），
/// 所以 trait 必须是**对象安全**的：方法只收 `&mut self` 与具体类型，
/// 不带泛型参数、不返回 `Self`。
pub trait Reasoner {
    /// 第一段：判断意图。
    fn judge(&mut self, ctx: &TurnContext) -> Result<Intent, ReasonError>;

    /// 第二段：带着判断起草候选。
    fn draft(&mut self, ctx: &TurnContext, intent: &Intent) -> Result<DraftSet, ReasonError>;

    /// 第三段：给候选排序。**返回的顺序即最终顺序。**
    fn rank(&mut self, ctx: &TurnContext, draft: &DraftSet) -> Result<Vec<Candidate>, ReasonError>;
}

/// 一轮对话的输入。**上下文裁剪的结果**（ticket 11 负责裁，这里只收）。
#[derive(Debug, Clone, PartialEq)]
pub struct TurnContext {
    /// 参与本轮的消息，已裁剪。
    pub messages: Vec<String>,
    /// 对端名，可能为空（未识别出会话）。
    pub peer: String,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ReasonError {
    #[error("判断失败：{0}")]
    Judge(String),

    #[error("起草失败：{0}")]
    Draft(String),

    #[error("排序失败：{0}")]
    Rank(String),

    /// **空候选是硬错误。** 上层取 `recommended()` 时会 panic，
    /// 与其让 panic 出现在取第一条的地方，不如在这里就说清楚。
    #[error("起草返回了空候选——无法推荐任何一条")]
    EmptyDraft,

    /// **静默期：没有新消息，一段模型都不该调。**
    ///
    /// 这不是失败——它是最常见的情况（绝大多数时间窗口都不动）。
    /// 调用方要**显式**处理它，别让它冒泡成界面上的错误：
    ///
    /// ```ignore
    /// match reason::turn(&mut r, &ctx) {
    ///     Ok(t) => emit(t),
    ///     Err(ReasonError::NothingToDo) => {} // 静默期，什么都不做
    ///     Err(e) => show_error(e),
    /// }
    /// ```
    #[error("静默期：没有新消息，不该调用模型")]
    NothingToDo,

    #[error("候选清洗后变成空串：原文 {0:?}")]
    CleanedToEmpty(String),
}

/// 给 `Box<dyn Reasoner>` 用的空实现之一：什么都不做的 `turn` 不用它，
/// 但 `Box<dyn Reasoner>` 需要 trait 是对象安全的，这里做编译期断言。
const _: () = {
    fn assert_object_safe<T: ?Sized + Reasoner>() {}
    fn check() {
        assert_object_safe::<dyn Reasoner>();
    }
    let _ = check;
};
