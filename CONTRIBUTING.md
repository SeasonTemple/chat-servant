# Contributing

本仓库目前由**一位维护者**开发。欢迎 issue 与讨论；PR 需要先开 issue 讨论方向，
避免在硬约束上各改各的。

## 开发环境

**只支持 Windows。** 捕获链路走 Windows Graphics Capture + `Windows.Media.Ocr`，
没有等价实现——在 Linux 上它连编译都过不了（`crates/capture` 整块是 Win32 FFI）。

```powershell
# 前端依赖（tauri CLI 已是 devDependency，不用另外 cargo install）
cd ui; pnpm install; cd ..

# 装 cargo-deny（门禁第 6 步要它）
cargo install cargo-deny --locked

pwsh -NoProfile -File scripts\gate.ps1     # 九步全绿
cd ui; pnpm tauri dev                       # 开发模式
```

一次性打开本地钩子：

```powershell
git config core.hooksPath .githooks
```

`.githooks/commit-msg` 校验 Conventional Commits，`.githooks/pre-commit`
在凭据进历史前拦一道。两者都可 `git commit --no-verify` 跳过——但**不推荐**。

## 提交约定

遵循 [Conventional Commits](https://www.conventionalcommits.org/)：

```
<type>(<scope>): <summary>
```

**scope 用中文**——仓库历史就是中文 scope（`feat(推理):`、`fix(感知):`），
校验器为此专门放宽了字符类。常见 scope：`推理`、`感知`、`窗口`、`发送`、
`ocr`、`界面`、`托盘`、`门禁`、`docs`。

```powershell
feat(推理): 补上「我自己的说话风格」这一路
fix(感知): 切回会话后消息变少 —— 水位线住在引擎里
```

## 门禁

`scripts/gate.ps1` 是本机九步门禁，`.github/workflows/ci.yml` 与它一一对应。
**两边要同步改**，否则会各说各话：

| # | 步骤 |
|---|---|
| 1 | 编码损坏扫描（U+FFFD） |
| 2 | `cargo fmt --all --check` |
| 3 | `cargo clippy --workspace --all-targets -- -D warnings` |
| 4 | `cargo test --workspace` |
| 5 | `cargo xtask secret-scan`（仓库里不许残留 key） |
| 6 | `cargo deny check`（许可证 / advisory / 来源，见 ADR-0003） |
| 7 | `pnpm check`（svelte-check） |
| 8 | `pnpm lint` + `svelte-autofixer` |
| 9 | `pnpm build` |

> ⚠ **判断门禁成不成功看它打印的那两行**（`GATE: ALL GREEN` / `GATE: FAILED`），
> 不要只看退出码。2026-10-11 实测踩过：脚本只打印 `GATE_EXIT` 而没 `exit`，
> 于是 `pwsh -File` 永远返回 0，一个带红门禁的提交就提交上去了。

## 硬约束

这七条是产品底线，改动不得破坏。完整说明见 `AGENTS.md`。

1. 只读**你自己设备上、你有权查看**的对话。
2. 只用窗口级捕获 + 本地离线 OCR。不 hook、不注入、不读对方数据库、不解密。
3. **截图不落盘**。唯一例外是显式调试命令写 `target/probe/`，且必须提示用户。
4. **不碰钱**。转账、红包、收款相关界面元素一律不碰。
5. 发送分三档（`Manual` → `Confirm` → `Auto`），默认最低档，升档必须显式、
   可回退、会话结束自动回落。**档位由 Rust 侧持有**，前端只能请求改。
6. 静默期零调用——没有新的对方消息时不调任何模型。
7. API key 不落普通文件、不进日志。只从 Windows 凭据管理器或环境变量读。

### 仓库里不该出现的东西

- **凭据**。`.env`、`config.json`、`*.key` 已被 gitignore，`.githooks/pre-commit`
  再拦一道。但真正的防线是**别把它写下来**——只走凭据管理器或环境变量。
- **真实会话内容**。包括「脱敏过」的：`crates/capture/fixtures/` 里的黄金测试夹具
  来自实机抓帧，因此**不入库**，依赖它们的测试在缺件时**跳过**而不是失败。
- **`examples/`**。第三方参考项目，自带 GPL 内容，只读不入库。
- **`.claude/` `.minimax/` `.zcode/`**。技能库在各 CLI 下的副本，仓库只留
  `.agents/` 一份。

## 发布

发布由 **tag 驱动**，版本号是人打的，不是从 commit 算的——
这个项目 0.x 且硬约束还在改，「这一版该叫什么」不是机器能替人做的决定。

```powershell
# 1. 写双语发布说明（两个标题段缺一不可，release workflow 会校验）
#    docs/release-notes/v0.1.0.md，必须同时有 `## 中文` 和 `## English`

# 2. 本地门禁全绿
pwsh -NoProfile -File scripts\gate.ps1

# 3. 打 tag 推送——`.github/workflows/release.yml` 接手，
#    在 windows-latest 上出 msi + nsis 并挂到 GitHub Release
git tag -a v0.1.0 -m "v0.1.0"
git push origin main --follow-tags
```

没有 `docs/release-notes/vX.Y.Z.md` 时发布**不会失败**，但会保留 GitHub 自动生成的
notes 并打一条警告——不要把那条警告读成「策展已经做过了」。

> 发布 job 需要 `Settings → Actions → General → Workflow permissions`
> 为 **Read and write permissions**。

## 许可

本项目采用 [Business Source License 1.1](LICENSE)。到 **2029-10-12** 转为
Apache-2.0。在那之前允许评估、内部使用与修改，但不得把本项目作为
功能重叠的产品或服务对外提供。

贡献即表示你同意以本许可授权你的贡献。
