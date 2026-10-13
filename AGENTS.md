# chat-servant

桌面聊天回复助手。Rust 侧是**唯一事实来源**——前端只渲染后端发过去的东西，不自己推断。

Rust 核心 + Tauri 2 外壳 + Svelte 5 前端，先打通微信单聊。

## 硬约束

这几条是产品底线，代码里必须能看出对应实现，不要在重构中破掉：

1. **只读自己设备上、自己有权查看的对话。** 不代替任何人查看别人的聊天。
2. **采集只用窗口级捕获 + 本地离线 OCR。** 不 hook、不注入、不读对方数据库、
   不解密、不碰对方进程内存。
3. **截图不落盘。** 捕获到的帧常驻内存，不写磁盘、不进日志、不上传。唯一的例外是
   `xtask probe` 这类显式调试命令写入 `target/probe/`，且必须提示用户。
4. **不碰钱。** 转账、红包、收款相关界面元素一律不碰；起草的 system prompt 里
   也禁了这几个话题。
5. **发送是分档的，且默认档最低。** 只有两档：`Manual`（只填入）→ `Confirm`
   （二次确认后发送）。**2026-10-13 取消了第三档 `Auto`**（填入即发）——
   决定与理由见 `docs/设计规范.md` §10 第 3b 条，不要凭空加回来。
   升档必须是显式的、可回退的、会话结束自动回落到 `Manual` 的操作。
   ⚠ 这是**安全模型**，不是排版：代码与本文必须逐字一致，
   守卫是 `crates/core/src/dispatch.rs` 的 `the_tier_set_is_exactly_two_and_named`。
6. **静默期零调用。** 没有新的对方消息时不调任何模型。十分钟没人说话就是十分钟零 token。
7. **API key 不落普通文件、不进日志。** 只从 Windows 凭据管理器或环境变量读，
   报错文本一律脱敏。

## 工具调用纪律

**这台机器上装好的工具，优先用；别逐个文件翻，别把整份构建日志灌进上下文。**

| 工具 | 状态（2026-10-11 实测） | 什么时候用 |
|---|---|---|
| `rg` | ✅ 已装 | 跨文件找同构模式。**一次找齐**，别等编译报错一个一个冒出来 |
| `rtk` | ✅ 已装 | 输出过滤代理。`rtk err <cmd>` 只留错误、`rtk test <cmd>` 只留失败 |
| `pwsh` 7 | ✅ 已装 | 多行脚本、重定向、编码 |
| `ast-grep` | ✅ 已装（npm `@ast-grep/cli`） | 结构化搜索。**用 `ast-grep`，别用 `sg`**（已废弃） |

- 会话默认 shell 是 **Windows PowerShell 5.1**：`>` 默认写 UTF-16、`Format-Hex -Count`
  不存在。**脚本一律 `pwsh -NoProfile -File x.ps1`**，不要跨层传 `-Command` 引号。
- **PS 5.1 的 `Set-Content -Encoding UTF8` 会写 BOM**。写源码/配置用 `write` 工具，
  或 `pwsh` 里用 `[IO.File]::WriteAllText($p,$t,(New-Object Text.UTF8Encoding $false))`。
- 跑 build / lint / test 前面包一层 `rtk err` 或 `rtk test`。
- 用户点名某个工具时，**先 `Get-Command <name>` 验一下**再用；反过来，
  **别用「我没搜到」去反驳用户对环境的描述**——先问路径。

### ast-grep 在本项目的已知边界

- **这版不支持 `.svelte`**（`--lang svelte` 直接报 not supported）。本项目的 `invoke()`
  调用几乎全在 `.svelte` 里，**所以它在这里主要用 `.rs`**；前端侧用 `rg` 抓。
- 规则文件用 `ast-grep scan -r <rule.yaml>`，**不是 `--config`**（`--config` 会被静默忽略，
  表现为「规则没命中」，很容易误判成代码有问题）。
- `.rs` 里 `#[...]` 属性**不是 `function_item` 的子节点**。取「带 `#[tauri::command]`
  的函数」要分两步：先定位属性行号，再往下找 `pub fn`。

