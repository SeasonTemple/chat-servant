//! 把逐行归类归并成一句条消息。
//!
//! 微信里一条消息可能占好几行。归并依据是**同一个说话人 + 垂直间距足够近**。
//! 用 [`OcrLine::center_y`] 而不是 `y0`：`y0` 会随行内字形高度抖动，
//! 同一句话的第二行和第一行的 `y0` 差值不稳定。

use chatservant_core::frame::Rect;

use crate::attribute::{Attribution, NoiseClass, Speaker};

/// 归并成一条消息。
///
/// `Serialize` 是因为 UI 要渲染它——**但归并仍然只在 Rust 侧**。
///
/// ## 字段名是**契约**，不是随手起的
///
/// 界面按 `{#each messages as m (m.id)}` 做 keyed 渲染，并显示 `m.evidence`。
/// 这个结构**少一个字段，界面就整块空白**——而且后端日志一切正常：
/// 实测 Rust 推了 7 条消息，前端一个都没渲染出来，
/// 因为当时这个结构里**没有 `id`**，7 条消息的 key 全是 `undefined`，
/// Svelte 的 keyed each 直接把它们判成同一个节点。
///
/// 所以 `id` / `evidence` 由 `group` 负责填，且有测试守着
/// （见 `the_serialized_shape_carries_what_the_panel_binds_to`）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Message {
    /// **稳定键。** 由「说话人 + 正文」导出，见 [`merge_into_stream`] 的唯一化。
    /// 界面拿它做 keyed 渲染与入场动画的锚点，所以**同一句话在多轮之间
    /// 必须得到同一个 id**，否则每轮都会重画一遍动画。
    pub id: String,
    /// 谁说的。归并的第一判据：只有**同一个** [`Speaker`] 的相邻行才并。
    pub speaker: Speaker,
    /// 并出来的正文。仅含 [`NoiseClass::Text`]，标题、灰字、图内字、碎片
    /// 一律不进消息流——它们不是任何一句话的内容。
    pub text: String,
    /// 这一条消息在帧里的位置，取被并入各行矩形的并集区域。
    ///
    /// 界面上就是这行文字气泡画在哪，也是送视觉模型时该配的提示位置。
    pub rect: Rect,
    /// 这条为什么被这样判。**界面上常驻显示**——认错了要能立刻看见，
    /// 而不是靠猜。
    pub evidence: String,
}

/// 从「说话人 + 正文」导出稳定键。
///
/// 用原文而不是哈希：哈希会碰撞，而 Svelte 的 keyed each 遇到重复 key
/// 会**静默丢掉后面那条**，面板上就少一条消息且没有任何报错。
/// `MESSAGE_WINDOW` 只有 24 条，直接带原文的代价可以忽略。
fn message_id(speaker: Speaker, text: &str) -> String {
    format!("{speaker:?}:{text}")
}

/// 同一说话人的两行之间，间距小于「较高那行的行高」就并成一条。
///
/// **尚未实测**：手头这帧微信里的气泡都是单行，没测到多行气泡的真实间距。
/// 取值偏保守——宁可少并（多切几条），也不要跨气泡粘连。
/// 拿到有换行气泡的帧后要回来重标。
pub const MERGE_GAP_RATIO: f32 = 1.0;

