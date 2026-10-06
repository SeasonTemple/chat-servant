//! 把逐行归类归并成一句条消息。
//!
//! 微信里一条消息可能占好几行。归并依据是**同一个说话人 + 垂直间距足够近**。
//! 用 [`OcrLine::center_y`] 而不是 `y0`：`y0` 会随行内字形高度抖动，
//! 同一句话的第二行和第一行的 `y0` 差值不稳定。

use chatservant_core::frame::Rect;

use crate::attribute::{Attribution, NoiseClass, Speaker};

/// 归并成一条消息。
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub speaker: Speaker,
    pub text: String,
    pub rect: Rect,
}

/// 同一说话人的两行之间，间距小于「较高那行的行高」就并成一条。
///
/// ⚠ **尚未实测**：手头这帧微信里的气泡都是单行，没测到多行气泡的真实间距。
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
            }
            _ => out.push(Message {
                speaker: a.speaker,
                text: a.text.clone(),
                rect: a.rect,
            }),
        }
    }
    out
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