## Project layout

- `crates/core` — 域类型与纯逻辑。`#![forbid(unsafe_code)]`
- `crates/capture` — 窗口发现 + Windows Graphics Capture + OCR
- `crates/perceive` — 把 OCR 框归类成对话人 / 噪声 + 会话身份闸
- `crates/reason` — **唯一允许出网做模型调用的阶段**
- `xtask` — 开发者探针（thread-probe、e2e）
- `ui/` — SvelteKit 5 前端
- `ui/src-tauri/` — Tauri 2 外壳：命令层、托盘、观察循环
- `ui/static/demo/*.json` — 八态演示夹具，`?demo=1&state=<名字>`
- `.agents/skills/` — 技能库，68 个，**实际只有 6 个可用**（见「Skill 库」一节）
- `.github/` — CI（九步，与 `gate.ps1` 对齐）、release（tag 驱动，出 msi+nsis）
- `.githooks/` — commit-msg 校验 Conventional Commits、pre-commit 凭据闸

真实 `unsafe` 分布（`rg -c 'unsafe \{|unsafe fn '`，2026-10-11 实测）：

| 位置 | 处数 |
|---|---|
| `crates/capture` | 50 |
| `xtask` | 8 |
| `ui/src-tauri` | 4 |
| `core` / `perceive` / `reason` | **0** ✓ |

三处 `#![forbid(unsafe_code)]` 是真的守住了，不是摆设。

## Code style

⚠ **动 Rust 之前先读 `.agents/skills/rust-skills/SKILL.md`**（265 条规则的索引），
按主题取用，别整份灌进上下文。

- Rust edition 2024。lint 策略**单一出处**在根 `Cargo.toml` 的 `[workspace.lints]`。
- **per-crate 的锁写进 `src/lib.rs`**（如 `#![forbid(unsafe_code)]`），不要写进
  `[lints.rust]`——cargo **不允许 `[lints]` 继承与局部覆盖并存**，写了整个工作区
  manifest 都解析不了（报 `cannot override workspace.lints in lints`）。
- 门禁是 `-D warnings`，所以「warn 起步、逐条升 deny」在这套配置里不成立：warn ≡ deny。
  非默认 lint（如 `missing_docs`）**不要工作区一刀切**，逐 crate 补齐后再开。
- 中文注释解释**为什么**，不解释是什么。注释里写了实测日期的结论，改了代码要同步改。
- 档位、会话名这类值由 Rust 持有，界面只显示，不自己存第二份。
- **`///` 文档注释不能放在 `let` 语句前**——clippy `unused doc comment` 会让
  `-D warnings` 失败。写在语句上方的注释用 `//`。

## 常用命令

> ⚠ 下面这六条与 `scripts/gate.ps1`、`CONTRIBUTING.md`、以及 CI 的
> `ci.yml` / `release.yml` **逐字一致**。要改就四处一起改——本仓踩过
> 「两套门禁各说各话」，所以这里曾经写成 `cargo test --workspace
> --lib --bins --tests`，而门禁与 CI 跑的是不带过滤器的版本。

```bash
# 本机一次跑完全部门禁（九步）
pwsh -NoProfile -File scripts/gate.ps1; echo "exit=$LASTEXITCODE"

# 或者分开跑
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check                 # 许可证/advisory/来源合规，见 ADR-0003
cd ui && pnpm check && pnpm lint

# ⚠ `cargo deny` 要联网拉 advisory-db，而 github.com 直连不通（实测 2026-10-12：
#   `unable to access ... SSL_ERROR_SYSCALL`）。跑完整门禁前先把代理挂上，
#   环境变量会被 `gate.ps1` 继承：
$env:HTTPS_PROXY='http://127.0.0.1:7897'; $env:HTTP_PROXY='http://127.0.0.1:7897'
pwsh -NoProfile -File scripts\gate.ps1

# 启动与探针
cd ui && pnpm tauri dev           # 前端 dev server 在 http://localhost:1420
cargo xtask probe frame           # 抓一帧聊天窗口到 target/probe/，人工看像素
cargo xtask layout                # 打印消息区像素锚点结果（⚠ 顶层命令，不是 probe layout）
pwsh -NoProfile -File scripts\fgwin.ps1      # 前台窗口
pwsh -NoProfile -File scripts\winscan.ps1    # 只报可见窗口
pwsh -NoProfile -File scripts\shotwin.ps1 <hwnd> <out.png>   # 截窗口
```

