//! 真实的模型传输。**唯一出网的地方。**
//!
//! ## 分层：哪一层自己写，哪一层外包
//!
//! - **接缝层**（三段式、降级路径、`redact`、同步的 `Reasoner`）——自己写。
//!   它回答的是「我们怎么问」，是产品决策，不是协议细节。
//! - **协议层**（请求体拼装 / 响应解析 / 鉴权头 / 端点 / 错误分类）——**交给
//!   [`genai`]**。这正是 ADR-0003 的 crate 选型表划的那条线。
//!
//! ## 早先不是这样，以及为什么现在是这样
//!
//! 2026-10-12 之前这里有一层手写的协议层（≈120 行：`build_body` /
//! `extract_text` / `headers` / `chat_url`）加一个 `ureq_http`，
//! 前面还架了一个 `trait Http` 用来测。**现在三样都没了。**
//!
//! 不换的理由当时写的是许可证门禁，而**那条理由 ADR 自己已经推翻了**：
//! `webpki-roots` 实为 `CDLA-Permissive-2.0`，不是早先误记的 MPL-2.0，
//! 已在 `deny.toml` 的 allow 白名单里，不需要任何豁免。
//!
//! `trait Http` 一并删掉的理由不是「顺手」：它是**裸 JSON POST/GET**，
//! 位置比 genai 的 chat 抽象**更低**。留着它当 genai 的下一层，
//! genai 就成了摆设——请求体还是我们自己拼的，那 ADR 想省掉的 120 行
//! 一行都没省掉。
//!
//! ## 删掉 `trait Http` 之后，「传输写错了」靠什么保证
//!
//! `FakeHttp` 那三个断言（URL、鉴权头、请求体）断言的是**我们自己拼的 JSON**，
//! 而那正是被删掉的东西。现在这三件事由 `mock_upstream` 兜住：
//! **真的起一个回环端口**，让 genai 真的发一次 HTTP，把**字节**记下来
//! 再断言。见 [`crate::mock_upstream`]。
//!
//! 单元测试全绿而这条路从没走通过，这个项目已经吃过一次亏（ureq 没开
//! TLS 后端，全部单测都是绿的）。
//!
//! ## key 的处理
//!
//! key 从参数进来，**只交给 genai 的 `AuthResolver`**。它不会进日志、
//! 不进错误信息、不进任何 struct 的 `Debug` 输出。
//!
//! ⚠ **genai 的错误 `Display` 不能直接用**：`Error::ChatResponseGeneration`
//! 会把**整个请求体**印出来，而请求体里有用户的聊天内容。
//! 所以所有 genai 错误一律过 [`brief`]，它只取诊断要用的那几个字段。

use std::sync::{Arc, RwLock};
use std::time::Duration;

use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, ContentPart, MessageContent};
use genai::resolver::{AuthData, Endpoint};
use genai::{Client, ModelIden, ServiceTarget, WebConfig};

use crate::model::{Candidate, DraftSet, Intent, ReasonError, Reasoner, StageCalls, TurnContext};
use crate::vision::ImagePayload;

