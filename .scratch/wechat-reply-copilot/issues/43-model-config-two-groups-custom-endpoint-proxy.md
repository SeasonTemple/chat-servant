# 43: 模型配置应该是**两组**，且要能配自定义端点与代理

**What to build:** 把「一个供应商列表 + 一把 key」扩成
**两组独立配置 + 常见 provider 预设 + 自定义兼容端点 + 代理**。

**Status:** ready-for-agent · **2026-10-15 用户提出，待做**

## 现状（2026-10-15 核实）

```rust
pub enum Provider { OpenAi, Anthropic, MiniMax }        // transport.rs:62

pub fn base_url(self) -> &'static str {                 // transport.rs:169
    Self::OpenAi    => "https://api.openai.com/v1/",
    Self::Anthropic => "https://api.anthropic.com/v1/",
    Self::MiniMax   => "https://api.minimaxi.com/v1/",   // 中国区
}
```

- **端点是硬编码字面量**。有 `with_endpoint(..)`，但**只有测试和 mock 在用**，
  产品路径 `assemble(provider, api_key, model, provider.base_url())` 拿的是常量。
- **没有 base URL 配置项**，**没有代理配置项**。
- 设置页只有一个「模型 key」段，三家并列，**共用一把 key 形态**。

## 用户要的两组

用户原话：「模型配置不应该是两组？1 个决策模型的配置目前默认是配置 jev 的 key、
还要获取模型，1 个 LLM 的」。

拆开：

| 组 | 干什么 | 配置项 |
|---|---|---|
| **① 决策模型**（视觉/版面那一层） | 现在默认是 jev（票面写「获取模型」即拉取可用模型清单） | provider 预设 / api_key / **自定义 base_url** / 模型名 / 代理 |
| **② LLM**（起草回复那一层） | 出候选 | 同上，但**独立**的一套 |

⚠ **两组必须独立**。它们可以指向同一家，也可以一家用云端一家用本地。
现在合并成一个列表，天然表达不了「视觉走 A、文字走 B」。

## 必备的三件事（用户点名）

### ① 常见 provider 预设，而不是三家硬编码

至少要覆盖 OpenAI / Anthropic / MiniMax 之外常见的：
**DeepSeek / 通义 / 智谱 / 本地 Ollama / LM Studio / 任意 OpenAI 兼容端点**。

设计要点：**「OpenAI 兼容」应该是一个协议选择，而不是一家供应商**——
大部分第三方服务都只兼容它。⇒ provider 列表应该是
「预设（填好端点）+ 自定义（端点自己填）」两类，不是「一家一行」。

⚠ 端点常量那份「本机实测过的字面量」注释要跟着调整——
新增项时端点要么实测要么标注来源，不能照抄。

### ② 自定义 anthropic / openai 兼容 URL

- 自定义端点必须**与 api_key 分开存**：同一个 key 可能配不同端点。
- **`PROVIDERS` 那张白名单要一起改**（`commands.rs` 的 `save_setting` /
  `forget_key` 都在 `if !PROVIDERS.contains(..)` 上挡未知值）。
- **端点不能进日志**。`secret.rs` 的 `redact` 现在只抹 key 值与 `sk-` 形状，
  **不抹 URL**。自定义端点常带 token 路径（如
  `https://x.example.com/v1/<token>/`），⇒ `redact` 要一起覆盖。
  ⚠ 这是硬约束第 7 条（key 不进日志）的延伸，不能漏。

### ③ 代理配置

用户原话：「考虑到可能还需要提供一个代理配置」。

设计要点：
- **代理不是 provider 的属性，是网络层的属性**。它对两组都生效，
  所以应该**单独一栏**，而不是每家填一次。
- 优先复用 `reqwest` 已有的 `Proxy::all(url)`，不要自己实现 CONNECT。
- **代理地址本身也要过 `redact`**——它常带 `user:pass@`。

## 验收项

- [x] **现状核实**：端点硬编码、无自定义 URL、无代理（2026-10-15 读码）
- [ ] 设置页拆成**两组独立配置**，互不影响
- [ ] provider 列表支持「预设 + 自定义」两类，协议选择与厂商解耦
- [ ] 自定义 base_url 能生效（产品路径走得到，不只是测试路径）
- [ ] `redact` 覆盖**端点与代理地址**里的 token 形状 + 守卫测试
- [ ] 代理配置对两组都生效，单独一栏不重复填
- [ ] `save_setting`/`forget_key` 的 `PROVIDERS` 白名单同步放开
- [ ] 门禁九步全绿

## ⚠ 这一条会碰到硬约束

自定义端点 + 代理意味着**用户可以把数据发到任意地址**。
落地上至少要：
1. 界面上明说「请求会发到这里」；
2. 硬约束第 1 条（只读自己有权查看的对话）不受影响，
   但**图片外发那闸（`get_images_out`）的语义要重新对齐**——
   端点可换之后，「第三方模型」这个词不再准确。
