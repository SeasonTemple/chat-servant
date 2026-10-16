//! 三段式编排：**判断 → 带着判断起草 → 排序**。
//!
//! ## 三段为什么必须分开
//!
//! 因为它们**各自会挂，且降级路径完全不同**：
//!
//! | 挂的是 | 降级成 | 用户看到什么 |
//! |---|---|---|
//! | 判断 | **盲拟**（起草不带判断） | 「没读懂对话」+ 仍然有候选 |
//! | 起草 | **报错** | 没有候选，且明确说为什么 |
//! | 排序 | **按第一条** | 候选照常，判断摘要照常显示 |
//!
//! 合成一个 `generate()` 就没法单独测「某一段挂了会怎样」，
//! 也没法在判断降级时保住排序——那正是**盲拟**的用处。
//!
//! ## 静默期零调用
//!
//! [`Reasoner::call_count`] 是**唯一**的调用计数来源，由传输层自己累加。
//! 测试用预置传输断言「没有新消息 → 计数为 0」——
//! 不用 mock 任何东西（AGENTS.md 的规矩：测试里一处都不 mock）。

use crate::clean;
use crate::model::{Candidate, DraftSet, Intent, ReasonError, Reasoner, TurnContext};

/// 一轮推理的结果。**判断可能没有**（盲拟），候选一定有。
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    /// 判断结果。**判断失败时是 [`Intent::unknown`]**，`Intent::summary()` 会说「没判断」。
    pub intent: Intent,
    /// 候选集合。**已排序、已清洗**：这一份可以直接交给界面或输入框，
    /// 不用再做任何后处理——排序与清洗都在 [`turn`] 里做完了。
    pub draft: DraftSet,
    /// 这个 `Intent` 是真的判断出来的，还是降级来的。
    pub judged: bool,
}

impl Turn {
    /// 界面上显示的完整说明。**降级时必须说清楚**——
    /// 用户以为程序理解了对话，而它其实什么都没读到。
    pub fn explain(&self) -> String {
        if self.judged {
            format!(
                "{}｜{} 条候选，推荐概率 {:.0}%",
                self.intent.summary(),
                self.draft.candidates.len(),
                self.draft
                    .sorted()
                    .first()
                    .map(|c| c.probability)
                    .unwrap_or(0.0)
                    * 100.0
            )
        } else {
            format!(
                "判断服务没响应，盲拟了 {} 条（**没读懂对话**）",
                self.draft.candidates.len()
            )
        }
    }

    /// 推荐条：排序后第一条，也就是用户最可能选的那一条。
    ///
    /// 走 [`DraftSet::recommended`]——排序失败降级成按概率排时，
    /// 「第一条」的含义不变：它仍然是概率最高的那条，不是生成顺序的第一条。
    /// `None` 只在一条候选都没有时出现，而 [`turn`] 保证不会这样返回。
    pub fn recommended(&self) -> Option<&Candidate> {
        self.draft.sorted().first().copied()
    }

    /// 候选条数。
    pub fn candidates_count(&self) -> usize {
        self.draft.candidates.len()
    }
}

