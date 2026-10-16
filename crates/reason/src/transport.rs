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
//! ## 端点、协议与代理（2026-10-15，票 43）
//!
//! 早先这里是「三家硬编码端点 + 一把共用 key」。现在是三层：
//!
//! - [`Provider`]：**预设**（厂商自带端点与模型名）。
//! - [`Route`]：**去哪儿、用什么协议**。预设走 [`Route::preset`]，
//!   「OpenAI 兼容但我们没收录的服务」走 [`Route::custom`]——
//!   协议与厂商在这里解耦。
//! - [`ProxyCell`]：**网络层**的代理。它不是任何一家的属性，
//!   所以对所有出网请求只填一次。
//!
//! 同一把 key 不再是全局一份：凭据目标按 [`ModelGroup`] 分组，
//! 见 `ui/src-tauri/src/secret.rs`。
//!
//! ## key 的处理
//!
//! key 从参数进来，**只交给 genai 的 `AuthResolver`**。它不会进日志、
//! 不进错误信息、不进任何 struct 的 `Debug` 输出。
//!
//! **genai 的错误 `Display` 不能直接用**：`Error::ChatResponseGeneration`
//! 会把**整个请求体**印出来，而请求体里有用户的聊天内容。
//! 所以所有 genai 错误一律过 [`brief`]，它只取诊断要用的那几个字段。
//!
//! [`redact`] 挡的是 **key 形状**（`sk-` 开头的 token）。
//! **端点与代理地址里的凭据**（`user:pass@`、`?access_token=`）由
//! `ui/src-tauri/src/secret.rs` 的 `redact` 负责——同一个函数，
//! 两边的调用点都在各自模块的对外文本上。

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

/// 两组模型配置里的**哪一组**。
///
/// ## 为什么必须拆成两组
///
/// 用户要的是「1 个决策模型的配置 + 1 个 LLM 的」（票 43）。合并成一个
/// 供应商列表时，「视觉/判断走 A、文字走 B」这件事**在数据结构上就表达不了**——
/// 不是界面上少一个下拉框，是那份列表只有一格模型名。
///
/// 两组互相独立，可以指向同一家，也可以一家云端一家本地。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelGroup {
    /// ① 决策模型：判断对方意图的那一段（`Reasoner::judge`）。
    ///
    /// 这一段的输出只有几十个字，通常配一个**便宜**的模型就够。
    Decision,
    /// ② LLM：起草与排序那两段（`Reasoner::draft` / `Reasoner::rank`）。
    ///
    /// 图片（如果外发闸开着）跟着这一段走——见 [`crate::seam`]。
    Llm,
}

impl ModelGroup {
    /// 两组，顺序就是设置页里的呈现顺序。
    pub const ALL: [Self; 2] = [Self::Decision, Self::Llm];

    /// 配置与凭据目标名里用的短名。**稳定标识，不是显示名。**
    pub fn key(self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Llm => "llm",
        }
    }

    /// 反过来。由 [`ModelGroup::key`] 的短名找回枚举值。
    ///
    /// 认不出来返回 `None`——**不猜**。短名会从设置页、凭据目标名、
    /// 命令行多处进来，静默兜一个默认值只会把错带到出网那一刻。
    pub fn from_key(key: &str) -> Option<Self> {
        match key.trim().to_ascii_lowercase().as_str() {
            "decision" => Some(Self::Decision),
            "llm" => Some(Self::Llm),
            _ => None,
        }
    }

    /// 界面上那一节的名称。
    pub fn label(self) -> &'static str {
        match self {
            Self::Decision => "决策模型",
            Self::Llm => "回复模型",
        }
    }
}

/// **协议**。与厂商解耦——「OpenAI 兼容」是一种协议，不是一家供应商。
///
/// ## 为什么不是一家供应商
///
/// 绝大多数第三方服务（DeepSeek / 通义 / 智谱 / 本地推理…）只兼容
/// OpenAI 那**一种**协议。把「OpenAI 兼容」做成枚举里的一项，就等于把
/// 协议和厂商绑死，用户想接一个我们没收录的服务只能等我们发版。
/// 拆成两个维度之后，「预设 = 厂商 + 端点」与「自定义 = 协议 + 自己填的端点」
/// 各自成立。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// OpenAI 兼容：请求体 / `Bearer` 鉴权 / `choices[].message.content`。
    OpenAiCompatible,
    /// Anthropic 原生：`x-api-key` + `system` 段 + `content[]`。
    Anthropic,
}

impl Protocol {
    /// 配置与设置页里的短名。
    pub fn key(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai-compatible",
            Self::Anthropic => "anthropic",
        }
    }

    /// 由短名找回枚举值。认不出来返回 `None`。
    pub fn from_key(key: &str) -> Option<Self> {
        match key.trim().to_ascii_lowercase().as_str() {
            "openai-compatible" | "openai_compatible" | "openai" => Some(Self::OpenAiCompatible),
            "anthropic" => Some(Self::Anthropic),
            _ => None,
        }
    }

    /// 界面上那一项的名称。
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "OpenAI 兼容",
            Self::Anthropic => "Anthropic 原生",
        }
    }

    /// genai 适配器。**必须显式给，不许把模型名字符串直接丢给 genai。**
    ///
    /// **这是个真会咬人的默认值**：genai 靠模型名前缀猜适配器
    /// （`gpt*` → OpenAI、`claude*` → Anthropic、**其余一律 → Ollama**）。
    /// 我们的模型叫 `abab6.5s-chat`，不带前缀，于是它会**静默落到 Ollama**，
    /// 打到 `http://localhost:11434` 去——症状是「配置成功、一调用就 404」。
    ///
    /// 所以每次调用都用 `ModelIden::new(adapter, name)` 显式钉死。
    pub fn adapter(self) -> AdapterKind {
        match self {
            Self::OpenAiCompatible => AdapterKind::OpenAI,
            Self::Anthropic => AdapterKind::Anthropic,
        }
    }

    /// 是不是 OpenAI 兼容协议。**分协议的地方都判这一个函数。**
    pub fn is_openai_compatible(self) -> bool {
        matches!(self, Self::OpenAiCompatible)
    }

    /// 出站参数。**协议差异由适配器负责，这里只剩我们自己的取值。**
    pub fn chat_options(self) -> ChatOptions {
        let o = ChatOptions::default().with_temperature(0.2);
        if self.is_openai_compatible() {
            return o;
        }
        // 只给 Anthropic 封顶。早先手写协议时也只有它带 `max_tokens`，
        // 沿用同样的口径：这一家的**接口**要求必须给，别家给不给是可选的。
        // 顺带把输出长度卡住——三段要的输出都是几十到几百字。
        o.with_max_tokens(256)
    }

    /// 探测端点需要的头。
    ///
    /// **只有探测走这里。** 聊天请求的鉴权头由 genai 的适配器生成
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