/// 只保留正常正文，其余（标题、灰字、图内字、碎片）不进消息流。
pub fn group(attrs: &[Attribution]) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();

    for a in attrs {
        if a.noise != NoiseClass::Text {
            continue;
        }
        let can_merge = out.last().is_some_and(|m: &Message| {
            m.speaker == a.speaker && {
                let gap = a.rect.y0 - m.rect.y1;
                let h = a.rect.height().max(m.rect.height()) as f32;
                (gap as f32) < h * MERGE_GAP_RATIO
            }
        });

        match out.last_mut() {
            Some(m) if can_merge => {
                if needs_space(&m.text, &a.text) {
                    m.text.push(' ');
                }
                m.text.push_str(&a.text);
                m.rect.x0 = m.rect.x0.min(a.rect.x0);
                m.rect.x1 = m.rect.x1.max(a.rect.x1);
                m.rect.y1 = a.rect.y1;
                // 证据取**第一行**那条：多行合并后每一行的判据可能不同，
                // 而用户看到的是一条消息，显示第一条最贴近「它是怎么进来的」。
            }
            _ => out.push(Message {
                id: message_id(a.speaker, &a.text),
                speaker: a.speaker,
                text: a.text.clone(),
                rect: a.rect,
                evidence: a.evidence.to_string(),
            }),
        }
    }
    out
}

/// 面板里保留多少条消息。只为给人看，不参与推理——推理走 `Context`。
pub const MESSAGE_WINDOW: usize = 24;

/// 几轮之内视为「同一条重读」。
///
/// 观察循环每轮只读**增量 ROI**（水位线以下那条带子），而窗口滚动会让
/// 已经读过的内容重新落进下一轮的 ROI，rect 也跟着挪。所以要按
/// 「同一个人 + 同一段话」去重。**只看最近这么几条**：同样的内容隔很久
/// 再发一次是合法的对话，全量去重会把真消息吃掉。
const DEDUP_TAIL: usize = 6;

/// 把新一轮读到的消息并进已有的列表，得到面板该显示的那一份。
///
/// ## 为什么需要它
///
/// 观察循环只读**增量 ROI**，而 ROI 被水位线从下往上裁。直接把每轮的
/// 结果整份发给界面，面板上的聊天记录就会在「本轮那几条」和「空」之间
/// 来回跳——看起来像坏了。累积之后面板显示的是**这个会话最近的一段对话**，
/// 跟推理用的历史是一个道理。
///
/// 滚动会让已读内容重进 ROI，所以要去重；否则同一条消息会一路重复下去。
pub fn merge_into_stream(prev: &[Message], new: &[Message]) -> Vec<Message> {
    let mut out: Vec<Message> = prev.to_vec();
    for m in new {
        let dup = out
            .iter()
            .rev()
            .take(DEDUP_TAIL)
            .any(|p| p.speaker == m.speaker && p.text == m.text);
        if dup {
            continue;
        }
        out.push(m.clone());
    }
    if out.len() > MESSAGE_WINDOW {
        out.drain(..out.len() - MESSAGE_WINDOW);
    }
    // **id 必须在这份列表内部唯一。**
    //
    // 去重只看最近 [`DEDUP_TAIL`] 条，所以「同样一句话隔很久又说一遍」
    // 会合法地共存两条，而它们的 `message_id` 是一样的。
    // 界面是 `{#each ... (m.id)}`——重复 key 会让后一条**静默消失**，
    // 面板上凭空少一条消息，而前后端都没有任何报错。
    // 撞上的按次序加后缀，保证唯一。
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for m in out.iter_mut() {
        if seen.insert(m.id.clone()) {
            continue;
        }
        let base = m.id.clone();
        for n in 2u32.. {
            let cand = format!("{base}#{n}");
            if seen.insert(cand.clone()) {
                m.id = cand;
                break;
            }
        }
    }
    out
}