⚠ **手写 Rust 之后必须先 `cargo fmt --all`**——门禁第一步就是 `fmt --check`，超长行必红。

## 前端约定

UI 在 `ui/`，Svelte 5 + Vite + Tauri 2。

⚠ **动 `.svelte` / `.svelte.ts` 之前先读 `.agents/skills/svelte-core-bestpractices/SKILL.md`**
和 `svelte-code-writer`（见「Skill 库」一节）。这俩是真货，不要凭手感写。

- 写或改任何 `.svelte` / `.svelte.ts` 之前，用官方 autofixer 校验并迭代到零告警：
  `npx -y @sveltejs/mcp svelte-autofixer <path>`
- 查 Svelte 5 / SvelteKit 文档：`npx -y @sveltejs/mcp list-sections` 然后
  `npx -y @sveltejs/mcp get-documentation "<section>,<section>"`
- 后台一律走 Tauri `invoke` / `emit`，不轮询。Rust 侧是单一事实来源，
  前端不缓存会话状态。
- 所有网络调用都在 Rust 侧。前端拿不到 API key，也不该知道模型路由。
- **设计定稿在 `docs/设计规范.md`，改界面前先读它。**
  ⚠ **`docs/面板设计.md` 已作废**（它第一行就写着），它保留是因为记着
  被否决的每一条做法连同理由——照着它做会重犯。

## 依赖约束

**只允许 permissive 许可证。** GPL / AGPL / LGPL / MPL、源码可见但限制商用的
（BUSL/SSPL/Elastic）、以及未声明许可证的依赖一律不用。

新增依赖时：先查 crates.io 上成熟 crate，再考虑自己写；许可证从 crates.io
元数据确认，不要从项目主页推测。策略由根目录 `deny.toml` 机器执行，
`cargo deny check` 是门禁之一。理由与选型记录见 `docs/adr/0003-dependency-strategy.md`。

⚠ **新增工作区成员时必须去 `deny.toml` 补一行。** 本仓自己的 crate 是
BUSL-1.1（见 `LICENSE`），而依赖白名单禁 BUSL——不矛盾，那条管的是别人的 crate。
但 cargo-deny 0.20 的 `exceptions` **必须**带 `crate`、`private = true` 不认，
所以只能逐个点名：`{ allow = ["BUSL-1.1"], crate = "<新包名>" }`。忘了补，
`cargo deny check` 就以 `license is not explicitly allowed` 失败。
完整陷阱清单见 [`CONTRIBUTING.md`](CONTRIBUTING.md) 的「维护陷阱」。

读取参考项目时遵守净室：**只读实测结论，不复制任何代码**
（它的发布包受 GPLv3 污染，见 ADR-0003）。

## 文档分工

**记东西之前先查这张表。** 每条信息只有一个落点，落在第二处就会立刻分叉。

| 要记的 | 落在 | 形式 |
|---|---|---|
| 硬约束、常用命令、踩过的坑 | **本文** | 逐字一致的条款 |
| 域词汇与硬约束的现在时 | `CONTEXT.md` | 术语表 |
| 一个决策，尤其是被否掉的替代方案 | `docs/adr/NNNN-*.md` | 被 supersede 而非原地改 |
| **待办与进展**（做到哪一步、验收过没有） | `.scratch/<feature>/issues/*.md` | 勾 `- [x]`，与提交同 commit |
| 设计规范、未决设计问题 | `docs/设计规范.md` / `docs/面板设计.md` | `§` 编号，可引用 |
| **前端做到哪一步了** | `docs/前端设计进度.md` | 单一出处，别在别处重复 |
| 环境事实（依赖版本、当前状态） | **直接跑命令** | `git log` / `cargo deny check` |

