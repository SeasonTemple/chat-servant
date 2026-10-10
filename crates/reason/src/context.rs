//! 上下文裁剪与惰性摘要。
//!
//! ## 这张票的核心张力
//!
//! 摘要是**一次模型调用**。为了「让模型看得更全」而在每帧都摘要，
//! 「十分钟没人说话就是十分钟零 token」这条硬约束当场作废。
//!
//! 解法是**惰性**：摘要只在**触发事件**（对方来了新消息）发生时算一次，
//! 结果缓存下来，静默期**一次调用都没有**。
//!
//! ## 为什么不能硬截断
//!
//! 直接取最近 N 条，长对话会在第 N+1 条处**断掉**：模型看到的是
//! 「好的，我查一下时间」这样的半句，语境丢了。摘要把那截断处压成一段话，
//! 接上最近的消息，读起来仍是连续对话。
//!
//! 但摘要**不参与最近 N 条**——最近的那些**原样**送。
//! 摘要只负责「N 条之前发生过什么」。最要紧的上下文一个字节都不改。

use std::collections::HashMap;

/// 参考上下文条数（最近多少条**原样**送）。可在设置里调，改动即时生效。
pub const DEFAULT_RECENT: usize = 12;

/// 下界。**小于 4 条等于没给上下文**——模型只能看到碎片，
/// 那不是「更省」，是「更差」。
pub const MIN_RECENT: usize = 4;

/// 上界。往上调要多花钱，得让用户自己决定，但**得有个头**：
/// 无限大不是选项，是把预算开关交出去。
pub const MAX_RECENT: usize = 40;

/// 把用户给的条数夹进 `[MIN_RECENT, MAX_RECENT]`。
///
/// **这条函数是边界的唯一事实来源**——设置页的滑块上下限也从它派生。
/// 前端写死一份数字，迟早和这里对不上，而对不上的后果是
/// 「滑块能拖到 3，但后端按 4 跑」，用户看到的值和真实值永远差一格。
pub fn clamp_recent(n: usize) -> usize {
    n.clamp(MIN_RECENT, MAX_RECENT)
}

/// 超过多少条才开始摘要。低于这个数直接截，**不调用任何模型**。
///
/// 这条线的作用是让短对话（绝大多数）**完全零成本**。
pub const SUMMARY_THRESHOLD: usize = 24;

/// 摘要的目标长度。**要短**——摘要太长就变成另一段正文了。
pub const SUMMARY_MAX_CHARS: usize = 300;

/// 缺口累积到这么多条才重算一次摘要。
///
/// ## 为什么需要它
///
/// 早先的版本「缓存覆盖到 `keep_from` 就复用」，看起来对，实际是**每来一条
/// 新消息就重算一次**：新消息把截断点右移一格，缓存立刻「不够覆盖」。
/// 那等于「为了摘要而摘要」，静默期零调用这条硬约束当场作废——而且
/// 恰恰是**对方越频繁说话**，我们调用得越勤，方向完全反了。
///
/// 按块重算把频率钉死成**每 N 条消息一次**，与消息到达速度无关。
pub const RESUMMARIZE_CHUNK: usize = 20;

/// 一次裁剪的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trimmed {
    /// 最近 N 条，**原样**。
    pub recent: Vec<String>,
    /// 更早部分的摘要。**没触发摘要时是 `None`**。
    pub summary: Option<String>,
    /// 这次裁剪让出了多少条。
    pub dropped: usize,
    /// 这次裁剪**是否触发了摘要调用**。
    ///
    /// 单独暴露出来，是为了让「静默期零调用」能被直接断言，而不是靠推断。
    pub summarized_now: bool,
}

impl Trimmed {
    /// 拼给模型看的一段文本。
    ///
    /// 形状固定：`摘要\n\n最近消息…`。**摘要为空时就是最近消息**，
    /// 不留空行——空行会让模型以为中间漏了东西。
    pub fn render(&self) -> String {
        let mut s = String::new();
        if let Some(sum) = &self.summary {
            s.push_str("[之前的对话摘要]\n");
            s.push_str(sum);
            s.push_str("\n\n");
        }
        s.push_str("[最近的对话]\n");
        for line in &self.recent {
            s.push_str(line);
            s.push('\n');
        }
        s
    }