/// **这一轮里，哪几条是「新到的」。**
///
/// ## 为什么需要它（2026-10-23 实测逼出来的）
///
/// 微信的消息列表是**底部对齐**的：对话变长时**视口整体上移**，
/// 最新的那条始终贴在输入框上方。⇒ 对方又发了消息之后，
/// **新消息落在旧水位线之上**，而按像素行划出来的增量带
/// `[水位线, 消息区底部)` 里**一条新消息都没有**。
///
/// 真机实测（同一会话的两张真机帧，夹具换文件模拟「新消息到达」）：
///
/// ```text
/// 第 1 圈  roi y0=60   本轮行=14  消息=5   ← 首读，水位线推到 569
/// 换帧后  roi y0=569  本轮行=0   消息=0   ← 新增带是空的：一条都没进来
/// ```
///
/// 而换帧后**新增的那 5 条**在 y = 130..567 ⇒ **全在 569 之上**。
///
/// ⇒ 判据不能是「在第几行」，只能是「**这条消息之前有没有出现过**」。
///
/// ## 判据：**身份计数**（多重集），不是集合，也不是窗口
///
/// 消息身份 = **说话人 + 正文**（与 `message_id` 同一口径）。
/// `seen` 里有几条这个身份，本轮就抵掉几条；抵不动的那几条 ⇒ 新的。
///
/// ### 为什么用**计数**而不是「在不在集合里」
///
/// 对方把同一句又说一遍（「在吗」…「在吗」）是合法对话。
/// 用集合判 ⇒ 第二句被当成「已经见过」⇒ **不回复**。
/// 用计数判 ⇒ 第一句抵掉、第二句抵不动 ⇒ **第二句算新的** ✔
///
/// ### 为什么**不用窗口**（`DEDUP_TAIL` 那套）
///
/// 窗口比的是「最近几条」，可**用户往上翻历史**时，画面里全是**更早**的消息
/// ——它们落在窗口之外 ⇒ 被当成新的 ⇒ **对旧内容触发一次推理**
/// （Auto 档下还可能误发）。
///
/// ⚠ 计数法也有边界：`seen` 只留最近若干条 ⇒ **比那更早**的旧消息滚出来时
/// 身份已不在账本里 ⇒ 仍可能被当成新的。这是已知残留（票 06 的 `Novelty`）。
///
/// ## 为什么不用「位置」判
///
/// 位置（y）会随视口滚动整体平移——底部对齐的窗里新消息的 y
/// 反而比旧消息**小**。位置判据在这个软件上**结构性失效**。
///
/// ## 与 [`merge_into_stream`] 的区别
///
/// `merge_into_stream` 管的是**面板显示哪一份**（累积 + 去重）；
/// 本函数管的是「**哪几条是刚到的**」（驱动推理）。
/// 两者都要：前者防重复显示，后者防**对旧消息重复推理**。
pub fn novel_messages(seen: &[Message], current: &[Message]) -> Vec<Message> {
    // 「已经处理过」的身份**计数**（不是集合）。
    let mut left: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for m in seen {
        *left.entry(message_id(m.speaker, &m.text)).or_insert(0) += 1;
    }
    let mut out = Vec::new();
    for m in current {
        let key = message_id(m.speaker, &m.text);
        match left.get_mut(&key) {
            // 抵掉一个 ⇒ 这条是**处理过的**旧消息。
            Some(c) if *c > 0 => *c -= 1,
            // 抵不动 ⇒ 没处理过 ⇒ 新的。
            _ => out.push(m.clone()),
        }
    }
    out
}
/// 聊天区顶部的**会话名**。///
/// 头部那一行白字大标题（[`NoiseClass::Header`]）就是它。
/// 多个 Header 时取**最靠上**的那一条——会话名在标题栏，别的 Header
/// （群公告之类）在下面，取最上的那条才是会话名本身。
///
/// ## `header_h`：只看标题栏那条带子
///
/// 这一条是**实测逼出来的**：实机日志里 `peer` 认成了 `22 ： 22`
/// ——一个**时间戳**。原因是调用方传进来的 `attrs` 来自**增量 ROI**，
/// 而 ROI 被水位线从下往上裁，标题栏常常根本不在里面；剩下几行里
/// 恰好有一行被归成 Header，就被当成了会话名。
///
/// 会话名按定义住在标题栏里，所以判据里必须带上这条带子的高度。
/// 取不到就返回 `None`，**不要退回空串**：空串和「真的没有会话名」
/// 在调用方眼里是两回事，调用方要靠它决定敢不敢动手。
pub fn conversation_name(attrs: &[Attribution], header_h: i32) -> Option<String> {
    let mut best: Option<&Attribution> = None;
    for a in attrs {
        if a.noise != NoiseClass::Header || a.rect.y1 > header_h {
            continue;
        }
        let t = a.text.trim();
        if t.is_empty() {
            continue;
        }
        best = match best {
            None => Some(a),
            Some(b) if a.rect.y0 < b.rect.y0 => Some(a),
            keep => keep,
        };
    }
    best.map(|a| a.text.trim().to_string())
}

