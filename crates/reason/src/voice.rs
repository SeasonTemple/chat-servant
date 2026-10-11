//! 我自己的说话风格样本 —— **单独一路**，不混进对话上下文。
//!
//! `FEATURE.md` 写着「自己的口吻样本单独一路：最近若干条短消息原样喂给起草」，
//! 对应 user story 26：「回得像我自己比回得『完美』更重要」。
//!
//! ## 为什么必须**单独**一路
//!
//! 用途不一样，所以不能并进 `messages`：
//!
//! | | 对话上下文 | 口吻样本 |
//! |---|---|---|
//! | 回答的问题 | 现在该回什么 | 我平时怎么说话 |
//! | 时效 | 只在最近 N 条内有效 | 跨整个会话都有效 |
//! | 混进去的后果 | —— | 模型会当成「对方说过的话」，或当成待回复的内容 |
//!
//! 混在一起那句「模型会当成对方说过的话」不是危言耸听：`messages` 里的每一行
//! 在渲染出来时都不带说话人标签（见 [`crate::context::Trimmed::render`]），
//! 所以模型根本无从区分「这是上下文」和「这是风格参考」。给它单独一节、
//! 单独一个标题，它才知道该照着学而不是接着回。
//!
//! ## 为什么只取短的
//!
//! 长回复（几百字）反映的是**当时在谈什么**，不是**怎么说话**。喂长段有两个
//! 坏处：一是白占预算，二是模型会顺手模仿那个长度——而真实聊天里用户发的
//! 消息绝大多数是几个字。
//!
//! ## 为什么不送链接
//!
//! 分享一个链接是**内容**，不是语气。模型从 URL 里学不到任何口吻，
//! 只会学到一件坏事：回复里可以带链接。
//!
//! ## 本模块是纯函数
//!
//! 不发网络请求、不读文件、不碰全局状态。所以「静默期零调用」这条硬约束
//! 在这里天然成立——它压根没有能力发起调用。

use chatservant_perceive::Speaker;

/// 最多取几条样本。
///
/// 8 条是权衡出来的：少于 5 条模型学不到稳定特征（更像在背原句），
/// 多于 12 条这些样本本身就要占掉可观的 token，而它们**每轮都重发**。
pub const VOICE_MAX_SAMPLES: usize = 8;

/// 单条样本的字数上限。**超过这个长度的不是口吻样本。**
///
/// 40 字：微信里「随手一句」的长度上限。实测口吻特征（语气词、句长、
/// 标点习惯、称呼）在这个尺度上已经很清楚了，再长只增加内容噪声。
pub const VOICE_MAX_CHARS: usize = 40;

/// 样本少于这个字数就不算样本——一个「嗯」说明不了任何口吻。
pub const VOICE_MIN_CHARS: usize = 2;

/// 把一整段消息流收成「我这一侧」的文本，**按时间正序**。
///
/// 调用方（观察循环）持有的是 [`chatservant_perceive::Message`]，
/// 而这里要的是 `&[String]`——因为**筛选策略全在本模块**，
/// 调用方不该只挑一半再把另一半的活儿推回来。
///
/// 「挑出我这一侧」本身留在调用方：那是感知层的事实，
/// 而「哪些算口吻样本」是本模块的判断。混在一起两边都会长出半个对方。
pub fn my_side(messages: &[chatservant_perceive::Message]) -> Vec<String> {
    messages
        .iter()
        .filter(|m| m.speaker == Speaker::Me)
        .map(|m| m.text.trim().to_string())
        .collect()
}