/// 一家模型服务。**「预设」那一类**：自带端点、模型名与协议。
///
/// ## 端点常量的来源（2026-10-15，票 43）
///
/// 这个文件原先只有三家，且三家的端点都是**本机实测过**的。
/// 新增的这几家**没有在本机实测**——加它们的那天没有网络、也没有任何一家的
/// 凭据。所以每条都标了来源，并在这里集中说一句：**下面标「文档」的
/// 端点与模型名只是抄自官方文档，没在本机跑通过**。它们不该被当成
/// 「实测能通」——判定看 [`Provider::measured`]，界面上也是它决定要不要
/// 标「未核实」。
///
/// 抄错的代价是「设置成功、一调用就 404」，所以宁可短、宁可标出来，
/// 也不要看起来很全。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// OpenAI：**OpenAI 兼容**协议。端点与 `gpt-4o-mini` **本机实测过**。
    OpenAi,
    /// Anthropic：**原生协议**，不是 OpenAI 兼容的。
    ///
    /// 鉴权头（`x-api-key` 而非 `Bearer`）、请求体里的 `system`、
    /// 响应里取文字的指针，三处都不一样。
    Anthropic,
    /// MiniMax，走 **OpenAI 兼容**接口（实测 `abab6.5s-chat` 在
    /// `https://api.minimaxi.com/v1/chat/completions` 返回 HTTP 200 并正常回话）。
    ///
    /// ## 这里**不用** genai 的 `AdapterKind::MiniMax`
    ///
    /// genai 自带一个 MiniMax 适配器，而它是 **Anthropic 兼容**协议、
    /// 打的是 `https://api.minimax.io/anthropic/v1/`——**跟我们实测能通的
    /// 那一家不是同一家**。用它等于把一个没验过的路径接上去，
    /// 症状是「配置成功、一调用就 404/401」。
    ///
    /// 所以这里显式选 `AdapterKind::OpenAI` + 端点覆盖。
    MiniMax,
    /// DeepSeek。OpenAI 兼容协议。
    ///
    /// 端点来源：DeepSeek 官方 API 文档（`api-docs.deepseek.com`）的
    /// 「OpenAI 兼容」段。**未在本机实测。**
    DeepSeek,
    /// 阿里通义千问（DashScope / 百炼）。OpenAI 兼容协议。
    ///
    /// 端点来源：阿里云百炼的「OpenAI 兼容」接入文档。
    /// 注意路径是 `compatible-mode/v1/`，**不是** `v1/`——
    /// 少这一段就是 404。**未在本机实测。**
    Qwen,
    /// 智谱 GLM（BigModel）。OpenAI 兼容协议。
    ///
    /// 端点来源：智谱开放平台的「OpenAI SDK 兼容」文档。
    /// 路径是 `api/paas/v4/`，版本号是 `v4` 而非 `v1`。**未在本机实测。**
    Zhipu,
    /// 本机 Ollama。**走它的 OpenAI 兼容接口**（`/v1`）。
    ///
    /// ## 为什么不用 genai 的 `AdapterKind::Ollama`
    ///
    /// genai 自带 Ollama 适配器，走的是 Ollama 的**原生** `/api/chat`。
    /// 那条路我们没验过；而 `/v1` 是 Ollama 官方文档明确提供的兼容入口，
    /// 协议与其它 OpenAI 兼容服务一致，少一条要验的路。
    ///
    /// 端点来源：Ollama 官方文档的 OpenAI 兼容段；`11434` 是它的默认端口。
    /// **未在本机实测（本机没装）。**
    Ollama,
    /// 本机 LM Studio。**走它的 OpenAI 兼容接口**（`/v1`）。
    ///
    /// 端点来源：LM Studio 官方文档的本地服务器段；`1234` 是默认端口。
    /// **未在本机实测（本机没装）。**
    LmStudio,
    /// **自定义**：协议自己选，端点自己填。
    ///
    /// 它不是「又一家供应商」，而是**预设填不了的那一格**。
    /// 任何只兼容 OpenAI 或 Anthropic 那两种协议的服务都能接进来，
    /// 不必等我们发版。
    Custom,
}

impl Provider {
    /// 设置页按这个列表渲染「预设」。**不含 `Custom`**——
    /// 自定义不是一家供应商，它是「自己填」那个动作。
    pub const PRESETS: [Self; 8] = [
        Self::OpenAi,
        Self::Anthropic,
        Self::MiniMax,
        Self::DeepSeek,
        Self::Qwen,
        Self::Zhipu,
        Self::Ollama,
        Self::LmStudio,
    ];

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
            Self::DeepSeek => "deepseek",
            Self::Qwen => "qwen",
            Self::Zhipu => "zhipu",
            Self::Ollama => "ollama",
            Self::LmStudio => "lmstudio",
            Self::Custom => "custom",
        }
    }

    /// 界面上显示的名字。`key` 是给配置与凭据目标用的稳定标识。
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::MiniMax => "MiniMax",
            Self::DeepSeek => "DeepSeek",
            Self::Qwen => "通义千问",
            Self::Zhipu => "智谱 GLM",
            Self::Ollama => "Ollama（本机）",
            Self::LmStudio => "LM Studio（本机）",
            Self::Custom => "自定义端点",
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
            "deepseek" => Some(Self::DeepSeek),
            "qwen" | "dashscope" | "tongyi" => Some(Self::Qwen),
            "zhipu" | "glm" | "bigmodel" => Some(Self::Zhipu),
            "ollama" => Some(Self::Ollama),
            "lmstudio" | "lm-studio" => Some(Self::LmStudio),
            "custom" => Some(Self::Custom),
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
    /// 抄文档端点的那几家（见文件头）**只给通用写法**：它们的变量名
    /// 我们没有核实过，而一个**猜错的变量名**比没有更糟——它会让用户
    /// 以为自己配好了，而程序读的是另一个地方。
    ///
    /// `Custom` 那一条是**我们自己的约定**，不是任何服务商的：
    /// 自定义端点背后的服务五花八门，唯一稳定的只有这个约定名。
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
            Self::DeepSeek => &["DEEPSEEK_API_KEY"],
            Self::Qwen => &["DASHSCOPE_API_KEY"],
            Self::Zhipu => &["ZHIPU_API_KEY"],
            // 本机两个一般不配 key；给了也只是省一次输入。
            Self::Ollama => &["OLLAMA_API_KEY"],
            Self::LmStudio => &["LMSTUDIO_API_KEY"],
            Self::Custom => &["CUSTOM_API_KEY"],
        }
    }

    /// 这一家走哪种协议。`Custom` 的协议由用户选，不走这个函数。
    pub fn protocol(self) -> Protocol {
        match self {
            Self::Anthropic => Protocol::Anthropic,
            _ => Protocol::OpenAiCompatible,
        }
    }

    /// 是不是走 **OpenAI 兼容**协议（请求体 / 鉴权 / 响应结构全一致）。
    ///
    /// 本文件里所有「分协议」的地方都判这个，**而不是**逐个列 variant。
    /// 少写一个 arm 就少一处「新 provider 忘了改这里」的坑。
    pub fn is_openai_compatible(self) -> bool {
        self.protocol().is_openai_compatible()
    }

    /// genai 适配器。等价于 `self.protocol().adapter()`，留在这一层是为了
    /// 读代码时不必先跳进 [`Protocol`]。
    pub fn adapter(self) -> AdapterKind {
        self.protocol().adapter()
    }

    /// 端点。**`Custom` 没有预设端点**——它就是让用户自己填的那一格，
    /// 这里返回 `None` 而不是编一个。
    ///
    /// 理由：内置默认值是**别人的数据**，而且 genai 的内置值已经错过一次了
    /// （它的 MiniMax 指向 `api.minimax.io`，与我们实测的那家不同）。
    /// 自己写成字面量，读代码的人一眼能核对——外加本文件头那段
    /// 「哪些实测过、哪些抄自文档」的交代。
    pub fn preset_base_url(self) -> Option<&'static str> {
        Some(match self {
            Self::OpenAi => "https://api.openai.com/v1/",
            Self::Anthropic => "https://api.anthropic.com/v1/",
            // 中国区端点。genai 自带的 MiniMax 适配器指向 api.minimax.io，
            // 那是另一家、另一种协议，见上面 `MiniMax` 的文档。
            Self::MiniMax => "https://api.minimaxi.com/v1/",
            // ↓↓↓ 以下三家的端点抄自官方文档，**未在本机实测**。见文件头。
            Self::DeepSeek => "https://api.deepseek.com/v1/",
            Self::Qwen => "https://dashscope.aliyuncs.com/compatible-mode/v1/",
            Self::Zhipu => "https://open.bigmodel.cn/api/paas/v4/",
            // ↓↓↓ 本机两个，同样**未实测**（本机没装）。
            Self::Ollama => "http://localhost:11434/v1/",
            Self::LmStudio => "http://localhost:1234/v1/",
            Self::Custom => return None,
        })
    }

    /// 端点。预设用 [`Provider::preset_base_url`]；`Custom` 恒返回 `None`。
    pub fn base_url(self) -> Option<&'static str> {
        self.preset_base_url()
    }

    /// 组一次调用用的模型标识。**适配器来自协议，不来自模型名。**
    pub fn model_iden(self, model: &str) -> ModelIden {
        ModelIden::new(self.adapter(), model)
    }

    /// 这一家**能用的模型清单**。设置页从这里取选项。
    ///
    /// **实测过的三家非常短**（各一个）：填一个没验过的名字，表现会是
    /// 「设置成功、一调用就 404」，比明确不支持更难查。
    ///
    /// **本地两个与 `Custom` 是空的**：模型名由用户自己装了什么决定，
    /// 我们无从列举。空清单**不是没做完**，是「没有权威清单可抄」。
    pub fn models(self) -> &'static [&'static str] {
        match self {
            Self::OpenAi => &["gpt-4o-mini"],
            Self::Anthropic => &["claude-sonnet-5"],
            Self::MiniMax => &["abab6.5s-chat"],
            // ↓↓ 抄自文档，未实测；清单只是给界面一个候选，不构成「验过」。
            Self::DeepSeek => &["deepseek-chat"],
            Self::Qwen => &["qwen-plus"],
            Self::Zhipu => &["glm-4-flash"],
            Self::Ollama | Self::LmStudio | Self::Custom => &[],
        }
    }

    /// **这一家的端点与模型名是否在本机实测过。**
    ///
    /// 界面用它决定要不要标「未核实」；[`Provider::accepts_model`] 用它
    /// 决定要不要**拒收清单外的名字**。
    pub fn measured(self) -> bool {
        matches!(self, Self::OpenAi | Self::Anthropic | Self::MiniMax)
    }

    /// 默认模型 = 清单里的第一个。**空清单返回空串**——
    /// 那几家（本地与自定义）的模型名只能由用户给，此时「没有默认」是如实的。
    pub fn default_model(self) -> &'static str {
        self.models().first().copied().unwrap_or("")
    }

    /// 这个模型名收不收。
    ///
    /// ## 两种口径，界线是 [`Provider::measured`]
    ///
    /// - **实测过的三家**：不在清单里就是不支持，**当场拒收**。
    ///   放行的话症状是「设置成功、一调用就 404」，而那个 404 会被归类成
    ///   「网络层/上游失败」，用户完全看不出是自己刚选了个不存在的模型。
    /// - **其余各家**：我们没有核实过的清单，**拒收任何非空名字都是装的**——
    ///   那只会让本地模型和自定义端点一个都用不了。所以这里只挡空串。
    ///
    /// 代价是诚实的：没核实的那几家，真填错了仍然是 404。
    /// 那正是界面必须标「未核实」的原因。
    pub fn accepts_model(self, model: &str) -> bool {
        let m = model.trim();
        if m.is_empty() {
            return false;
        }
        if !self.measured() {
            return true;
        }
        self.models().contains(&m)
    }
}