/// 拼接处要不要补一个空格。
///
/// 中文按字断行，直接拼是对的；英文/数字断在词中间，不补空格会粘成
/// `Bloomer deKOjin`。判据：拼接两侧**都**是拉丁字母或数字时才补——
/// 只看一侧会在「中文…English」和「English…中文」上补出多余空格。
pub(crate) fn needs_space(left: &str, right: &str) -> bool {
    let is_word_char = |c: char| c.is_ascii_alphanumeric();
    match (left.chars().next_back(), right.chars().next()) {
        (Some(a), Some(b)) => is_word_char(a) && is_word_char(b),
        _ => false,
    }
}

#[cfg(test)]
mod session_gate {
    use super::*;
    use crate::attribute::{Evidence, Speaker};
    use chatservant_core::frame::Rect;

    fn line(noise: NoiseClass, text: &str, y0: i32) -> Attribution {
        Attribution {
            speaker: Speaker::System,
            noise,
            text: text.into(),
            rect: Rect::new(10, y0, 120, y0 + 18),
            evidence: Evidence {
                bg: [30, 30, 31],
                ink: [240, 240, 240],
                contrast: 200.0,
                on_panel: true,
                chroma: 0,
                purity: 1.0,
                height: 18,
            },
        }
    }

    /// 认会话名时只看消息区顶部这么多像素。与观察循环、xtask 同值。
    const HEADER_BAND_H: i32 = 80;

    fn msg(sp: Speaker, text: &str) -> Message {
        msg_at(sp, text, 0)
    }

    fn msg_at(sp: Speaker, text: &str, y: i32) -> Message {
        Message {
            id: message_id(sp, text),
            speaker: sp,
            text: text.to_string(),
            rect: chatservant_core::frame::Rect::new(0, y, 100, y + 12),
            evidence: format!("测试 高={}px", 12),
        }
    }

    /// **标题栏之外被误归成 Header 的行，绝不能当会话名。**
    ///
    /// 2026-10-07 实机事故：观察循环日志里 `peer` 被认成 `22 ： 22`
    /// ——一个**时间戳**。原因是调用方传的 `attrs` 来自增量 ROI，
    /// 标题栏常常不在里面，剩下几行里恰好有一行被归成了 Header，
    /// 「最靠上」就选中了它。会话名按定义住在标题栏里。
    #[test]
    fn a_header_below_the_title_bar_is_not_the_conversation_name() {
        let attrs = vec![
            line(NoiseClass::Header, "22 ： 22", 520),
            line(NoiseClass::Header, "小号", 20),
        ];
        assert_eq!(
            conversation_name(&attrs, HEADER_BAND_H).as_deref(),
            Some("小号"),
            "标题栏下面的 Header 不该被当成会话名"
        );
    }

    /// 只有标题栏之外有 Header 时，宁可**认不出来**，也不猜一个。
    ///
    /// 猜错的后果是把 A 的对话当成 B 的处理——那是正确性事故。
    #[test]
    fn a_header_below_the_title_bar_alone_yields_none() {
        let attrs = vec![line(NoiseClass::Header, "22 ： 22", 520)];
        assert_eq!(conversation_name(&attrs, HEADER_BAND_H), None);
    }

    /// 累积：第二轮读到新消息时，两轮都还在。
    #[test]
    fn merging_keeps_earlier_messages() {
        let prev = vec![msg(Speaker::Them, "乐"), msg(Speaker::Me, "在的")];
        let new = vec![msg(Speaker::Them, "在吗")];
        let out = merge_into_stream(&prev, &new);
        assert_eq!(
            out.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            vec!["乐", "在的", "在吗"]
        );
    }

