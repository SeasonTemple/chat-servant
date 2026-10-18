# Contributing

本仓库目前由**一位维护者**开发。欢迎 issue 与讨论；PR 需要先开 issue 讨论方向，
避免在硬约束上各改各的。

## 开发环境

**只支持 Windows。** 捕获链路走 Windows Graphics Capture + 本地离线 OCR
（默认 **PP-OCRv6 small + ONNX Runtime**；`Windows.Media.Ocr` 降为显式回退），
没有等价实现——在 Linux 上它连编译都过不了（`crates/capture` 整块是 Win32 FFI）。

```powershell
# 前端依赖（tauri CLI 已是 devDependency，不用另外 cargo install）
cd ui; pnpm install; cd ..

# 装 cargo-deny（门禁第 6 步要它）
cargo install cargo-deny --locked

pwsh -NoProfile -File scripts\gate.ps1     # 全绿
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

`scripts/gate.ps1` 是本机门禁，`.github/workflows/ci.yml` 与它的**步的集合**一一对应。
**两边要同步改**，否则会各说各话。

> 这句「一一对应」**现在有机器守着**：`scripts/check-gate-parity.ps1` 从
> `gate.ps1` 反推步名清单，要求每一步在 `ci.yml` 里都有一个见证命令；
> 少一步就红。它自己也在两边名单里。
>
> 2026-10-18 第一次真去核，才发现 `sheet pack in sync` **本地每次都跑、CI 里根本没有**
> ——而在此之前，`CONTRIBUTING.md` / `README.md` / `README.zh-CN.md` 三处都写着
> 「逐条对应」。**三份文档一致并不能让这句话更真一点**，那是最容易骗人的东西。

| # | 步骤 |
|---|---|
| 1 | 编码损坏扫描（U+FFFD） |
| 2 | `cargo fmt --all --check` |
| 3 | `cargo clippy --workspace --all-targets -- -D warnings` |
| 4 | `cargo test --workspace` |
| 5 | `cargo xtask secret-scan`（仓库里不许残留 key；**没设 `CHATSERVANT_PROBE_KEY` 时会跳过**，见下方陷阱 3） |
| 6 | `cargo deny check`（许可证 / advisory / 来源，见 ADR-0003） |
| 7 | `pnpm check`（svelte-check） |
| 8 | `pnpm lint` + `svelte-autofixer` |
| 9 | `pnpm build` |
| — | 贴纸包同步（`ui/scripts/sheetpack.ps1 -Check`）、门禁自身与 CI 的对齐、**版本号四处一致**（`scripts/check-version.ps1`）——这三步上表没单列，见下方注 |

> ⚠ **判断门禁成不成功看它打印的那两行**（`GATE: ALL GREEN` / `GATE: FAILED`），
> 不要只看退出码。2026-10-11 实测踩过：脚本只打印 `GATE_EXIT` 而不 `exit`，
> 于是 `pwsh -File` 永远返回 0，一个带红门禁的提交就提交上去了。

> ⚠ 上表是**清单，不是执行顺序**，也不是完整清单：`gate.ps1` 实际还会跑
> `sheet pack in sync`（纸的源与生成物必须同步）、`gate/CI parity`、
> `version consistency`、`license consistency`，并把 `pnpm lint` 与 `svelte-autofixer`
> 拆成两个 `Step`。
> 2026-10-16 起脚本每轮都会打印**自己的分步耗时表**，
> 那才是唯一准确的清单；**别数「几步」**——`secret scan` 在没设 `CHATSERVANT_PROBE_KEY` 时会跳过，两侧天生不等。

> ⚠ **改版本号要改四处**，一起改：`Cargo.toml` 的 `[workspace.package]`、
> `ui/src-tauri/Cargo.toml` 的 `[package]`、`ui/package.json`、
> `ui/src-tauri/tauri.conf.json`。后两个是 JSON，**没有继承可借**，只能手动对齐。
> 而 `release.yml` 的发版闸只比 **tag ↔ `tauri.conf.json`**——另外三处对不上它**不会拦**，
> 所以那道「四处一致」才有必要。守卫：`scripts/check-version.ps1`。

> ⚠ **license 只有一处真相**：根 `Cargo.toml` 的 `[workspace.package] license`（BUSL-1.1）。
> 其余五个成员各写一份。守卫：`scripts/check-license.ps1`，成员清单由
> `cargo metadata --no-deps` 给 ⇒ **新增 crate 自动被覆盖**，不用改守卫。
>
> ⚠⚠ **这道不能省，也不能指望 `cargo deny check` 顺带抓**（2026-10-18 实测）：
> MIT / Apache-2.0 都在 `deny.toml` 的白名单里，把某个成员写错成
> `MIT OR Apache-2.0` **不会**触发 "license is not explicitly allowed"；
> `deny.toml` 里那条 `{ allow = ["BUSL-1.1"], crate = "…" }` 只是变成一条
> `warning[license-exception-not-encountered]`，而 `cargo deny check` **仍然 exit 0**。
> ⇒ 在「写错成 permissive」这个方向上 `cargo deny` 是瞎的。
> （反方向是有效的：写错成 BUSL 而没进 exceptions，会 exit 1。
> 所以这道守卫守的是**白名单方向**，不是「许可没被允许」。）

### 内循环：改一行代码不必跑全门禁

```powershell
pwsh -NoProfile -File scripts\dev.ps1 help   # 动词清单
```

`dev.ps1` **不复制上面任何一条命令**，只转发给 `gate.ps1` 与 cargo/pnpm，
所以这张表与 `gate.ps1` 仍然各自只有一处真相。
完整门禁也能只跑一部分：`gate.ps1 -Skip 'deny,ui build'` / `-Only 'cargo,ui'`——
但那是**部分门禁**，脚本会在结论旁边打 `GATE_SCOPE: PARTIAL`。

## 维护陷阱

这几条每一条都真的让门禁红过，且症状都**不在日志里**。它们写在对应文件的行内
注释里，但那份文件你未必会打开，所以这里再列一遍。

### 1. 新增工作区成员 = `deny.toml` 必须补一行

本项目自己的 crate 用 **BSL-1.1**，而依赖白名单里**禁止** BUSL。这不矛盾：白名单
管的是**别人的 crate**，本项目的发布物不在其管辖内。但 cargo-deny 0.20 有两条限制，
导致这个口子只能**按包名逐个点名**开：

- `[licenses.private]` 表**没有** `allow` 键（只认 `ignore` / `ignore-sources` / `registries`）；
- `exceptions` 每项**必须**带 `crate`，`private = true` 这个键它不认。

所以**往工作区加一个新 crate** 时，`cargo deny check` 会以
`license is not explicitly allowed` 失败，必须去 `deny.toml` 的 `exceptions` 加一行
`{ allow = ["BUSL-1.1"], crate = "<新包名>" }`。

**判据**：只有本项目自己的包能这么开。**绝不要**把 `BUSL-1.1` 塞进 `allow`——
那等于给 BUSL 依赖也开了口子，直接违反 ADR-0003。

### 2. 本机 `core.autocrlf=true` 会被 `.gitattributes` 覆盖

仓库用 `.gitattributes` 的 `* text=auto eol=lf` 把行尾钉死成 LF，**覆盖**每台机器
自己的 git 配置。这不是多余的：GitHub 的 `windows-latest` runner 全局配置是
`core.autocrlf=true`，checkout 会把仓库里的 LF 转成 CRLF 落盘，而 prettier 3 默认
`endOfLine: "lf"`——于是同一个 prettier 版本、同一个 lockfile、同一批源码，
**本地全绿、CI 报 35 个文件**。

若哪天看到「同一条命令两台机器结论相反」，**先查行尾**，再看代码。
另外：别删 `.gitattributes` 里的 binary 规则（`*.png` 等），否则 git 可能开始
尝试对图片做行尾转换。

### 3. CI 的 secret-scan 证明不了「仓库里没有真 key」

CI 上**没有真 key 可查**，它只做两件事：确认扫描器跑得起来，以及
**先埋探针要求它报警、再撤掉要求它闭嘴**。

最后一条更重要：只跑一遍「没报警」证明不了任何事——**扫描器坏了和仓库干净，
屏幕上长得一模一样**。

真要查「这台机器的 key 有没有泄进仓库」，得在本机设了
`CHATSERVANT_PROBE_KEY` 再跑 `cargo xtask secret-scan`（不设就跳过）。

给 `.githooks/pre-commit` 加豁免用文件内标记：写 `secret-scan:` 后接一个空格、
再写 `allow`（钩子里那个变量就是这两段拼出来的）。**不要**习惯
`--no-verify`：一个总被绕过的钩子等于没有钩子。

⚠⚠ **这条本身就踩过一次坑，记在这里免得再踩**：判据是「文件里**任何一行**
含完整标记就豁免整个文件」，而**这段说明里若写出完整标记，这份文档就把自己
豁免了**——2026-10-18 就是这样，`.githooks/pre-commit` 与本文件双双免检，
往里粘一个真 key 闸门不会响（钩子已把标记拆两段修好，见那里的注释）。
⇒ **凡是要写这个标记的地方，先确认自己没有把两段连起来写。**
⚠ 需要加豁免的现有文件有四处（都持有**故意的假 key**）：
`crates/core/src/redact.rs`、`crates/reason/src/transport.rs`、
`ui/src-tauri/src/secret.rs`、`scripts/secret-scan-selfcheck.ps1`。

### 4. GitHub 上许可显示为 `Other`

BSL 1.1 不在 GitHub 的许可列表里，所以仓库页面会显示 `Other`。这是正常的，
以 [`LICENSE`](LICENSE) 文件为准——别为了让它显示成某个已知许可而去改许可。

### 5. `.githooks/` 只在本机生效

`git config core.hooksPath .githooks` 是**每个 clone 各配一次**的本地设置，
`.githooks/` 目录本身入库，但它不会自动被启用。克隆后记得跑上面那条命令，
否则 pre-commit 凭据闸和 commit-msg 校验对你不存在。

> ⚠ **判断门禁成不成功看它打印的那两行**（`GATE: ALL GREEN` / `GATE: FAILED`），
> 不要只看退出码。2026-10-11 实测踩过：脚本只打印 `GATE_EXIT` 而没 `exit`，
> 于是 `pwsh -File` 永远返回 0，一个带红门禁的提交就提交上去了。

### 6. 门禁全绿证明不了「OCR 认字还准」——动过 OCR 就自己跑一次台架

票 30 在「定线」那一节钉了三条质量下限（召回率 ≥80% / 整条全对率 ≥50% /
字符准确率 ≥75%），⚠ 而 `cargo test` **一个都不检查**。

按能不能拿到权重分三层：

| 层 | 需要 ONNX 权重？ | CI 上 |
|---|---|---|
| det 整链路（二值化 + 连通域 + unclip） | 否 | ✅ 真跑 |
| **rec 预处理**（裁剪 → resize → 归一化） | 否 | ✅ 真跑（2026-10-18 补的守卫） |
| rec 解码（推理 → CTC → 认出的字） | **是** | ⚠ **必然跳过** |

第三层要的那些权重在 `target/models/`、是 gitignored 的 31 MB，CI 上没有——
与 `crates/perceive/tests/golden.rs` 的「缺件即跳过」同一模式。
⚠ 而**真帧与答案键**也在 `target/` 下（真人会话内容，不入库），
所以**这一步只能人工跑**。它不是缺陷，是隐私约束的必然结果。

⚠⚠ **2026-10-18 起第三层多了一道机器守卫，但它守的不是真帧**
（票 50 的 L3）：`crates/capture/tests/ocr_accuracy.rs`
的 `the_synthetic_corpus_keeps_its_characters` 跑**完整 rec 推理**并算 CER，
夹具是 `scripts/ocrbench/gen_corpus.py` 造的**合成语料**（逐字答案键、不含真人内容）。

```bash
cargo test -p chatservant-capture --test ocr_accuracy -- --nocapture   # 默认 12 张，约 19 秒
$env:CS_OCR_CER_FULL='1'; cargo test -p chatservant-capture --test ocr_accuracy -- --nocapture   # 全量 60 张，约 88 秒
```

⚠ **它与下面的真帧台架不能互相替代**，两条各有各的射程：

| | L3 合成语料测试 | `cer_real_frame.py` 真帧台架 |
|---|---|---|
| 跑进 `cargo test` | ✅ 本机有权重时自动跑 | ❌ 只能人工 |
| 覆盖真实气泡/头像/缩放 | ❌ | ✅ |
| 覆盖「换引擎后字全崩」 | ✅ | ✅ |

⇒ 所以上面那张三层表里第三层「CI 必然跳过」的结论**没变**：
它在本机自动跑、在 CI 上照样跳过。⚠ **别把「CI 绿」读成「字准验过了」**。

⇒ **换了引擎、改了预处理、调了 det 阈值、动了 det/rec 后处理之后，
门禁绿了不算数**。有合成语料就先跑 L3（自动），再跑真帧台架（人工）：

```bash
pwsh -NoProfile -File target\probe\run_cer2.ps1            # 生成 4 份后端输出
target\bench-venv\Scripts\python.exe scripts\ocrbench\cer_real_frame.py --explain
```

⚠ 三个判据陷阱（都实测踩过）：

1. **看 stderr 有没有 `WARN …回落 Windows OCR`，别只看退出码。**
   回落时退出码**仍是 0**（回落是设计内行为，不是错误）⇒ 拿退出码判会全绿。
   而且 WinRT 约 34 ms、PP-OCR 约 550 ms——
   ⚠ **「快得不正常」本身就是回落信号**。
2. **两次跑必须用同一把尺子。** `cer_real_frame.py`（框顶 / ±30px）与
   `bench_real_cer.py`（框中心 / ±8px）算出来的数**不可比**。
   2026-10-18 就因为把这两个数并排，得出过一个错的结论（ADR-0006 第（四）节）。
3. ⚠ **L3 与台架也必须是同一把尺子。** L3 在 Rust 侧复刻了
   `bench_lib.normalize`（NFKC → 删全部空白 → 小写），为此专门加了
   `unicode-normalization`——⚠ 少这一步，两边的数字**不可比**。
   而 `normalize` 为什么删光所有空白，见 `bench_lib.py` 的模块 docstring。

台架的 `--explain`（逐条看配对）、`--drop-line`（测某一行的贡献度）、
`--tol`（扫容差、确认结论在稳定区）三个开关是定位问题的工具，
用法见 `scripts/ocrbench/cer_real_frame.py` 的 docstring 与 ADR-0006 第（四）节。

## 硬约束

这七条是产品底线，改动不得破坏。完整说明见 `AGENTS.md`。

1. 只读**你自己设备上、你有权查看**的对话。
2. 只用窗口级捕获 + 本地离线 OCR。不 hook、不注入、不读对方数据库、不解密。
3. **截图不落盘**。唯一例外是显式调试命令写 `target/probe/`，且必须提示用户。
4. **不碰钱**。转账、红包、收款相关界面元素一律不碰。
5. 发送分两档（`Manual` 仅填入 → `Auto` 自动发送），默认最低档，升档必须显式、
   可回退、会话结束自动回落。**档位由 Rust 侧持有**，前端只能请求改。
   ⚠ 与根 `AGENTS.md` 硬约束第 5 条逐字一致，改一边就要改另一边。
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