    /// 总条数（摘要算一条）。
    pub fn total_items(&self) -> usize {
        self.recent.len() + usize::from(self.summary.is_some())
    }
}

/// 摘要器。**这是本模块唯一会额外发起模型调用的地方。**
pub trait Summarizer {
    /// 把一串旧消息压成一段话。实现方负责调模型。
    fn summarize(&mut self, older: &[String]) -> Result<String, String>;
}

/// 一个会话的摘要状态。
#[derive(Default, Clone)]
struct SessionState {
    summary: Option<String>,
    /// 摘要覆盖到第几条了。**条数比内容可靠**——
    /// 内容会抖，条数不会。
    covered_upto: usize,
    /// 摘要调用次数。测试靠它断言「同一会话不重复摘要」。
    calls: u32,
}

/// 会话的上下文缓存。**一个实例对应一个会话空间。**
///
/// ⚠ 早先它是 `static Mutex<HashMap<...>>`。那有两个问题：
/// 1. **全局可变状态**——测试并行跑时互相串味（三个测试同时用 `"s"` 会打架）。
///    「测试必须串行才过」本身就是设计坏了的信号。
/// 2. 进程里没有第二个会话空间，清缓存只能全清。
///
/// 现在它是普通 struct，由**拥有会话状态的那一层**（观察循环）持有。
#[derive(Default)]
pub struct Context {
    sessions: HashMap<String, SessionState>,
}

impl Context {
    /// 建一个空的上下文缓存。**没有会话状态**——
    /// 摘要按 `session` 惰性建，第一次 [`Context::trim`] 到那个会话才出现。
    ///
    /// 等价于 [`Context::default`]：显式留一个构造函数，
    /// 是为了让「怎么建」在类型上只有一个答案，不给别处留直接
    /// `Context { sessions: … }` 的口子。
    pub fn new() -> Self {
        Self::default()
    }

    /// 该会话累计的摘要调用次数。测试用它断言「不重复摘要」。
    pub fn summary_calls(&self, session: &str) -> u32 {
        self.sessions.get(session).map_or(0, |s| s.calls)
    }

    /// 清掉某个会话的摘要。**换会话必须调**，
    /// 否则会带着上一个会话的摘要去见对端。
    pub fn forget(&mut self, session: &str) {
        self.sessions.remove(session);
    }

    fn summary_of(&self, session: &str) -> Option<String> {
        self.sessions.get(session)?.summary.clone()
    }

    fn covered(&self, session: &str) -> usize {
        self.sessions.get(session).map_or(0, |s| s.covered_upto)
    }

    /// 裁剪一段对话。摘要**只在这一条路径上**发生，且结果进缓存。
    ///
    /// `messages` 是**按时间正序**的完整历史（最新的在最后）。
    ///
    /// ## `recent_n` 会被两层夹
    ///
    /// ① 这里 `max(1)`：**引擎兜底**。传 0 不报错，但至少给一条——
    ///    什么都不给模型比给一条差。
    /// ② 设置页那层 `clamp_recent`（4..40）：**用户档位边界**。
    ///
    /// 这两层**故意不一样**，别"顺手统一"：① 是防止调用方写出
    /// 荒谬值，② 是产品上「最近多少条」这个旋钮的可动范围。
    /// 合成一个的话，引擎兜底会静默改写调用方的意图（0 变 4）。
    pub fn trim<S: Summarizer>(
        &mut self,
        session: &str,
        messages: &[String],
        recent_n: usize,
        summarizer: &mut S,
    ) -> Result<Trimmed, String> {
        let recent_n = recent_n.max(1);
        let total = messages.len();

        // ① 短对话：直接截，**一次模型调用都没有**。
        if total <= recent_n.max(SUMMARY_THRESHOLD) {
            let start = total.saturating_sub(recent_n);
            return Ok(Trimmed {
                recent: messages[start..].to_vec(),
                summary: None,
                dropped: start,
                summarized_now: false,
            });
        }

        let keep_from = total - recent_n;
        let older = &messages[..keep_from];

        // ② 缓存够新就复用，不调模型。
        //
        // 判据是**缺口**（keep_from 超出已覆盖多少），不是「正好覆盖」。
        // 按后者判会让每条新消息都触发一次重算——见 RESUMMARIZE_CHUNK。
        let gap = keep_from.saturating_sub(self.covered(session));
        if let Some(sum) = self.summary_of(session)
            && gap < RESUMMARIZE_CHUNK
        {
            return Ok(Trimmed {
                recent: messages[keep_from..].to_vec(),
                summary: Some(sum),
                dropped: keep_from,
                summarized_now: false,
            });
        }

        // ③ 真要摘要。**只在这一处**发生模型调用。
        let sum = clamp(&summarizer.summarize(older)?);

        let st = self.sessions.entry(session.to_string()).or_default();
        st.summary = Some(sum.clone());
        st.covered_upto = keep_from;
        st.calls += 1;

        Ok(Trimmed {
            recent: messages[keep_from..].to_vec(),
            summary: Some(sum),
            dropped: keep_from,
            summarized_now: true,
        })
    }
}