/// 一次调用**去哪儿、用什么协议**。**厂商与协议解耦后的那一个值。**
///
/// 预设是 [`Route::preset`]，自定义是 [`Route::custom`]；
/// 出网时 [`HttpReasoner`] 只认这一个值，不再回头看 [`Provider`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    provider: Provider,
    protocol: Protocol,
    /// 归一化过：**末尾一定带 `/`**。见 [`Route::custom`]。
    base_url: String,
}

impl Route {
    /// 预设：厂商自带的端点与协议。
    ///
    /// `Custom` 没有预设端点 ⇒ 这里返回错误，**不给兜底**。
    /// 兜一个 OpenAI 端点会让用户以为自己接到别处去了。
    pub fn preset(provider: Provider) -> Result<Self, String> {
        let Some(base_url) = provider.preset_base_url() else {
            return Err(format!(
                "「{}」没有预设端点——它就是让用户自己填地址的那一项",
                provider.label()
            ));
        };
        Ok(Self {
            provider,
            protocol: provider.protocol(),
            base_url: base_url.to_string(),
        })
    }

    /// 自定义：协议 + 自己填的端点。
    ///
    /// ## 这里校验什么
    ///
    /// 1. **不是空串**。
    /// 2. **能解析成 URL**，且 scheme 是 `http` / `https`——
    ///    `file://` 那类地址对一次 HTTP 请求没有意义。
    /// 3. **有 host**（`https:///` 这种解析得出来、连不上）。
    ///
    /// ## 归一化：末尾一定补一个 `/`
    ///
    /// genai 的 OpenAI 适配器是拿 `base_url.join("chat/completions")`
    /// 拼 URL 的（`adapter_shared.rs` 的 `util_get_service_url`）。
    /// URL join 的语义是**按 `/` 分段替换最后一段**，所以
    /// `https://x.example.com/v1` + `chat/completions`
    /// = `https://x.example.com/chat/completions`——**`v1` 被整段吃掉**。
    /// 用户少打一个斜杠的代价是 404，而且是那种看不出所以然的 404。
    ///
    /// 查询参数**保留**（`?access_token=…` 这类端点是真实存在的），
    /// 片段**丢掉**（`#…` 不会发给服务器，留着只会让日志里的地址变形）。
    ///
    /// ## 端点会把请求带去用户指定的任意地址
    ///
    /// 这一格连同 key 一起决定了「对方的聊天内容发到哪儿」。
    /// 所以：**界面上必须明说「请求会发到这里」**，而错误文本一律过
    /// [`redact`]（端点里的 `user:pass@` 与 `?token=` 会被抹掉）。
    pub fn custom(protocol: Protocol, base_url: &str) -> Result<Self, String> {
        let raw = base_url.trim();
        if raw.is_empty() {
            return Err("端点不能为空".to_string());
        }
        // **先按用户写的字面判「有没有主机」，再交给 URL 解析器。**
        //
        // 顺序反了会踩一个很贵的坑：URL 解析器把 `https:///v1` **解析成功**，
        // 并把 `v1` 认成主机名、归一化成 `https://v1/`。那是把用户的 key
        // 发到一个他没写过的域名去，而界面上显示的还是他填的那个地址。
        // 本机实测（2026-10-15）：`Url::parse("https:///v1")` ⇒
        // `Ok("https://v1/")`。
        let Some((_, after_scheme)) = raw.split_once("://") else {
            return Err(format!("端点得写成 https://主机/… 这样：{raw}"));
        };
        if after_scheme.is_empty() || after_scheme.starts_with('/') {
            return Err(format!("端点里没有主机名：{raw}"));
        }
        let mut u =
            reqwest::Url::parse(raw).map_err(|e| format!("这不像一个网址：{raw}（{e}）"))?;
        if !matches!(u.scheme(), "http" | "https") {
            return Err(format!(
                "端点只能是 http 或 https，现在这一条是 {}://",
                u.scheme()
            ));
        }
        if u.host_str().is_none_or(str::is_empty) {
            return Err(format!("端点里没有主机名：{raw}"));
        }
        let mut path = u.path().to_string();
        if !path.ends_with('/') {
            // 少了这一个斜杠，`/v1` 会在 join 时被吃掉，见上面那段。
            path.push('/');
        }
        u.set_path(&path);
        u.set_fragment(None);
        Ok(Self {
            provider: Provider::Custom,
            protocol,
            base_url: u.to_string(),
        })
    }

    /// 哪一家。`Custom` 时是 `Provider::Custom`。
    pub fn provider(&self) -> Provider {
        self.provider
    }

    /// 用哪种协议。**出站参数与适配器都看它。**
    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    /// 真实生效的端点（已归一化）。会出现在日志里，**必须过 [`redact`]**。
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// 这一家**有没有在本机实测过**。界面上标「未核实」看它。
    pub fn measured(&self) -> bool {
        self.provider.measured()
    }

    /// 这一路能用哪些模型名（可能为空——本地与自定义就是空的）。
    pub fn models(&self) -> &'static [&'static str] {
        self.provider.models()
    }

    /// 收不收这个模型名。口径见 [`Provider::accepts_model`]。
    pub fn accepts_model(&self, model: &str) -> bool {
        self.provider.accepts_model(model)
    }

    /// 默认模型名。**空清单时是空串**，调用方要自己挡住空名。
    pub fn default_model(&self) -> &'static str {
        self.provider.default_model()
    }

    /// 测连通性用的只读端点。**不调用模型、不产生 token 费用。**
    ///
    /// 它就是 `base_url` 后面接一个 `models`：预设的端点都以 `/v1/` 结尾，
    /// 所以拼出来正是 `/v1/models`；自定义端点同理。**不单独列一张常量表**
    /// 是有意的——那张表和端点会各漂各的（这已经发生过一次：
    /// 端点写对了、`models_url` 忘了改）。
    pub fn models_url(&self) -> String {
        format!("{}models", self.base_url)
    }

    /// 探测端点需要的头。协议差异由 [`Protocol`] 负责。
    pub fn probe_headers(&self, api_key: &str) -> Vec<(String, String)> {
        self.protocol.probe_headers(api_key)
    }
}