/// 单次请求的上限。**三段各一次**，所以一轮的上界是三倍。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// 连接上限。比请求短，因为「连不上」再等 20 秒只是在浪费用户的耐心。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// 各家模型 API 的差异都收在这一处。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// OpenAI：**OpenAI 兼容**协议。
    ///
    /// 是 [`Provider::is_openai_compatible`] 判定的基准那一家，
    /// 新接的兼容服务默认跟它对齐，差异只在端点与模型名两项数据上。
    OpenAi,
    /// Anthropic：**原生协议**，不是 OpenAI 兼容的。
    ///
    /// 鉴权头（`x-api-key` 而非 `Bearer`）、请求体里的 `system`、
    /// 响应里取文字的指针，三处都不一样。所以「加一家 provider 就是加一行」
    /// 在这里不成立，它要占一个独立 arm。
    Anthropic,
    /// MiniMax，走 **OpenAI 兼容**接口（实测 `abab6.5s-chat` 在
    /// `https://api.minimaxi.com/v1/chat/completions` 返回 HTTP 200 并正常回话）。
    ///
    /// ## ⚠ 这里**不用** genai 的 `AdapterKind::MiniMax`
    ///
    /// genai 自带一个 MiniMax 适配器，而它是 **Anthropic 兼容**协议、
    /// 打的是 `https://api.minimax.io/anthropic/v1/`——**跟我们实测能通的
    /// 那一家不是同一家**。用它等于把一个没验过的路径接上去，
    /// 症状是「配置成功、一调用就 404/401」。
    ///
    /// 所以这里显式选 `AdapterKind::OpenAI` + 端点覆盖。
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
            // 官方文档与各家接入指南给的就是 `_CN_`那个变量名。
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

    /// genai 适配器。**必须显式给，不许把模型名字符串直接丢给 genai。**
    ///
    /// ⚠ **这是个真会咬人的默认值**：genai 靠模型名前缀猜适配器
    /// （`gpt*` → OpenAI、`claude*` → Anthropic、**其余一律 → Ollama**）。
    /// 我们的模型叫 `abab6.5s-chat`，不带前缀，于是它会**静默落到 Ollama**，
    /// 打到 `http://localhost:11434` 去——症状是「配置成功、一调用就 404」。
    ///
    /// 所以 [`Provider::model_iden`] 永远用 `ModelIden::new(adapter, name)`
    /// 显式钉死。
    pub fn adapter(self) -> AdapterKind {
        match self {
            Self::OpenAi | Self::MiniMax => AdapterKind::OpenAI,
            Self::Anthropic => AdapterKind::Anthropic,
        }
    }

    /// 端点。**三家都显式写出来**，不用 genai 的内置默认值。
    ///
    /// 理由：内置默认值是**别人的数据**，而且它已经错过一次了
    /// （genai 的 MiniMax 指向 `api.minimax.io`，与我们实测的那家不同）。
    /// 端点是我们在本机实测过的，把它写成字面量，读代码的人一眼能核对。
    pub fn base_url(self) -> &'static str {
        match self {
            Self::OpenAi => "https://api.openai.com/v1/",
            Self::Anthropic => "https://api.anthropic.com/v1/",
            Self::MiniMax => "https://api.minimaxi.com/v1/",
        }
    }

    /// 组一次调用用的模型标识。**适配器写死在上面那个函数里。**
    pub fn model_iden(self, model: &str) -> ModelIden {
        ModelIden::new(self.adapter(), model)
    }

    /// 这一家**能用的模型清单**。设置页从这里取选项，Rust 侧从这里校验。
    ///
    /// ⚠ **只填实测/官方文档确认过的。** 填一个没验过的名字，表现会是
    /// 「设置成功、一调用就 404」，比明确不支持更难查。所以这份清单
    /// **故意很短**：宁可只有一两个，也不要拿一堆看着很全、其实没验过的
    /// 名字去撑场面。长度不够说明「该去实测了」，不是「该编几个」。
    pub fn models(self) -> &'static [&'static str] {
        match self {
            Self::OpenAi => &["gpt-4o-mini"],
            Self::Anthropic => &["claude-sonnet-5"],
            Self::MiniMax => &["abab6.5s-chat"],
        }
    }

    /// 默认模型 = 清单里的第一个。**判断与起草两段共用同一个**——
    /// 判断那段的输出只有几十个字，换模型换不出什么，
    /// 却是多一份费用和一份延迟。
    pub fn default_model(self) -> &'static str {
        self.models()[0]
    }

    /// 这个模型名在不在清单里。**不在就是不支持，不接受。**
    pub fn accepts_model(self, model: &str) -> bool {
        self.models().contains(&model.trim())
    }

    /// 每家的出站参数。**协议差异由适配器负责，这里只剩我们自己的取值。**
    fn chat_options(self) -> ChatOptions {
        let o = ChatOptions::default().with_temperature(0.2);
        if self.is_openai_compatible() {
            return o;
        }
        // ⚠ 只给 Anthropic 封顶。早先手写协议时也只有它带 `max_tokens`，
        // 沿用同样的口径：这一家的**接口**要求必须给，别家给不给是可选的。
        // 顺带把输出长度卡住——三段要的输出都是几十到几百字。
        o.with_max_tokens(256)
    }

    /// 测连通性用的只读端点。**不调用模型、不产生 token 费用。**
    ///
    /// 两家都有 `/v1/models`。它既不需要模型、也不花钱——
    /// 拿 `POST /messages` 去测会为了一次点按真的产生一次模型调用。
    pub fn models_url(self) -> &'static str {
        match self {
            Self::Anthropic => "https://api.anthropic.com/v1/models",
            Self::OpenAi => "https://api.openai.com/v1/models",
            Self::MiniMax => "https://api.minimaxi.com/v1/models",
        }
    }

    /// 探测端点需要的头。
    ///
    /// ⚠ **只有探测走这里。** 聊天请求的鉴权头由 genai 的适配器生成
    /// （`Authorization: Bearer` / `x-api-key` + `anthropic-version`），
    /// 那一段不归我们写——它正是这次外包出去的东西。
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

/// 「用哪个模型」的可改句柄。形状与设置层的其它旋钮一样：
/// **一份 `Arc`，命令层和推理器共用**——改完下一圈就生效，不用重启。
///
/// ## 为什么**拒收**不在清单里的名字
///
/// 这个句柄直接决定出网时 URL 里的模型名。放一个没验过的名字进去，
/// 症状是「设置成功、一调用就 404」，而那个 404 会被 `brief` 归类成
/// 「网络层/上游失败」，用户完全看不出是自己刚选了个不存在的模型。
/// 在**写入这一格**的时候拒掉，是唯一能让这件事当场可见的地方。
#[derive(Clone)]
pub struct ModelCell(Arc<RwLock<(Provider, String)>>);

impl ModelCell {
    /// 按该供应商的默认模型建一个格子。
    pub fn new(provider: Provider) -> Self {
        Self(Arc::new(RwLock::new((
            provider,
            provider.default_model().to_string(),
        ))))
    }

    /// 当前选中的模型名。
    pub fn get(&self) -> String {
        self.0.read().expect("锁").1.clone()
    }

    /// 当前是哪一家。
    pub fn provider(&self) -> Provider {
        self.0.read().expect("锁").0
    }

