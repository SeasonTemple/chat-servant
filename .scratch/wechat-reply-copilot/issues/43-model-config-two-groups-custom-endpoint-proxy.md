# 43: 模型配置应该是**两组**，且要能配自定义端点与代理

**What to build:** 把「一个供应商列表 + 一把 key」扩成
**两组独立配置 + 常见 provider 预设 + 自定义兼容端点 + 代理**。

**Status:** done · **2026-10-15 九项验收全勾、门禁九步全绿（`GATE: ALL GREEN`）。2026-10-16 订正标签：原先一直挂着 `ready-for-agent · 待做`，而票面九项早已全勾——是 `ticketstatus.py` 改用「标签优先」之后才把它从「未收」里捞出来**

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
- [x] 设置页拆成**两组独立配置**，互不影响
      ——`view.groups` 两节，key 目标名 `chat-servant/<组>/<供应商>`；
      守卫 `the_two_groups_are_independent_and_share_one_proxy`
      （2026-10-15 落实现）
- [x] provider 列表支持「预设 + 自定义」两类，协议选择与厂商解耦
      ——`Provider::PRESETS` + `Provider::Custom`，协议单独一选
- [x] 自定义 base_url 能生效（产品路径走得到，不只是测试路径）
      ——⚠ 关键在于**把 `with_endpoint` 降级成 `#[cfg(test)]`**
      （生产构建里那个函数根本不存在），产品路径只剩 `Route::custom`；
      守卫 `the_product_path_reaches_a_custom_endpoint` 断言的是
      **真回环端口上实际发出的字节**，走 `with_endpoint` 的测试证明不了这一条。
      另有 `switching_the_route_takes_effect_on_the_next_request`、
      `a_custom_endpoint_keeps_its_path_prefix`、
      `the_endpoints_are_our_own_measured_literals`
      （并钉住 `Provider::Custom.preset_base_url() == None`：**不许兜一个端点**）
- [x] **`redact` 覆盖端点与代理地址里的 token 形状** + 4 条守卫测试
      （2026-10-15 单独做了，不必等本票其余部分）——
      覆盖 `scheme://user:pass@host` 的用户名密码段与
      `?access_token=…&model=…` 这类查询参数；
      **同时钉住「无凭据的 URL 不许被动」**，否则报错会失去排查价值
- [x] **⚠ 补：上面那条原先只覆盖了一半 —— 强的那份够不着报错实际走的路**
      （2026-10-15，本轮补）
      代理与自定义端点的报错走 `crates/reason`（`ProxyCell::set` /
      `build_client` / `probe_route`），而那里原本有一份**只认 `sk-`** 的
      弱版 `redact` ⇒ 一个 `http://alice:hunter2@proxy.internal:8080`
      填错时口令会原样进日志。**两份能力不同的 `redact` ＝ 两条旁路。**
      ⇒ 纯字符串逻辑收到 `crates/core/src/redact.rs`（`shapes()`），
      `reason` 与 `secret.rs` 都调同一份；`secret.rs` 只保留
      「已登记的 key 值」那一路（那需要凭据管理器）。
      守卫：`a_proxy_address_never_keeps_its_credentials` /
      `a_custom_endpoint_never_keeps_its_token` /
      `a_clean_endpoint_stays_readable`（反向那条：宁可漏抹也不能没线索）
      + core 侧 6 条。
      **已验证会响**：临时把 `redact` 退回弱版，
      `a_proxy_address_never_keeps_its_credentials` 当场报
      「代理口令泄了：装不上这个代理：http://alice:hunter2@proxy.internal:8080」。
- [x] 代理配置对两组都生效，单独一栏不重复填
      ——`ProxyCell` 两组共用一个，`set_proxy` 一处写；
      守卫 `the_two_groups_are_independent_and_share_one_proxy`
      （清空代理后两组的端点都不变）
- [x] `save_setting`/`forget_key` 的 `PROVIDERS` 白名单同步放开
      ——**白名单整个没了**，不是放开：`Provider::from_key` 成了唯一的
      合法性判据，两处命令都改成按 `(group, provider)` 写格子。
      守卫 `provider_and_protocol_keys_round_trip`
      （预设与 `custom` 都要 `from_key` 得回来，`"nope"` 必须得 `None`）
- [x] 门禁九步全绿（2026-10-15，`GATE: ALL GREEN` / `GATE_EXIT=0`）

## ⚠ 这张票**没有**验到的部分（别当成已完成）

1. **两组设置 UI 从没被眼睛看过。** `?demo=1` 下那一屏走
   「演示模式没有后端，这里不列模型配置。」⇒ 两组区块**在演示页里
   根本渲染不出来**。它只过了 `svelte-check`（0 error 0 warning）与
   `pnpm lint`。补真机图要起 `pnpm tauri dev` 进设置页。
2. **自定义端点与代理从未在真机上试过。** 守卫全部跑在回环端口上
   （这是刻意的：不外网、不花钱），而真机的凭据管理器读写、
   环境变量优先级、实际连通性都没碰过。
3. **`get_images_out` 那道图片外发闸的语义**只改了界面文案
   （显示真实目标地址），闸本身的判定口径**没有重新对齐**——
   票面最后一条要求重新审视，尚未做。
4. **⚠ 顺带查出来的一个既有缺口，本轮扩大了命中面但没修**
   （`ui/src-tauri/src/watch.rs` 的 `Scratch::billing`）：
   视觉预算用 `core::scale::Billing`，而它只有 `Patch` / `Tile` 两档，
   **没有「未知」** ⇒ 签名强迫每个厂商返回一个数。
   `_ =>` 那一档退回 Anthropic 的 28px patch，
   原先只命中 MiniMax，本轮 provider 列表扩大后变成「除 OpenAI/Anthropic
   之外的一切」。
   ⇒ 对 DeepSeek / 通式 / 智谱 / Ollama / 自定义端点，**图片尺寸是用一个
   没核实过的公式算出来的**。真正的修法是给 `Billing` 加 `Unknown`、
   让调用方拒绝定尺寸（宁可不出图）。那是改 `core::scale` 公开枚举的事，
   会牵动 `encode_region` 与预算滑块，**不在本票范围内**。
   已在代码注释里写明「下面这个 `_` 不是不产出预算，它退回 Anthropic」——
   ⚠ 原注释写的是「如实不产出这个家的预算」，与代码不符。

## ⚠ 这一条会碰到硬约束

自定义端点 + 代理意味着**用户可以把数据发到任意地址**。
落地上至少要：
1. 界面上明说「请求会发到这里」；
2. 硬约束第 1 条（只读自己有权查看的对话）不受影响，
   但**图片外发那闸（`get_images_out`）的语义要重新对齐**——
   端点可换之后，「第三方模型」这个词不再准确。