/// 「去哪儿 + 用哪个模型」的可改句柄。形状与设置层的其它旋钮一样：
/// **一份 `Arc`，命令层和推理器共用**——改完下一圈就生效，不用重启。
///
/// ## 为什么端点与模型名在**同一个格子**里
///
/// 它们是同一件事的两半：换了端点，清单里那份模型名多半就不适用了。
/// 拆成两个格子就会出现「模型名还留着、端点已经换了」这种组合——
/// 出网时报 404，而界面显示的是一个自洽的配置。
///
/// ## 为什么**拒收**不在清单里的名字
///
/// 这个句柄直接决定出网时 URL 里的模型名。放一个没验过的名字进去，
/// 症状是「设置成功、一调用就 404」，而那个 404 会被 `brief` 归类成
/// 「网络层/上游失败」，用户完全看不出是自己刚选了个不存在的模型。
/// 在**写入这一格**的时候拒掉，是唯一能让这件事当场可见的地方。
/// 但口径按 [`Provider::measured`] 分两种，见 [`Provider::accepts_model`]：
/// 没有核实过清单的那几家只挡空串，硬拒会让人一个模型都用不了。
#[derive(Clone)]
pub struct ModelCell(Arc<RwLock<(Route, String)>>);

impl ModelCell {
    /// 建一格。**模型名取端点的默认**，空清单时留空（调用方挡住空名）。
    pub fn new(route: Route) -> Self {
        let m = route.default_model().to_string();
        Self(Arc::new(RwLock::new((route, m))))
    }

    /// 建一格并指定模型名。**不校验**——给「按已存配置恢复」用。
    pub fn with_model(route: Route, model: &str) -> Self {
        Self(Arc::new(RwLock::new((route, model.trim().to_string()))))
    }

    /// 当前这一路（端点 + 协议 + 厂商）。
    pub fn route(&self) -> Route {
        self.0.read().expect("锁").0.clone()
    }

    /// 当前是哪一家。
    pub fn provider(&self) -> Provider {
        self.route().provider()
    }

    /// 当前这一路能选的模型名（可能为空）。
    pub fn models(&self) -> &'static [&'static str] {
        self.route().models()
    }

    /// 当前选中的模型名。
    pub fn get(&self) -> String {
        self.0.read().expect("锁").1.clone()
    }

    /// 改模型。**返回夹取/校验后的真实生效值**——
    /// 命令层要拿它回给界面，不能拿自己传进去的字符串糊弄过去。
    ///
    /// 不被接受就**返回错误、并且不改格子**。失败方向必须是「不变」。
    pub fn set(&self, model: &str) -> Result<String, String> {
        let m = model.trim();
        let mut g = self.0.write().expect("锁");
        if !g.0.accepts_model(m) {
            return Err(if g.0.models().is_empty() {
                format!(
                    "模型名不能为空（{} 没有可选清单，名字要你自己给）",
                    g.0.provider.key()
                )
            } else {
                format!(
                    "{} 不支持模型「{m}」。目前只列了实测能通的：{}",
                    g.0.provider.key(),
                    g.0.models().join(" / ")
                )
            });
        }
        g.1 = m.to_string();
        Ok(g.1.clone())
    }

    /// 换端点/协议/厂商。**返回归一化后的真实生效值。**
    ///
    /// ## 模型名要跟着复核
    ///
    /// **换了厂商就重来一次**——`gpt-4o-mini` 拿去问 DeepSeek 一定不对，
    /// 而留着它等于配置出一个必然 404、界面上却看着自洽的组合。
    /// 厂商没变（只是换了自定义地址）时才保留名字：那是同一家服务的
    /// 另一个入口，模型名多半不变。
    ///
    /// 复核不通过就回落到新这一路的默认模型名；**没有默认就留空**
    /// （本地与自定义就是这样），由界面要求用户填。此时
    /// [`HttpReasoner::ask`] 会在发出请求**之前**就拒绝，不去换一个
    /// 更难懂的 404。
    pub fn set_route(&self, route: Route) -> Route {
        let mut g = self.0.write().expect("锁");
        if g.0.provider() != route.provider() || !route.accepts_model(&g.1) {
            g.1 = route.default_model().to_string();
        }
        g.0 = route;
        g.0.clone()
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

/// 「走不走代理」的共享句柄。**网络层的属性，不是任何一家供应商的属性。**
///
/// 为什么单独一栏（票 43）：代理对**所有**出网请求生效，两组模型都走它。
/// 放进 provider 里就意味着每家填一次，而填了两次的那两份迟早不一样——
/// 「视觉组走直连、文字组走代理」是一个没人主动想要、却真实会发生的状态。
#[derive(Clone, Default)]
pub struct ProxyCell(Arc<RwLock<Option<String>>>);

impl ProxyCell {
    /// 建一个空格子（= 直连）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前代理地址。`None` = 直连。
    pub fn get(&self) -> Option<String> {
        self.0.read().expect("锁").clone()
    }

    /// 写代理地址。**空串 = 不走代理。**
    ///
    /// 写入前用 [`reqwest::Proxy::all`] 本身验一遍：它就是出网时真正要用的
    /// 那一个对象，所以「能不能装上」在**存进去之前**就该知道，
    /// 而不是等到发第一条请求时才发现填错了。
    ///
    /// 地址常带 `user:pass@`，所以它**必须过 [`redact`]** 才能进日志或
    /// 报错文本——`secret.rs` 那边已覆盖这一形状。
    pub fn set(&self, url: &str) -> Result<Option<String>, String> {
        let t = url.trim();
        if t.is_empty() {
            *self.0.write().expect("锁") = None;
            return Ok(None);
        }
        // **`Proxy::all` 自己不够严**（本机实测：它把「这不是地址」也收下，
        //   当成一个不带 scheme 的主机）。所以先自己判 scheme 与主机 ——
        //   存进去的必须是「出网时真的能用」的那种地址。
        let u = reqwest::Url::parse(t).map_err(|_| "这不像一个网址".to_string())?;
        if !matches!(u.scheme(), "http" | "https" | "socks5" | "socks5h") {
            return Err(format!(
                "代理地址只认 http / https / socks5，现在这一条是 {}://",
                u.scheme()
            ));
        }
        if u.host_str().is_none_or(str::is_empty) {
            return Err("代理地址里没有主机名".to_string());
        }
        // 再用它自己要用的那一个对象过一遍：装不上就别存。
        if let Err(e) = reqwest::Proxy::all(t) {
            return Err(redact(&format!("这个代理地址用不了：{e}")));
        }
        *self.0.write().expect("锁") = Some(t.to_string());
        Ok(self.get())
    }
}

/// 装好的协议层客户端，**连同它建的时候用的代理**。
///
/// 代理是**建 client 时**定死的（`reqwest::Proxy` 装在 `ClientBuilder` 上），
/// 换代理就得重建。而重建的判据是「代理变没变」，所以这里把代理与
/// client 存成一对：变了就重建，没变就直接复用。
#[derive(Default)]
struct ClientSlot {
    /// 建这份 client 时用的代理。`None` = 直连。
    proxy: Option<String>,
    client: Option<Client>,
}

/// 一轮推理用的模型配置。
///
/// **对外仍然是同步的**——`Reasoner` trait 是同步的，三段式、降级路径、
/// 界面上的调用计数全都建立在这一点上。同步与 async 之间那道桥在
/// [`HttpReasoner::ask`] 里的 `block_on`。
#[derive(Clone)]
pub struct HttpReasoner {
    /// 去哪儿 + 用哪个模型。**每次发请求都重读这一格**，
    /// 所以改端点/改模型下一圈就生效，不用重启。
    route: ModelCell,
    /// 网络层的代理。**同样每次重读**。
    proxy: ProxyCell,
    /// 按代理缓存的协议层客户端。**它的鉴权头、请求体、响应解析全由它负责。**
    clients: std::sync::Arc<std::sync::Mutex<ClientSlot>>,
    /// **key 只有这一份副本**，而且它是 [`std::sync::Arc`] 包着的——
    /// 建 client 时的 `AuthResolver` 闭包捕获的是**同一个** `Arc`，
    /// 不是又一份 `String`。换代理要重建 client、闭包要重新捕获，
    /// 这才让它留在这个结构里。早先它是「构造完就移进 genai、结构里不留」，
    /// 现在为了**代理改了立刻生效**必须能再交出去一次。
    ///
    /// 它**不参与任何 `Debug` / `Display`**：[`std::fmt::Debug`] 是手写的，
    /// 只打那几项；对外文本一律再过 [`redact`]。
    key: Arc<std::sync::Mutex<String>>,
    /// async → sync 的桥。`current_thread` 就够：
    /// 推理层每条新消息最多 3 次**串行**调用，引入多线程运行时
    /// 只会多背依赖、多一份线程开销，买不到任何东西。
    ///
    /// 用 `Arc` 是因为 `Runtime` 不 `Clone`，而 `HttpReasoner` 需要 `Clone`。
    rt: Arc<tokio::runtime::Runtime>,
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
    /// 而 Debug 输出会被日志吃掉。**`key` 字段被这一行挡住**。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let route = self.route.route();
        f.debug_struct("HttpReasoner")
            .field("provider", &route.provider())
            .field("protocol", &route.protocol())
            // 端点里可能带 `user:pass@` 或 `?token=`。**照打不误**是错的，
            // 而打整条 URL 会让人看不出是哪个端点——所以只打 host。
            .field("endpoint", &redact(route.base_url()))
            .field("model", &self.route.get())
            .field("api_key", &"<redacted：已移交给 genai 的 AuthResolver>")
            .field("proxy", &redact(&self.proxy.get().unwrap_or_default()))
            .field("calls", &self.calls.get())
            .finish()
    }
}