    /// 滚动会让已读内容**重新落进下一轮的 ROI**。不去重，面板上同一条
    /// 消息会一路重复下去——这是最容易被忽略的那种坏。
    #[test]
    fn a_reread_message_is_not_repeated() {
        let a = msg(Speaker::Them, "乐");
        let b = msg(Speaker::Me, "在的");
        let once = merge_into_stream(&[], &[a.clone(), b.clone()]);
        // 第二轮 ROI 又把「乐」读到了（rect 还挪了位置）
        let a_moved = msg_at(Speaker::Them, "乐", 40);
        let twice = merge_into_stream(&once, &[a_moved, b.clone()]);
        assert_eq!(
            twice.iter().filter(|m| m.text == "乐").count(),
            1,
            "同一条消息被读了两遍就只该留一份"
        );
    }

    /// 同样的内容**隔久了**再发一次是合法对话，不能当重读吃掉。
    #[test]
    fn a_genuine_repeat_later_is_kept() {
        let filler: Vec<Message> = (0..DEDUP_TAIL + 2)
            .map(|i| msg(Speaker::Me, &format!("第 {i} 条")))
            .collect();
        let mut prev = vec![msg(Speaker::Them, "乐")];
        prev.extend(filler);
        let out = merge_into_stream(&prev, &[msg(Speaker::Them, "乐")]);
        assert_eq!(
            out.iter().filter(|m| m.text == "乐").count(),
            2,
            "隔久了的同文消息是新的，不该被去重吃掉"
        );
    }

    /// 超过窗口就丢最旧的。面板只是给人看的窗口，不是账本。
    #[test]
    fn the_stream_is_capped() {
        let many: Vec<Message> = (0..MESSAGE_WINDOW + 10)
            .map(|i| msg(Speaker::Me, &format!("第 {i} 条")))
            .collect();
        let out = merge_into_stream(&[], &many);
        assert_eq!(out.len(), MESSAGE_WINDOW);
        assert_eq!(
            out.last().unwrap().text,
            format!("第 {} 条", MESSAGE_WINDOW + 9)
        );
    }

    /// **同一份列表里 id 必须唯一。**
    ///
    /// 2026-10-08 实机事故的另一半：去重只看最近 [`DEDUP_TAIL`] 条，
    /// 所以「隔久了又说一遍同样的话」会合法地留下两条、内容与
    /// `message_id` 都一样。界面是 `{#each messages as m (m.id)}`——
    /// 重复 key 让后一条**静默消失**，面板上凭空少一条，两端都不报错。
    #[test]
    fn ids_are_unique_inside_one_stream() {
        let filler: Vec<Message> = (0..DEDUP_TAIL + 2)
            .map(|i| msg(Speaker::Me, &format!("第 {i} 条")))
            .collect();
        let mut prev = vec![msg(Speaker::Them, "乐")];
        prev.extend(filler);
        let out = merge_into_stream(&prev, &[msg(Speaker::Them, "乐")]);

        let mut ids: Vec<&str> = out.iter().map(|m| m.id.as_str()).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(
            ids.len(),
            before,
            "同一份列表里出现了重复 id：{}",
            out.iter()
                .map(|m| format!("{}#{}", m.text, m.id))
                .collect::<Vec<_>>()
                .join(" / ")
        );
    }

    /// 同一句话在多轮之间必须拿到**同一个 id**。
    ///
    /// 界面拿 id 当入场动画的锚点；id 每轮变，动画就会一轮轮重放，
    /// 看起来像界面在不停闪。
    #[test]
    fn the_same_message_keeps_its_id_across_rounds() {
        let first = merge_into_stream(&[], &[msg(Speaker::Them, "乐")]);
        let second = merge_into_stream(&first, &[msg_at(Speaker::Them, "乐", 40)]);
        assert_eq!(
            first[0].id, second[0].id,
            "同一句话两轮之间 id 变了，入场动画会反复重放"
        );
    }

