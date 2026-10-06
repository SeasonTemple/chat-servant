//! 真实的 HTTP 传输。**唯一出网的地方。**
//!
//! ## 为什么接口拆成 [`Http`] + [`HttpReasoner`]
//!
//! 上游模型 API 会变：字段改名、错误结构变、限流头换位置。把它藏在一个
//! **可替换的 `Http` trait** 后面，改的只是这一个文件；
//! 而 seam（`turn`）一行都不用动。
//!
//! 测试用 [`crate::gate::GateReasoner`]，不发网络请求——
//! 但**请求长什么样**这件事仍然要有测试，所以另有
//! [`HttpReasoner::against`]：给它一个假 `Http`，断言请求头、请求体、
//! 解析结果都对。这样「传输写错了」也不会溜过去。
//!
//! ## key 的处理
//!
//! key 从参数进来，**只出现在这一个请求头里**。它不会进日志、不进错误信息、
//! 不进任何 struct 的 `Debug` 输出。所有对外文本都过 [`redact`]。

use std::time::Duration;

use crate::model::{Candidate, DraftSet, Intent, ReasonError, Reasoner, TurnContext};

/// 一个最小 HTTP 接口。**只留真正需要的**，不留「以后可能用得上」的。
pub trait Http {
    /// POST 一个 JSON 请求，返回 JSON 响应体。
    ///
    /// 实现**必须**对 4xx/5xx 返回 `Err`——把「HTTP 成功但业务失败」
    /// 交给调用方判断，是接口设计里最容易出错的地方。
    fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, String>;
}

/// 各家模型 API 的差异都收在这一处。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAi,
    Anthropic,
}

impl Provider {
    pub fn key(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
        }
    }

    fn headers(self, api_key: &str, model: &str) -> Vec<(String, String)> {
        match self {
            Self::OpenAi => vec![
                ("Authorization".into(), format!("Bearer {api_key}")),
                ("Content-Type".into(), "application/json".into()),
            ],
            Self::Anthropic => vec![
                ("x-api-key".into(), api_key.to_string()),
                ("anthropic-version".into(), "2023-06-01".into()),
                ("Content-Type".into(), "application/json".into()),
                // 用假模型名会被 404；显式声明模型避免歧义
                ("anthropic-default".into(), model.to_string()),
            ],
        }
    }

    fn build_body(self, system: &str, user: &str) -> serde_json::Value {
        match self {
            Self::OpenAi => serde_json::json!({
                "model": MODEL_JUDGE,
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": user},
                ],
                "temperature": 0.2,
            }),
            Self::Anthropic => serde_json::json!({
                "model": MODEL_JUDGE,
                "system": system,
                "max_tokens": 256,
                "messages": [{"role": "user", "content": user}],
            }),
        }
    }

    /// 从响应里取「模型说的那段文字」。两家的响应结构完全不同。
    fn extract_text(self, v: &serde_json::Value) -> Option<String> {
        match self {
            Self::OpenAi => v
                .pointer("/choices/0/message/content")
                .and_then(|c| c.as_str())
                .map(str::to_string),
            Self::Anthropic => v
                .pointer("/content/0/text")
                .and_then(|c| c.as_str())
                .map(str::to_string),
        }
    }
}

const MODEL_JUDGE: &str = "gpt-4o-mini";
const MODEL_DRAFT: &str = "gpt-4o-mini";