    /// 改模型。**返回夹取/校验后的真实生效值**——
    /// 命令层要拿它回给界面，不能拿自己传进去的字符串糊弄过去。
    ///
    /// 不在清单里就**返回错误、并且不改格子**。失败方向必须是「不变」。
    pub fn set(&self, model: &str) -> Result<String, String> {
        let m = model.trim();
        let mut g = self.0.write().expect("锁");
        if !g.0.accepts_model(m) {
            return Err(format!(
                "{} 不支持模型「{m}」。目前只列了实测能通的：{}",
                g.0.key(),
                g.0.models().join(" / ")
            ));
        }
        g.1 = m.to_string();
        Ok(g.1.clone())
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
///
/// **对外仍然是同步的**——`Reasoner` trait 是同步的，三段式、降级路径、
/// 界面上的调用计数全都建立在这一点上。同步与 async 之间那道桥在
/// [`HttpReasoner::ask`] 里的 `block_on`。
#[derive(Clone)]
pub struct HttpReasoner {
    /// 协议层。**它的鉴权头、端点、请求体、响应解析全由它负责。**
    client: Client,
    /// async → sync 的桥。`current_thread` 就够：
    /// 推理层每条新消息最多 3 次**串行**调用，引入多线程运行时
    /// 只会多背依赖、多一份线程开销，买不到任何东西。
    ///
    /// 用 `Arc` 是因为 `Runtime` 不 `Clone`，而 `HttpReasoner` 需要 `Clone`。
    rt: Arc<tokio::runtime::Runtime>,
    provider: Provider,
    /// ⚠ **这里没有 `api_key` 字段了**——早先有，是为了拼 `Authorization` 头。
    /// 协议层交给 genai 之后，key 在构造那一刻就被**移进** `AuthResolver`
    /// 的闭包里了，本结构连一份副本都不留。
    ///
    /// 少一份副本就少一处能把它印出去的地方，这是这次外包顺带买到的。
    /// [`std::fmt::Debug`] 里那一行 `api_key: <redacted>` 保留着，
    /// 是为了让「这里曾经有过一个 key」这件事留在日志里看得见。
    model: ModelCell,
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
    /// 手写：**不能** derive——derive 会把每一条字段打进 Debug 输出，
    /// 而 Debug 输出会被日志吃掉。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpReasoner")
            .field("provider", &self.provider)
            .field("model", &self.model.get())
            // key 已经不在这个结构里了（见上面那个字段的注释），
            // 仍然把这一行留着：日志里能看出「它去哪儿了」。
            .field("api_key", &"<redacted：已移交给 genai 的 AuthResolver>")
            .field("calls", &self.calls.get())
            .finish()
    }
}

impl HttpReasoner {
    /// 装好一个发真请求的推理器。**模型名取 [`Provider::default_model`]。**
    ///
    /// ## 为什么返回 `Result`
    ///
    /// 里面要起一个 tokio runtime，那是**真的会失败**的一步。
    /// 早先的版本用 `ureq` 时没有这一步，于是这个构造函数是 `fn -> Self`。
    /// 让它继续是 `fn -> Self` 就得 `expect`，而这一条路径上**没有断言**：
    /// 失败时调用方（设置层）能做的正确动作是**退回预置传输**，
    /// 不是让整个面板起不来。
    pub fn new(provider: Provider, api_key: String) -> Result<Self, String> {
        Self::assemble(
            provider,
            api_key,
            ModelCell::new(provider),
            provider.base_url().to_string(),
        )
    }

    /// 装一个**和命令层共用模型格**的推理器。
    ///
    /// 设置页改模型改的就是外面那个格子——所以推理器必须持有同一份，
    /// 而不是启动时抄一个副本进自己的字段。
    pub fn with_model_cell(
        provider: Provider,
        api_key: String,
        model: ModelCell,
    ) -> Result<Self, String> {
        Self::assemble(provider, api_key, model, provider.base_url().to_string())
    }

    /// 把端点指到别处。**只给测试用**（对着 [`crate::mock_upstream`]）。
    ///
    /// ⚠ 它是 `#[cfg(test)]`：能让调用方决定「请求发到哪个地址」的构造
    /// 函数，在生产构建里不该存在——那等于把「把 key 发到任意主机」
    /// 做成一个公开 API。
    #[cfg(test)]
    pub(crate) fn with_endpoint(
        provider: Provider,
        api_key: String,
        endpoint: String,
    ) -> Result<Self, String> {
        Self::assemble(provider, api_key, ModelCell::new(provider), endpoint)
    }

    fn assemble(
        provider: Provider,
        api_key: String,
        model: ModelCell,
        base_url: String,
    ) -> Result<Self, String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("起不出异步运行时：{e}"))?;

        // 鉴权交给 `AuthResolver`：key 从此**只活在 genai 内部**。
        // `AuthData` 的 `Debug` 自带 redact（打出来是 `AuthData::Single(REDACTED)`），
        // 但那只是兜底——对外文本一律还要过 [`redact`]。
        let auth_key = api_key.clone();
        // 端点覆盖写死在 [`Provider::base_url`]，不看 genai 的内置默认值。
        let client = Client::builder()
            .with_web_config(
                WebConfig::default()
                    .with_timeout(REQUEST_TIMEOUT)
                    .with_connect_timeout(CONNECT_TIMEOUT),
            )
            .with_auth_resolver_fn(move |_m: ModelIden| {
                Ok(Some(AuthData::from_single(auth_key.clone())))
            })
            .with_service_target_resolver_fn(move |t: ServiceTarget| {
                Ok(ServiceTarget {
                    endpoint: Endpoint::from_owned(base_url),
                    ..t
                })
            })
            .build();

