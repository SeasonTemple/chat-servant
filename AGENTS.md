# chat-servant

桌面聊天回复助手。Rust 核心 + Tauri 2 外壳 + Svelte 5 前端，先打通微信单聊。

它挂在你自己的聊天窗口旁边：读到对方最新消息 → 轻量决策模型判意图 → 生成候选回复
→ 填入输入框，或按开关确认后发送。

## 硬约束

这几条是产品底线，代码里必须能看出对应实现，不要在重构中破掉：

1. **只读自己设备上、自己有权查看的对话。** 不代替任何人查看别人的聊天。
2. **采集只用窗口级捕获 + 本地离线 OCR。** 不 hook、不注入、不读对方数据库、
   不解密、不碰对方进程内存。
3. **截图不落盘。** 捕获到的帧常驻内存，不写磁盘、不进日志、不上传。唯一的例外是
   `xtask probe` 这类显式调试命令写入 `target/probe/`，且必须提示用户。
4. **不碰钱。** 转账、红包、收款相关界面元素一律不碰；起草的 system prompt 里
   也禁了这几个话题。
5. **发送是分档的，且默认档最低。** `Manual`（只填入）→ `Confirm`（二次确认后发送）
   → `Auto`。升档必须是显式的、可回退的、会话结束自动回落到 `Manual` 的操作。
6. **静默期零调用。** 没有新的对方消息时不调任何模型。十分钟没人说话就是十分钟零 token。
7. **API key 不落普通文件、不进日志。** 只从 Windows 凭据管理器或环境变量读，
   报错文本一律脱敏。

## Agent skills

### Issue tracker

Issues are markdown files under `.scratch/<feature>/`, with sequential permanent
numbering. See `docs/agents/issue-tracker.md`.

### Triage labels

Five canonical roles, default strings: `needs-triage`, `needs-info`,
`ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` at the root, ADRs in `docs/adr/`. Read `CONTEXT.md`
before writing code that uses domain terms. See `docs/agents/domain.md`.

## 常用命令

```bash
cargo xtask probe frame      # 抓一帧聊天窗口到 target/probe/，人工看像素
cargo xtask probe layout     # 打印消息区像素锚点结果

pwsh -File scripts/gate.ps1  # 本地一次跑完下面四道门禁
```

或者分开跑：

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo deny check             # 许可证/advisory/来源合规，见 docs/adr/0003
cd ui && pnpm check && pnpm lint
```

## 依赖约束

**只允许 permissive 许可证。** GPL / AGPL / LGPL / MPL、源码可见但限制商用的
（BUSL/SSPL/Elastic）、以及未声明许可证的依赖一律不用。

新增依赖时：先查 crates.io 上成熟 crate，再考虑自己写；许可证从 crates.io
元数据确认，不要从项目主页推测。策略由根目录 `deny.toml` 机器执行，
`cargo deny check` 是门禁之一。理由与选型记录见 `docs/adr/0003-dependency-strategy.md`。

读取参考项目时遵守净室：**只读实测结论，不复制任何代码**
（它的发布包受 GPLv3 污染，见 ADR-0003）。

## 前端约定

UI 在 `ui/`，Svelte 5 + Vite + Tauri 2。

- 写或改任何 `.svelte` / `.svelte.ts` 之前，用官方 autofixer 校验并迭代到零告警：
  `npx -y @sveltejs/mcp svelte-autofixer <path>`
- 查 Svelte 5 / SvelteKit 文档：`npx -y @sveltejs/mcp list-sections` 然后
  `npx -y @sveltejs/mcp get-documentation "<section>,<section>"`
- 后台一律走 Tauri `invoke` / `emit`，不轮询。Rust 侧是单一事实来源，
  前端不缓存会话状态。
- 所有网络调用都在 Rust 侧。前端拿不到 API key，也不该知道模型路由。

## 参考资料

- `.agent/skills/` 下有三份 ComputerUse 对照研究（GLM ZCode、MiniMax、Claude/Codex 对比）。
  **写捕获、窗口查找、输入注入之前先读**，里面已经踩过的坑不要重踩。
- `examples/jev-chat-windows/` 是被替代的参考实现（Python）。可以读它的
  `docs/KICKOFF.md`、`app/ocr.py`、`app/capture.py` 了解已实测的结论，
  **但不要移植它的代码**。
