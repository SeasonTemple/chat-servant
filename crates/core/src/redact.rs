//! 凭据脱敏的**唯一定义处**。
//!
//! ## 为什么从 `secret.rs` 搬到这里（2026-10-15，票 43）
//!
//! 原本有两份 `redact`，**强弱不一样**：
//!
//! | 位置 | 认得什么 |
//! |---|---|
//! | `ui/src-tauri/src/secret.rs` | 已知 key 值 + `sk-` 形状 + `api_key=` 赋值 + **URL 里的 `user:pass@` 与 `?access_token=`** |
//! | `crates/reason/src/transport.rs` | **只有** `sk-` / `sk-ant-` 形状 |
//!
//! 而自定义端点与代理的**报错全走 `reason` 那一侧**
//! （`ProxyCell::set` / `build_client` / `probe_route` 三处），
//! 于是一句 `http://alice:hunter2@proxy.internal:8080 连不上`
//! 会被那个只认 `sk-` 的弱版原样放行——**代理口令进日志**。
//!
//! 这不是「重复代码不好看」，是**两条能力不同的旁路**：
//! 强的那份再强，只要调用方挑了弱的那份就等于没有。
//! ⇒ 纯字符串逻辑收在这里，两边都调同一份，别再分叉。
//!
//! 这里**不含**「已登记的 key 值」那一路——那需要进程内的凭据缓存，
//! 属于 `secret.rs` 的职责（它才有 Windows 凭据管理器）。
//! 本模块只做**形状**上的遮蔽，不需要任何状态。

/// 掩码。**长度刻意不等于真实长度**——那会泄漏信息。
pub const MASK: &str = "••••••••";

/// 只按**形状**遮蔽：已登记的 key 值那一步由调用方先做。
///
/// 调用顺序有讲究：**先**做形状遮蔽的逆运算会更难复原，
/// 所以这里固定 `sk-` 形状 → URL 凭据。两者互不覆盖，顺序不影响结果，
/// 但固定下来比每次想一遍便宜。
pub fn shapes(text: &str) -> String {
    mask_url_credentials(&mask_sk_shaped(text))
}

/// 扫出所有 `sk-…` 形状的 token 并替换。
fn mask_sk_shaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(at) = rest.find("sk-") else {
            out.push_str(rest);
            return out;
        };
        let tail = &rest[at..];
        let end = tail
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
            .unwrap_or(tail.len());
        out.push_str(&rest[..at]);
        out.push_str(MASK);
        rest = &tail[end..];
    }
}

