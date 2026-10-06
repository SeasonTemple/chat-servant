//! 守卫用的预置传输。**不发任何网络请求**——ticket 明确要这个：
//! 「推理接缝只换模型传输（假实现返回预置 JSON）」。
//!
//! 它是一个**真实实现 `Reasoner` 的类型**，不是 mock：seam 对它没有
//! 任何特殊认知，换成真的 HTTP 客户端时 seam 一行都不用改。

use std::cell::Cell;

use crate::{Candidate, DraftSet, Intent, ReasonError, Reasoner, TurnContext};

/// 一段确定性的候选集。**测试不测「回复好不好」**——那是模型的事。
pub struct GateReasoner {
    pub calls: Cell<u32>,
    judge_fails: bool,
}

impl GateReasoner {
    pub fn new() -> Self {
        Self {
            calls: Cell::new(0),
            judge_fails: false,
        }
    }

    /// 累计调用次数。测试用它断言「静默期零调用」。
    ///
    /// 用 `Cell` 而不是裸字段：传输可能被 `Box<dyn Reasoner>` 拿走，
    /// 计数必须在里面自己维护，读的时候走方法。
    pub fn calls(&self) -> u32 {
        self.calls.get()
    }

    /// 判断返回 429——模拟上游限流。
    pub fn judge_fails() -> Self {
        Self {
            calls: Cell::new(0),
            judge_fails: true,
        }
    }

    fn tick(&self) {
        self.calls.set(self.calls.get() + 1);
    }
}

impl Default for GateReasoner {
    fn default() -> Self {
        Self::new()
    }
}

impl Reasoner for GateReasoner {
    fn judge(&mut self, ctx: &TurnContext) -> Result<Intent, ReasonError> {
        self.tick();
        if self.judge_fails {
            return Err(ReasonError::Judge("429 限流".into()));
        }
        // 判断内容跟着输入走，这样测试能看出「判断确实读了对话」。
        let last = ctx.messages.last().map(String::as_str).unwrap_or("");
        Ok(Intent {
            intent: format!("回应「{last}」"),
            need: "确认意图".into(),
            action: "reply".into(),
            tension: 0.2,
        })
    }

    fn draft(&mut self, _ctx: &TurnContext, intent: &Intent) -> Result<DraftSet, ReasonError> {
        self.tick();
        Ok(DraftSet {
            candidates: vec![
                Candidate {
                    // 故意带壳：清洗是 seam 的职责，传输层不该替它做
                    text: "1. **me: 「行，我看看」**".into(),
                    probability: 0.8,
                },
                Candidate {
                    text: "我查一下时间".into(),
                    probability: 0.4,
                },
            ],
            // 判断失败时起草仍然照常跑——这就是「盲拟」
            blind: intent.action == "blind",
            calls: self.calls.get(),
        })
    }

    fn rank(&mut self, _ctx: &TurnContext, _d: &DraftSet) -> Result<Vec<Candidate>, ReasonError> {
        self.tick();
        Ok(vec![
            Candidate {
                text: "1. **me: 「行，我看看」**".into(),
                probability: 0.8,
            },
            Candidate {
                text: "我查一下时间".into(),
                probability: 0.4,
            },
        ])
    }
}

/// 装一个带计数的传输，返回传输本身。
pub fn counted(r: GateReasoner) -> GateReasoner {
    r
}
