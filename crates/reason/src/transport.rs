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

use crate::model::{Candidate, DraftSet, Intent, ReasonError, Reasoner, StageCalls, TurnContext};
use crate::vision::ImagePayload;

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

    /// GET 一个只读端点。**只为测连通性存在**——见 [`Provider::models_url`]。
    ///
    /// 单独一个方法而不是复用 `post_json`：测连通性要的是
    /// 「这个 key 能不能访问 API」，用一个**不调用模型**的端点才既准确又不花钱。
    /// 拿 `POST /messages` 去测会为了一次点按真的产生一次模型调用。
    fn get_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        timeout: Duration,
    ) -> Result<serde_json::Value, String>;
}

/// 测连通性用的只读端点。**两家都有 `/v1/models`**，
/// 且它既不需要模型、也不产生 token 费用。
impl Provider {
    /// 测连通性用的只读端点。**不调用模型、不产生 token 费用。**
    pub fn models_url(self) -> &'static str {
        match self {
            Self::Anthropic => "https://api.anthropic.com/v1/models",
            Self::OpenAi => "https://api.openai.com/v1/models",
            Self::MiniMax => "https://api.minimaxi.com/v1/models",
        }
    }

    /// 端点需要的 header。与 [`Provider::headers`] 同源，但**不带**模型名——
    /// `anthropic-default` 那个头是本项目自己加的，上游的 models 端点不认。
    pub fn probe_headers(self, api_key: &str) -> Vec<(String, String)> {
        if self.is_openai_compatible() {
            return vec![("Authorization".into(), format!("Bearer {api_key}"))];
        }
        vec![
            ("x-api-key".into(), api_key.to_string()),
            ("anthropic-version".into(), "2023-06-01".into()),
        ]
    }
}

/// 各家模型 API 的差异都收在这一处。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// OpenAI：走 **OpenAI 兼容**协议（`POST /v1/chat/completions?model=…`）。
    ///
    /// 是 [`Provider::is_openai_compatible`] 判定的基准那一家，
    /// 新接的兼容服务默认跟它对齐，差异只在端点与模型名两项数据上。
    OpenAi,
    /// Anthropic：**原生协议**，不是 OpenAI 兼容的。
    ///
    /// 三处都不一样：鉴权头（`x-api-key` 而非 `Bearer`）、
    /// 模型的传递方式（请求体而非 query）、响应里取文字的指针
    /// （`/content/0/text`）。而且图要排在文字**前面**。
    /// 所以「加一家 provider 就是加一行」在这里不成立，它要占一个独立 arm。
    Anthropic,
    /// MiniMax，走 **OpenAI 兼容**接口。
    ///
    /// ## 为什么它是「另一种协议」而不是「OpenAi 换个 URL」
    ///
    /// 请求体、鉴权头、响应结构三样都和 OpenAI 一致（实测 `abab6.5s-chat`
    /// 在 `https://api.minimaxi.com/v1/chat/completions` 返回 HTTP 200 并
    /// 正常回话）。**但差异只有端点与模型名两项**——把它们也变成
    /// 数据（[`Provider::chat_url`] / [`Provider::models`]），
    /// 加上家就是加一行，而不是在 `headers` / `build_body` /
    /// `extract_text` / `models_url` / `probe_headers` 每处都加一个 arm。
    /// 以后接 Groq、DeepSeek、硅基流动同理。
    MiniMax,
}

