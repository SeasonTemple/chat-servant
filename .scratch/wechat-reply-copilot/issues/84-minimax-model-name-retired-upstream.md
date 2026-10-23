# 84: 上游把 MiniMax 的模型名下线了——清单里那唯一一个名字 404

**Status:** done（2026-10-23 定位、修复；四条守卫；真机真调复核）

## 一、现象

面板能读到对方的消息、会进「正在起草」，但**永远 0 候选**。

日志里其实有，只是不显眼：

```text
WARN chatservant_reason::seam: 判断失败，降级为盲拟 error=判断失败：
abab6.5s-chat (adapter: OpenAI) 请求失败：上游返回 404：
{"type":"error","error":{"type":"not_found_error",
"message":"The model \"abab6.5s-chat\" is no longer available. (2069)","http_code":"404"}}
```

⇒ 症状是「**上游把这个模型下线了**」，而我们**没有别的模型可用**。

## 二、根因：清单是**白名单**，而清单里那唯一一个名字死了

`Provider::MiniMax::models()` 当时只有**一个**名字：`abab6.5s-chat`。
而 [`Provider::accepts_model`] 对「实测过的厂商」是**白名单**：
不在清单里就当场拒收 ⇒ **清单里那个死名字 = 唯一能选的名字**
⇒ 设置页也选不出任何一个能用的模型。

⚠ 上游会下线模型名，而清单是**写死在代码里的快照** ⇒ 这类缺陷不会自己暴露，
只有真调一次才会撞上。

## 三、实测（2026-10-23，真调上游）

`https://api.minimaxi.com/v1/chat/completions`（key 只从环境变量读，不落盘、不进日志）

| 名字 | 上游 `/v1/models` 里列了吗 | 真调结果 |
|---|---|---|
| `MiniMax-M3.1-Flash` | 列了 | **HTTP 200** ✓ |
| `MiniMax-M3` | 列了 | HTTP 200 ✓ |
| `MiniMax-M2.7` | 列了 | HTTP 200 ✓ |
| `MiniMax-M2.7-highspeed` | 列了 | HTTP 200 ✓ |
| `MiniMax-M2.5` | 列了 | HTTP 200 ✓ |
| `MiniMax-M2.5-highspeed` | 列了 | HTTP 200 ✓ |
| `MiniMax-M2` | **列了** | **404** ✗ |
| `MiniMax-M2.1` | **列了** | **404** ✗ |
| `MiniMax-M2.1-highspeed` | **列了** | **404** ✗ |
| `abab6.5s-chat`（老名字） | 不列了 | **404** ✗ |

⇒ ⚠⚠ **上游的 `/v1/models` 清单会说谎**：它列出来的三个真调全是 404
⇒ **只有真调才算数**，清单只是候选。

## 四、还有一个坑：旗舰模型把思考塞在 `content` 里

真调一次 `MiniMax-M3`（用的是本仓库起草提示词的原话）拿到的是：

```text
<think>We need output JSON only candidates 1-3 …</think>

{"candidates":[{"text":"有呀，大概几点？","probability":0.5}, …]}
```

而本 crate 的判断/起草/排序三步都要**从 `content` 里解析 JSON**
⇒ 前面多一段 `<think>` ⇒ **解析失败** ⇒ 起草空候选 ⇒ 面板上什么也没有。

而 `MiniMax-M3.1-Flash` 与 `M2.7-highspeed` 把思考放进**独立字段**
`reasoning_content`，`content` 里就是干净的 JSON。

⇒ 修法两条：① 清单换成今天真调得通的 6 个；② 传输层剥掉 `<think>…</think>`。

## 五、改了什么

- `crates/reason/src/transport.rs`
  - `Provider::MiniMax::models()`：换成 6 个**当天真调过**的名字，
    默认取 `MiniMax-M3.1-Flash`（content 干净、且是最新）；
  - 新增 `strip_think()`，`ask()` 的返回统一过它；
  - `Provider::adapter()` 的注释订正：**genai 猜的适配器变了**——
    `MiniMax-*` 现在会被 genai 猜成**它自带的 MiniMax 适配器**
    （Anthropic 兼容协议），不是老名字那会儿的 Ollama
    ⇒ **同样错，错法不同**（依据：`genai-0.6.5` 的 `from_model`）；
  - `Provider::MiniMax` 的文档订正（老名字已 404，清单是快照）。
- `ui/src-tauri/src/watch.rs`
  - **推理失败的原因也进日志**（原来只在面板的 `reason` 里，
    日志里没有 ⇒ 线上只能靠看界面截图）。

