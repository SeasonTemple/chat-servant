# chat-servant

挂在你自己那个微信聊天窗口旁边的副驾：读到对方最新消息 → 判意图 →
给候选回复 → 按你选的档位填回输入框。

**它不理解对话，只观察和转述。** 截图不落盘，key 不进日志，
默认档位是「只填入、不发送」。

## 跑起来

```bash
# 装前端依赖（tauri CLI 已经是 devDependency，不用另外 cargo install）
cd ui && pnpm install && cd ..

# 开发模式（带热更新，前端走 devUrl）
cd ui && pnpm tauri dev

# 出安装包
cd ui && pnpm tauri build
```

首次运行会起一个常驻观察循环（`cs-watch` 线程），每秒抓一帧微信窗口。

**没有 API key 也能跑**：观察、OCR、锚定、候选卡片全部正常，
只是候选内容来自明确标注的预置传输（每条都带「预置」字样，不是模型输出）。

## 配置 key

key **只去两处**，配置文件、日志、命令行、前端 bundle 里一律没有：

1. **Windows 凭据管理器**，目标名 `chat-servant/<provider>`；
2. 环境变量 `<PROVIDER>_API_KEY`。

在设置页里填，存进凭据管理器。填完可以点「测连通」——
它打的是只读的 `/v1/models`，**不产生任何 token 费用**。

支持的 provider：`openai`、`anthropic`。

## 三个开关，每个都默认在安全的一侧

| 开关 | 默认 | 管什么 |
| --- | --- | --- |
| 发送档位 | **只填入** | 填进输入框停在光标处。要发送得你自己升档。 |
| 图片外发 | **关** | 对方发来的图片区域会不会被裁出、编码、发给模型。 |
| 会话身份闸 | 第一帧自动锁定 | 程序只对锁定的那一个会话动手；锁错了点「锁这个会话」。 |

档位与外发总闸**都由 Rust 侧持有**（`TierCell` / `ImagesOut`），
前端只负责请求改——「这条消息会不会真的发出去」不交给界面说了算。

## 隐私边界

- 只读**你自己设备上、你有权查看**的对话。不 hook、不注入、不读数据库、
  不解密、不碰对方进程内存。
- 截图**不落盘**。唯一的例外是 `cargo xtask probe` 这类显式调试命令，
  写到 `target/probe/`（已在 `.gitignore` 里）。
- 帧只驻留内存，不进日志、不上传。视觉调用是**整条链里唯一**会把画面
  发到第三方的地方，所以它有自己的总闸，默认关、开启必须显式。
- 转账、红包、收款相关一律不碰。
- key 只从凭据管理器或环境变量读，所有对外文本一律脱敏。

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
cargo xtask inject-probe    # 剪贴板逐字节往返 + 证明鼠标指针没动过
cargo xtask e2e             # 端到端（默认 dry-run，加 --inject 才真的填输入框）
cargo xtask secret-scan     # 仓库里有没有残留 key
```

**「为什么不动」时先跑 `caps` 和 `occluded`**——这两条能排掉绝大多数
「程序是不是坏了」的困惑。抓不到时错误信息会直接说清原因
（被盖住 / 已最小化 / 不可见），不会只丢一句「抓帧失败」。

## 门禁

提交前跑这一条，**八道全绿**才算完：

```bash
pwsh -File scripts/gate.ps1
```

fmt / clippy `-D warnings` / test / secret-scan / cargo deny /
svelte-check / lint / svelte-autofixer / build。

## 文档

- [`CONTEXT.md`](CONTEXT.md) —— 域术语与系统边界，**改代码前先读**
- [`docs/adr/`](docs/adr/) —— 决策记录（为什么这么做、以及否掉了什么）
- [`.scratch/wechat-reply-copilot/`](.scratch/wechat-reply-copilot/) —— 票据
- [`AGENTS.md`](AGENTS.md) —— 给 agent 的工作约定