/// 跑一轮推理。**保证要么有候选，要么报明确错误。**
///
/// 永不返回空候选集：起草返回空会报 [`ReasonError::EmptyDraft`]，
/// 而不是让调用方在 `recommended()` 上炸掉。
///
/// ## 静默期零调用是**这个函数的职责**
///
/// `ctx.messages` 为空时直接返回 [`ReasonError::NothingToDo`]，
/// **一段模型都不调**。不放在这里放哪？放在调用方就等于「每个调用点
/// 都要记得判断」——迟早有人忘，约束就作废了，而且测试全绿。
///
/// 这条对应 AGENTS.md 硬约束第 6 条：没有新的对方消息时不调任何模型。
pub fn turn<R: Reasoner + ?Sized>(
    reasoner: &mut R,
    ctx: &TurnContext,
) -> Result<Turn, ReasonError> {
    // ── 零：静默期闸门 ──
    // 放在最前面：判断/起草/排序一律不跑。
    if ctx.messages.is_empty() {
        return Err(ReasonError::NothingToDo);
    }

    // ── 第一段：判断 ──
    // 判断失败**不算整轮失败**：降级成盲拟，起草照样要给出候选。
    let (intent, judged) = match reasoner.judge(ctx) {
        Ok(i) => (i, true),
        Err(e) => {
            tracing::warn!(error = %e, "判断失败，降级为盲拟");
            (Intent::unknown(), false)
        }
    };

    // ── 第二段：起草 ──
    // 起草挂了就没候选了，这里**必须**报出去而不是降级。
    // 「不给出候选」和「给出瞎编的候选」不是一回事。
    let mut draft = reasoner.draft(ctx, &intent)?;

    if draft.candidates.is_empty() {
        return Err(ReasonError::EmptyDraft);
    }

    // ── 第三段：排序 ──
    // 排序挂了**降级成起草给的顺序**：候选照常，判断摘要照常。
    // 排序只是「把最看好的放上面」，排序挂了不该让整个功能不可用。
    match reasoner.rank(ctx, &draft) {
        Ok(ranked) if !ranked.is_empty() => {
            draft.candidates = ranked;
        }
        Ok(_) | Err(_) => {
            tracing::warn!("排序失败，按起草的原始顺序");
            // 按概率兜底——起草给的顺序是模型生成顺序，
            // 概率字段通常已经在了，用它排比用生成顺序好。
            draft.candidates.sort_by(|a, b| {
                b.probability
                    .partial_cmp(&a.probability)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
    }

    // 清洗放在**最后**。
    //
    // 早先版本在起草后就清洗，结果排序阶段返回的新候选把清洗过的文本
    // 整个换掉——模型可能顺手改写，壳又回来了。清洗必须盖住**最终**
    // 那一份，否则「干净」只是看起来成立。
    for c in &mut draft.candidates {
        c.text = clean::clean(&c.text);
    }
    draft.candidates.retain(|c| !c.text.trim().is_empty());

    // 清洗后仍然要有东西
    if draft.candidates.is_empty() {
        return Err(ReasonError::EmptyDraft);
    }
    draft.blind = !judged;

    Ok(Turn {
        intent,
        draft,
        judged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// 预置传输：每段的返回值都能预设，也能让它挂。
    ///
    /// **不 mock**——它是真实实现了 `Reasoner` 的一个类型，
    /// 只是不发网络请求。ticket 明确要这个：「只换模型传输」。
    struct Fake {
        judge: Result<Intent, ReasonError>,
        draft: Result<DraftSet, ReasonError>,
        rank: Result<Vec<Candidate>, ReasonError>,
        calls: Cell<u32>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                judge: Ok(Intent {
                    intent: "约时间".into(),
                    need: "确认时间".into(),
                    action: "reply".into(),
                    tension: 0.3,
                }),
                draft: Ok(DraftSet {
                    candidates: vec![
                        Candidate {
                            text: "今晚八点可以".into(),
                            probability: 0.7,
                        },
                        Candidate {
                            text: "我看看时间".into(),
                            probability: 0.3,
                        },
                    ],
                    blind: false,
                    calls: 1,
                }),
                rank: Ok(vec![
                    Candidate {
                        text: "今晚八点可以".into(),
                        probability: 0.7,
                    },
                    Candidate {
                        text: "我看看时间".into(),
                        probability: 0.3,
                    },
                ]),
                calls: Cell::new(0),
            }
        }
        fn calls(&self) -> u32 {
            self.calls.get()
        }
    }

    impl Reasoner for Fake {
        fn judge(&mut self, _ctx: &TurnContext) -> Result<Intent, ReasonError> {
            self.calls.set(self.calls.get() + 1);
            self.judge.clone()
        }
        fn draft(&mut self, _ctx: &TurnContext, _i: &Intent) -> Result<DraftSet, ReasonError> {
            self.calls.set(self.calls.get() + 1);
            self.draft.clone()
        }
        fn rank(
            &mut self,
            _ctx: &TurnContext,
            _d: &DraftSet,
        ) -> Result<Vec<Candidate>, ReasonError> {
            self.calls.set(self.calls.get() + 1);
            self.rank.clone()
        }
    }

    fn ctx() -> TurnContext {
        TurnContext::text(vec!["在吗".into(), "今晚有空吗".into()], "小号".into())
    }

    /// 正常路径：三段都跑，推荐条概率最高。
    #[test]
    fn happy_path_runs_all_three_stages() {
        let mut f = Fake::new();
        let t = turn(&mut f, &ctx()).expect("推理成功");
        assert!(t.judged, "判断成功了，不该是盲拟");
        assert_eq!(f.calls(), 3, "判断+起草+排序 = 3 次");
        assert_eq!(t.recommended().unwrap().text, "今晚八点可以");
    }

    /// **判断挂 → 仍然有候选**，走盲拟路径。
    #[test]
    fn judge_failure_degrades_to_blind_but_still_yields_candidates() {
        let mut f = Fake::new();
        f.judge = Err(ReasonError::Judge("429 限流".into()));
        let t = turn(&mut f, &ctx()).expect("判断失败不该让整轮失败");
        assert!(!t.judged, "应是盲拟");
        assert!(t.draft.candidates.len() >= 2, "盲拟也要给候选");
        assert_eq!(f.calls(), 3, "判断失败后仍跑了起草+排序");
        // 界面上必须说清楚「没读懂」
        assert!(
            t.explain().contains("没读懂"),
            "提示语要说没读懂：{}",
            t.explain()
        );
        assert!(
            t.intent.summary().contains("没响应"),
            "判断摘要件说明没判断"
        );
    }

    /// **排序挂 → 按第一条，判断摘要照常显示。**
    #[test]
    fn rank_failure_falls_back_to_probability_but_keeps_intent() {
        let mut f = Fake::new();
        f.rank = Err(ReasonError::Rank("超时".into()));
        let t = turn(&mut f, &ctx()).expect("排序失败不该让整轮失败");
        assert!(t.judged, "判断摘要照常");
        assert_eq!(t.draft.candidates.len(), 2, "候选照常");
        // 降级成按概率排：仍是高概率在前
        assert_eq!(t.recommended().unwrap().probability, 0.7);
    }

    /// **起草空候选 → 明确报错**，不 panic、不返回空。
    #[test]
    fn empty_draft_is_a_clear_error_not_a_panic() {
        let mut f = Fake::new();
        f.draft = Ok(DraftSet {
            candidates: vec![],
            blind: false,
            calls: 1,
        });
        let err = turn(&mut f, &ctx()).expect_err("空候选必须报错");
        assert_eq!(err, ReasonError::EmptyDraft);
    }

    /// 起草挂 → 明确报错。这一段**没有**降级路径。
    #[test]
    fn draft_failure_is_reported_not_swallowed() {
        let mut f = Fake::new();
        f.draft = Err(ReasonError::Draft("上游 500".into()));
        let err = turn(&mut f, &ctx()).expect_err("起草失败必须报错");
        assert!(matches!(err, ReasonError::Draft(_)));
    }

    /// 候选清洗在推理层就做完——模型返回的壳不该流到界面或输入框。
    ///
    /// 这里**故意**让排序阶段返回一份**没洗过**的文本：
    /// 清洗若只做在起草之后，排序的返回值会把干净文本整个换掉。
    /// 这条测试锁的就是那个顺序。
    #[test]
    fn candidates_are_cleaned_after_ranking_not_before() {
        let mut f = Fake::new();
        let raw = "1. **me: 「今晚八点可以」**";
        f.draft = Ok(DraftSet {
            candidates: vec![Candidate {
                text: raw.into(),
                probability: 0.9,
            }],
            blind: false,
            calls: 1,
        });
        f.rank = Ok(vec![Candidate {
            text: raw.into(),
            probability: 0.9,
        }]);
        let t = turn(&mut f, &ctx()).expect("推理成功");
        assert_eq!(t.recommended().unwrap().text, "今晚八点可以");
    }

    /// 清洗把候选全洗成空 → **剔除**，而不是留下一堆空候选。
    #[test]
    fn candidates_cleaned_to_empty_are_dropped() {
        let mut f = Fake::new();
        f.draft = Ok(DraftSet {
            candidates: vec![
                Candidate {
                    text: "「」".into(),
                    probability: 0.9,
                },
                Candidate {
                    text: "好的".into(),
                    probability: 0.5,
                },
            ],
            blind: false,
            calls: 1,
        });
        f.rank = Ok(vec![]); // 空排序 → 走降级分支
        let t = turn(&mut f, &ctx()).expect("推理成功");
        assert!(
            t.draft.candidates.iter().all(|c| !c.text.trim().is_empty()),
            "不该留下空候选：{:?}",
            t.draft.candidates
        );
    }
}