⚠ **最后一行是最容易违反的一条。** 抄一份 `git status` 或 `cargo test` 的
输出进仓库，必然过期，而过期之后没人会去删它。

⚠ **`docs/HANDOFF.md` 是切换设备时的冷启动包，不是进展日志。**
它只装「现在能干什么 / 哪些路已证伪 / 命令怎么跑」三样。
2026-10-13 之前它有九节按日追加的日记（770 行），把真正的冷启动信息压到了
一百多行处——那是 sediment，已经沉底了。

## Issue tracker

Issues are markdown files under `.scratch/<feature>/`, with sequential permanent
numbering. See `docs/agents/issue-tracker.md`.

**每完成一项就立刻勾 `- [x]`**，和提交放在同一个 commit——否则票据会变成反向骗人的东西。

### Triage labels

Five canonical roles, default strings: `needs-triage`, `needs-info`,
`ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` at the root, ADRs in `docs/adr/`. Read `CONTEXT.md`
before writing code that uses domain terms. See `docs/agents/domain.md`.

## Skill 库：`.agents/skills/`（68 个，一半有毒）

装着的 68 个 skill 里，**只有 6 个真的能用**，52 个 Tauri 系全是模板生成的空壳，
其中 **12 个还带编造的包名与命令**（npm 上查无此包）。所以「主动用 skill」
在本仓的正确含义是**先查这张表再决定读不读**，而不是照单全收。

### 唯一可信的 6 个

| skill | 什么时候读 | 为什么可信 |
|---|---|---|
| `.agents/skills/rust-skills/` | 动 Rust 代码前 | 265 条规则、285 个文件，逐条带 Bad/Good 代码。实测有用：`anti-empty-catch.md` 正好对应本仓「空的 `catch {}` 会藏 bug」 |
| `.agents/skills/svelte-core-bestpractices/` | 动 `.svelte` / `.svelte.ts` 前 | 有真实 Svelte 5 代码示例 |
| `.agents/skills/svelte-code-writer/` | 同上，作为文档查询入口 | CLI 工具，查 Svelte 5 官方文档 |
| `.agents/skills/ui-ux-pro-max/` | 要配色/字体/间距/UX 准则/图表时 | `data/` 下是可检索的真实数据，**含 `svelte.csv`** |
| `.agents/skills/frontend-design/` | 定视觉方向、判断「这设计是不是模板货」前 | 专治「一眼就是模板」的那种审美判断 |
| `.agents/skills/asd-ste100/` | 写难读的函数/命名/注释时 | 带 linter |

**调用方式**：这些在 `.agents/` 下，**不是**我这边的内置 skill 工具。
要读就 `read` 对应的 `SKILL.md`（如 `.agents/skills/rust-skills/SKILL.md`），
大 skill 先读 `SKILL.md` 索引再只取需要的规则文件，**别把 265 条全灌进上下文**。

### 半个可用

- `.agents/skills/ui-styling/` — 98 文件 5.6 MB，但它是 **shadcn/ui + Tailwind**
  方案。本仓 `ui/package.json` 里**没有 tailwind、没有 shadcn、没有 bits-ui/melt**，
  只有裸 CSS + `@lucide/svelte`。→ **抄它的 token / 无障碍 / 响应式**思路，
  **不要**引入它假设的组件库。只在明确需要无障碍或主题体系时才翻。
- `.agents/skills/design-system/` — token 分层（primitive→semantic→component）思路可用，
  后半段是讲 slide 的，本仓用不上。
- `.agents/skills/openspec-*/` — 4 个，**但仓库里没有 `openspec/` 目录，流程未初始化**。
  现有票据走 `.scratch/`，见上面「Issue tracker」一节。要用得先 `openspec init`。

### 明确不适用（别浪费时间）

`brand/`、`design/`、`banner-design/`、`slides/` — 品牌 VI、海报、HTML 演示。
本仓是桌面工具面板，不是品牌资产项目。

