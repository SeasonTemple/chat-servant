<div align="center">

# chat-servant

**挂在你自己那个微信聊天窗口旁边的副驾：读到对方最新消息 → 判意图 → 给候选回复 → 按你选的档位填回输入框。**

**它不理解对话，只观察和转述。截图不落盘，key 不进日志，默认档位是「只填入、不发送」。**

[![许可: BSL 1.1](https://img.shields.io/badge/license-BSL_1.1-orange.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-black.svg)](https://www.rust-lang.org/)
[![仅 Windows](https://img.shields.io/badge/platform-Windows%2010%2B-0078D4.svg)](https://learn.microsoft.com/windows/)
[![CI](https://github.com/SeasonTemple/chat-servant/actions/workflows/ci.yml/badge.svg)](https://github.com/SeasonTemple/chat-servant/actions/workflows/ci.yml)
[![版本: v0.1.0](https://img.shields.io/badge/release-v0.1.0-brightgreen.svg)](https://github.com/SeasonTemple/chat-servant/releases)

[English](README.md) · [简体中文](README.zh-CN.md) · [隐私边界](#隐私边界) · [跑起来](#跑起来) · [探针](#开发者探针) · [门禁](#门禁) · [参与贡献](CONTRIBUTING.md)

</div>

---

## 为什么是这么个形状

工作群消息多的时候，累的不是「读」，是「切上下文」。chat-servant 就挂在你
本来就开着的那个窗口旁边，把读的活干了，把草稿递给你。

真正起作用的是三条决定：

- **它不理解对话，只观察和转述。** 不读数据库、不 hook、不解密。捕获只有一条
  路：窗口级 WGC 帧 + 本地 OCR。
- **发送是分档的，且默认档最低。** 它把文本填进输入框就停住。到底有没有东西
  离开你的机器，由 Rust 侧回答，不由界面回答。
- **安静不要钱。** 对方没有新消息就不调模型。十分钟没人说话就是十分钟零 token。

## 隐私边界

这些是产品要求，不是实现细节。

- **只读你自己设备上、你有权查看的对话。** 不代替任何人查看别人的聊天。
- **不 hook、不注入。** 不读对方数据库、不解密、不碰对方进程内存。
- **截图不落盘。** 帧只驻留内存——不写盘、不进日志、不上传。唯一的例外是显式
  调试命令 `cargo xtask probe`，它写到 `target/probe/`，并且会告诉你它写了。
- **不碰钱。** 转账、红包、收款相关元素在界面上和起草提示词里都禁掉了。
- **key 只从 Windows 凭据管理器或环境变量读**，配置文件、命令行、日志、前端
  bundle 里一律没有。所有对外文本一律脱敏。
- **黄金测试夹具来自真实会话**，因此不入本仓库。依赖它们的测试在缺件时
  **跳过**，而不是通过。

## 三个开关，每个都默认在安全的一侧

| 开关 | 默认 | 管什么 |
| --- | --- | --- |
| 发送档位 | **只填入** | 填进输入框停在光标处。要发送得你自己升档。 |
| 图片外发 | **关** | 对方发来的图片区域会不会被裁出、编码、发给模型。 |
| 会话身份闸 | 第一帧自动锁定 | 程序只对锁定的那一个会话动手；锁错了点「锁这个会话」。 |

档位与外发总闸**都由 Rust 侧持有**（`TierCell` / `ImagesOut`），前端只负责请求改
——「这条消息会不会真的发出去」不交给界面说了算。

## 跑起来

**只支持 Windows 10 1803+**（需要 WGC）。Windows 11 更稳。

```bash
# 装前端依赖（tauri CLI 已经是 devDependency，不用另外 cargo install）
cd ui && pnpm install && cd ..

# 开发模式（带热更新，前端走 devUrl）
cd ui && pnpm tauri dev

# 出安装包（msi + nsis）
cd ui && pnpm tauri build
```

首次运行会起一个常驻观察循环（`cs-watch` 线程），每秒抓一帧微信窗口。

### 没有 API key 也能跑

观察、OCR、锚定、候选卡片全部正常，只是候选内容来自明确标注的预置传输——每条
都带「预置」字样，不可能被当成模型输出。

### 配置 key

key **只去两处**，配置文件、日志、命令行、前端 bundle 里一律没有：

1. **Windows 凭据管理器**，目标名 `chat-servant/<provider>`；
2. 环境变量 `<PROVIDER>_API_KEY`。

在设置页里填，存进凭据管理器。填完可以点「测连通」——它打的是只读的
`/v1/models`，**不产生任何 token 费用**。

支持的 provider：`openai`、`anthropic`。

## 开发者探针

诊断问题先跑探针，不要猜。全部**只读**，不写盘（`probe frame` 除外）。

```bash
cargo xtask caps            # 这台机器到底支不支持 WGC
cargo xtask occluded        # 窗口现在被别的东西盖住了吗
cargo xtask probe frame     # 抓一帧写 target/probe/，肉眼核对像素
cargo xtask read            # 抓一帧并立刻本地 OCR，打印读到的文字
cargo xtask perceive --file <png>   # 感知层：逐行归类 + 归并成消息
cargo xtask measure         # 标定：气泡底色、墨色、对比度
cargo xtask media <png>     # 媒体区域：检出「这里有一张图」，并走一遍编码链路
cargo xtask e2e             # 端到端（默认 dry-run，加 --inject 才真的填输入框）
cargo xtask secret-scan     # 仓库里有没有残留 key
```

**「为什么不动」时先跑 `caps` 和 `occluded`**——这两条能排掉绝大多数
「程序是不是坏了」的困惑。抓不到时错误信息会直接说清原因（被盖住 / 已最小化 /
不可见），不会只丢一句「抓帧失败」。

## 门禁

提交前跑这一条，**九道全绿**才算完：

```bash
pwsh -NoProfile -File scripts/gate.ps1
```

编码扫描 / `cargo fmt` / `cargo clippy -D warnings` / `cargo test` / secret-scan /
`cargo deny check` / `svelte-check` / lint + svelte-autofixer / `vite build`。
`.github/workflows/ci.yml` 与它逐条对应。

> **看它打印的那一行**（`GATE: ALL GREEN` / `GATE: FAILED`），不要只看退出码。

## 结构

| crate | 职责 |
| --- | --- |
| `crates/core` | 域类型与纯逻辑。`#![forbid(unsafe_code)]` |
| `crates/capture` | 窗口发现 + Windows Graphics Capture + OCR |
| `crates/perceive` | 把 OCR 框归类成对话人 / 噪声 + 会话身份闸 |
| `crates/reason` | **唯一允许出网做模型调用的阶段** |
| `xtask` | 开发者探针 |
| `ui/` | SvelteKit 5 前端 · `ui/src-tauri/` Tauri 2 外壳 |

Rust 侧是单一事实来源：前端只渲染后端发过去的东西，不自己存第二份会话状态。

## 文档

- [`CONTEXT.md`](CONTEXT.md) —— 域术语与系统边界，**改代码前先读**
- [`docs/adr/`](docs/adr/) —— 决策记录（含否掉了什么）
- [`docs/release-notes/`](docs/release-notes/) —— 双语发布说明
- [`AGENTS.md`](AGENTS.md) —— 给 agent 的工作约定
- [`.scratch/`](.scratch/) —— 票据

## 许可

[Business Source License 1.1](LICENSE) © SeasonTemple —— 于 **2029-10-12**
转为 Apache-2.0。