impl Provider {
    /// 配置、设置页、命令行里用的短名。**稳定标识，不是显示名**——
    /// 改名会让用户已存的配置读不出来。
    ///
    /// 反向是 [`Provider::from_key`]；日志里报 provider 用的也是它
    /// （而不用枚举的 Debug，那是给开发者看的）。
    pub fn key(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::MiniMax => "minimax",
        }
    }

    /// 反过来：由 [`Provider::key`] 的短名（`"openai"` 等）找回枚举值。
    ///
    /// 认不出来返回 `None`——**不猜**。短名会从配置、设置页、
    /// 命令行多处进来，静默兜一个默认值只会把错带到出网那一刻。
    pub fn from_key(key: &str) -> Option<Self> {
        match key.trim().to_ascii_lowercase().as_str() {
            "openai" => Some(Self::OpenAi),
            "anthropic" => Some(Self::Anthropic),
            "minimax" => Some(Self::MiniMax),
            _ => None,
        }
    }

    /// 读 key 时**按顺序试**的环境变量名。
    ///
    /// 第一个是通用写法 `<PROVIDER>_API_KEY`；后面是该服务商
    /// 自己文档里实际在用的名字，**只填确知存在的**。
    ///
    /// ## 为什么不能只有一个名字
    ///
    /// 实机核对：本机只设了 `MINIMAX_CN_API_KEY`，没有
    /// `MINIMAX_API_KEY`。只认通用写法的话，应用会静默降级成
    /// **预置假传输**——界面上每条候选都标着「预置」，但那意味着
    /// 用户配好的 key 根本没被用上，而界面没有任何地方说这件事。
    ///
    /// 规则收在这里而不是各处 `env::var`：应用和 `xtask e2e` 必须
    /// 用同一份，否则又会出现「诊断命令和产品读的不是同一个变量」
    /// 这种已经犯过的病。
    pub fn key_env_names(self) -> &'static [&'static str] {
        match self {
            Self::OpenAi => &["OPENAI_API_KEY"],
            Self::Anthropic => &["ANTHROPIC_API_KEY"],
            // `abab6.5s-chat` / `api.minimaxi.com` 走的是中国区端点，
            // 官方文档与各家接入指南给的就是 `_CN_` 那个变量名。
            Self::MiniMax => &["MINIMAX_API_KEY", "MINIMAX_CN_API_KEY"],
        }
    }

    /// 是不是走 **OpenAI 兼容**协议（请求体 / 鉴权 / 响应结构全一致）。
    ///
    /// 本文件里所有「分协议」的地方都判这个，**而不是**逐个列 variant。
    /// 少写一个 arm 就少一处「新 provider 忘了改这里」的坑。
    pub fn is_openai_compatible(self) -> bool {
        !matches!(self, Self::Anthropic)
    }

    /// 聊天补全端点。**OpenAI 把模型放 query，MiniMax 放请求体**——
    /// 这是两者唯一的请求侧差别，各自的 `build_body` 已经照顾到了。
    pub fn chat_url(self, model: &str) -> String {
        match self {
            Self::Anthropic => "https://api.anthropic.com/v1/messages".to_string(),
            Self::OpenAi => format!("https://api.openai.com/v1/chat/completions?model={model}"),
            Self::MiniMax => "https://api.minimaxi.com/v1/chat/completions".to_string(),
        }
    }

    /// 判断模型 / 起草模型。
    ///
    /// ⚠ **只填实测过能通的模型名。** 填一个没验过的名字，表现会是
    /// 「设置成功、一调用就 404」，比明确不支持更难查。
    /// 三个都是实测/官方文档确认的：gpt-4o-mini、claude-sonnet-5、abab6.5s-chat。
    pub fn models(self) -> (&'static str, &'static str) {
        match self {
            Self::OpenAi => ("gpt-4o-mini", "gpt-4o-mini"),
            Self::Anthropic => ("claude-sonnet-5", "claude-sonnet-5"),
            Self::MiniMax => ("abab6.5s-chat", "abab6.5s-chat"),
        }
    }

    fn headers(self, api_key: &str, model: &str) -> Vec<(String, String)> {
        if self.is_openai_compatible() {
            return vec![
                ("Authorization".into(), format!("Bearer {api_key}")),
                ("Content-Type".into(), "application/json".into()),
            ];
        }
        vec![
            ("x-api-key".into(), api_key.to_string()),
            ("anthropic-version".into(), "2023-06-01".into()),
            ("Content-Type".into(), "application/json".into()),
            // 用假模型名会被 404；显式声明模型避免歧义
            ("anthropic-default".into(), model.to_string()),
        ]
    }

    /// 拼请求体。**带图时两种协议结构完全不同，所以这里不是一个模版。**
    ///
    /// `model` 必须**传进来**而不是在这里取常量：各家模型名不同，
    /// 早先的版本在这里写死 `gpt-4o-mini`，于是**选 Anthropic 时请求体里
    /// 发的也是 `gpt-4o-mini`**——而 `anthropic-default` 那个头只是我们
    /// 自己加的，上游认的是 body。症状会是「配置成功、一调用就 400」。
    ///
    /// ⚠ **Anthropic 官方建议图放在文字之前**（原文：*"Claude works best when
    /// images come before text … prefer an image-then-text structure."*），
    /// 所以这家的 content 是 `[图…, 文字]`。顺序不是随手排的。
    fn build_body(
        self,
        system: &str,
        user: &str,
        model: &str,
        images: &[ImagePayload],
    ) -> serde_json::Value {
        if self.is_openai_compatible() {
            // OpenAI 兼容这边文字在前、图在后即可，与它的 chat 文档示例一致。
            let mut content = vec![serde_json::json!({
                "type": "text",
                "text": user,
            })];
            for i in images {
                content.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": {
                        "url": format!("data:{};base64,{}", i.media_type, i.base64),
                    },
                }));
            }
            return serde_json::json!({
                "model": model,
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": content},
                ],
                "temperature": 0.2,
            });
        }

        // 到这里只可能是 Anthropic（原生协议）。
        let mut content: Vec<serde_json::Value> = images
            .iter()
            .map(|i| {
                serde_json::json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": i.media_type,
                        "data": i.base64,
                    },
                })
            })
            .collect();
        content.push(serde_json::json!({
            "type": "text",
            "text": user,
        }));
        serde_json::json!({
            "model": model,
            "system": system,
            "max_tokens": 256,
            "messages": [{"role": "user", "content": content}],
        })
    }

    /// 从响应里取「模型说的那段文字」。**两种协议结构不同。**
    fn extract_text(self, v: &serde_json::Value) -> Option<String> {
        if self.is_openai_compatible() {
            return v
                .pointer("/choices/0/message/content")
                .and_then(|c| c.as_str())
                .map(str::to_string);
        }
        v.pointer("/content/0/text")
            .and_then(|c| c.as_str())
            .map(str::to_string)
    }
}

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
    /// 分段计数。**和 `calls` 分开存**，因为界面要分开显示，
    /// 而只有一个总数的话，任何一段挂了都分不清是哪段。
    judge_calls: std::cell::Cell<u32>,
    draft_calls: std::cell::Cell<u32>,
    rank_calls: std::cell::Cell<u32>,
}

