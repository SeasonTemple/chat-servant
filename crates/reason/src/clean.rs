//! 候选清洗。
//!
//! ## 为什么必须洗
//!
//! 模型很爱在正文外面包一层壳：`1. 好，明白了`、`「好的」`、
//! `me: 好，明白了`、`**好的**`。这些壳进了输入框就是
//! 「代写」的痕迹——用户发出去的东西带着不该有的前缀。
//!
//! 更糟的是**照抄的说话人前缀**：`me:` / `user:` / `assistant:`
//! 会被微信原样显示，对方看到「me: 好的」——
//! 那不是一个人在回消息，是一个程序在说话。
//!
//! ## 洗到哪一步为止
//!
//! **只洗壳，不改内容。** 不做标点规范化、不纠错、不换措辞——
//! 那是模型的事，不是这里的事。唯一动的结构是「句尾句号」：
//! 聊天里没人给每句话打句号，模型打了就说明它是在「写」而不是「说」。
//!
//! 清洗**不能把文本洗成空串**——那会产生一条空候选，
//!   比留着壳更糟（用户点下去输入框里什么都没有）。
//!   洗不出东西就**保留原文**。

/// 成对包裹：`(开, 闭)`。**两边都要非空**——把空串当闭合符
/// 会让 `strip_suffix("")` 恒成功，于是 `「好的」` 被剥成 `好的」`。
const WRAPPER_PAIRS: &[(&str, &str)] = &[
    ("**", "**"),
    ("__", "__"),
    ("`", "`"),
    ("「", "」"),
    ("『", "』"),
    ("《", "》"),
    ("【", "】"),
    ("〔", "〕"),
    ("［", "］"),
    ("【", "】"),
    ("（", "）"),
    ("(", ")"),
    ("\"", "\""),
    ("'", "'"),
    ("“", "”"),
    ("‘", "’"),
];

/// 无配对的包裹标记。模型经常只加不闭（`**好的`），
/// 所以这些要在两端**各自**剥掉，不要求成对。
const WRAPPER_LOOSE: &[&str] = &["**", "__", "`", "*", "#"];

/// 会被剥掉的说话人前缀。**大小写不敏感。**
///
/// `me:` / `user:` / `assistant:` 会被原样显示出去——那是在替用户说话，
/// 而不是帮他说话。中文的「我：」同理。
const SPEAKER_PREFIXES: &[&str] = &[
    "assistant:",
    "assistant：",
    "user:",
    "user：",
    "me:",
    "me：",
    "我：",
    "回答：",
    "reply:",
    "reply：",
];

/// 编号前缀：`1.` `2、` `(3)` `（4）` `3)`
///
/// 括号要先剥，否则 `(3)` 的第一个字符不是数字，整条规则直接跳过。
fn strip_numbering(s: &str) -> &str {
    let t = s.trim_start();
    let t = t
        .strip_prefix('(')
        .or_else(|| t.strip_prefix('（'))
        .map(str::trim_start)
        .unwrap_or(t);

    let b = t.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i > 3 {
        // 没有数字，或者数字太多（那不是编号，是内容）
        return s;
    }
    let rest = &t[i..];
    let rest = rest
        .strip_prefix(". ")
        .or_else(|| rest.strip_prefix(") "))
        .or_else(|| rest.strip_prefix(')'))
        .or_else(|| rest.strip_prefix('、'))
        .or_else(|| rest.strip_prefix('.'))
        // 括号形式：`(3) 行` 的收尾括号
        .or_else(|| rest.strip_prefix(')'));
    match rest {
        Some(r) if !r.trim().is_empty() => r.trim_start(),
        _ => s,
    }
}

/// 清一条候选。
///
/// 返回清洗后的文本；**洗不干净时返回原文**（宁可留着壳，也不要空串）。
///
/// ## 为什么是「循环到不动点」而不是「按固定顺序剥一遍」
///
/// 壳的**嵌套顺序不固定**。模型会写 `1. **me: 「好的」**`——
/// 编号在最外、说话人前缀在中间、引号在最内。按「先剥编号、再剥前缀、
/// 最后剥引号」的一遍式，前缀那一轮看到的还是 `**me: ...`，
/// 匹配不上；要等引号那一轮剥完，前缀才露出来——而那一轮已经过去了。
///
/// 循环到不动点就没有顺序问题：每一轮把任意一种壳剥掉一层，
/// 直到再也剥不动。
pub fn clean(raw: &str) -> String {
    let original = raw.trim();
    let mut s = original;

    // 上限防跑飞：每轮至少剥掉一个字符，最多剥 10 层足够。
    for _ in 0..10 {
        let before = s;

        s = strip_numbering(s);
        s = strip_speaker(s);
        s = strip_wrappers(s);
        s = s.trim();

        if s == before {
            break;
        }
    }

    s = strip_trailing_period(s);

    // 洗成空串 = 失败。**保留原文**比给一条空候选好得多。
    if s.is_empty() {
        return original.to_string();
    }
    s.to_string()
}

/// 剥掉说话人前缀。**要循环**：模型有时写成 `me: me: xxx`。
fn strip_speaker(s: &str) -> &str {
    let mut cur = s;
    for _ in 0..3 {
        let lower = cur.to_ascii_lowercase();
        let mut hit = None;
        for p in SPEAKER_PREFIXES {
            let pl = p.to_ascii_lowercase();
            if lower.starts_with(&pl) {
                let rest = cur[p.len()..].trim_start();
                // 前缀后**没有内容**时不能剥（剥了就空了）
                if !rest.is_empty() {
                    hit = Some(rest);
                    break;
                }
            }
        }
        match hit {
            Some(rest) => cur = rest,
            None => break,
        }
    }
    cur
}