/// 挑出「我自己的说话风格样本」，**最近的排在最后**。
///
/// `mine` 是**按时间正序**的「我这一侧」全文（通常是 [`my_side`] 的产物，
/// 跨轮次累积），和 [`crate::context::Context::trim`] 收的那个列表同一顺序约定。
///
/// 全部筛选策略都在这里，调用方只负责提供原文——这样「什么样的算样本」
/// 只有一个答案，不会随调用点变化。
pub fn voice_samples(mine: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    // 从新往旧扫，凑满就停。
    //
    // 倒着扫不是为了性能，是为了**语义**：最近的话最代表用户现在的心情，
    // 拿旧的凑数会在用户换了说话方式之后继续教旧风格。
    for raw in mine.iter().rev() {
        if out.len() == VOICE_MAX_SAMPLES {
            break;
        }
        let text = raw.trim();
        let n = text.chars().count();
        if !(VOICE_MIN_CHARS..=VOICE_MAX_CHARS).contains(&n) {
            continue;
        }
        if is_linky(text) {
            continue;
        }
        // 同一句话原样发两次（很常见：「好的」「好」）**只算一个样本**。
        //
        // 不去重的话，「好的」连发四条会占掉四个位子，
        // 模型看到四行一模一样的文字，以为自己就是这么说话的。
        if out.iter().any(|s| s == text) {
            continue;
        }
        out.push(text.to_string());
    }

    out.reverse(); // 回到「最新在最后」，和 mine 同一顺序约定
    out
}

/// 渲染成送模型的那一段。
///
/// **没有样本时返回空串**——调用方不该往提示里塞一个空标题，
/// 那会让模型去找一个不存在的「风格」并自己编。
pub fn render(samples: &[String]) -> String {
    if samples.is_empty() {
        return String::new();
    }
    let mut s = String::from("[我自己平时的说话风格 · 照着这个语气写]\n");
    for line in samples {
        s.push_str(line);
        s.push('\n');
    }
    s
}

/// 是不是「分享链接」而不是「说话」。
///
/// 判据故意只认**确定是**的形式（`http(s)://`、`www.`、裸域名）。
/// 宁可漏掉一条样本，也不要把普通的话判成链接踢掉——
///
/// 漏掉的代价是模型少一条风格参考；错踢的代价是**用户正常说话被系统
/// 当成噪声丢掉**，而那在界面上完全看不出来。
fn is_linky(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    if lower.contains("http://") || lower.contains("https://") || lower.contains("www.") {
        return true;
    }
    // 裸域名可以出现在句子里任何位置——「看看 example.com 这个」也是分享链接。
    s.split_whitespace().any(is_bare_domain)
}