/// 哪一段在发请求。**只为计数用**，不进请求体。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Judge,
    Draft,
    Rank,
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
    /// 装好一个发真请求的推理器。**模型名取 [`Provider::models`] 的默认值**，
    /// 两段同一个模型——判断那段的输出只有几十个字，换模型换不出什么，
    /// 却是多一份费用和一份延迟。
    ///
    /// 超时固定 20 秒，出网全在 [`Http`] 后面，
    /// 所以测试传假 `Http` 就完全不联网。
    pub fn new(
        http: std::sync::Arc<dyn Http + Send + Sync>,
        provider: Provider,
        api_key: String,
    ) -> Self {
        Self {
            http,
            provider,
            api_key,
            judge_model: provider.models().0.to_string(),
            draft_model: provider.models().1.to_string(),
            timeout: Duration::from_secs(20),
            calls: std::cell::Cell::new(0),
            judge_calls: std::cell::Cell::new(0),
            draft_calls: std::cell::Cell::new(0),
            rank_calls: std::cell::Cell::new(0),
        }
    }

    /// 累计发出的模型请求数（三段合计）。
    ///
    /// **失败的请求也算一次**——计数在发送前就加了，理由是它要回答
    /// 「我花了多少次请求」，而不是「成功了几次」。
    /// 「静默期零调用」那条硬约束就靠它在静默期断言为 0；
    /// 分段数字走 [`Reasoner::stage_calls`]。
    pub fn calls(&self) -> u32 {
        self.calls.get()
    }

    fn ask(
        &self,
        system: &str,
        user: &str,
        model: &str,
        stage: Stage,
        images: &[ImagePayload],
    ) -> Result<String, String> {
        let url = self.provider.chat_url(model);
        let headers = self.provider.headers(&self.api_key, model);
        let body = self.provider.build_body(system, user, model, images);
        self.calls.set(self.calls.get() + 1);
        // 分段计数。**先加再发**——发失败也算一次尝试，
        // 界面上「调用了几次」要是用户实际掏的钱，不是成功次数。
        match stage {
            Stage::Judge => self.judge_calls.set(self.judge_calls.get() + 1),
            Stage::Draft => self.draft_calls.set(self.draft_calls.get() + 1),
            Stage::Rank => self.rank_calls.set(self.rank_calls.get() + 1),
        }
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
    fn ask_json(
        &self,
        system: &str,
        user: &str,
        model: &str,
        stage: Stage,
        images: &[ImagePayload],
    ) -> Result<serde_json::Value, String> {
        let raw = self.ask(system, user, model, stage, images)?;
        let cleaned = strip_code_fence(&raw);
        serde_json::from_str(&cleaned).map_err(|e| {
            format!(
                "模型没返回合法 JSON（{e}）。原文前 200 字：{}",
                truncate(cleaned.trim(), 200)
            )
        })
    }

    /// 判断那一段用的上下文。**刻意不含口吻样本。**
    ///
    /// 口吻是「怎么写」，判断问的是「对方想干什么」——这两件事没有因果。
    /// 塞进去有两个坏处：一是每条新消息都要为判断这一段**白付一次**
    /// 样本的 token（一条消息触发 3 次调用，见 `StageCalls`），
    /// 二是模型可能把样本当成「用户之前怎么回的」，拿它去推断意图。
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

    /// 起草那一段用的上下文 = 口吻样本 + [`HttpReasoner::render_ctx`]。
    ///
    /// 样本**排在对话之前**。顺序是有讲究的：起草读的是「先建立语气，
    /// 再看内容」。放最后的话，刚读完对方的话就去看样本，最后一行
    /// （语气最重）反而是风格参考，对话内容会被冲淡。
    fn render_draft_ctx(ctx: &TurnContext) -> String {
        let voice = crate::voice::render(&ctx.voice);
        if voice.is_empty() {
            return Self::render_ctx(ctx);
        }
        let mut s = voice;
        s.push('\n');
        s.push_str(&Self::render_ctx(ctx));
        s
    }
}