/// 剥包裹字符。
fn strip_wrappers(s: &str) -> &str {
    let mut cur = s;
    for _ in 0..6 {
        let before = cur;
        // 先剥成对的
        for (a, b) in WRAPPER_PAIRS {
            if let Some(rest) = cur.strip_prefix(*a)
                && let Some(inner) = rest.strip_suffix(*b)
                && !inner.trim().is_empty()
            {
                cur = inner.trim();
                break;
            }
        }
        // 再剥不成对的（模型常只加不闭）
        for m in WRAPPER_LOOSE {
            if let Some(rest) = cur.strip_prefix(*m)
                && !rest.trim().is_empty()
            {
                cur = rest.trim();
                break;
            }
            if let Some(rest) = cur.strip_suffix(*m)
                && !rest.trim().is_empty()
            {
                cur = rest.trim();
                break;
            }
        }
        if cur == before {
            break;
        }
    }
    cur
}

/// 剥句尾的一个句号。
///
/// **只剥一个，且不吃省略号**：`好的...` 要保留成 `好的...`，
/// 剥成 `好的..` 是把三个点啃成两个，比留着更糟。
fn strip_trailing_period(s: &str) -> &str {
    let t = s.trim_end();
    let Some(rest) = t.strip_suffix('.') else {
        return t;
    };
    // 后面还有句号 → 这是省略号的一部分，不动
    if rest.ends_with('.') {
        return t;
    }
    // 剥完只剩符号（没有实质内容）→ 不动
    if rest.trim().is_empty() || !rest.trim().chars().any(|c| !c.is_ascii_punctuation()) {
        return t;
    }
    rest.trim_end()
}

/// 清洗一组候选。**保持顺序**——排序是接缝层第三段的事，不是这儿。
pub fn clean_all<'a, I: IntoIterator<Item = &'a str>>(raws: I) -> Vec<String> {
    raws.into_iter().map(clean).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 编号是最常见的壳。中文顿号、英文点号、括号都要认。
    #[test]
    fn strips_numbering_prefixes() {
        assert_eq!(clean("1. 好，明白了"), "好，明白了");
        assert_eq!(clean("2、好的"), "好的");
        assert_eq!(clean("(3) 行"), "行");
        assert_eq!(clean("1.2. 真的"), "真的");
        assert_eq!(clean("12. 十二"), "十二");
        // 数字开头的**正常句子**不能被剥
        assert_eq!(clean("3个人一起去"), "3个人一起去");
    }

    /// **照抄的说话人前缀是最严重的一种**——它会被对方看到，
    /// 看起来像程序在替用户说话。
    #[test]
    fn strips_speaker_prefixes() {
        assert_eq!(clean("me: 好的"), "好的");
        assert_eq!(clean("Me: 好的"), "好的");
        assert_eq!(clean("assistant: 我来"), "我来");
        assert_eq!(clean("user: 嗯"), "嗯");
        assert_eq!(clean("我：在的"), "在的");
        // 重复的也要剥
        assert_eq!(clean("me: me: 好的"), "好的");
        // 前缀后**没有内容**时不能剥（剥了就空了）
        assert_eq!(clean("me:"), "me:");
    }

    /// 引号与 Markdown。聊天里没人发 `**加粗**`。
    #[test]
    fn strips_wrappers() {
        assert_eq!(clean("**好的**"), "好的");
        assert_eq!(clean("「好的」"), "好的");
        assert_eq!(clean("\"好的\""), "好的");
        assert_eq!(clean("`好的`"), "好的");
        assert_eq!(clean("【好的】"), "好的");
        // 嵌套也要剥干净
        assert_eq!(clean("**\"好的\"**"), "好的");
    }

    /// 句尾句号。聊天里没人给每句话打句号。
    #[test]
    fn strips_trailing_period_but_keeps_real_content() {
        assert_eq!(clean("好的."), "好的");
        assert_eq!(clean("好的。"), "好的。"); // 中文句号保留：那是中文标点
        // 省略号**不能**被啃成两点
        assert_eq!(clean("好的..."), "好的...");
        assert_eq!(clean("好的……"), "好的……");
    }

    /// **清洗不能把有内容的候选洗成空串。**
    ///
    /// 空候选会让上层取 `recommended()` 时拿不到东西——
    /// 比留着 `""` 这种壳更糟。
    ///
    /// 断言的是「非空」而不是「与原文相同」：`1.` 会被洗成 `1`，
    /// 那是**合理**的清洗；真正的不变量只是「不能什么都没有」。
    /// 空串输入输出空串是对的——那不是「洗没了」，本来就没有。
    #[test]
    fn never_cleans_to_empty() {
        for raw in ["1.", "**", "me:", "\"\"", "「」", "  x  "] {
            let out = clean(raw);
            assert!(!out.is_empty(), "输入 {raw:?} 被洗成了空串");
        }
        assert_eq!(clean("1."), "1");
        assert_eq!(clean(""), "");
    }

    /// 正常文本**不能被动过**。清洗不是「顺手改改」。
    #[test]
    fn leaves_normal_text_alone() {
        for raw in [
            "好的",
            "行，我看看",
            "哈哈哈哈哈",
            "今晚八点？",
            "I see, sounds good",
            "3个人一起去",
        ] {
            assert_eq!(clean(raw), raw, "不该动 {raw:?}");
        }
    }

    /// 全套壳叠在一起。这是模型最常见的输出形状。
    #[test]
    fn handles_a_fully_wrapped_candidate() {
        assert_eq!(clean("1. **me: 「好的」**"), "好的");
    }

    /// 批量清洗保持顺序。
    #[test]
    fn clean_all_preserves_order() {
        let got = clean_all(["1. 甲", "me: 乙", "丙"]);
        assert_eq!(got, vec!["甲", "乙", "丙"]);
    }
}
