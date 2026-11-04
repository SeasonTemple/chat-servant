<div align="center">

<img src="docs/assets/hero-banner.webp" alt="言侍 · chat-servant：暖黄撕边棉纸便笺与朱砂批注勾号" width="100%" />

# 言侍 · chat-servant

**桌面微信单聊辅助工具：通过窗口级捕获与本地离线 OCR 读取最新消息，分析对话意图并起草候选回复，按所选档位填入输入框或自动发送。**

**截图常驻内存不落盘 · API 密钥仅存系统凭据库 · 默认仅填入不自动发送**

[![许可: BSL 1.1](https://img.shields.io/badge/license-BSL_1.1-orange.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-black.svg)](https://www.rust-lang.org/)
[![仅 Windows](https://img.shields.io/badge/platform-Windows%2010%2B-0078D4.svg)](https://learn.microsoft.com/windows/)
[![CI](https://github.com/SeasonTemple/chat-servant/actions/workflows/ci.yml/badge.svg)](https://github.com/SeasonTemple/chat-servant/actions/workflows/ci.yml)

[简体中文](README.md) · [English](README.en.md) · [隐私与安全边界](#隐私与安全边界) · [快速开始](#快速开始) · [诊断探针](#开发者诊断探针) · [质量门禁](#质量门禁) · [参与贡献](CONTRIBUTING.md)

</div>

---

## 核心设计

**言侍（`chat-servant`）** 作为独立便笺面板停靠在微信聊天窗口旁，自动完成消息识别、意图判断与回复起草，减少高频沟通中的上下文切换。系统设计遵循以下三项基本原则：

- **非侵入式视觉采集**：不读取聊天数据库、不挂载进程钩子（Hook）、不解密本地存储。唯一的数据来源是 Windows Graphics Capture（WGC）窗口级画面捕获与本地离线 OCR（PP-OCRv6）。
- **分档发送与后端权威控制**：发送行为分为 `Manual`（仅填入输入框）与 `Auto`（自动发送）两档，默认处于最低档 `Manual`。发送档位与外发开关均由 Rust 后端统一持有，前端仅负责渲染状态与转发用户指令。
- **静默期零调用**：仅在感知层确认出现对方新消息时才触发模型推理；会话静默期间不发起任何网络请求，不产生 Token 消耗。

## 隐私与安全边界

以下约束属于产品级硬性边界，并由编译期检查或自动化测试强制执行：

- **仅限本机可见会话**：仅读取当前设备上用户本人已打开且有权查看的聊天窗口，不代管或采集他人会话。
- **零进程侵入**：不注入代码、不挂载钩子、不读取或解密对方客户端数据库、不访问目标进程内存。
- **画面帧不落盘**：捕获的窗口帧仅驻留内存，不写入磁盘、不计入日志、不上传云端。唯一例外是开发者显式执行调试命令 `cargo xtask probe frame` 时，会将单帧输出至本地 `target/probe/`（已被 `.gitignore` 排除）并在终端明确提示。
- **规避资金操作**：界面交互与起草系统提示词均严格屏蔽**转账、红包、收款、付款、支付**相关内容，拒绝代写任何涉钱回复（由单元测试 `the_draft_prompt_still_forbids_money_topics` 强制校验）。
- **凭据隔离与脱敏**：API 密钥仅从 Windows 凭据管理器或系统环境变量读取，绝不写入配置文件、命令行参数、日志或前端打包产物；所有对外日志与报错文本均经过脱敏处理。
- **真实语料与夹具隔离**：源自真实会话的像素级黄金测试夹具均通过 `.gitignore` 排除在公开仓库之外。依赖本地夹具的测试标记为 `#[ignore]`，常规 `cargo test --workspace` 会在汇总行中以 `N ignored` 显式标示未执行项；本机具备夹具时可通过 `cargo test --workspace -- --include-ignored` 完整运行。

## 安全控制开关

核心外发与会话控制项默认均置于最保守档位：

| 控制项 | 默认状态 | 作用说明 |
| --- | --- | --- |
| 发送档位 | **仅填入（`Manual`）** | 将候选回复填入聊天输入框并停留在光标处；需用户显式升档至 `Auto` 才会自动发送，且会话结束自动回落至 `Manual`。 |
| 图片外发 | **关闭** | 控制是否将对方消息中的图片区域裁剪、编码并发送至多模态模型。 |
| 会话身份闸 | **首帧自动锁定** | 基于会话标题栏像素指纹（`HeaderPrint`）识别目标会话，按指纹隔离维护最多 16 个会话分桶；可通过系统托盘菜单「锁定当前会话」重新绑定，或在设置中切换「锁定当前 / 自动跟随」模式。 |

发送档位（`TierCell`）与图片外发总闸（`ImagesOut`）均由 Rust 后端持有，前端不维护独立副本，确保外发边界不受界面层状态影响。

## 快速开始

**系统要求**：Windows 10 1803 及以上版本（依赖 Windows Graphics Capture API），推荐使用 Windows 11。

```bash
# 1. 安装前端依赖（tauri CLI 已作为 devDependency 内置，无需额外 cargo install）
cd ui && pnpm install && cd ..

# 2. 启动开发模式（支持前端热更新）
cd ui && pnpm tauri dev

# 3. 构建安装包（生成 msi 与 nsis 安装程序）
cd ui && pnpm tauri build
```

启动后，后端常驻观察线程（`cs-watch`）会监听目标微信窗口，结合本地离线 OCR 与增量行级缓存实时感知新消息。

### 无密钥体验模式

未配置 API 密钥时，窗口捕获、本地 OCR、气泡锚定与候选纸条交互均可完整运行。此时候选回复由内置的预置传输层提供，每条候选均带有明确的「预置」标识，便于在接入在线模型前验证本地全链路。

### 配置模型与 API 密钥

API 密钥仅从以下两处读取：

1. **Windows 凭据管理器**：凭据目标名为 `chat-servant/<provider>`；
2. **系统环境变量**：变量名为 `<PROVIDER>_API_KEY`。

可在应用设置界面直接录入密钥并保存至 Windows 凭据管理器。保存后可点击「测连通」调用只读的 `/v1/models` 接口验证连通性，该检查**不产生任何 Token 费用**。

支持两类模型服务商接入：

- **内置预设（Presets）**：涵盖 `openai`、`anthropic`、`minimax`、`deepseek` 及 Jev 专用决策端点等（完整预设列表以 `crates/reason/src/transport.rs` 中的 `Provider::PRESETS` 为准）；
- **自定义端点（`custom`）**：支持填入任意兼容 OpenAI 或 Anthropic 协议的服务端地址。

## 开发者诊断探针

项目内置一组 `xtask` 命令行探针，用于逐层诊断窗口捕获、OCR 与感知链路。除 `probe frame` 外，所有探针均为纯内存只读操作：

```bash
cargo xtask caps                    # 检查当前系统是否支持 Windows Graphics Capture (WGC)
cargo xtask occluded                # 检测目标微信窗口是否被遮挡、最小化或不可见
cargo xtask probe frame             # 捕获单帧并保存至 target/probe/ 供人工核对像素
cargo xtask read                    # 捕获单帧并立即执行本地 OCR，输出识别文本
cargo xtask perceive --file <png>   # 运行感知层：逐行归类发言方并归并为结构化消息
cargo xtask measure                 # 测量并标定聊天气泡底色、文字墨色与对比度
cargo xtask media <png>             # 检测图片消息区域并验证图像裁剪与编码链路
cargo xtask e2e                     # 运行端到端全链路测试（默认 dry-run，附加 --inject 才写入输入框）
cargo xtask secret-scan             # 扫描仓库中是否存在疑似泄露的密钥或凭据
```

若遇到面板无响应或未更新，建议优先执行 `cargo xtask caps` 与 `cargo xtask occluded`。捕获异常会直接返回具体原因（如窗口被遮挡、已最小化或不可见），便于快速排查环境问题。

## 质量门禁

提交代码前请运行完整本地门禁脚本，确保所有检查项全部通过：

```bash
pwsh -NoProfile -File scripts/gate.ps1
```

门禁涵盖字符编码与控制符扫描、`cargo fmt`、`cargo clippy -D warnings`、`cargo test`、密钥扫描、`cargo deny check`、`svelte-check`、前端 Lint + `svelte-autofixer` 以及 `vite build`。`.github/workflows/ci.yml` 与本地门禁步骤保持严格对齐，并由 `scripts/check-gate-parity.ps1` 自动校验步骤一致性。

> 请以终端末尾输出的 `GATE: ALL GREEN` / `GATE: FAILED` 横幅（而非单纯依赖进程退出码）作为门禁通过的最终判据。

## 项目结构

| 模块 / 目录 | 职责说明 |
| --- | --- |
| `crates/core` | 核心域类型、纯逻辑与状态机（启用 `#![forbid(unsafe_code)]`） |
| `crates/capture` | 窗口发现、Windows Graphics Capture 画面捕获与本地离线 OCR |
| `crates/perceive` | OCR 文本框发言方归类、噪声过滤与会话身份闸（`#![forbid(unsafe_code)]`） |
| `crates/reason` | 意图判断与候选回复生成——**唯一允许发起网络模型调用的阶段**（`#![forbid(unsafe_code)]`） |
| `xtask` | 开发者命令行诊断探针与自动化辅助工具 |
| `ui/` · `ui/src-tauri/` | SvelteKit 5 纸面前端与 Tauri 2 桌面外壳（IPC 命令、系统托盘、观察循环） |

架构遵循后端单一事实来源（Single Source of Truth）原则：所有会话状态、发送档位与外发控制均由 Rust 核心维护，前端仅负责视图渲染与交互事件转发。

## 相关文档

- [`CONTEXT.md`](CONTEXT.md) —— 核心域术语表与系统边界定义（修改代码前必读）
- [`docs/设计规范.md`](docs/设计规范.md) —— 纸面面板视觉与交互设计规范
- [`docs/adr/`](docs/adr/) —— 架构决策记录（含已否决方案与选型依据）
- [`docs/release-notes/`](docs/release-notes/) —— 中英双语版本发布说明
- [`CONTRIBUTING.md`](CONTRIBUTING.md) —— 贡献指南与常见维护陷阱
- [`AGENTS.md`](AGENTS.md) —— 面向协作 Agent 的工程纪律与工作流约定

## 许可证

本项目基于 [Business Source License 1.1 (BSL-1.1)](LICENSE) © SeasonTemple 发布：

- **源码可见与基本权利**：允许自由复制、修改与再分发，**非生产环境使用免费**；
- **附加使用授权（Additional Use Grant）**：试用评估、个人使用、组织内部使用及二次修改**始终允许**。受限范围仅限「将本作品作为面向第三方可访问的托管或嵌入式产品/服务提供，且其主要功能与本项目相同或实质重叠」（具体法律条款请以 [`LICENSE`](LICENSE) 原文为准）；
- **开源转换日（Change Date）**：自 **2029-10-12** 起，本项目许可证将自动转换为 **Apache-2.0**。

> **关于商业授权**：本仓库当前未开放商业授权购买通道，亦不默认授予上述受限场景下的生产使用权。如需商业生产授权，需事先与权利人（Licensor）另行协商。