### ⚠ 52 个 Tauri 系 skill：默认按「不存在」处理

`.agents/skills/tauri*/` 整个家族（52 个）是**同一模板批量生成**的，
`SKILL.md` 约 70 行 + `examples/usage.md` 恰好 53 行、代码块数完全相同，
正文只有 When-to-use / Outputs / Scope / Gotchas（Gotchas 五条还是
「注意版本兼容性」这种通用废话），**没有一个字是本仓需要的具体 API**。

更糟的是**生成器把 skill 目录名直接当成了包名**，于是示例里出现查无此包的东西
（2026-10-13 复核：`https://registry.npmjs.org/tauri-plugin-tauri-ipc` 返回
`Not found`）：

```ts
// .agents/skills/tauri-ipc/examples/usage.md 原文
import { init } from 'tauri-plugin-tauri-ipc';        // ← npm 上不存在这个包
await invoke('plugin:tauri-ipc|do_something');        // ← 不存在这个命令
emit('tauri-ipc-event', { status: 'active' });        // ← 事件名就是目录名 + '-event'
```

真实写法是 `@tauri-apps/api/core` 的 `invoke('我的命令名')`，命令名必须对应
自己写的 `#[tauri::command]`，不可能长这样。照抄这段**编译不过**。

含编造包名/命令的 12 个：`tauri`、`tauri-ipc`、`tauri-window`、`tauri-security`、
`tauri-config`、`tauri-concept`、`tauri-build`、`tauri-mobile`、`tauri-scaffold`、
`tauri-setup`、`tauri-framework-security`、`tauri-framework-upgrade`。

⚠ **别把「5 个包名是真的」当成「这 5 个能用」**：`tauri-app-clipboard` / `-dialog` /
`-file-system` / `-http-client` / `-notification` 的示例确实引了真实包
（`@tauri-apps/plugin-clipboard-manager` 等），但它们仍是同一份 53 行模板，
而且**假设本仓装了这些插件——本仓 `ui/src-tauri/Cargo.toml` 里一个
`tauri-plugin-*` 都没有**，照抄等于引入一整个本项目不存在的依赖面。
**判据是「本项目用不用得上」，不是「API 是不是真的」。**

**规矩**：Tauri 2 的写法**以 `AGENTS.md` 的「容易踩的坑」和 `docs/` 为准**，
不引这些 skill 的示例。确实要查 Tauri 官方文档，走 `web_fetch https://v2.tauri.app/...`
或它们的 References 里列的链接，别读本地模板。
顺带一提：本仓 `tauri` 钉 `=2.12.1` 而 `tauri-build` 钉 `=2.7.1`，
这是**实测过的真实坑**（见 `ui/src-tauri/Cargo.toml` 注释），
而 `tauri-framework-upgrade` 讲的是 v1→v2 迁移——**跟这个问题无关，别指望它**。

### 其余约定

`skills-lock.json` 记录装了哪些版本。`.claude/` / `.minimax/` / `.zcode/` 是 CLI
为别的运行时做的 symlink，**不要手改它们的内容**。⚠ **仓库只跟踪 `.agents/`
一份**（2026-10-12 起）——那三个副本已 gitignore，入库的是四份逐字相同的
第三方文件，约 2400 个，纯冗余。

`examples/jev-chat-windows/` 是被替代的参考实现（Python）。可以读它的
`docs/KICKOFF.md`、`app/ocr.py`、`app/capture.py` 了解已实测的结论，
**但不要移植它的代码**。

## 容易踩的坑

- ⚠ **别只信门禁的退出码，要看它打印的那两行**（`GATE: ALL GREEN` / `GATE: FAILED`）。
  2026-10-11 实测：`gate.ps1` 原来只 `Write-Output "GATE_EXIT=$fail"` 而**没有 `exit`**，
  于是 `pwsh -File gate.ps1` 永远返回 0 —— clippy 红了、日志打了 `GATE: FAILED`，
  而外层 `$LASTEXITCODE` 是 0。后果是**一个带红门禁的 commit 被提交了**。
  现在补了 `exit $fail`，两者一致；但判断时仍以打印的那两行为准。
  **每次给 `gate.ps1` 加新步骤，都顺手确认它红了之后 `exit` 确实非 0。**