/// 一个词本身长得像域名吗。
///
/// 三条排除，缺一不可：
/// - 首尾的点号：「行吧.」「这样也行.」是**句末标点**，不是域名。
///   判错方向正好是最坏的那种——把用户正常说话当噪声丢掉。
/// - 末段全是数字：「1.5」「2.0」是数值。
/// - 末段没有 ASCII 字母：「第2.0版」末段是「0版」，数字加汉字，不是域名。
fn is_bare_domain(tok: &str) -> bool {
    if !tok.contains('.') || tok.starts_with('.') || tok.ends_with('.') {
        return false;
    }
    let last = tok.rsplit('.').next().unwrap_or("");
    last.chars().any(|c| c.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chatservant_core::frame::Rect;

    /// `my_side` 只看 `speaker` 和 `text`，`evidence` / `rect` 填什么都一样。
    /// `evidence` 是给界面看的一段**文字**说明（不是结构体），所以填空串。
    fn msg(speaker: Speaker, text: &str) -> chatservant_perceive::Message {
        chatservant_perceive::Message {
            id: format!("{speaker:?}-{text}"),
            speaker,
            text: text.into(),
            rect: Rect::new(0, 0, 10, 10),
            evidence: String::new(),
        }
    }

    fn mine(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| (*s).to_string()).collect()
    }

    /// 只有**我这一侧**能进样本池。对方说的、系统提示的都不行。
    #[test]
    fn my_side_keeps_only_my_own_lines() {
        let m = vec![
            msg(Speaker::Them, "在吗"),
            msg(Speaker::Me, "在的，你说"),
            msg(Speaker::Them, "有空吗"),
            msg(Speaker::Me, "有"),
            msg(Speaker::System, "对方撤回了一条消息"),
        ];
        assert_eq!(my_side(&m), mine(&["在的，你说", "有"]));
    }

    /// 长的不是口吻样本。**这条不能被「多喂点上下文」的直觉推翻。**
    #[test]
    fn long_messages_are_left_out() {
        let long = "啊".repeat(VOICE_MAX_CHARS + 1);
        let m = mine(&[&long, "好吧"]);
        assert_eq!(voice_samples(&m), mine(&["好吧"]));
    }

    /// 链接是**内容**不是语气——模型从 URL 里学不到任何口吻，
    /// 只会学到一件坏事：回复里可以带链接。
    #[test]
    fn links_are_left_out() {
        let m = mine(&[
            "https://example.com/a?b=1",
            "www.qq.com",
            "看看 example.com 这个",
            "行吧.",
            "嗯",
        ]);
        assert_eq!(voice_samples(&m), mine(&["行吧."]));
    }

    /// 句末标点**不是**裸域名。「行吧.」被踢掉就是把人正常说话当噪声丢，
    /// 而那种错误在界面上完全看不出来。
    #[test]
    fn a_sentence_ending_in_a_dot_is_not_a_bare_domain() {
        let m = mine(&["行吧.", "这样也行.", "收到"]);
        assert_eq!(voice_samples(&m), mine(&["行吧.", "这样也行.", "收到"]));
    }

    /// 同一个字重复多次**只算一个样本**。
    ///
    /// 不去重的话「好的」连发四条会占满四个位子，
    /// 而模型看到四行一模一样的文字，以为自己就是这么说话的。
    #[test]
    fn repeated_lines_occupy_one_slot() {
        let m = mine(&["好的", "好的", "好的", "好的", "收到"]);
        assert_eq!(voice_samples(&m), mine(&["好的", "收到"]));
    }

    /// 只取最近的 K 条，且**最近的排在最后**（与 `mine` 同一顺序约定）。
    #[test]
    fn keeps_the_most_recent_and_keeps_the_order() {
        let m: Vec<String> = (1..=20).map(|i| format!("第{i}句")).collect();
        let s = voice_samples(&m);
        assert_eq!(s.len(), VOICE_MAX_SAMPLES);
        assert_eq!(s.last().unwrap(), "第20句", "最新的要排最后");
        assert_eq!(s.first().unwrap(), "第13句");
    }

    /// 一个字不构成样本。
    #[test]
    fn a_single_character_is_not_a_sample() {
        assert!(voice_samples(&mine(&["嗯", "对"])).is_empty());
    }

    /// 一个样本都没有时**不留空标题**——那会让模型去找一个不存在的风格并自己编。
    #[test]
    fn no_samples_means_no_section() {
        assert_eq!(render(&[]), "");
        assert!(voice_samples(&mine(&["嗯"])).is_empty());
    }

    #[test]
    fn rendered_section_carries_its_own_label() {
        let t = render(&mine(&["好吧", "行"]));
        assert!(
            t.starts_with("[我自己平时的说话风格"),
            "样本要有自己的标题：{t}"
        );
        assert!(t.contains("好吧\n"));
        assert!(t.contains("行\n"));
    }

    /// 返回的是**新 Vec**，不是对输入的借用视图。
    ///
    /// 少这条，调用方把样本和 `recent` 放同一个结构里改时，
    /// 会静默改到 `recent`——而那种 bug 只表现为「口吻样本莫名其妙变了」。
    #[test]
    fn the_result_is_owned_not_a_view() {
        let m = mine(&["好吧"]);
        let s = voice_samples(&m);
        drop(m);
        assert_eq!(s, mine(&["好吧"]), "输入没了样本还在");
    }

    /// 本模块**一次网络请求都不该发**——「静默期零调用」在这里天然成立，
    /// 因为它压根没有发起调用的能力。
    #[test]
    fn picking_samples_costs_no_model_call() {
        let m = mine(&["在的", "行吧", "https://a.com"]);
        // 有样本、无输出、无 Err —— 这个函数压根没有能失败的地方。
        let s = voice_samples(&m);
        assert_eq!(s, mine(&["在的", "行吧"]));
    }
}