impl Reasoner for HttpReasoner {
    fn judge(&mut self, ctx: &TurnContext) -> Result<Intent, ReasonError> {
        let v = self
            .ask_json(
                SYS_JUDGE,
                &Self::render_ctx(ctx),
                &self.judge_model,
                Stage::Judge,
                &[],
            )
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
            Self::render_draft_ctx(ctx),
            intent.summary()
        );
        let v = self
            .ask_json(
                SYS_DRAFT,
                &user,
                &self.draft_model,
                Stage::Draft,
                &ctx.images,
            )
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
            .ask_json(SYS_RANK, &lines, &self.judge_model, Stage::Rank, &[])
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

    /// 三段分别计数。**视觉调用不在这三个数里**——
    /// 它属于 `VisionBridge`，默认配置下恒为 0。
    fn stage_calls(&self) -> StageCalls {
        StageCalls {
            judge: self.judge_calls.get(),
            draft: self.draft_calls.get(),
            rank: self.rank_calls.get(),
        }
    }
}

/// 测一次连通性。**只回「通不通」和一句人话，不回任何响应体。**
///
/// ## 为什么必须真的发一次请求
///
/// 早先的 `probe_key` 恒返回 `false`，注释写着「网络层 ticket 10 接上」。
/// 但网络层早就是真的了——于是设置页那个「测试连通」按钮**永远说不通**。
/// 一个恒假的指示器比没有更糟：用户会以为 key 坏了，去反复重填。
///
/// ## 为什么不调模型
///
/// 走 `GET /v1/models`：不产生 token、不花钱、也不占用速率额度，
/// 而「这个 key 能不能访问 API」这个问题它答得一样准。
///
/// ## 错误里的 key
///
/// 上游的错误正文**可能带上你刚发的 header**。所以整条错误都过一遍
/// [`redact`]，并且只截前 200 字符——这两步都必要。
pub fn probe_connectivity(
    http: &dyn Http,
    provider: Provider,
    api_key: &str,
    timeout: Duration,
) -> Result<(), String> {
    let resp = http
        .get_json(
            provider.models_url(),
            &provider.probe_headers(api_key),
            timeout,
        )
        .map_err(|e| redact(&e))?;
    // 能解析出 JSON 说明契约没变。反过来不检查内容：models 列表长什么样
    // 不影响「通不通」这个答案，硬要校验字段只会引入新的脆弱点。
    if resp.is_object() {
        Ok(())
    } else {
        Err(redact("上游返回的不是 JSON 对象——端点可能变了"))
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
        /// 测连通性用的假实现。**不消耗 reply 队列**——
        /// 那队列是给三段式准备的，测一次连通不该把下一轮的回复吃掉。
        fn get_json(
            &self,
            url: &str,
            headers: &[(String, String)],
            _t: Duration,
        ) -> Result<serde_json::Value, String> {
            let auth = headers
                .iter()
                .find(|(k, _)| k == "Authorization" || k == "x-api-key")
                .map(|(_, v)| v.clone());
            self.seen.lock().expect("锁").push(Seen {
                url: url.to_string(),
                auth_header: auth,
                body: serde_json::Value::Null,
            });
            Ok(serde_json::json!({ "data": [] }))
        }

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

        let ctx = TurnContext::text(vec!["今晚有空吗".into()], "小号".into());
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

    /// **口吻样本只进起草那一段，判断那一段不许收到。**
    ///
    /// 两个理由，少一条都不够：
    /// ① **成本**。一条新消息触发 3 次调用（判断 1 · 起草 1 · 排序 1），
    ///    判断是**最快最便宜**的那一档，却要为一份它用不上的样本付一次钱。
    /// ② **污染**。判断问的是「对方想干什么」，模型看到样本就可能理解成
    ///    「用户之前是怎么回的」，拿它去反推意图——那是凭空多的一条推理路径。
    #[test]
    fn voice_samples_reach_the_draft_but_not_the_judge() {
        let http = std::sync::Arc::new(FakeHttp::new(vec![
            judge_json().into(),
            draft_json().into(),
            r#"{"order":["今晚八点可以","我看看时间"]}"#.into(),
        ]));
        let mut r = HttpReasoner::new(http.clone(), Provider::OpenAi, KEY.into());

        let ctx = TurnContext::text(vec!["今晚有空吗".into()], "小号".into())
            .with_voice(vec!["嗯我看看时间".into(), "行".into()]);
        crate::turn(&mut r, &ctx).expect("推理成功");

        let seen = http.seen.lock().expect("锁");
        let judge_body = seen[0].body.to_string();
        let draft_body = seen[1].body.to_string();

        assert!(
            !judge_body.contains("嗯我看看时间"),
            "判断请求不该带口吻样本：{judge_body}"
        );
        assert!(
            draft_body.contains("嗯我看看时间"),
            "起草请求必须带口吻样本：{draft_body}"
        );
        assert!(
            draft_body.contains("[我自己平时的说话风格"),
            "样本要有自己的标题，否则模型会当成对方说的话：{draft_body}"
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

    /// **测连通性必须打到 `/v1/models`，不能调模型。**
    ///
    /// 早先的 `probe_key` 恒返回 false，注释说「网络层 ticket 10 接上」——
    /// 但网络层早就是真的了，于是那个按钮**永远说不通**。
    /// 恒假的指示器比没有更糟：用户会以为 key 坏了，反复重填。
    #[test]
    fn probing_hits_the_models_endpoint_not_the_model() {
        let http = std::sync::Arc::new(FakeHttp::new(vec![]));
        probe_connectivity(&*http, Provider::OpenAi, KEY, Duration::from_secs(5))
            .expect("假 HTTP 应当通");
        probe_connectivity(&*http, Provider::Anthropic, KEY, Duration::from_secs(5))
            .expect("假 HTTP 应当通");

        let seen = http.seen.lock().expect("锁").clone();
        assert_eq!(seen.len(), 2, "两次探测应各留一条记录");
        for s in &seen {
            assert!(
                s.url.ends_with("/v1/models"),
                "探测打到了 {} —— 调模型是要花钱的",
                s.url
            );
            assert!(
                !s.url.contains("/messages"),
                "探测不该打 messages 端点：{}",
                s.url
            );
        }
    }

    /// **探测的错误里不能有 key。** 上游的错误正文可能带上刚发的 header。
    #[test]
    fn a_probe_error_never_contains_the_key() {
        /// 把 key 写进错误正文的假上游——模拟最坏情况。
        struct Leaky;
        impl Http for Leaky {
            fn get_json(
                &self,
                _u: &str,
                _h: &[(String, String)],
                _t: Duration,
            ) -> Result<serde_json::Value, String> {
                Err(format!("上游返回 401：invalid key {KEY}"))
            }
            fn post_json(
                &self,
                _u: &str,
                _h: &[(String, String)],
                _b: &serde_json::Value,
                _t: Duration,
            ) -> Result<serde_json::Value, String> {
                Ok(serde_json::json!({}))
            }
        }
        let e = probe_connectivity(&Leaky, Provider::OpenAi, KEY, Duration::from_secs(5))
            .expect_err("应当失败");
        assert!(!e.contains(KEY), "错误泄了 key：{e}");
        assert!(e.contains("401"), "状态码要留着，那才是有用的信息：{e}");
    }

    /// 探测**不消耗**三段式的回复队列——测一次连通不该影响下一轮。
    #[test]
    fn probing_does_not_consume_the_reply_queue() {
        let http = std::sync::Arc::new(FakeHttp::new(vec![judge_json().into()]));
        probe_connectivity(&*http, Provider::OpenAi, KEY, Duration::from_secs(5)).expect("探测");
        // 队列里那条还在——下一轮 judge 拿得到它。
        let left = http.replies.lock().expect("锁").len();
        assert_eq!(left, 1, "探测把回复队列吃掉了");
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