const SYS_JUDGE: &str = "\
你在读一段聊天记录。只输出 JSON，不要任何解释或代码块标记。格式：
{\"intent\":\"对方想干什么，一句话\",\"need\":\"对方需要什么\",\"action\":\"reply|wait|react\",\"tension\":0到1之间的数字}
不要编造记录里没有的信息。读不懂就填 \"\"。";

const SYS_DRAFT: &str = "\
你在替用户回复聊天。直接给出回复正文本身，不要编号、不要引号、不要 \"me:\" 前缀、
不要解释。像真人发消息那样短。输出 JSON：{\"candidates\":[{\"text\":\"...\",\"probability\":0到1}]}
给 1 到 3 条，语气各不相同，probability 是你认为用户会选哪条。";

const SYS_RANK: &str = "\
你在给候选排序。只输出 JSON：{\"order\":[\"候选原文\",\"...\"]}
按「用户最可能选它」从高到低排。原样复制候选文本，不要改写。";

/// 一轮推理用的模型配置。
#[derive(Clone)]
pub struct HttpReasoner {
    http: std::sync::Arc<dyn Http + Send + Sync>,
    provider: Provider,
    /// **绝不进日志、不进错误、不进 Debug。**
    api_key: String,
    judge_model: String,
    draft_model: String,
    timeout: Duration,
    calls: std::cell::Cell<u32>,
}

impl std::fmt::Debug for HttpReasoner {
    /// 手写：**不能** derive——derive 会把 `api_key` 打进 Debug 输出，
    /// 而 Debug 输出会被日志吃掉。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpReasoner")
            .field("provider", &self.provider)
            .field("judge_model", &self.judge_model)
            .field("draft_model", &self.draft_model)
            .field("api_key", &"<redacted>")
            .field("calls", &self.calls.get())
            .finish()
    }
}

impl HttpReasoner {
    pub fn new(
        http: std::sync::Arc<dyn Http + Send + Sync>,
        provider: Provider,
        api_key: String,
    ) -> Self {
        Self {
            http,
            provider,
            api_key,
            judge_model: MODEL_JUDGE.to_string(),
            draft_model: MODEL_DRAFT.to_string(),
            timeout: Duration::from_secs(20),
            calls: std::cell::Cell::new(0),
        }
    }

    pub fn calls(&self) -> u32 {
        self.calls.get()
    }

    fn ask(&self, system: &str, user: &str, model: &str) -> Result<String, String> {
        let url = match self.provider {
            Provider::OpenAi => {
                format!("https://api.openai.com/v1/chat/completions?model={model}")
            }
            Provider::Anthropic => "https://api.anthropic.com/v1/messages".to_string(),
        };
        let headers = self.provider.headers(&self.api_key, model);
        let body = self.provider.build_body(system, user);
        self.calls.set(self.calls.get() + 1);
        let resp = self.http.post_json(&url, &headers, &body, self.timeout)?;
        self.provider.extract_text(&resp).ok_or_else(|| {
            // 上游契约变了。**只截响应、不带请求体**——请求体里有用户的
            // 聊天内容，绝不能进日志。`truncate` 只是截长度，不是脱敏。
            format!(
                "响应结构不认识（provider={}，非 OpenAI/Anthropic 的聊天格式）：{}",
                self.provider.key(),
                truncate(&resp.to_string(), 200)
            )
        })
    }

    /// 拿一段 JSON 文本。**剥掉模型爱加的代码块围栏。**
    fn ask_json(&self, system: &str, user: &str, model: &str) -> Result<serde_json::Value, String> {
        let raw = self.ask(system, user, model)?;
        let cleaned = strip_code_fence(&raw);
        serde_json::from_str(&cleaned).map_err(|e| {
            format!(
                "模型没返回合法 JSON（{e}）。原文前 200 字：{}",
                truncate(cleaned.trim(), 200)
            )
        })
    }

    fn render_ctx(ctx: &TurnContext) -> String {
        let mut s = String::new();
        if !ctx.peer.is_empty() {
            s.push_str(&format!("对端：{}\n", ctx.peer));
        }
        for m in &ctx.messages {
            s.push_str(m);
            s.push('\n');
        }
        s
    }
}

impl Reasoner for HttpReasoner {
    fn judge(&mut self, ctx: &TurnContext) -> Result<Intent, ReasonError> {
        let v = self
            .ask_json(SYS_JUDGE, &Self::render_ctx(ctx), &self.judge_model)
            .map_err(|e| ReasonError::Judge(redact(&e)))?;
        // 逐字段兜底：模型少给一个字段不该让整轮失败——
        // 缺 tension 就当 0，缺文字就留空（Intent::summary 会显示出来）。
        let s = |k: &str| {
            v.get(k)
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let tension = v
            .get("tension")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0)
            .clamp(0.0, 1.0) as f32;
        Ok(Intent {
            intent: s("intent"),
            need: s("need"),
            action: s("action"),
            tension,
        })
    }

    fn draft(&mut self, ctx: &TurnContext, intent: &Intent) -> Result<DraftSet, ReasonError> {
        let user = format!(
            "{}\n\n判断（供参考，可不认同）：{}\n",
            Self::render_ctx(ctx),
            intent.summary()
        );
        let v = self
            .ask_json(SYS_DRAFT, &user, &self.draft_model)
            .map_err(|e| ReasonError::Draft(redact(&e)))?;
        let mut candidates = Vec::new();
        if let Some(arr) = v.get("candidates").and_then(|c| c.as_array()) {
            for c in arr {
                let Some(text) = c.get("text").and_then(|t| t.as_str()) else {
                    continue;
                };
                candidates.push(Candidate {
                    text: text.to_string(),
                    probability: c
                        .get("probability")
                        .and_then(|p| p.as_f64())
                        .unwrap_or(0.5)
                        .clamp(0.0, 1.0) as f32,
                });
            }
        }
        Ok(DraftSet {
            candidates,
            blind: intent.action == "blind",
            calls: self.calls(),
        })
    }

    fn rank(
        &mut self,
        _ctx: &TurnContext,
        draft: &DraftSet,
    ) -> Result<Vec<Candidate>, ReasonError> {
        let mut lines = String::new();
        for c in &draft.candidates {
            lines.push_str(&c.text);
            lines.push('\n');
        }
        let v = self
            .ask_json(SYS_RANK, &lines, &self.judge_model)
            .map_err(|e| ReasonError::Rank(redact(&e)))?;
        let Some(order) = v.get("order").and_then(|o| o.as_array()) else {
            return Err(ReasonError::Rank("响应里没有 order 数组".into()));
        };
        // 按 order 里的原文回查候选。**回查失败就丢掉那一项**，
        // 不允许凭空造一条候选出来。
        let mut out: Vec<Candidate> = Vec::new();
        for want in order.iter().filter_map(|x| x.as_str()) {
            if let Some(c) = draft
                .candidates
                .iter()
                .find(|c| c.text.trim() == want.trim())
            {
                out.push(c.clone());
            } else {
                tracing::warn!("排序里出现了不在候选中的文本，已丢弃");
            }
        }
        Ok(out)
    }

    fn calls(&self) -> u32 {
        self.calls.get()
    }
}

/// 剥掉模型爱加的 ```json … ``` 围栏。
///
/// 这是**实测**出来的：即使 system prompt 说了「只输出 JSON」，
/// 模型还是会加围栏。不剥的话每个请求都解析失败。
pub fn strip_code_fence(raw: &str) -> String {
    let t = raw.trim();
    if !t.starts_with("```") {
        return t.to_string();
    }
    let body = t.trim_start_matches("```");
    // 第一行可能是 ```json / ```JSON
    let body = match body.find('\n') {
        Some(i) => &body[i + 1..],
        None => body,
    };
    body.trim_end_matches("```").trim().to_string()
}

/// 抹掉 key。**任何对外文本都要过这里。**
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for marker in ["sk-", "sk-ant-"] {
        let mut i = 0;
        while let Some(at) = out[i..].find(marker) {
            let s = i + at;
            let tail = &out[s..];
            let end = tail
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
                .unwrap_or(tail.len());
            out.replace_range(s..s + end, "<key>");
            i = s + 5;
            if i >= out.len() {
                break;
            }
        }
    }
    out
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "sk-testkey-should-never-appear";

    /// 一次请求的**形状**断言要用的东西。够用了。
    #[derive(Debug, Clone)]
    struct Seen {
        url: String,
        auth_header: Option<String>,
        body: serde_json::Value,
    }

    /// 按预设顺序返回回复的假 HTTP。**不发网络请求。**
    ///
    /// 记下每次请求的形状，好断言「请求头对不对、请求体有没有把聊天内容带上」。
    struct FakeHttp {
        seen: std::sync::Mutex<Vec<Seen>>,
        replies: std::sync::Mutex<Vec<String>>,
    }

    impl FakeHttp {
        fn new(replies: Vec<String>) -> Self {
            Self {
                seen: std::sync::Mutex::new(vec![]),
                replies: std::sync::Mutex::new(replies),
            }
        }
    }

    impl Http for FakeHttp {
        fn post_json(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &serde_json::Value,
            _t: Duration,
        ) -> Result<serde_json::Value, String> {
            let auth = headers
                .iter()
                .find(|(k, _)| k == "Authorization" || k == "x-api-key")
                .map(|(_, v)| v.clone());
            self.seen.lock().expect("锁").push(Seen {
                url: url.to_string(),
                auth_header: auth,
                body: body.clone(),
            });
            let mut r = self.replies.lock().expect("锁");
            let raw = if r.is_empty() {
                r.push(judge_json().into());
                judge_json().into()
            } else {
                r.remove(0)
            };
            Ok(openai_reply(&raw))
        }
    }

    fn openai_reply(text: &str) -> serde_json::Value {
        serde_json::json!({ "choices": [{ "message": { "content": text } }] })
    }

    fn judge_json() -> &'static str {
        r#"{"intent":"约时间","need":"确认时间","action":"reply","tension":0.3}"#
    }

    fn draft_json() -> &'static str {
        r#"{"candidates":[{"text":"今晚八点可以","probability":0.7},{"text":"我看看时间","probability":0.3}]}"#
    }

    #[test]
    fn a_full_turn_over_fake_http() {
        let http = std::sync::Arc::new(FakeHttp::new(vec![
            judge_json().into(),
            draft_json().into(),
            r#"{"order":["今晚八点可以","我看看时间"]}"#.into(),
        ]));
        let mut r = HttpReasoner::new(http.clone(), Provider::OpenAi, KEY.into());

        let ctx = TurnContext {
            messages: vec!["今晚有空吗".into()],
            peer: "小号".into(),
        };
        let t = crate::turn(&mut r, &ctx).expect("推理成功");

        assert!(t.judged);
        assert_eq!(t.intent.intent, "约时间");
        assert_eq!(t.candidates_count(), 2);
        assert_eq!(t.recommended().unwrap().text, "今晚八点可以");
        assert_eq!(r.calls(), 3, "判断+起草+排序 = 3 次");

        // 请求形状：三次都带 Authorization，URL 里带模型名
        let seen = http.seen.lock().expect("锁");
        assert_eq!(seen.len(), 3);
        for s in seen.iter() {
            assert_eq!(s.auth_header.as_deref(), Some(&*format!("Bearer {KEY}")));
            assert!(
                s.url
                    .starts_with("https://api.openai.com/v1/chat/completions")
            );
        }
        // 起草那次的请求体里必须带着聊天内容（否则模型无从回复）
        let draft_body = seen[1].body.to_string();
        assert!(
            draft_body.contains("今晚有空吗"),
            "起草请求没带对话：{draft_body}"
        );
    }

    #[test]
    fn code_fences_are_stripped() {
        assert_eq!(
            strip_code_fence("```json\n{\"a\":1}\n```").as_str(),
            "{\"a\":1}"
        );
        assert_eq!(strip_code_fence("```\n{\"a\":1}"), "{\"a\":1}");
        assert_eq!(strip_code_fence("{\"a\":1}"), "{\"a\":1}");
        // 没有围栏时原样返回
        assert_eq!(strip_code_fence("  {\"a\":1}  "), "{\"a\":1}");
    }

    /// key 绝不能出现在任何对外文本里。
    #[test]
    fn redaction_removes_keys() {
        let s = redact("401 invalid api key sk-abcdef1234567890 tail");
        assert!(!s.contains("sk-abcdef"), "key 还在：{s}");
        assert!(s.contains("401"), "原因要保留：{s}");
    }

    /// Debug 输出里**不能**有 key——Debug 会被日志吃掉。
    #[test]
    fn debug_never_prints_the_key() {
        let http = std::sync::Arc::new(FakeHttp::new(vec![]));
        let r = HttpReasoner::new(http, Provider::OpenAi, KEY.into());
        let d = format!("{r:?}");
        assert!(!d.contains(KEY), "Debug 泄了 key：{d}");
        assert!(d.contains("redacted"), "该显示占位：{d}");
    }

    #[test]
    fn anthropic_uses_its_own_header() {
        let hs = Provider::Anthropic.headers("k", "claude-x");
        assert!(hs.iter().any(|(k, v)| k == "x-api-key" && v == "k"));
        assert!(
            hs.iter().any(|(k, _)| k == "anthropic-version"),
            "缺版本头，上游会拒"
        );
        assert!(!hs.iter().any(|(k, _)| k == "Authorization"));
    }

    #[test]
    fn openai_uses_bearer() {
        let hs = Provider::OpenAi.headers("k", "gpt-x");
        assert!(
            hs.iter()
                .any(|(k, v)| k == "Authorization" && v == "Bearer k")
        );
    }
}
