//! `PeerName` 的抖动归并。
//!
//! ## 为什么需要
//!
//! 对端名是 **OCR 出来的**，会抖：「小分队」↔「小分认」。
//! 若按字符串精确比较，一次抖动就会**分裂出一个新会话**——用户看到的是
//! 历史丢了、候选清空了、上下文归零，而程序「什么都没报错」。
//!
//! 这类 bug 极难查：它不崩、不报错，只是**静默地丢东西**。
//!
//! ## 判据：相似度，不是精确相等
//!
//! 用**最长公共子序列**（LCS）的归一化长度。
//!
//! 为什么不编辑距离：编辑距离要调换顺序、插入删除，算「小分队」→「小分认」
//! 只差 1 个字但需要一次替换操作；LCS 直接给出「有多少字是原样对上的」，
//! 对**识别错一两个字**这个真实故障模式更贴。
//!
//! 为什么不字符集合：字符集合会认为「甲乙」和「乙甲」一样，
//! 而这两个是完全不同的名字。

/// 两个名字的相似度，0..=1。
pub fn similarity(a: &str, b: &str) -> f32 {
    if a.is_empty() || b.is_empty() {
        return if a == b { 1.0 } else { 0.0 };
    }
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let lcs = lcs_len(&ac, &bc);
    lcs as f32 / ac.len().max(bc.len()) as f32
}

/// 最长公共子序列长度。**滚动数组**，O(n·m) 时间、O(min(n,m)) 空间。
fn lcs_len(a: &[char], b: &[char]) -> usize {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let mut prev = vec![0usize; short.len() + 1];
    let mut cur = vec![0usize; short.len() + 1];
    for &lb in long {
        for (j, &sa) in short.iter().enumerate() {
            cur[j + 1] = if sa == lb {
                prev[j] + 1
            } else {
                // 「不匹配 sa」看左边 cur[j]（本行已算好），
                // 「不匹配 lb」看**上一行**的 prev[j + 1]。
                //
                // 这里**不能**用 cur[j+1]：那还是两行前的残留值，
                // 会把 LCS 算短。短名字里重复字一多就出错。
                cur[j].max(prev[j + 1])
            };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[short.len()]
}

/// 相似到这个程度就算同一个对端。
///
/// 0.66 的来历：三个字的里错一个字 → LCS=2，`2/3 = 0.667`；
/// 四个字错一个 → `3/4 = 0.75`；两个字错一个 → `1/2 = 0.5`（**不够**，
/// 因为两个字的名字本来就容易撞，见下面的二次判定）。
pub const SAME_PEER: f32 = 0.66;

/// 两个名字是不是同一个对端。
///
/// 加了一层**二次判定**：两个字的短名字靠相似度不够——
/// 「小王」和「小李」相似度 0.5，但它们很可能真是两个人。
/// 短名字要求**逐字相同**，宁可多切一次会话，也不要把两个人合并——
/// 合并的代价是拿 A 的历史去回复 B。
pub fn same_peer(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() || b.is_empty() {
        return a == b;
    }
    let la = a.chars().count();
    let lb = b.chars().count();
    if la <= 2 && lb <= 2 {
        return a == b;
    }
    similarity(a, b) >= SAME_PEER
}

#[cfg(test)]
mod tests {
    use super::*;

    /// OCR 错一个字不该分裂出新会话——**这是整张票存在的理由**。
    ///
    /// 只测**三个字及以上**。两个字的名字做不到：
    /// 「张伟」和「张玮」到底是同一个人 OCR 认错了，还是两个不同的人，
    /// **在像素层面无法区分**。见 `two_character_names_require_exact_match`。
    #[test]
    fn one_wrong_character_does_not_split_a_session() {
        assert!(same_peer("小分队", "小分认"));
        assert!(same_peer("王小明", "王小名"));
        assert!(same_peer("项目负责人", "项目负责入"));
    }

    /// 真的不是一个人，不能合并。
    ///
    /// 合并的代价是**拿 A 的历史去回复 B**，比多切一次会话严重得多。
    #[test]
    fn different_names_are_not_merged() {
        assert!(!same_peer("小分队", "大部队"));
        assert!(!same_peer("张三", "李四"));
        assert!(!same_peer("Alice", "Bob"));
    }

    /// **两个字的短名字必须逐字相同。**
    ///
    /// 「小王」和「小李」相似度 0.5，低于 0.66 阈值——但这说明相似度
    /// 在短名字上**没有区分力**：两个字里任意两个不同的名字相似度都是 0.5。
    /// 所以短名字走**严格相等**，宁多切不误合。
    #[test]
    fn two_character_names_require_exact_match() {
        assert!(same_peer("小王", "小王"));
        assert!(!same_peer("小王", "小李"));
        assert!(!same_peer("阿明", "阿红"));
    }

    /// 空名相等，不空则不等。
    #[test]
    fn empty_names_are_handled() {
        assert!(same_peer("", ""));
        assert!(!same_peer("", "小王"));
        assert!(!same_peer("小王", ""));
        assert_eq!(similarity("", ""), 1.0);
    }

    /// 前后空白不该影响判定——OCR 常带尾随空格。
    #[test]
    fn surrounding_whitespace_is_ignored() {
        assert!(same_peer(" 小分队 ", "小分认"));
    }

    /// 相似度本身要算对。
    #[test]
    fn similarity_is_symmetric_and_bounded() {
        assert!((similarity("abc", "abc") - 1.0).abs() < 1e-6);
        assert_eq!(similarity("abc", "xyz"), 0.0);
        let a = similarity("小分队", "小分认");
        let b = similarity("小分认", "小分队");
        assert!((a - b).abs() < 1e-6, "相似度必须对称：{a} vs {b}");
        // 三个字错一个 → 2/3
        assert!((a - 2.0 / 3.0).abs() < 1e-6, "错一字应是 2/3，实际 {a}");
    }

    /// 一个长名字里重复的字多时，LCS 不能算短。
    ///
    /// 这是滚动数组实现最容易写错的地方：转移只取一个方向就会漏。
    #[test]
    fn repeated_characters_do_not_shorten_the_lcs() {
        // "aaa" 与 "aaaa" 的 LCS 必须是 3
        assert_eq!(
            lcs_len(
                &"aaa".chars().collect::<Vec<_>>(),
                &"aaaa".chars().collect::<Vec<_>>()
            ),
            3
        );
    }
}
