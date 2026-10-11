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
    /// **建议动作**：`reply` / `wait` / `react`。不是给模型看的内部标签，
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
    /// 回复正文。**到这里应当已经是干净的**——
    /// 推理层在 [`crate::seam::turn`] 的最后一步统一清洗，模型爱加的
    /// `1. ` 编号、`me:` 前缀、加粗壳都在那一步剥掉。
    /// 脏文本不该从任何路径流到输入框或界面上。
    pub text: String,
    /// 胜出概率 0..=1。界面按它排序，**不按生成顺序**。
    pub probability: f32,
}

/// 一组候选。**至少一条**——空集是错误，不是「没有结果」。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DraftSet {
    /// 候选正文。顺序是**起草给出的原始顺序**，不是展示顺序——
    /// 展示一律走 [`DraftSet::sorted`]，按概率排。
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

    /// 推荐条：概率最高的那一条。
    ///
    /// 返回 `None` 只意味着**一条候选都没有**——正常推理路径上
    /// [`crate::seam::turn`] 已经把空集拦成 [`ReasonError::EmptyDraft`]，
    /// 所以拿到 `None` 说明这个 `DraftSet` 是调用方自己拼的，它得自己兜住。
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

    /// 累计发出的模型请求数。
    ///
    /// **默认 0**：预置传输不计数，它本来就不发请求。
    /// 有真实网络的传输要覆写——「静默期零调用」那条硬约束要靠它断言。
    fn calls(&self) -> u32 {
        0
    }

    /// **分段的**调用次数。界面上要显示「判断 2 次 / 起草 1 次 / 视觉 0 次」。
    ///
    /// 只有一个总数是不够的：三段里任何一段挂掉都会让总数变化，
    /// 分不开就看不出是哪一段出的问题。**默认全 0**，预置传输不发请求。
    fn stage_calls(&self) -> StageCalls {
        StageCalls::default()
    }
}

/// 三段式各自的调用次数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub struct StageCalls {
    /// 判断这一段发出的请求数。
    ///
    /// 预置传输不发请求，这个数恒为 0——**「判断 0 次」是正常状态**，
    /// 别拿它反推判断功能坏了。
    pub judge: u32,
    /// 起草这一段发出的请求数。
    ///
    /// 随文带图的轮次，图也走**这一次**请求，不另计——
    /// 额外的视觉调用挂在 [`crate::vision::VisionBridge`] 上，默认关闭时恒为 0。
    pub draft: u32,
    /// 排序这一段发出的请求数。
    ///
    /// 判断失败降级成盲拟时它**照发**：盲拟出来的候选同样要排，
    /// 否则用户看到的顺序就是模型的生成顺序。
    pub rank: u32,
}

impl StageCalls {
    /// 界面显示的一行字。**三段都报，包括 0**——
    /// 「视觉 0 次」是默认配置下的正常状态，不报出来用户会以为没统计。
    pub fn explain(&self) -> String {
        format!(
            "判断 {} 次 · 起草 {} 次 · 排序 {} 次",
            self.judge, self.draft, self.rank
        )
    }
}

/// 一轮对话的输入。**上下文裁剪的结果**（ticket 11 负责裁，这里只收）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TurnContext {
    /// 参与本轮的消息，已裁剪。
    pub messages: Vec<String>,
    /// 对端名，可能为空（未识别出会话）。
    pub peer: String,
    /// 要随文本一起送模型的图。**已经缩放并编码好**。
    ///
    /// ## 默认必须是空的
    ///
    /// 绝大多数轮次对方发的是文字，这时候这里是空的，一条视觉请求都不该发。
    /// 「默认多模态配置下永远是 2 + 1 + 0」那条断言就是对这个字段的。
    pub images: Vec<crate::vision::ImagePayload>,
    /// 我自己的说话风格样本。**单独一路**，见 [`crate::voice`]。
    ///
    /// ## 为什么不并进 `messages`
    ///
    /// `messages` 里的每行渲染出来时**不带说话人标签**，模型无从区分
    /// 「这是上下文」和「这是风格参考」。混在一起它就会当成对方说过的话，
    /// 或者当成待回复的内容。
    ///
    /// ## 默认必须是空的
    ///
    /// 和 `images` 同理：短对话里用户可能一句自己的话都没说过，
    /// 那时候一节空的「我的风格」标题比没有更糟——模型会去凭空编一个。
    pub voice: Vec<String>,
}

impl TurnContext {
    /// 纯文本上下文。**绝大多数轮次都走这个构造。**
    pub fn text(messages: Vec<String>, peer: String) -> Self {
        Self {
            messages,
            peer,
            images: Vec::new(),
            voice: Vec::new(),
        }
    }

    /// 带口吻样本的上下文。**由观察循环在真读到「我」说过的话时**用。
    pub fn with_voice(mut self, voice: Vec<String>) -> Self {
        self.voice = voice;
        self
    }
}

/// 三段式报出来的失败。**按段分开是有意的**：调用方要靠这一层决定
/// 怎么降级、要不要重试，合成一个 `Other(String)` 就只剩「重试一下试试」。
///
/// 三段的风险等级本来就不同：判断挂了可以退化成盲拟再来，
/// 起草挂了重试也只是再花一次钱、拿不到东西，排序挂了压根不必重试。
/// 携带的文本来自传输层，**已过 [`crate::transport::redact`]**，不含 API key。
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ReasonError {
    /// **第一段判断失败**：请求发不出去、超时、限流，或模型没回合法 JSON。
    ///
    /// 这一段**有降级路径**：[`crate::seam::turn`] 会把它降级成
    /// [`Intent::unknown`] 继续起草，界面上显示「没读懂对话」并标 `judged = false`。
    /// 所以这一条**不该冒泡成错误提示**——它出现时用户仍然拿得到候选。
    #[error("判断失败：{0}")]
    Judge(String),

    /// **第二段起草失败**：候选正文的唯一来源，这一段挂了就没有候选了。
    ///
    /// **没有任何降级路径**——拿不出正文时给一条瞎编的，比明说「没写出来」糟得多。
    /// [`crate::seam::turn`] 直接往上抛，界面要显示错误并停在原地。
    /// 可重试，但每次重试都要重新付一次起草的钱。
    #[error("起草失败：{0}")]
    Draft(String),

    /// **第三段排序失败**：请求失败，或响应里根本没有 `order` 数组。
    ///
    /// 这一段**有降级路径**：[`crate::seam::turn`] 保留起草给的候选，
    /// 按起草时附带的 `probability` 排。所以它出现时**候选照常可用**，
    /// 调用方不必把它当成失败处理。
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

    /// 候选清洗后变成了空串——原文只剩壳（引号、编号、加粗标记之类）。
    ///
    /// 携带原文（用 `:?` 转义）是给**指认具体是哪一条**用的：
    /// 只知道「有候选洗空了」没法排查，而清洗失败往往就是模型
    /// 照抄了某种格式壳。
    ///
    /// 当前 [`crate::seam::turn`] 在这个情形上报的是 [`ReasonError::EmptyDraft`]
    /// （并把洗空的候选直接剔除）——它不需要指认是哪一条；
    /// 这个变体留给需要知道是哪条的调用方。
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