## 六、守卫

| 守卫 | 钉住什么 |
|---|---|
| `the_dead_minimax_model_name_is_not_offered` | 死名字不许回清单 |
| `the_minimax_list_only_holds_models_that_answered` | 清单逐字等于那 6 个；默认名在清单里 |
| `think_blocks_are_stripped` | 剥思考块的四种形态 |
| `a_thinking_model_still_yields_candidates` | **走产品路径**（假上游 + `crate::turn`）：带 `<think>` 的 JSON 也必须解析出候选 |

⚠ 最后那条**走产品路径**：只测 `strip_think` 那个函数量的是「它自己」，
不是「产品那条路」——与票 83 的教训同族。

## 七、验收项

- [x] 现象定位到**上游 404**，不是本地解析或网络问题
- [x] 清单里每个名字**当天真调过**（含「上游清单会说谎」那三个反例）
- [x] 传输层剥 `<think>`，且**走产品路径**验过（不是只测那个函数）
- [x] 死名字不许回清单（有守卫）
- [x] 推理失败的原因进日志
- [x] genai 猜适配器那条注释按**现在的** genai 源码订正
- [x] 真机复验两遍都出候选：夹具帧一遍、**实时抓帧**一遍（见第八节）

## 八、真机复验（夹具帧源，真实 OCR + 真实模型调用）

用夹具帧（`wechat-now.png`）起应用：第一轮读到对方的话 ⇒ 触发推理
⇒ 模型这次真调得通 ⇒ 面板上出现**候选**（此前是 0 候选）。

```text
01:08:25.084  INFO  chat_servant::watch: tick=1 peer="" candidates=0
01:08:27.189  INFO  chat_servant::pump: emit readout tick=1 phase=drafting peer="小号" messages=5 candidates=0
01:08:33.799  INFO  chat_servant::pump: emit readout tick=1 phase=drafted  peer="小号" messages=5 candidates=3
```

⇒ `phase=drafted` + `candidates=3`：**判断、起草、排序三步都跑通了**。
⇒ **零告警**（没有 404、没有「判断失败、降级为盲拟」、没有「推理失败」）。
⇒ 这一轮模型耗时 **≈6.6 s**（27.189 → 33.799），与代码里记的「6~9 秒」吻合。

⚠ 用的是**清单默认**（`MiniMax-M3.1-Flash`）：凭据管理器里那把 key 是
minimax 的（`chat-servant/llm/minimax`），而**模型名不持久化**
（`ModelCell::new` 每次取 `default_model()`）⇒ 改清单即刻生效，
不需要用户去设置页改任何东西。

### 实时模式（**真窗口 + 真模型**）也复验过

同一套二进制改成实时抓帧（不夹具）再起一次：

```text
01:11:35.586  INFO  chat_servant::watch: tick=1 peer="" candidates=0
01:11:37.557  DEBUG roi=Rect { x0: 0, y0: 60, x1: 840, y1: 587 } ocr总行=20 本轮行=17 归类=17 消息=7 本轮新到=7
01:11:37.692  INFO  chat_servant::pump: emit readout tick=1 phase=drafting peer="小号" messages=7 candidates=0
01:11:58.715  INFO  chat_servant::pump: emit readout tick=1 phase=drafted  peer="小号" messages=7 candidates=3
```

⇒ 实时抓帧读到 7 条消息 ⇒ 推理三步跑通 ⇒ **3 条候选**，全程**零告警**。

⚠ 这一轮模型耗时 **≈21 s**（夹具那轮是 6.6 s）⇒ 上下文更长、模型更慢。
⇒ 面板在这 21 秒里显示的是「正在起草」那一帧（轮中推出去的那一帧），
**不是一个不动的面板**——这正是票 21 当初要的效果。
⇒ 但 21 秒对用户来说仍然偏长，**这一条留着观察**（不是本票的范围）。

## 九、仍然没做的

- **上游模型清单仍然是快照**。今天验过的 6 个名字明天也可能被下线
  ⇒ 仓库里没有任何机制能自动发现这件事，只有真调会撞上。
  正解是「设置页里加一个『测一下这个模型』的按钮」——那是另一张票的事。
- **其它厂商的清单没重验**：OpenAI / Anthropic / DeepSeek / Qwen / Zhipu
  那些名字都是抄自文档的（代码里自己写着「未实测」），今天没动。
- **真机上「对方发来新消息 ⇒ 出候选」仍待人眼**（这台机器禁止输入注入，
  最后一步要人从小号发一条消息）。