impl HttpReasoner {
    /// 装好一个发真请求的推理器，端点用该供应商的预设。
    ///
    /// ## 为什么返回 `Result`
    ///
    /// 里面要起一个 tokio runtime，那是**真的会失败**的一步。
    /// 早先的版本用 `ureq` 时没有这一步，于是这个构造函数是 `fn -> Self`。
    /// 让它继续是 `fn -> Self` 就得 `expect`，而这一条路径上**没有断言**：
    /// 失败时调用方（设置层）能做的正确动作是**退回预置传输**，
    /// 不是让整个面板起不来。
    ///
    /// `Custom` 没有预设端点，所以它在这里**一定失败**——
    ///   它只能经 [`HttpReasoner::with_route`] 装。
    pub fn new(provider: Provider, api_key: String) -> Result<Self, String> {
        let route = Route::preset(provider)?;
        let cell = ModelCell::new(route);
        Self::with_config(cell, api_key, ProxyCell::new())
    }

    /// **产品路径上的那一个构造**：共用配置格 + key + 代理。
    ///
    /// `model` 这一格**同时装着端点、协议、厂商与模型名**，而且是**共用**的
    /// ——设置页改的就是它，推理器每次发请求都重读，所以不用重启。
    /// 要换端点：`cell.set_route(Route::custom(..)?)`。
    ///
    /// `proxy` 是两组**共用**的那一格：代理是网络层的属性，不属于任何一家。
    pub fn with_config(
        model: ModelCell,
        api_key: String,
        proxy: ProxyCell,
    ) -> Result<Self, String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("起不出异步运行时：{e}"))?;
        Ok(Self {
            route: model,
            proxy,
            clients: std::sync::Arc::new(std::sync::Mutex::new(ClientSlot::default())),
            key: Arc::new(std::sync::Mutex::new(api_key)),
            rt: Arc::new(rt),
            calls: std::cell::Cell::new(0),
            judge_calls: std::cell::Cell::new(0),
            draft_calls: std::cell::Cell::new(0),
            rank_calls: std::cell::Cell::new(0),
        })
    }

    /// 把端点指到别处。**只给测试用**（对着 [`crate::mock_upstream`]）。
    ///
    /// 它是 `#[cfg(test)]`：产品路径上的自定义端点走
    ///   [`HttpReasoner::with_config`] + [`Route::custom`]+[`ModelCell::set_route`]，
    ///   那一格是**设置页能写到的**（有校验、有界面说明、有 redact）。
    ///   而这里这个构造**绕过校验直接吃一个字符串**，在生产构建里不该存在。
    #[cfg(test)]
    pub(crate) fn with_endpoint(
        provider: Provider,
        api_key: String,
        endpoint: String,
    ) -> Result<Self, String> {
        let route = Route::custom(provider.protocol(), &endpoint)?;
        let cell = ModelCell::new(route);
        // 假上游不认清单外的模型名，而 `Provider::Custom` 恰好只挡空串，
        // 所以这里的名字一定能收下。
        cell.set(provider.default_model()).expect("默认模型名非空");
        Self::with_config(cell, api_key, ProxyCell::new())
    }

    /// 拿到（必要时重建）协议层客户端。**代理变了才重建**。
    ///
    /// ## 为什么要在这一层判「变没变」
    ///
    /// 代理是装在 `reqwest::ClientBuilder` 上的，**建好就定死**。
    /// 而设置页的约定是「改完立刻生效，不用重启」——所以代理一改就得
    /// 重建 client。这里把「建它时的代理」与 client 存成一对，
    /// 变的判定就成了一次字符串比较。
    ///
    /// 重建本身很便宜：`reqwest::Client::builder().build()` 只做连接池与
    /// TLS 配置，不发任何请求；而它只在用户真的改了代理时才发生。
    fn client(&self) -> Result<Client, String> {
        let want = self.proxy.get();
        let mut g = self
            .clients
            .lock()
            .map_err(|_| "客户端缓存的锁中毒了".to_string())?;
        if g.proxy == want
            && let Some(c) = g.client.clone()
        {
            return Ok(c);
        }
        let client = self.build_client(want.clone())?;
        *g = ClientSlot {
            proxy: want,
            client: Some(client.clone()),
        };
        Ok(client)
    }

    /// 真建一份协议层客户端。**每次换代理走这里一次。**
    fn build_client(&self, proxy: Option<String>) -> Result<Client, String> {
        let mut web = WebConfig::default()
            .with_timeout(REQUEST_TIMEOUT)
            .with_connect_timeout(CONNECT_TIMEOUT);
        if let Some(url) = &proxy {
            // **复用 reqwest 已有的 `Proxy::all`**，不要自己实现 CONNECT。
            //   代理地址常带 `user:pass@`，它自己会处理认证。
            web = web
                .with_all_proxy_url(url)
                .map_err(|e| redact(&format!("装不上这个代理：{e}")))?;
        }

        // 鉴权交给 `AuthResolver`：key 从此**只活在 genai 内部**。
        // `AuthData` 的 `Debug` 自带 redact（打出来是 `AuthData::Single(REDACTED)`），
        // 但那只是兜底——对外文本一律还要过 [`redact`]。
        let key = self.key.clone();
        // 端点**每次请求现读**：`with_service_target_resolver_fn` 的闭包
        // 是 genai 每次解析时调用的，所以设置页改完下一圈就用新地址。
        let route = self.route.clone();
        Ok(Client::builder()
            .with_web_config(web)
            .with_auth_resolver_fn(move |_m: ModelIden| {
                let v = key
                    .lock()
                    .map_err(|e| genai::resolver::Error::Custom(e.to_string()))?
                    .clone();
                Ok(Some(AuthData::from_single(v)))
            })
            .with_service_target_resolver_fn(move |t: ServiceTarget| {
                Ok(ServiceTarget {
                    endpoint: Endpoint::from_owned(route.route().base_url().to_string()),
                    ..t
                })
            })
            .build())
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
        self.route.get()
    }

    /// 现在走的那一路（端点 / 协议 / 厂商）。
    pub fn route(&self) -> Route {
        self.route.route()
    }

    /// 这一格的句柄。命令层要靠它改模型与端点。
    pub fn model_cell(&self) -> ModelCell {
        self.route.clone()
    }

    fn ask(
        &self,
        system: &str,
        user: &str,
        stage: Stage,
        images: &[ImagePayload],
    ) -> Result<String, String> {
        let req = build_request(system, user, images);
        let route = self.route.route();
        // **空模型名不许发出去。** 本机推理与自定义端点没有默认清单，
        // 格子里就可能是空的，而 `ModelIden::new(_, "")` 出去只会换一个
        // 更难懂的 404。这里当场说清楚「还没选模型」。
        let name = self.route.get();
        if name.trim().is_empty() {
            return Err(format!(
                "还没选模型（当前这一路：{}）",
                route.provider().label()
            ));
        }
        let model = ModelIden::new(route.protocol().adapter(), &name);
        let client = self.client()?;
        self.calls.set(self.calls.get() + 1);
        // 分段计数。**先加再发**——发失败也算一次尝试，
        // 界面上「调用了几次」要是用户实际掏的钱，不是成功次数。
        match stage {
            Stage::Judge => self.judge_calls.set(self.judge_calls.get() + 1),
            Stage::Draft => self.draft_calls.set(self.draft_calls.get() + 1),
            Stage::Rank => self.rank_calls.set(self.rank_calls.get() + 1),
        }
        let opts = route.protocol().chat_options();
        // **这里就是 async → sync 那道桥。** 对外（`Reasoner`）仍然是同步的。
        let resp = self
            .rt
            .block_on(client.exec_chat(model, req, Some(&opts)))
            .map_err(|e| brief(&e))?;
        resp.first_text().map(str::to_string).ok_or_else(|| {
            // 上游契约变了。**只说结构不认识**，不带任何请求/响应正文——
            // 请求体里有用户的聊天内容，而 genai 的错误里**带着整个请求体**。
            format!("响应里没有文字（provider={}）", route.provider().key())
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
/// 走 `GET {base_url}models`：不产生 token、不花钱、也不占用速率额度，
/// 而「这个 key 能不能访问 API」这个问题它答得一样准。
///
/// ## 为什么这一条**没有**外包给 genai
///
/// `/v1/models` 不是任何一个适配器的操作，genai 里没有它的等价物；
/// 而它自己的 `WebClient` 是 `pub(crate)`，外部拿不到。
/// 所以这里**只**这一个 GET 用了 `reqwest` 直接发——版本与 genai 内部
/// 那条一致，依赖树里不新增任何 crate。
///
/// ## 自定义端点也走这一条
///
/// 所以「测试连通」测的是**当前这一路**（含用户自己填的地址），不是某家
/// 预设。地址里带的凭据由 [`redact`] 处理，见 [`Route::custom`]。
///
/// ## 错误里的 key
///
/// 上游的错误正文**可能带上你刚发的 header**。所以整条错误都过一遍
/// [`redact`]，并且只截前 200 字符——这两步都必要。
pub fn probe_route(
    route: &Route,
    api_key: &str,
    timeout: Duration,
    proxy: Option<&str>,
) -> Result<(), String> {
    probe_at(
        &route.models_url(),
        &route.probe_headers(api_key),
        timeout,
        proxy,
    )
}

/// [`probe_route`] 的真正实现。**把 URL、头、代理拆出来当参数**，
/// 是为了让「探测打的是 `/models` 而不是聊天端点」「代理真的套上了」
/// 这两件事**能对着一个真的回环端口断言**，而不是对着常量断言。
fn probe_at(
    url: &str,
    headers: &[(String, String)],
    timeout: Duration,
    proxy: Option<&str>,
) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("起不出异步运行时：{e}"))?;
    let headers = headers.to_vec();
    let proxy = proxy.map(str::to_string);
    rt.block_on(async move {
        let mut builder = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(CONNECT_TIMEOUT);
        // 与聊天那条路**同一个** `reqwest::Proxy::all`，
        // 别在这里另写一套代理逻辑——两套不一致的症状是
        // 「聊天走代理、测连通不走」，而用户看不出该信哪个。
        if let Some(p) = &proxy {
            let px = reqwest::Proxy::all(p.clone())
                .map_err(|e| redact(&format!("装不上这个代理：{e}")))?;
            builder = builder.proxy(px);
        }
        let client = builder
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
        // 失败时**先读正文再拼错误**：`reqwest` 的 `status()` 之后
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
///
/// **2026-10-15 不再自己实现了**。这里原来有一份只认 `sk-` / `sk-ant-`
/// 的弱版，而**自定义端点与代理的报错全走这里**
/// （`ProxyCell::set` / `build_client` / `probe_route` 三处）⇒
/// 一个带 `user:pass@` 的代理地址填错时，口令会被原样放进日志。
///
/// 强版住在 `ui/src-tauri/src/secret.rs`，但它管着 Windows 凭据管理器，
/// 本 crate 够不着——于是同一件「脱敏」的事被写了两遍，
/// 且两遍能力不同（`reason` 这份连 URL 里的凭据都不认）。
///
/// 两条**能力不同**的旁路等于没有脱敏：强的那份再强，
/// 调用方挑了弱的那份就白做。
/// ⇒ 纯字符串的那部分收到 [`chatservant_core::redact`]，两边调同一份。
/// 本 crate 拿不到「已登记的 key 值」那一路（那需要凭据管理器），
/// 所以这里只做形状遮蔽——对本 crate 够用：它经手的错误里
/// 出现的是地址与响应体，不是我们发出去的 key。
pub fn redact(text: &str) -> String {
    chatservant_core::redact::shapes(text)
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

    // secret-scan: allow —— 本模块里那两串 `sk-…` 是**故意的假 key**：
    // 一处是 `KEY` 这个测试常量，一处是喂给 `redact()` 的样本。
    // pre-commit 的钩子只放行 `tests/` **目录**，放行不了 `src/` 里的
    // `#[cfg(test)] mod tests`，所以任何一次碰这个文件的提交都会被拦。
    // 按钩子自己写的规矩（见 .githooks/pre-commit）走**文件内豁免标记**，
    // 而不是让人养成 `--no-verify`——那个习惯一旦成形，真 key 也一起过去了。
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

    /// OpenAI 预设那一路。反复出现的 `Route::preset(..).expect(..)` 收在这里。
    fn openai_route() -> Route {
        Route::preset(Provider::OpenAi).expect("OpenAI 有预设端点")
    }

    /// Anthropic 预设那一路。
    fn anthropic_route() -> Route {
        Route::preset(Provider::Anthropic).expect("Anthropic 有预设端点")
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
    /// 断言的**形状变了**：早先读的是我们自己拼的请求体，
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
        // **先断条数，再断内容。**
        //
        // 少了这两行，一次「少记了一个请求」会以最误导人的形式冒出来：
        // `seen[0]` 变成起草那一次，于是下面那条断言报「判断请求带了口吻样本」——
        // 读起来像是判断把口吻样本泄给了上游，真凶其实在测试夹具的读失败上
        // （见 `mock_upstream`）。断条数会让它当场报「少了一条」，那才是真的。
        assert_eq!(seen.len(), 3, "一轮 = 判断 + 起草 + 排序，各一次请求");
        assert!(
            seen[0].body.contains("\"system\""),
            "第一条应当是判断请求，它带 system 提示：{}",
            seen[0].body
        );
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

    /// **代理与自定义端点的报错必须抹掉 URL 里的凭据**（票 43 第 ③ 条）。
    ///
    /// 这条守的是 2026-10-15 修掉的那个破口：本 crate 的 `redact`
    ///   原来只认 `sk-` / `sk-ant-`，而**代理地址的报错全走这里**
    ///   （`ProxyCell::set` / `build_client` / `probe_route`），
    ///   于是一句「代理连不上」会把 `alice:hunter2` 原样带进日志。
    ///
    /// 形状照抄代理最常见的样子：`http://user:pass@host:port`。
    #[test]
    fn a_proxy_address_never_keeps_its_credentials() {
        let s = redact("装不上这个代理：http://alice:hunter2@proxy.internal:8080");
        assert!(!s.contains("hunter2"), "代理口令泄了：{s}");
        assert!(!s.contains("alice"), "代理账号泄了：{s}");
        assert!(
            s.contains("proxy.internal:8080"),
            "host 与端口得留着，否则报错失去排查价值：{s}"
        );
    }

    /// 自定义端点那侧同理：token 常被塞在**路径**或**查询参数**里。
    #[test]
    fn a_custom_endpoint_never_keeps_its_token() {
        let s = redact("GET https://x.example.com/v1/?access_token=abc123DEF 失败");
        assert!(!s.contains("abc123DEF"), "查询参数里的 token 泄了：{s}");
    }

    /// **钉住反向的一条**：无凭据的地址不许被抹。
    ///
    /// 宁可漏抹也不能把线索全抹掉——「是哪一家、哪个端点」是排错的唯一依据。
    /// 这条与上面两条成对，少了它就会出现「全抹成 `••••`，报错等于没报」。
    #[test]
    fn a_clean_endpoint_stays_readable() {
        let msg = "连不上 https://api.openai.com/v1/ 超时";
        assert_eq!(redact(msg), msg, "无凭据的地址被动了：{msg}");
    }

    /// **测连通性必须打到 `/v1/models`，不能调模型。**
    ///
    /// 早先的 `probe_key` 恒返回 false，注释说「网络层 ticket 10 接上」——
    /// 但网络层早就是真的了，于是那个按钮**永远说不通**。
    /// 恒假的指示器比没有更糟：用户会以为 key 坏了，反复重填。
    #[test]
    fn probing_hits_the_models_endpoint_not_the_model() {
        for p in Provider::PRESETS {
            let route = Route::preset(p).expect("预设都有端点");
            let url = route.models_url();
            assert!(
                url.ends_with("/models"),
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
    /// 这是**真的 socket**，不是假 `Http`：早先那条断言只能证明
    /// 「我们调了自己的函数，参数长这样」，证明不了「线上真发了这个」。
    #[test]
    fn probing_over_a_real_socket_sends_one_get_to_models() {
        let up = upstream(vec![Reply(200, r#"{"data":[]}"#.into())]);
        probe_at(
            &format!("{}models", up.base_url()),
            &openai_route().probe_headers(KEY),
            Duration::from_secs(5),
            None,
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
    /// 这次是真的：假上游**把 key 原样写回错误正文**，
    /// 模拟最坏情况，而错误是从真的响应里读出来的。
    #[test]
    fn a_probe_error_never_contains_the_key() {
        let up = upstream(vec![Reply(401, format!("invalid api key {KEY}"))]);
        let e = probe_at(
            &format!("{}models", up.base_url()),
            &openai_route().probe_headers(KEY),
            Duration::from_secs(5),
            None,
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
            &anthropic_route().probe_headers(KEY),
            Duration::from_secs(5),
            None,
        )
        .expect("假上游应当通");
        let seen = up.seen();
        assert_eq!(seen[0].header("x-api-key"), Some(KEY));
        assert!(
            seen[0].header("authorization").is_none(),
            "Anthropic 探测不该带 Bearer"
        );
    }

    /// **实测过的三家拒收清单外的名字，其余只挡空串。**
    ///
    /// 这条是「不要编造没验过的模型名」的可执行版本：放一个没验过的名字进去，
    /// 症状是「设置成功、一调用就 404」，而 404 会被归类成上游失败，
    /// 用户完全看不出是自己选了个不存在的模型。
    ///
    /// 而界线是 [`Provider::measured`]，不是「有没有列清单」：
    ///   本地两个与自定义**列不出**清单，对它们硬拒 = 一个模型都用不了。
    #[test]
    fn only_measured_providers_reject_unlisted_models() {
        for p in Provider::PRESETS {
            if p.measured() {
                assert!(!p.models().is_empty(), "{} 说验过却一条没列", p.key());
                for m in p.models() {
                    assert!(p.accepts_model(m), "{} 自己列的 {m} 却不被接受", p.key());
                }
                assert!(!p.accepts_model("no-such-model"));
            } else {
                assert!(
                    p.accepts_model("whatever-the-user-installed"),
                    "{} 没有核实过的清单，硬拒等于让用户一个模型都用不了：{}",
                    p.key(),
                    p.label()
                );
            }
            assert!(!p.accepts_model(""), "空名一律拒收");
            assert!(!p.accepts_model("   "));
        }
    }

    #[test]
    fn the_model_cell_takes_effect_without_a_restart() {
        let cell = ModelCell::new(openai_route());
        let r = HttpReasoner::with_config(cell.clone(), KEY.into(), ProxyCell::new())
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
        let hs = anthropic_route().probe_headers("k");
        assert!(hs.iter().any(|(k, v)| k == "x-api-key" && v == "k"));
        assert!(
            hs.iter().any(|(k, _)| k == "anthropic-version"),
            "缺版本头，上游会拒"
        );
        assert!(!hs.iter().any(|(k, _)| k == "Authorization"));
    }

    #[test]
    fn openai_uses_bearer() {
        let hs = openai_route().probe_headers("k");
        assert!(
            hs.iter()
                .any(|(k, v)| k == "Authorization" && v == "Bearer k")
        );
    }

    /// **端点是字面量，因为它是我们在本机实测过的。**
    /// genai 的内置默认值错过一次（MiniMax），所以这里逐个钉住。
    #[test]
    fn the_endpoints_are_our_own_measured_literals() {
        assert_eq!(
            Provider::OpenAi.preset_base_url(),
            Some("https://api.openai.com/v1/")
        );
        assert_eq!(
            Provider::Anthropic.preset_base_url(),
            Some("https://api.anthropic.com/v1/")
        );
        // 中国区端点。genai 自带的 MiniMax 适配器指向 api.minimax.io，
        // 那是另一家、另一种协议，见 `Provider::MiniMax` 的文档。
        assert_eq!(
            Provider::MiniMax.preset_base_url(),
            Some("https://api.minimaxi.com/v1/")
        );
        // 「自定义」没有预设端点——**不许兜一个**。兜了的话用户会以为
        // 自己接到了别处，而症状是发出去的请求打在一个他没填的地址上。
        assert_eq!(Provider::Custom.preset_base_url(), None);
    }

    /// **自定义端点必须把 `/v1` 保住**——少一个斜杠就会被 join 吃掉。
    ///
    /// 这是本票最容易踩、也最像 404 的一个坑：用户填
    /// `https://x.example.com/v1`，我们照单全收，而 genai 拼出来的
    /// 完整地址是 `https://x.example.com/chat/completions`。
    #[test]
    fn a_custom_endpoint_keeps_its_path_prefix() {
        let r = Route::custom(Protocol::OpenAiCompatible, "https://x.example.com/v1")
            .expect("这是合法端点");
        assert_eq!(r.base_url(), "https://x.example.com/v1/");
        // 而这正是 genai 的拼法（`adapter_shared.rs::util_get_service_url`）。
        let joined = reqwest::Url::parse(r.base_url())
            .expect("归一化之后一定解析得开")
            .join("chat/completions")
            .expect("能 join");
        assert_eq!(
            joined.as_str(),
            "https://x.example.com/v1/chat/completions",
            "/v1 被吃掉 ⇒ 这个端点必然 404"
        );
    }

    /// 自定义端点的坏形状**当场拒**，不给兜底。
    ///
    /// 重点是 `https:///v1`：URL 解析器**解析成功**并把它归一化成
    /// `https://v1/`，也就是把用户没写过的域名当成主机——那会把 key
    /// 和聊天内容发到一个陌生地址去。所以判「有没有主机」要按**字面**判。
    #[test]
    fn a_custom_endpoint_rejects_what_it_cannot_use() {
        assert!(Route::custom(Protocol::OpenAiCompatible, "   ").is_err());
        assert!(Route::custom(Protocol::OpenAiCompatible, "不是网址").is_err());
        assert!(Route::custom(Protocol::OpenAiCompatible, "file:///tmp/x").is_err());
        assert!(Route::custom(Protocol::OpenAiCompatible, "https:///v1").is_err());
        assert!(Route::custom(Protocol::OpenAiCompatible, "api.openai.com/v1").is_err());
    }

    /// **查询参数要留着**（`?access_token=…` 是真实存在的端点形状），
    /// 片段要丢掉（它不会发给服务器，留着只会让日志里的地址变形）。
    #[test]
    fn a_custom_endpoint_keeps_the_query_and_drops_the_fragment() {
        let r = Route::custom(
            Protocol::OpenAiCompatible,
            "https://x.example.com/v1?access_token=abc#frag",
        )
        .expect("合法");
        assert!(
            r.base_url().contains("access_token=abc"),
            "{}",
            r.base_url()
        );
        assert!(!r.base_url().contains("frag"), "{}", r.base_url());
    }

    /// **产品路径真的能走到自定义端点**：这不是 `with_endpoint`（测试专用），
    /// 而是设置页写得到的那条路——`Route::custom` + 共用的格子。
    ///
    /// 断言的是**真的发出去的字节**：路径必须是回环端口上那个 `/v1/`。
    /// 走 `with_endpoint` 的那些测试证明不了这一条，因为那条路生产构建里
    /// 根本不存在。
    #[test]
    fn the_product_path_reaches_a_custom_endpoint() {
        let up = upstream(three_replies());
        // **故意少给一个结尾斜杠**：`http://…/v1` 而不是 `…/v1/`。
        //   产品路径上的归一化就在这一行生效——而少了它，genai 拼出来的
        //   是 `/chat/completions`，下面那条路径断言当场就红。
        let route = Route::custom(
            Protocol::OpenAiCompatible,
            up.base_url().trim_end_matches('/'),
        )
        .expect("假上游的地址是合法端点");
        assert_eq!(route.provider(), Provider::Custom);
        assert!(!route.measured(), "自定义端点不可能是实测过的");

        let cell = ModelCell::new(route);
        // 自定义端点的模型名只能由用户给 ⇒ 这一格必须能收任意非空名。
        cell.set("some-local-model").expect("自定义端点不挑模型名");
        let mut r =
            HttpReasoner::with_config(cell, KEY.into(), ProxyCell::new()).expect("起运行时");

        let ctx = TurnContext::text(vec!["今晚有空吗".into()], "小号".into());
        crate::turn(&mut r, &ctx).expect("推理成功");

        let seen = up.seen();
        assert_eq!(seen.len(), 3, "一轮三段各一次");
        for s in seen.iter() {
            assert_eq!(s.path, "/v1/chat/completions", "端点没生效：{}", s.path);
            assert!(s.body.contains("some-local-model"), "模型名没生效");
        }
    }

    /// **换了模型名而没换格子，推理器就发新名字。**
    ///
    /// 「设置页改了、界面显示新的、线上还是旧的」是本项目反复出现的那类假控件。
    /// 这条钉住端点/模型这一格确实与推理器共用。
    #[test]
    fn switching_the_route_takes_effect_on_the_next_request() {
        let a = upstream(vec![Reply(200, openai_reply(judge_json()))]);
        let b = upstream(vec![Reply(200, openai_reply(judge_json()))]);
        let cell = ModelCell::new(Route::custom(Protocol::OpenAiCompatible, a.base_url()).unwrap());
        cell.set("m").expect("自定义端点不挑模型名");
        let mut r = HttpReasoner::with_config(cell.clone(), KEY.into(), ProxyCell::new())
            .expect("起运行时");

        let ctx = TurnContext::text(vec!["今晚有空吗".into()], "小号".into());
        r.judge(&ctx).expect("第一次应当通");
        assert_eq!(a.seen().len(), 1);

        cell.set_route(Route::custom(Protocol::OpenAiCompatible, b.base_url()).unwrap());
        r.judge(&ctx).expect("第二次也应当通");

        assert_eq!(b.seen().len(), 1, "换端点之后请求应当打到新地址");
        assert_eq!(r.route().base_url(), b.base_url());
    }

    /// **换了厂商，模型名必须重来。**
    ///
    /// 留着 `gpt-4o-mini` 去问 DeepSeek 是必然 404，而界面上那一份配置
    /// 看着是自洽的——这正是「界面对了、系统错了」最难查的一种。
    /// 只换自定义地址（厂商还是 custom）时名字要留着：那是同一家服务的
    /// 另一个入口。
    #[test]
    fn switching_provider_revalidates_the_model_name() {
        let cell = ModelCell::new(openai_route());
        assert_eq!(cell.get(), "gpt-4o-mini");

        cell.set_route(anthropic_route());
        assert_eq!(cell.get(), "claude-sonnet-5", "换了厂商就该重来");

        // 没实测过清单的那几家同样重来——我们没有依据说旧名字还对。
        cell.set_route(Route::preset(Provider::DeepSeek).unwrap());
        assert_eq!(cell.get(), "deepseek-chat");

        // 只换自定义地址：厂商没变，名字留着。
        cell.set_route(Route::custom(Protocol::OpenAiCompatible, "https://o.example/v1").unwrap());
        cell.set("whatever").expect("自定义端点不挑模型名");
        cell.set_route(Route::custom(Protocol::OpenAiCompatible, "https://p.example/v1").unwrap());
        assert_eq!(cell.get(), "whatever", "同一家的另一个入口不该清掉模型名");
    }

    /// **代理是网络层的属性**：改完之后下一次请求就用新的地址。
    ///
    /// 走一个真的回环端口当「代理」：HTTP 代理收到的是绝对 URI 的请求行，
    /// 所以我们能断言「请求确实被发到了代理那里」。
    #[test]
    fn the_proxy_is_shared_and_takes_effect_without_a_restart() {
        // 两台**都在回环上**：`up` 当模型端点，`px` 当代理。
        //   这条测试**不发任何外网请求**——连「直连时打不到」的演示也不做，
        //   因为那既慢、又等于在测试里偷偷联网。
        let up = upstream(vec![Reply(200, openai_reply(judge_json()))]);
        let px = upstream(vec![Reply(200, openai_reply(judge_json()))]);

        let proxy = ProxyCell::new();
        assert_eq!(proxy.get(), None, "默认直连");

        // 坏的代理当场拒，不存。
        assert!(proxy.set("这不是地址").is_err());
        assert_eq!(proxy.get(), None, "写失败不许改变现状");

        let cell =
            ModelCell::new(Route::custom(Protocol::OpenAiCompatible, up.base_url()).unwrap());
        cell.set("m").expect("自定义端点不挑模型名");
        let mut r = HttpReasoner::with_config(cell, KEY.into(), proxy.clone()).expect("起运行时");
        let ctx = TurnContext::text(vec!["今晚有空吗".into()], "小号".into());

        // 直连：打的是模型端点。
        r.judge(&ctx).expect("直连应当通");
        assert_eq!(up.seen().len(), 1);
        assert_eq!(px.seen().len(), 0, "还没配代理，不该经过它");

        // 配了代理：下一次请求改道走代理。
        proxy
            .set(px.base_url().trim_end_matches("v1/"))
            .expect("回环地址是合法代理");
        r.judge(&ctx).expect("经代理也应当通");
        assert_eq!(px.seen().len(), 1, "请求应当经代理打到那台回环口");
        // HTTP 代理收到的是**绝对 URI** 的请求行，这就是「它确实当了代理」的证据。
        assert!(
            px.seen()[0].path.starts_with("http://"),
            "这不是代理形式的请求行：{}",
            px.seen()[0].path
        );
    }

    /// **两组模型各自独立**：改一组不许动另一组。
    ///
    /// 这一条是本票的验收核心——「合并成一个列表」时它在结构上就不可能成立。
    #[test]
    fn the_two_groups_hold_independent_configuration() {
        let decision = ModelCell::new(openai_route());
        let llm = ModelCell::new(Route::preset(Provider::MiniMax).unwrap());
        let proxy = ProxyCell::new();
        proxy.set("http://127.0.0.1:9/").expect("合法");

        let d = HttpReasoner::with_config(decision.clone(), KEY.into(), proxy.clone()).unwrap();
        let l = HttpReasoner::with_config(llm.clone(), KEY.into(), proxy.clone()).unwrap();

        decision.set_route(Route::preset(Provider::DeepSeek).unwrap());
        llm.set_route(
            Route::custom(Protocol::OpenAiCompatible, "https://other.example/v1").unwrap(),
        );

        assert_eq!(d.route().base_url(), "https://api.deepseek.com/v1/");
        assert_eq!(l.route().base_url(), "https://other.example/v1/");
        // 代理是**同一份**——单独一栏、不重复填，就该只有一处可改。
        proxy.set("").expect("清空");
        assert_eq!(proxy.get(), None);
        assert_eq!(d.route().base_url(), "https://api.deepseek.com/v1/");
        assert_eq!(l.route().base_url(), "https://other.example/v1/");
    }

    /// 组别短名与显示名都是**稳定标识**——凭据目标名要用它。
    #[test]
    fn group_ids_round_trip() {
        for g in ModelGroup::ALL {
            assert_eq!(ModelGroup::from_key(g.key()), Some(g), "{}", g.key());
            assert!(!g.label().is_empty());
        }
        assert_eq!(ModelGroup::from_key("nope"), None);
        assert_eq!(ModelGroup::ALL.len(), 2, "就是两组，别悄悄加第三组");
    }

    /// 短名能反查回枚举值。**改名会让用户已存的配置读不出来**，所以钉死。
    #[test]
    fn provider_and_protocol_keys_round_trip() {
        for p in Provider::PRESETS {
            assert_eq!(Provider::from_key(p.key()), Some(p), "{}", p.key());
            assert!(!p.label().is_empty(), "{}", p.key());
            assert!(p.preset_base_url().is_some(), "{}", p.key());
        }
        assert_eq!(Provider::from_key("custom"), Some(Provider::Custom));
        assert_eq!(Provider::from_key("nope"), None);
        for pr in [Protocol::OpenAiCompatible, Protocol::Anthropic] {
            assert_eq!(Protocol::from_key(pr.key()), Some(pr));
        }
        assert_eq!(Protocol::from_key("nope"), None);
    }
}