- **Tauri 2 ≠ Tauri 1。** `PredefinedMenuItem::separator`（小写，2.x）；
  `MenuItemBuilder::build(&manager)` 要传 manager。凭 Tauri 1 记忆写的代码编译不过。
- **`tauri.conf.json` 里定义的窗口在 `.setup()` 之前就创建并开始加载**（实测 2026-10-10）。
  所以面板能在 `build_tray` 跑之前就发出它的第一个 IPC —— 任何「托盘/全局状态准备好了吗」
  这类判断，**不能用「此刻查得到吗」当答案**。`commands.rs` 里的 `TRAY_STATE` 是三态
  （Pending/Ready/Failed），只有**明确失败**才降级。
- **`TrayIconBuilder::with_id(...)` 不会自动取应用图标**，而 `bundle.icon` 只喂安装包、
  不喂窗口 → 托盘**条目在、图标是空的**，而日志照样打「托盘图标已就绪」。必须显式
  `.icon(Image::from_bytes(include_bytes!(...)))`；`from_bytes` 被 `image-png` 特性门控。
- **ACL 少一条权限，界面上只会表现为「控件点不动 / 拖不动」**，控制台不冒泡、
  clippy / svelte-check / 测试一样都不报。已踩过：`capabilities/default.json` 缺
  `core:window:allow-start-dragging`，面板 `data-tauri-drag-region` 被静默拒绝。
- **主面板不许挪位置。** 托盘菜单「打开设置」走 `show_panel`（show+聚焦），
  **不是** `show_window`（那个会 `set_position(120,120)`）。
- **`cargo build --release` 改了 `ui/build` 不会重链**。改完前端要
  `touch ui/src-tauri/src/main.rs` 强制重链，否则你会拿一个陈旧的 embed 去验刚改的界面。
- **debug 二进制走 `devUrl`（localhost:1420），release 才走 `frontendDist`。**
- **`(await invoke<T>('...')) ?? x` 在 vite dev 的产物里会被编译成 `(await ...)()`**——
  多一个调用。生产产物正常。排查前端 bug 时**先确认 dev 与 prod 的产物差异**。
- **空的 `catch {}` 会把这类 bug 藏到看不见。**
- **`ui/pnpm-workspace.yaml` 的 `allowBuilds: {esbuild: true}` 不能删。** 设置名写错
  （用 `onlyBuiltDependencies`）**不会报错，只会静默不生效**，然后
  `pnpm install` 之后连 `pnpm build` 都跑不起来。
- **Win11 托盘溢出区（`TopLevelWindowForOverflowXamlIsland`）的 UIA 坐标不可信**——
  实测图标报的 `BoundingRectangle` 会跑到 popup 自己外面。展开按钮还是 **toggle**，
  连点两次会把已经展开的溢出区关掉。
- **`winscan.ps1` / `hittrace.ps1` 报的窗口矩形与 `SetCursorPos` 用的屏幕坐标不是
  一个空间**（DPI 虚拟化）。按坐标点东西**必须先 `SetProcessDPIAware()`**，
  或用 UIA 的 `BoundingRectangle`。
- **要抓窗口真实画面用屏幕 DC `BitBlt`，不要用 `PrintWindow`**——后者截 XAML island
  （托盘溢出面板就是）会把所有图标压成竖条。`BitBlt` 在 `gdi32.dll`，不在 user32。
- **`gate.ps1` 里的 `pnpm build` 会触发 Vite HMR，把前端状态重置回主区**——
  「设置屏莫名退回主区」不是产品缺陷，是 dev HMR。
- **造「另一个会话」的夹具，差别必须在字形/位置上，不许在亮度上。** 四个用亮度差
  造的夹具全绿，而会话身份闸从来没关过（硬约束第 1 条实测破口）。