/// 摘要过长会变成另一段正文，反而挤掉真正的最近消息。
fn clamp(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() <= SUMMARY_MAX_CHARS {
        return t.to_string();
    }
    let cut: String = t.chars().take(SUMMARY_MAX_CHARS).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 记录调用次数的假摘要器。**不发网络请求。**
    struct Counter(u32);

    impl Summarizer for Counter {
        fn summarize(&mut self, older: &[String]) -> Result<String, String> {
            self.0 += 1;
            Ok(format!("（摘要：{} 条旧消息）", older.len()))
        }
    }

    fn msgs(n: usize) -> Vec<String> {
        (1..=n).map(|i| format!("第{i}条")).collect()
    }

    /// 短对话**零模型调用**。绝大多数对话都走这条路径。
    #[test]
    fn short_conversation_costs_nothing() {
        for n in [0usize, 1, 5, SUMMARY_THRESHOLD] {
            let mut c = Counter(0);
            let mut ctx = Context::new();
            let t = ctx
                .trim("s", &msgs(n), DEFAULT_RECENT, &mut c)
                .expect("裁剪");
            assert_eq!(c.0, 0, "{n} 条不该触发摘要");
            assert!(!t.summarized_now);
            assert!(t.summary.is_none());
            assert_eq!(t.recent.len(), n.min(DEFAULT_RECENT));
        }
    }

    /// 超预算时**触发一次**摘要。
    #[test]
    fn over_budget_triggers_exactly_one_summary() {
        let mut c = Counter(0);
        let mut ctx = Context::new();
        let t = ctx
            .trim("s", &msgs(60), DEFAULT_RECENT, &mut c)
            .expect("裁剪");
        assert_eq!(c.0, 1);
        assert!(t.summarized_now);
        assert!(t.summary.is_some());
        assert_eq!(t.recent.len(), DEFAULT_RECENT, "最近 N 条要原样");
        assert_eq!(t.dropped, 60 - DEFAULT_RECENT);
    }

    /// **摘要频率与消息到达速度无关**——这是本级的核心不变量。
    ///
    /// 早先版本按「缓存正好覆盖截断点才复用」判，结果**每来一条新消息
    /// 就重算一次**：新消息把截断点右移一格，缓存立刻不够覆盖。
    /// 那等于「为了摘要而摘要」——而且恰恰是对方越频繁说话、我们调用越勤，
    /// 方向完全反了。静默期零调用这条硬约束当场作废。
    ///
    /// 现在按**块**判定：缺口不到 20 条就复用，累积到一个整块才重算。
    #[test]
    fn summary_frequency_is_decoupled_from_message_arrival_rate() {
        let mut c = Counter(0);
        let mut ctx = Context::new();
        let h = msgs(60);
        ctx.trim("s", &h, DEFAULT_RECENT, &mut c).expect("首次");
        assert_eq!(c.0, 1);

        // 接下来 19 条新消息：**一次都不该再调**
        for i in 0..(RESUMMARIZE_CHUNK - 1) {
            let mut h2 = h.clone();
            for k in 0..=i {
                h2.push(format!("新消息{k}"));
            }
            let t = ctx.trim("s", &h2, DEFAULT_RECENT, &mut c).expect("推进");
            assert!(!t.summarized_now, "第 {} 次不该重算", i + 1);
        }
        assert_eq!(c.0, 1, "19 条新消息不该带来任何额外调用");

        // 缺口满一个整块 → 才重算
        let mut h2 = h.clone();
        for k in 0..RESUMMARIZE_CHUNK {
            h2.push(format!("新消息{k}"));
        }
        let t = ctx.trim("s", &h2, DEFAULT_RECENT, &mut c).expect("满块");
        assert!(t.summarized_now, "缺口满一个整块应重算");
        assert_eq!(c.0, 2, "只该多这一次");
    }

    /// 不同会话各自缓存，互不串味。
    #[test]
    fn sessions_do_not_share_summaries() {
        let mut c = Counter(0);
        let mut ctx = Context::new();
        let h = msgs(60);
        ctx.trim("a", &h, DEFAULT_RECENT, &mut c).expect("a");
        ctx.trim("b", &h, DEFAULT_RECENT, &mut c).expect("b");
        assert_eq!(c.0, 2, "两个会话要各算一次");
        assert_eq!(ctx.summary_calls("a"), 1);
        assert_eq!(ctx.summary_calls("b"), 1);
    }

    /// 摘要 + 最近消息拼接起来**读起来连贯**——这是本级的主要目的。
    ///
    /// 断言两件事：摘要在前、最近消息**原样**在后。
    /// 如果最近的那些被摘要改写过，模型看到的就是二手信息。
    #[test]
    fn summary_and_recent_read_as_one_continuation() {
        let mut c = Counter(0);
        let mut ctx = Context::new();
        let t = ctx.trim("s", &msgs(60), 3, &mut c).expect("裁剪");
        let text = t.render();

        let sum_at = text.find("[之前的对话摘要]").expect("摘要段");
        let recent_at = text.find("[最近的对话]").expect("最近段");
        assert!(sum_at < recent_at, "摘要必须在前面");

        for m in ["第58条", "第59条", "第60条"] {
            assert!(text.contains(m), "最近消息 {m} 必须原样保留");
        }
        assert!(!text.contains("第10条\n"), "旧消息不该原样出现");
        assert_eq!(t.total_items(), 4, "摘要算一条");
    }

    /// 没有摘要时**不留空行**——空行会让模型以为中间漏了东西。
    #[test]
    fn no_summary_means_no_leading_blank_line() {
        let mut c = Counter(0);
        let mut ctx = Context::new();
        let t = ctx.trim("s", &msgs(5), 10, &mut c).expect("裁剪");
        assert!(t.render().starts_with("[最近的对话]"));
    }

    /// 摘要过长会被截断——太长就变成另一段正文，反而挤掉最近消息。
    #[test]
    fn an_overlong_summary_is_clamped() {
        struct Long;
        impl Summarizer for Long {
            fn summarize(&mut self, _: &[String]) -> Result<String, String> {
                Ok("很长的摘要".repeat(500))
            }
        }
        let mut ctx = Context::new();
        let t = ctx
            .trim("s", &msgs(60), DEFAULT_RECENT, &mut Long)
            .expect("裁剪");
        let sum = t.summary.expect("有摘要");
        assert!(sum.chars().count() <= SUMMARY_MAX_CHARS + 1, "摘要被截断了");
        assert!(sum.ends_with('…'), "截断处要有标记");
    }

    /// 摘要失败要**报错**，不能静默退化成硬截断。
    ///
    /// 静默退化的话，用户会在毫无提示的情况下丢掉语境——
    /// 而「丢了语境」看起来和「没触发摘要」一模一样。
    #[test]
    fn a_failing_summarizer_is_reported_not_swallowed() {
        struct Boom;
        impl Summarizer for Boom {
            fn summarize(&mut self, _: &[String]) -> Result<String, String> {
                Err("上游 429".into())
            }
        }
        let mut ctx = Context::new();
        let r = ctx.trim("s", &msgs(60), DEFAULT_RECENT, &mut Boom);
        assert_eq!(r.unwrap_err(), "上游 429", "失败必须报出去");
    }

    /// `recent_n` 改了**立即生效**——不重启、不缓存旧值。
    #[test]
    fn recent_count_is_applied_immediately() {
        let mut c = Counter(0);
        let mut ctx = Context::new();
        let h = msgs(30);
        for n in [4usize, 8, 20] {
            let t = ctx.trim("s", &h, n, &mut c).expect("裁剪");
            assert_eq!(t.recent.len(), n, "N={n} 未立即生效");
        }
    }

    /// N 至少为 1 —— 设成 0 会让模型什么都看不到，那不是「更省」。
    #[test]
    fn zero_recent_is_clamped_to_one() {
        let mut c = Counter(0);
        let mut ctx = Context::new();
        let t = ctx.trim("s", &msgs(10), 0, &mut c).expect("裁剪");
        assert_eq!(t.recent.len(), 1);
    }

    /// 换会话必须清缓存——否则会带着上一个会话的摘要去见对端。
    #[test]
    fn forgetting_a_session_drops_its_summary() {
        let mut c = Counter(0);
        let mut ctx = Context::new();
        let h = msgs(60);
        ctx.trim("s", &h, DEFAULT_RECENT, &mut c).expect("首次");
        assert_eq!(ctx.summary_calls("s"), 1);
        ctx.forget("s");
        assert_eq!(ctx.summary_calls("s"), 0);
        ctx.trim("s", &h, DEFAULT_RECENT, &mut c).expect("清后重算");
        assert_eq!(c.0, 2, "清掉缓存后该重新摘要");
    }

    /// 边界夹取。**越界要夹，不要报错，也不要默默接受。**
    ///
    /// 默默接受 0 的话，用户会得到「什么都没设进去」的结果，
    /// 而界面上还显示着他拖到的那个数——看起来能跑，其实没生效。
    #[test]
    fn clamp_recent_holds_the_user_facing_bounds() {
        for (input, want) in [
            (0usize, MIN_RECENT),
            (1, MIN_RECENT),
            (MIN_RECENT - 1, MIN_RECENT),
            (MIN_RECENT, MIN_RECENT),
            (12, 12),
            (MAX_RECENT, MAX_RECENT),
            (MAX_RECENT + 1, MAX_RECENT),
            (usize::MAX, MAX_RECENT),
        ] {
            assert_eq!(clamp_recent(input), want, "输入 {input} 应夹成 {want}");
        }
    }

    /// 默认值**必须落在边界内**。否则一开应用就处于非法档。
    #[test]
    fn the_default_sits_inside_the_bounds() {
        assert!(
            (MIN_RECENT..=MAX_RECENT).contains(&DEFAULT_RECENT),
            "默认值 {DEFAULT_RECENT} 跑出 [{MIN_RECENT}, {MAX_RECENT}]"
        );
    }

    /// **两个 Context 实例互不干扰。**
    ///
    /// 这条锁的是「缓存不是全局状态」——早先它是 `static Mutex`，
    /// 测试并行跑时互相串味。现在它是普通 struct，谁持有谁负责。
    #[test]
    fn two_contexts_do_not_share_a_cache() {
        let mut c = Counter(0);
        let mut a = Context::new();
        let mut b = Context::new();
        let h = msgs(60);
        a.trim("s", &h, DEFAULT_RECENT, &mut c).expect("a");
        // b 没见过这个会话 → 必须自己再算一次
        b.trim("s", &h, DEFAULT_RECENT, &mut c).expect("b");
        assert_eq!(a.summary_calls("s"), 1);
        assert_eq!(b.summary_calls("s"), 1);
        assert_eq!(c.0, 2);
    }
}