/// 抹掉 URL 里带的凭据。
///
/// **为什么必须单独做**（2026-10-15，票 43 的前置）：一旦允许配置自定义端点与
/// 代理，凭据就会以两种**新形状**出现在报错文本里，而
/// [`mask_sk_shaped`] 只认 `sk-` 开头——
///
/// 1. **用户名密码段**：`https://user:pass@host/v1/`（代理几乎都长这样）
/// 2. **查询参数里的 token**：`…/v1/?access_token=…&model=…`
///
/// URL 本身**要留着**——它是不含凭据时判断「是哪家、哪个端点」的唯一线索，
/// 抹掉等于让报错失去排查价值。所以只抹凭据那一段。
///
/// 宁可多抹：`user:pass@` 整段换成掩码是可接受的。
fn mask_url_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes: Vec<char> = text.chars().collect();
    let n = bytes.len();
    let mut i = 0;

    while i < n {
        if bytes[i] == ':' && i + 2 < n && bytes[i + 1] == '/' && bytes[i + 2] == '/' {
            i = mask_url_authority(&bytes, i, &mut out);
            continue;
        }
        if let Some(val_start) = sensitive_param_value_start(&bytes, i) {
            let mut val_end = val_start;
            while val_end < n && !is_url_value_delimiter(bytes[val_end]) {
                val_end += 1;
            }
            out.extend(bytes[i..val_start].iter());
            out.push_str(MASK);
            i = val_end;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn mask_url_authority(bytes: &[char], scheme: usize, out: &mut String) -> usize {
    let authority_start = scheme + 3;
    let mut authority_end = authority_start;
    while authority_end < bytes.len() && !is_url_authority_delimiter(bytes[authority_end]) {
        authority_end += 1;
    }
    if let Some(at_rel) = bytes[authority_start..authority_end]
        .iter()
        .rposition(|c| *c == '@')
    {
        let credential_end = authority_start + at_rel;
        out.extend(bytes[scheme..authority_start].iter());
        // 凭据段要跳过而不是推进去，否则 MASK 后仍然留着原文。
        out.push_str(MASK);
        out.push('@');
        return credential_end + 1;
    }
    out.extend(bytes[scheme..authority_end].iter());
    authority_end
}

fn sensitive_param_value_start(bytes: &[char], start: usize) -> Option<usize> {
    // 查询参数里那几个「名字里带 token/key/secret」的参数
    const SENSITIVE_PARAMS: [&str; 6] = [
        "api_key",
        "apikey",
        "access_token",
        "token",
        "secret",
        "password",
    ];
    SENSITIVE_PARAMS.iter().find_map(|param| {
        let len = param.chars().count();
        // 必须紧跟 '='，否则 `tokenizer` 之类会被误伤
        (start + len < bytes.len()
            && bytes[start..start + len].iter().collect::<String>() == *param
            && bytes[start + len] == '=')
            .then_some(start + len + 1)
    })
}

fn is_url_authority_delimiter(c: char) -> bool {
    matches!(
        c,
        '/' | '?' | '#' | ' ' | '"' | '\'' | '\n' | '\r' | '>' | ')'
    )
}

fn is_url_value_delimiter(c: char) -> bool {
    matches!(c, '&' | '#' | ' ' | '"' | '\'' | '\n' | '\r' | '>' | ')')
}

#[cfg(test)]
// secret-scan: allow —— 本文件里那两串 `sk-…` 是**故意的假 token**：
// 它们是脱敏器自己的测试样本，出现的地方就是要被它抹掉的输入。
// pre-commit 的钩子按内容匹配 `sk-` 形状，拦下来只会让人养成
// `--no-verify` 的习惯，而那个习惯一旦成形，真 key 也一起过去了。
// 按钩子自己写的规矩走**文件内豁免标记**，而不是绕过。
mod tests {
    use super::*;

    /// 代理地址里的用户名口令必须被抹掉——**代理几乎都长这样**
    /// （票 43 第 ③ 条明写「代理地址本身也要过 redact」）。
    #[test]
    fn a_url_with_basic_auth_credentials_is_masked() {
        let msg = "代理连不上：https://alice:hunter2@proxy.internal:8080/ 拒绝连接";
        let safe = shapes(msg);
        assert!(!safe.contains("hunter2"), "代理口令泄了：{safe}");
        assert!(!safe.contains("alice"), "代理账号泄了：{safe}");
        assert!(safe.contains("@proxy.internal:8080"), "host 得留着：{safe}");
    }

    #[test]
    fn a_query_string_api_key_is_masked() {
        let msg = "GET /v1/models?access_token=abc123DEF456&model=x 失败";
        let safe = shapes(msg);
        assert!(
            !safe.contains("abc123DEF456"),
            "查询参数里的 token 泄了：{safe}"
        );
        assert!(safe.contains("model=x"), "非敏感参数得留着：{safe}");
    }

    /// **钉住「无凭据的 URL 不许被动」**——否则报错失去排查价值，
    /// 这条比多抹更要紧：宁可漏抹也不能把线索全抹掉。
    #[test]
    fn a_url_without_credentials_is_left_alone() {
        let msg = "连不上 https://api.openai.com/v1/ 超时";
        assert_eq!(shapes(msg), msg, "无凭据的 URL 被动了");
    }

    /// 形近字段名不得误伤。`tokenizer` 里含 `token`，
    /// 没有紧跟的 `=` 就不是参数。
    #[test]
    fn a_parameter_merely_containing_the_word_token_is_not_masked() {
        let msg = "字段 tokenizer 缺失";
        assert_eq!(shapes(msg), msg, "误伤了非敏感字段名：{msg}");
    }

    /// `sk-` 形状的老规矩不能因为搬了家而失效。
    #[test]
    fn sk_shaped_tokens_are_still_masked() {
        let safe = shapes("401 invalid api key sk-ZZZZxxxxYYYYzzzz9999WWW tail");
        assert!(
            !safe.contains("sk-ZZZZxxxxYYYYzzzz9999WWW"),
            "key 还在：{safe}"
        );
        assert!(safe.contains("401"), "原因要保留：{safe}");
    }

    /// 两条规则叠在一起时的样子：既有 `sk-` 也有 URL 凭据，
    /// 两者都该被抹掉，且互不吞掉对方的掩码。
    #[test]
    fn both_shapes_are_masked_in_one_message() {
        let msg = "POST https://u:p@h/v1/?token=T0KENVAL failed with sk-abc123def456";
        let safe = shapes(msg);
        assert!(!safe.contains("u:p"), "用户名口令还在：{safe}");
        assert!(!safe.contains("T0KENVAL"), "查询 token 还在：{safe}");
        assert!(!safe.contains("sk-abc123def456"), "key 还在：{safe}");
    }

    /// PBT 属性测试：对任意伪随机生成的多语言文本（含中文、URL 用户名密码、敏感查询参数、sk- 令牌），
    /// `shapes` 满足三大不变量：
    /// 1. 零泄漏（No-Leak）：注入的所有敏感凭据片段在输出中彻底消失；
    /// 2. 幂等性（Idempotence）：`shapes(&shapes(&s)) == shapes(&s)`；
    /// 3. 锚点与结构保持：URL 的 `@host` 与 `#fragment` 不被误吞。
    #[test]
    fn pbt_redact_shapes_is_idempotent_and_never_leaks_injected_secrets() {
        let mut seed: u64 = 0x5EED_CAFE_2026_1103;
        let mut next_u64 = || -> u64 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let alphabet: &[char] = &[
            'a', 'b', 'z', '0', '9', '-', '_', '/', ':', '?', '&', '=', '#', '@', ' ', '\n', '好',
            '的', '微', '信', '测', '试', '（', '）', '「', '」',
        ];
        for case_idx in 0..500 {
            let len = (next_u64() % 48) as usize;
            let mut prefix = String::new();
            for _ in 0..len {
                let ch = alphabet[(next_u64() as usize) % alphabet.len()];
                prefix.push(ch);
            }
            let user_secret = format!("usr{case_idx}:pwSecret{case_idx}");
            let query_secret = format!("tokSecretVal_{case_idx}");
            let sk_secret = format!("sk-liveSecretKey{case_idx}XYZ");
            let raw = format!(
                "{prefix} https://{user_secret}@api.example.com/v1/chat?access_token={query_secret}#frag {sk_secret} 尾"
            );
            let once = shapes(&raw);
            let twice = shapes(&once);
            assert_eq!(once, twice, "PBT 幂等性破坏 (case {case_idx}): {raw:?}");
            assert!(
                !once.contains(&user_secret),
                "PBT 泄露 URL 账号密码: {once}"
            );
            assert!(!once.contains(&query_secret), "PBT 泄露查询 token: {once}");
            assert!(!once.contains(&sk_secret), "PBT 泄露 sk- 密钥: {once}");
            assert!(once.contains("@api.example.com"), "PBT 误吞主机名: {once}");
            assert!(once.contains("#frag"), "PBT 误吞 URL fragment: {once}");
        }
    }
}