- `docs/HANDOFF.md` 的冷启动内容（能干什么 / 已证伪的路 / 命令）**保持有效**，
  且每条都带实测日期与证据来源。2026-10-13 已把其中的按日日记与重复的硬约束
  清掉，改写后的版本是核实过的。若它与代码冲突，**以代码为准并立刻修文档**——
  它唯一可接受的陈旧形态是「正在清理」，而不是「说过算数」。

## 构建产物纪律（cargo 不会自动回收）

**stable 的 cargo 没有 GC。** 它只在产物失效时重新生成，从不删除旧的。
所以 `target/` 是**单调增长**的，除非手动清。

什么会让它突然翻倍——不是「构建次数多」，而是这两件事：

| 触发 | 后果 |
|---|---|
| 改 `[profile.*]` 任何一项 | 整份产物重编，**旧的一整套整体留下** |
| 增删任一 crate 的 feature | 同一个库产生**新哈希**的 rlib，旧的永不删除 |

实测 2026-10-10：一次改 profile + 一次改 feature，`target/` 从 4 GB 涨到
**10.4 GB**，其中 8 GB 是 cargo 永远不会自己清掉的垃圾。

**改完 feature 或 profile，立刻清对应 profile**（这两个是唯一会让占用「跳一下」的操作）：

```bash
cargo clean --profile dev      # 代价是下次全量重编 ~2 分钟
cargo clean --profile release   # 重新打包前再编回来
```

⚠ `cargo clean` **不带 `--profile` 会把两个都删掉**。清之前先确认那份产物是不是陈旧的。

根 `Cargo.toml` 已加 `[profile.dev] debug = 1` 与 `[profile.dev.package."*"] debug = 0`，
稳态 `target/` 从 10.4 GB 降到约 2.4 GB。

## 开发期纪律

- **前端改动靠 HMR，不要重启 exe。** dev server（1420）开着时改 `.svelte` / `.css`
  会实时推给正在运行的 debug 进程。只有改了 Rust 才需要重新编译。
- **取消后台任务 ≠ 杀掉它 spawn 的进程。** 实测：dev server 的后台任务被 cancel 后
  `node` 进程活了下来占着 1420，**而 HTTP 探 1420 又不通**——一个僵死的孤儿监听器，
  只看 HTTP 会误判成「没在跑」。重开之前先 `Get-Process node` 扫一遍。
- 脚本里**不要留中文字面量**：无 BOM 的 UTF-8 用 PS 5.1 跑会按 ANSI 解析。
  落文件后一律 `pwsh -NoProfile -File`。

## Security

- API key **只**进 Windows 凭据管理器，绝不落普通文件（`ui/src-tauri/src/secret.rs`）。
- `ui/static/demo/*.json` 不含真实聊天内容——演示页可能被部署到公开 URL。
- 截图不落盘（硬约束 3）。`target/probe/` 里的证据图是**显式调试命令**的产物，
  里面可能有真实聊天内容，**不要提交进 git**。
- ⚠ **`crates/capture/fixtures/` 已 gitignore（2026-10-12）。** 那里是
  `crates/perceive/tests/golden.rs` 与 `media.rs` 用的像素级黄金基线，
  **从实机真实聊天脱敏而来**——即使只剩布局几何与配色，来源仍指向用户会话，
  所以不入公开仓库。代价：依赖它们的用例**缺件即跳过**而不是失败。
  本机恢复：`cargo xtask fixture --name <n> --source target\probe\<帧>.png`。
  **别把「CI 全绿」读成「黄金基线验过了」。**
- 本仓库公开在 GitHub（`SeasonTemple/chat-servant`），许可为
  **BSL 1.1**（2029-10-12 转 Apache-2.0，见 `LICENSE`）。
  ⚠ 这与「依赖只允许 permissive」**不矛盾**：那条策略管的是**别人的 crate**
  （`deny.toml` 的 `allow` 白名单 + 逐包名的 `exceptions`），本项目自己的
  发布物不在其管辖内。别为了图省事把 `BUSL-1.1` 塞进 `deny.toml` 的 `allow`——
  那等于给 BUSL 依赖开了口子。