    /// **序列化出来的形状必须带上面板真正绑定的字段。**
    ///
    /// 2026-10-08 实机事故的根因：Rust 的 `Message` 只有
    /// `speaker/text/rect`，而界面按 `m.id` 做 keyed 渲染、按 `m.evidence`
    /// 显示判据。结果后端日志 `messages=7`，面板上一条都不渲染——
    /// 7 个 `undefined` key 被 Svelte 判成同一个节点，
    /// 而前后端**都没有任何报错**。
    ///
    /// 这条测试就是那道防线：字段改名或删除时它会立刻红。
    #[test]
    fn the_serialized_shape_carries_what_the_panel_binds_to() {
        let out = group(&[line(NoiseClass::Text, "来了吗", 100)]);
        let json: serde_json::Value = serde_json::to_value(&out[0]).expect("Message 必须能序列化");
        for key in ["id", "speaker", "text", "rect", "evidence"] {
            assert!(
                json.get(key).is_some(),
                "序列化的 Message 少了 `{key}` —— 面板绑定的是它。实际形状：{json}"
            );
        }
        assert_ne!(json["id"].as_str(), Some(""), "id 不能是空串");
        assert!(
            !json["id"]
                .as_str()
                .unwrap_or_default()
                .contains("undefined"),
            "id 不该是 undefined/null 拼出来的"
        );
        // 枚举必须序列化成**裸字符串**，前端 `speaker: 'Me' | 'Them' | 'System'`
        // 靠它分左右。序列化成 `{"Them":null}` 之类的话界面会全部当成「系统」。
        assert_eq!(json["speaker"].as_str(), Some("System"));
        assert_eq!(json["text"].as_str(), Some("来了吗"));
        assert!(
            !json["evidence"].as_str().unwrap_or_default().is_empty(),
            "判据必须能显示出来——空的话界面上那条依据就是空的"
        );
    }

    /// 会话名取**最靠上**的那条 Header。
    #[test]
    fn the_topmost_header_is_the_conversation_name() {
        let attrs = vec![
            line(NoiseClass::Header, "群公告在下面", 300),
            line(NoiseClass::Header, "小号", 20),
        ];
        assert_eq!(
            conversation_name(&attrs, HEADER_BAND_H).as_deref(),
            Some("小号")
        );
    }

    /// 正文和时间戳**不算**会话名。认错人就会把 A 的对话当成 B 的处理。
    #[test]
    fn text_and_gray_are_never_the_conversation_name() {
        let attrs = vec![
            line(NoiseClass::Text, "今晚有空吗", 100),
            line(NoiseClass::Gray, "昨天 22:31", 150),
        ];
        assert_eq!(
            conversation_name(&attrs, HEADER_BAND_H),
            None,
            "只有 Header 才是会话名"
        );
    }

    /// 认不出就返回 `None`，**不返回空串**。
    ///
    /// 空串会让调用方以为「名字是空的」，而 `None` 的意思是
    /// 「没认出来，更不该动手」——这是两种不同的结论。
    #[test]
    fn unreadable_header_is_none_not_empty() {
        let attrs = vec![line(NoiseClass::Header, "   ", 20)];
        assert_eq!(conversation_name(&attrs, HEADER_BAND_H), None);
    }

    /// 空白 Header 即使最靠上也要**跳过**，不因为位置就选它。
    #[test]
    fn blank_headers_are_skipped_even_when_topmost() {
        let attrs = vec![
            line(NoiseClass::Header, " ", 10),
            line(NoiseClass::Header, "小号", 40),
        ];
        assert_eq!(
            conversation_name(&attrs, HEADER_BAND_H).as_deref(),
            Some("小号")
        );
    }
}