        Ok(Self {
            client,
            rt: Arc::new(rt),
            provider,
            model,
            calls: std::cell::Cell::new(0),
            judge_calls: std::cell::Cell::new(0),
            draft_calls: std::cell::Cell::new(0),
            rank_calls: std::cell::Cell::new(0),
        })
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

    /// 现在用的模型名。**从共用的格子里读**，不是启动时抄的那一份。
    pub fn model(&self) -> String {
        self.model.get()
    }

    /// 这一格的句柄。命令层要靠它改模型。
    pub fn model_cell(&self) -> ModelCell {
        self.model.clone()
    }

    fn ask(
        &self,
        system: &str,
        user: &str,
        stage: Stage,
        images: &[ImagePayload],
    ) -> Result<String, String> {
        let req = build_request(system, user, images);
        let model = self.provider.model_iden(&self.model.get());
        self.calls.set(self.calls.get() + 1);
        // 分段计数。**先加再发**——发失败也算一次尝试，
        // 界面上「调用了几次」要是用户实际掏的钱，不是成功次数。
        match stage {
            Stage::Judge => self.judge_calls.set(self.judge_calls.get() + 1),
            Stage::Draft => self.draft_calls.set(self.draft_calls.get() + 1),
            Stage::Rank => self.rank_calls.set(self.rank_calls.get() + 1),
        }
        let opts = self.provider.chat_options();
        // ⚠ **这里就是 async → sync 那道桥。** 对外（`Reasoner`）仍然是同步的。
        let resp = self
            .rt
            .block_on(self.client.exec_chat(model, req, Some(&opts)))
            .map_err(|e| brief(&e))?;
        resp.first_text().map(str::to_string).ok_or_else(|| {
            // 上游契约变了。**只说结构不认识**，不带任何请求/响应正文——
            // 请求体里有用户的聊天内容，而 genai 的错误里**带着整个请求体**。
            format!("响应里没有文字（provider={}）", self.provider.key())
        })
    }

    /// 拿一段 JSON 文本。**剥掉模型爱加的代码块围栏。**
    fn ask_json(
        &self,
        system: &str,
        user: &str,
        stage: Stage,
        images: &[ImagePayload],
    ) -> Result<serde_json::Value, String> {
        let raw = self.ask(system, user, stage, images)?;
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

/// 拼一次聊天请求。**纯函数**——所以「请求里带了什么」可以脱离网络断言。
///
/// ## 图为什么排在文字**前面**
///
/// Anthropic 官方建议图放在文字之前（原文：*"Claude works best when
/// images come before text … prefer an image-then-text structure."*）。
/// OpenAI 兼容侧两种顺序都收，所以**一个顺序服务两家**——
/// 早先这里为两种协议各拼一套模板，现在这一行分支没有了，
/// 差异整个交给了适配器。
pub fn build_request(system: &str, user: &str, images: &[ImagePayload]) -> ChatRequest {
    if images.is_empty() {
        return ChatRequest::new(vec![ChatMessage::system(system), ChatMessage::user(user)]);
    }
    let mut parts: Vec<ContentPart> = images
        .iter()
        .map(|i| {
            ContentPart::from_binary_base64(
                i.media_type.clone(),
                i.base64.clone(),
                Some("对方发来的图片".to_string()),
            )
        })
        .collect();
    parts.push(ContentPart::from_text(user));
    ChatRequest::new(vec![
        ChatMessage::system(system),
        ChatMessage::user(MessageContent::from_parts(parts)),
    ])
}

impl Reasoner for HttpReasoner {
    fn judge(&mut self, ctx: &TurnContext) -> Result<Intent, ReasonError> {
        let v = self
            .ask_json(SYS_JUDGE, &Self::render_ctx(ctx), Stage::Judge, &[])
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
            .ask_json(SYS_DRAFT, &user, Stage::Draft, &ctx.images)
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
            .ask_json(SYS_RANK, &lines, Stage::Rank, &[])
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

/// 把一个 genai 错误压成**一句不含聊天内容**的话。
///
/// ## 为什么不能直接 `format!("{e}")`
///
/// `genai::Error::ChatResponseGeneration` 的 `Display` 里带
/// `Request Payload:\n{request_payload:#}`——**整个请求体**，
/// 也就是用户的聊天原文加上（图片外发打开时）base64 的图片。
/// 它还会进日志、进面板上的错误提示。那条约束比「日志好看」重要得多。
///
/// 同样地 `genai::webc::Error::ResponseFailedStatus` 的 `Display` 带上游
/// 正文，而上游的错误正文**可能把你刚发的 header 回显出来**。
///
/// 所以：**逐个变体挑字段**，一个都不整体打印。
fn brief(e: &genai::Error) -> String {
    use genai::Error as E;
    let s = match e {
        E::ChatResponseGeneration {
            model_iden, cause, ..
        } => format!("{model_iden} 的响应结构不认识：{}", truncate(cause, 200)),
        E::ChatResponse { model_iden, body } => format!(
            "{model_iden} 的响应结构不认识：{}",
            redact(&truncate(&body.to_string(), 200))
        ),
        E::WebModelCall {
            model_iden,
            webc_error,
        } => format!("{model_iden} 请求失败：{}", brief_web(webc_error)),
        E::WebAdapterCall {
            adapter_kind,
            webc_error,
        } => format!("{adapter_kind} 请求失败：{}", brief_web(webc_error)),
        E::RequiresApiKey { model_iden } => format!("{model_iden} 需要 API key"),
        E::AdapterKindMismatch {
            bound,
            requested,
            model,
        } => format!(
            "适配器错配：客户端绑定 {bound}，请求却指向 {requested}（模型 {model}）——这是我们自己的 bug"
        ),
        other => format!("模型调用失败：{}", truncate(&other.to_string(), 200)),
    };
    redact(&s)
}

/// 同上，压一个网络层错误。
fn brief_web(e: &genai::webc::Error) -> String {
    use genai::webc::Error as E;
    match e {
        E::ResponseFailedStatus { status, body, .. } => format!(
            "上游返回 {}：{}",
            status.as_u16(),
            redact(&truncate(body, 200))
        ),
        E::ResponseFailedNotJson { content_type, body } => format!(
            "上游返回的不是 JSON（{content_type}）：{}",
            redact(&truncate(body, 200))
        ),
        E::ResponseFailedInvalidJson { cause, .. } => {
            format!("上游返回的不是合法 JSON（{cause}）——上游契约可能变了")
        }
        other => format!("网络层失败：{}", truncate(&other.to_string(), 200)),
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
/// ## 为什么这一条**没有**外包给 genai
///
/// `/v1/models` 不是任何一个适配器的操作，genai 里没有它的等价物；
/// 而它自己的 `WebClient` 是 `pub(crate)`，外部拿不到。
/// 所以这里**只**这一个 GET 用了 `reqwest` 直接发——版本与 genai 内部
/// 那条一致，依赖树里不新增任何 crate。
///
/// ## 错误里的 key
///
/// 上游的错误正文**可能带上你刚发的 header**。所以整条错误都过一遍
/// [`redact`]，并且只截前 200 字符——这两步都必要。
pub fn probe_connectivity(
    provider: Provider,
    api_key: &str,
    timeout: Duration,
) -> Result<(), String> {
    probe_at(
        provider.models_url(),
        &provider.probe_headers(api_key),
        timeout,
    )
}

/// [`probe_connectivity`] 的真正实现。**把 URL 和头拆出来当参数**，
/// 是为了让「探测打的是 `/v1/models` 而不是聊天端点」这件事
/// **能对着一个真的回环端口断言**，而不是对着我们自己写的常量断言。
fn probe_at(url: &str, headers: &[(String, String)], timeout: Duration) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("起不出异步运行时：{e}"))?;
    let headers = headers.to_vec();
    rt.block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|e| redact(&format!("装不出 HTTP 客户端：{e}")))?;
        let mut req = client.get(url);
        for (k, v) in &headers {
            req = req.header(k, v);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| redact(&format!("网络层失败：{e}")))?;
        let status = resp.status();
        // ⚠ 失败时**先读正文再拼错误**：`reqwest` 的 `status()` 之后
        // 拿不到 body 了，所以顺序不能反。
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(redact(&format!(
                "上游返回 {}：{}",
                status.as_u16(),
                truncate(&body, 200)
            )));
        }
        // 能解析出 JSON 说明契约没变。反过来不检查内容：models 列表长什么样
        // 不影响「通不通」这个答案，硬要校验字段只会引入新的脆弱点。
        match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(v) if v.is_object() => Ok(()),
            _ => Err(redact("上游返回的不是 JSON 对象——端点可能变了")),
        }
    })
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
    use crate::mock_upstream::{MockUpstream, Reply};

    const KEY: &str = "sk-testkey-should-never-appear";

    fn judge_json() -> &'static str {
        r#"{"intent":"约时间","need":"确认时间","action":"reply","tension":0.3}"#
    }

    fn draft_json() -> &'static str {
        r#"{"candidates":[{"text":"今晚八点可以","probability":0.7},{"text":"我看看时间","probability":0.3}]}"#
    }

    /// OpenAI 兼容格式的回复体。**两家 OpenAI 兼容的上游都长这样。**
    fn openai_reply(text: &str) -> String {
        serde_json::json!({ "choices": [{ "message": { "content": text } }] }).to_string()
    }

    /// 造一台**真的在上收请求**的假上游。
    fn upstream(replies: Vec<Reply>) -> MockUpstream {
        MockUpstream::new(replies)
    }

    /// 三段各一条回复，按顺序发。
    fn three_replies() -> Vec<Reply> {
        vec![
            Reply(200, openai_reply(judge_json())),
            Reply(200, openai_reply(draft_json())),
            Reply(
                200,
                openai_reply(r#"{"order":["今晚八点可以","我看看时间"]}"#),
            ),
        ]
    }

    /// **一轮真的跑通：判断 → 起草 → 排序，三次真的 HTTP。**
    ///
    /// 断言的东西比早先的 `FakeHttp` 版**更多**：那时只看得见我们自己
    /// 拼的 JSON，现在看得见 genai 真的发出去的字节——路径、鉴权头、
    /// 请求体，一个都跑不掉。
    #[test]
    fn a_full_turn_over_a_real_socket() {
        let up = upstream(three_replies());
        let mut r =
            HttpReasoner::with_endpoint(Provider::OpenAi, KEY.into(), up.base_url().to_string())
                .expect("起运行时");

        let ctx = TurnContext::text(vec!["今晚有空吗".into()], "小号".into());
        let t = crate::turn(&mut r, &ctx).expect("推理成功");

        assert!(t.judged);
        assert_eq!(t.intent.intent, "约时间");
        assert_eq!(t.candidates_count(), 2);
        assert_eq!(t.recommended().unwrap().text, "今晚八点可以");
        assert_eq!(r.calls(), 3, "判断+起草+排序 = 3 次");

        // 请求形状：三次都打到 chat/completions、都带 Bearer 鉴权。
        let seen = up.seen();
        assert_eq!(seen.len(), 3);
        for s in seen.iter() {
            assert_eq!(s.method, "POST");
            assert_eq!(s.path, "/v1/chat/completions", "端点拼错了：{}", s.path);
            assert_eq!(
                s.header("authorization"),
                Some(&*format!("Bearer {KEY}")),
                "鉴权头不对"
            );
        }
        // 起草那次的请求体里必须带着聊天内容（否则模型无从回复）。
        let draft_body = &seen[1].body;
        assert!(
            draft_body.contains("今晚有空吗"),
            "起草请求没带对话：{draft_body}"
        );
        // 两次都**不该**把用户消息当成系统提示的一部分重复发一遍。
        assert!(
            seen[0].body.contains("\"system\""),
            "判断请求应当带 system 提示：{}",
            seen[0].body
        );
    }

    /// **口吻样本只进起草那一段，判断那一段不许收到。**
    ///
    /// 两个理由，少一条都不够：
    /// ① **成本**。一条新消息触发 3 次调用（判断 1 · 起草 1 · 排序 1），
    ///    判断是**最快最便宜**的那一档，却要为一份它用不上的样本付一次钱。
    /// ② **污染**。判断问的是「对方想干什么」，模型看到样本就可能理解成
    ///    「用户之前是怎么回的」，拿它去反推意图——那是凭空多的一条推理路径。
    ///
    /// ⚠ 断言的**形状变了**：早先读的是我们自己拼的请求体，
    /// 现在读的是**真的发出去的字节**。性质没变，证据更硬。
    #[test]
    fn voice_samples_reach_the_draft_but_not_the_judge() {
        let up = upstream(three_replies());
        let mut r =
            HttpReasoner::with_endpoint(Provider::OpenAi, KEY.into(), up.base_url().to_string())
                .expect("起运行时");

        let ctx = TurnContext::text(vec!["今晚有空吗".into()], "小号".into())
            .with_voice(vec!["嗯我看看时间".into(), "行".into()]);
        crate::turn(&mut r, &ctx).expect("推理成功");

        let seen = up.seen();
        let judge_body = &seen[0].body;
        let draft_body = &seen[1].body;

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

    /// **Anthropic 走的是它自己的路径、自己的鉴权头。**
    ///
    /// 早先这条断言读的是我们自己拼的头；现在读**真的发出去的字节**。
    /// 这条比原来更要紧：适配器一旦选错，症状是「静默打错人」，
    /// 而 `MockUpstream` 收不到请求时**一定会红**。
    #[test]
    fn anthropic_hits_its_own_endpoint_and_header() {
        let up = upstream(vec![Reply(
            200,
            serde_json::json!({
                "content": [{ "type": "text", "text": judge_json() }],
                "model": "claude-sonnet-5",
                "role": "assistant"
            })
            .to_string(),
        )]);
        let mut r =
            HttpReasoner::with_endpoint(Provider::Anthropic, KEY.into(), up.base_url().to_string())
                .expect("起运行时");

        let ctx = TurnContext::text(vec!["今晚有空吗".into()], "小号".into());
        r.judge(&ctx).expect("判断应当成功");

        let seen = up.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].path, "/v1/messages",
            "Anthropic 不是 chat/completions"
        );
        assert_eq!(seen[0].header("x-api-key"), Some(KEY));
        assert_eq!(
            seen[0].header("anthropic-version"),
            Some("2023-06-01"),
            "缺版本头，上游会拒"
        );
        assert!(
            seen[0].header("authorization").is_none(),
            "Anthropic 不该带 Bearer：{:?}",
            seen[0].headers
        );
    }

    /// **适配器必须显式钉死。**
    ///
    /// `abab6.5s-chat` 不带任何前缀——直接把它当字符串丢给 genai，
    /// genai 会按「其余 → Ollama」处理，打到 `localhost:11434` 去。
    /// 症状是「配置成功、一调用就 404」，所以这里钉死成断言。
    #[test]
    fn the_adapter_is_pinned_not_guessed_from_the_model_name() {
        use genai::adapter::AdapterKind as K;
        assert_eq!(Provider::MiniMax.adapter(), K::OpenAI);
        assert_eq!(Provider::OpenAi.adapter(), K::OpenAI);
        assert_eq!(Provider::Anthropic.adapter(), K::Anthropic);
        // 反证：裸传模型名**真的会**落到 Ollama。
        assert_eq!(K::from_model("abab6.5s-chat").expect("认得"), K::Ollama);
        // 所以我们走的每一条路都必须是显式 ModelIden。
        assert_eq!(
            Provider::MiniMax.model_iden("abab6.5s-chat").adapter_kind,
            K::OpenAI
        );
    }

    /// **静默期之外的一切都在这里**：一次调用 = 一次 HTTP。
    /// 上游报 401 时，错误里必须留着状态码（那是唯一有用的信息），
    /// 而**绝不能**留着 key。
    #[test]
    fn an_upstream_error_keeps_the_status_and_drops_the_key() {
        let up = upstream(vec![Reply(401, format!("invalid api key {KEY}"))]);
        let mut r =
            HttpReasoner::with_endpoint(Provider::OpenAi, KEY.into(), up.base_url().to_string())
                .expect("起运行时");

        let ctx = TurnContext::text(vec!["今晚有空吗".into()], "小号".into());
        let e = r.judge(&ctx).expect_err("401 应当失败").to_string();
        assert!(!e.contains(KEY), "错误泄了 key：{e}");
        assert!(e.contains("401"), "状态码要留着，那才是有用的信息：{e}");
    }

    /// **聊天原文不许随错误出去。**
    ///
    /// 这条比「key 不外泄」更要紧：key 泄了换一把，聊天原文泄了是
    /// 别人的私聊进了日志和面板上的错误提示。
    ///
    /// ## 为什么直接构造错误而不是走网络
    ///
    /// `genai::Error::ChatResponseGeneration` 的 `Display` 里带着
    /// `Request Payload:\n{request_payload:#}`——整个请求体。`brief()` 逐个
    /// 变体挑字段就是为了绕开它，而**绕开与否只有对着这个变体断言才算数**。
    /// 走网络的话要先把响应伪造成「结构不认识」才能碰巧走到这个变体——
    /// 上游哪天改了错误形状，那条测试就悄悄测不到它了。
    #[test]
    fn the_chat_text_never_rides_along_on_an_error() {
        use genai::adapter::AdapterKind;
        let canary = "对方说了一件很私密的事";
        let e = genai::Error::ChatResponseGeneration {
            model_iden: genai::ModelIden::new(AdapterKind::OpenAI, "gpt-x"),
            request_payload: Box::new(serde_json::json!({
                "messages": [{"role": "user", "content": canary}]
            })),
            response_body: Box::new(serde_json::json!({})),
            cause: "响应里没有 choices".into(),
        };
        let s = brief(&e);
        assert!(!s.contains(canary), "错误文本带出了聊天原文：{s}");
        assert!(
            s.contains("choices") || s.contains("不认识"),
            "原因要留着，那才是有用的信息：{s}"
        );
    }

    /// **`brief()` 的兜底分支不能成为漏点。**
    ///
    /// `genai::Error` **不是** `#[non_exhaustive]`，所以 `brief()` 里的
    /// `other =>` 会静默接住将来 genai 新增的任何变体——编译通过，
    /// 而如果那个变体的 `Display` 带请求体，聊天原文就出去了。
    ///
    /// 这里给兜底分支喂一个「最坏情况」：错误原因里就带着原文。
    /// 兜底必须靠 `brief_web` 逐字段挑，而不是整体 `to_string()`——
    /// 所以这里断言的是**输出被压到很短**，不是「原文一定消失」
    /// （原文是上游给的原因的一部分，压掉它反而会让人查不出问题）。
    #[test]
    fn the_fallback_branch_of_brief_also_truncates() {
        use genai::adapter::AdapterKind;
        let e = genai::Error::WebModelCall {
            model_iden: genai::ModelIden::new(AdapterKind::OpenAI, "gpt-x"),
            webc_error: genai::webc::Error::ResponseFailedInvalidJson {
                body: "很长".repeat(500),
                cause: "解析失败".into(),
            },
        };
        let s = brief(&e);
        assert!(!s.is_empty(), "兜底分支也得给一句话出来");
        assert!(s.len() < 400, "兜底把整段错误原样打印了：{s}");
    }

    /// **图排在文字前面**，而且确实进了请求体。
    ///
    /// Anthropic 官方建议就是这个顺序；OpenAI 兼容侧两种都收，
    /// 所以**一个顺序服务两家**。
    #[test]
    fn images_go_into_the_request_before_the_text() {
        let img = ImagePayload {
            media_type: "image/png".into(),
            base64: "AAAA".into(),
            tokens: 0,
            width: 0,
            height: 0,
        };
        let req = build_request("系统", "用户那句话", std::slice::from_ref(&img));
        let json = serde_json::to_string(&req).expect("请求体编得出来");
        let img_at = json.find("image/png").expect("图应当在里面");
        let text_at = json.find("用户那句话").expect("文字应当在里面");
        assert!(
            img_at < text_at,
            "图必须排在文字前面（Anthropic 官方建议）：{json}"
        );
    }

    /// 没有图的时候**不要**发一个空的多段结构——
    /// 那是多一段 JSON 包一层，模型看得见，用户看不懂。
    #[test]
    fn no_images_means_a_plain_text_message() {
        let req = build_request("系统", "用户那句话", &[]);
        let json = serde_json::to_string(&req).expect("请求体编得出来");
        assert!(json.contains("用户那句话"));
        assert!(
            !json.contains("image"),
            "没有图就不该出现任何图相关的字段：{json}"
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
        let r = HttpReasoner::new(Provider::OpenAi, KEY.into()).expect("起运行时");
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
        for p in [Provider::OpenAi, Provider::Anthropic, Provider::MiniMax] {
            let url = p.models_url();
            assert!(
                url.ends_with("/v1/models"),
                "探测打到了 {url} —— 调模型是要花钱的"
            );
            assert!(
                !url.contains("/messages"),
                "探测不该打 messages 端点：{url}"
            );
            assert!(
                !url.contains("/chat/completions"),
                "探测不该打聊天端点：{url}"
            );
        }
    }

    /// 探测**真的**只发一个 GET 到 `/v1/models`，而且带对鉴权头。
    ///
    /// ⚠ 这是**真的 socket**，不是假 `Http`：早先那条断言只能证明
    /// 「我们调了自己的函数，参数长这样」，证明不了「线上真发了这个」。
    #[test]
    fn probing_over_a_real_socket_sends_one_get_to_models() {
        let up = upstream(vec![Reply(200, r#"{"data":[]}"#.into())]);
        probe_at(
            &format!("{}models", up.base_url()),
            &Provider::OpenAi.probe_headers(KEY),
            Duration::from_secs(5),
        )
        .expect("假上游应当通");

        let seen = up.seen();
        assert_eq!(seen.len(), 1, "探测应当只发一次");
        assert_eq!(seen[0].method, "GET", "探测是只读的");
        assert_eq!(seen[0].path, "/v1/models");
        assert_eq!(
            seen[0].header("authorization"),
            Some(&*format!("Bearer {KEY}"))
        );
    }

    /// **探测的错误里不能有 key。** 上游的错误正文可能带上刚发的 header。
    ///
    /// ⚠ 这次是真的：假上游**把 key 原样写回错误正文**，
    /// 模拟最坏情况，而错误是从真的响应里读出来的。
    #[test]
    fn a_probe_error_never_contains_the_key() {
        let up = upstream(vec![Reply(401, format!("invalid api key {KEY}"))]);
        let e = probe_at(
            &format!("{}models", up.base_url()),
            &Provider::OpenAi.probe_headers(KEY),
            Duration::from_secs(5),
        )
        .expect_err("401 应当失败");
        assert!(!e.contains(KEY), "错误泄了 key：{e}");
        assert!(e.contains("401"), "状态码要留着，那才是有用的信息：{e}");
    }

    /// Anthropic 的探测走 `x-api-key`，不是 `Bearer`。
    #[test]
    fn probing_anthropic_uses_its_own_header() {
        let up = upstream(vec![Reply(200, r#"{"data":[]}"#.into())]);
        probe_at(
            &format!("{}models", up.base_url()),
            &Provider::Anthropic.probe_headers(KEY),
            Duration::from_secs(5),
        )
        .expect("假上游应当通");
        let seen = up.seen();
        assert_eq!(seen[0].header("x-api-key"), Some(KEY));
        assert!(
            seen[0].header("authorization").is_none(),
            "Anthropic 探测不该带 Bearer"
        );
    }

    /// **只列实测能通的模型，而且拒收清单外的名字。**
    ///
    /// 这条是「不要编造没验过的模型名」的可执行版本：放一个没验过的名字进去，
    /// 症状是「设置成功、一调用就 404」，而 404 会被归类成上游失败，
    /// 用户完全看不出是自己选了个不存在的模型。
    #[test]
    fn only_verified_models_are_accepted() {
        for p in [Provider::OpenAi, Provider::Anthropic, Provider::MiniMax] {
            assert!(!p.models().is_empty(), "{} 一条都没列", p.key());
            for m in p.models() {
                assert!(p.accepts_model(m), "{} 自己列的 {m} 却不被接受", p.key());
            }
            assert!(!p.accepts_model("no-such-model"));
            assert!(!p.accepts_model(""));
        }
    }

    #[test]
    fn the_model_cell_takes_effect_without_a_restart() {
        let cell = ModelCell::new(Provider::OpenAi);
        let r = HttpReasoner::with_model_cell(Provider::OpenAi, KEY.into(), cell.clone())
            .expect("起运行时");
        assert_eq!(r.model(), Provider::OpenAi.default_model());

        let applied = cell
            .set(Provider::OpenAi.models()[0])
            .expect("清单内的应当收");
        assert_eq!(applied, Provider::OpenAi.default_model());
        assert_eq!(r.model(), applied, "改完下一圈就该用新的");

        // 拒收清单外的名字，**而且格子不许变**。
        assert!(cell.set("gpt-9-ultra").is_err());
        assert_eq!(cell.get(), applied, "拒收之后格子必须保持原值");
    }

    #[test]
    fn anthropic_uses_its_own_header() {
        let hs = Provider::Anthropic.probe_headers("k");
        assert!(hs.iter().any(|(k, v)| k == "x-api-key" && v == "k"));
        assert!(
            hs.iter().any(|(k, _)| k == "anthropic-version"),
            "缺版本头，上游会拒"
        );
        assert!(!hs.iter().any(|(k, _)| k == "Authorization"));
    }

    #[test]
    fn openai_uses_bearer() {
        let hs = Provider::OpenAi.probe_headers("k");
        assert!(
            hs.iter()
                .any(|(k, v)| k == "Authorization" && v == "Bearer k")
        );
    }

    /// **端点是字面量，因为它是我们在本机实测过的。**
    /// genai 的内置默认值错过一次（MiniMax），所以这里逐个钉住。
    #[test]
    fn the_endpoints_are_our_own_measured_literals() {
        assert_eq!(Provider::OpenAi.base_url(), "https://api.openai.com/v1/");
        assert_eq!(
            Provider::Anthropic.base_url(),
            "https://api.anthropic.com/v1/"
        );
        // ⚠ 中国区端点。genai 自带的 MiniMax 适配器指向 api.minimax.io，
        // 那是另一家、另一种协议，见 `Provider::MiniMax` 的文档。
        assert_eq!(Provider::MiniMax.base_url(), "https://api.minimaxi.com/v1/");
    }
}
