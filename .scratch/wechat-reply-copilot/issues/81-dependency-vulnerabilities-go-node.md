# 81: 依赖漏洞核实——Go、Node 那批公告查下来，本仓要处理的是哪几条

**What to build:** 把 2026-10-21 前后那批 Go / Node 安全公告，
逐层核实本仓（Rust crate / npm 包 / CI 运行时与 action）**到底有没有踩到**，
踩到的修掉，没踩到的把「为什么没踩到」写清楚。

**Status:** 部分完成 · **2026-10-22 仓库侧已修完并验过；剩 4 条要仓库主人拍板或动手**

## 起因

用户问：「最近不是有 go、node 的安全漏洞，检查是否需要处理相关依赖」。

⚠ 时间点很近：**Node.js 于 2026-10-21 发布安全更新**（10 个漏洞、4 个高危），
**Go 于同日发布 1.27.3 / 1.26.10**（印度 CA 私钥外泄 ⇒ 强制拦截受影响的中间证书）。
⇒ 本票开工当天就是公告次日。

## 核实范围（三层，不能只看一层）

| 层 | 本仓有什么 | 结论 |
|---|---|---|
| **Rust crate** | 6 个工作区成员 | `cargo deny check advisories` → **ok** ✓ |
| **npm 包** | `ui/` 的一套 SvelteKit 5 + Vite 6 | **6 条漏洞**（1 high + 3 moderate + 2 low）⇒ 已修 |
| **Go** | 仓库**无** Go 代码（无 go.mod / go.sum / *.go） | Go 的面只有**一个**：Vite 依赖的 **esbuild 是用 Go 写的** |
| **CI 运行时** | `node-version: 22`、`pnpm/action-setup@v4` | `setup-node` 取**最新 22.x** ⇒ 自动拿到 22.23.4（已含昨天那批修复）✓ |
| **CI action** | 6 个：checkout@v6 / cache@v4 / setup-node@v5 / setup-python@v5 / pnpm/action-setup@v4 / dtolnay-rust-toolchain | **没有一个是 Go 写的**（前五个 Node/TS，最后一个 Rust）⇒ 无 Go 系 action 的风险 |

## npm 侧：6 条 ⇒ 0 条

| 严重度 | 包 | 公告 | 修法 |
|---|---|---|---|
| **high** | vite ≤6.4.3 | GHSA-587h-7wmj-h5pm（launch-editor 的 `fs.existsSync` 在 Windows 上可绕过） | 升 vite |
| moderate | vite ≤6.4.3 | GHSA-698v-4r3x-3vcv（`escapeCmdArgs` 漏 `%` 转义） | 同上 |
| moderate | vite ≤6.4.3 | GHSA-r825-w8mm-5pxw（Editor CLI 参数误读 ⇒ 某些编辑器下任意命令执行） | 同上 |
| moderate | vite ≤6.4.3 | GHSA-rq7h-c2jc-7f22（`server.fs.allow` 外的文件可能被服务） | 同上 |
| low | vite ≤6.4.3 | GHSA-9jrq-w75r-8gcw（自定义中间件的跨域脚本执行） | 同上 |
| low | cookie <0.7.0 | GHSA-pxg6-pf52-xh8x（接受带越界字符的 name/path/domain） | **override**（见下） |

### vite：锁文件内升级即可

`vite: ^6.0.0` → `^6.4.5`（`pnpm update vite`）⇒ 5 条全清。

### cookie：必须 override（上游的版本要求覆盖不到修复版）

`@sveltejs/kit` 2.x 要求 `cookie: ^0.6.0`——**0.x 的 caret 锁死在 0.6.x**，
而修复版是 0.7.x ⇒ 不 override 就永远是那个有洞的版本。

⇒ override 写在 **`ui/pnpm-workspace.yaml`**（不是 `package.json#pnpm`）：
⚠ **pnpm 11+ 不再读 `package.json` 里的 `pnpm` 字段**（pnpm 12 会直接警告
「The "pnpm" field in package.json is no longer read by pnpm」），
而本仓的 `pnpm-workspace.yaml` **本来就已经是设置落点**（`allowBuilds` 在里面）。

⚠ cookie 0.7.0 起对非法输入**抛错**，而 kit 用它解析 dev server 的 cookie
⇒ **实跑验证过**：dev server 起得来（HTTP 200），
带畸形 cookie（`bad=%%%; evil`）再打一次也是 200 ⇒ 没炸。

## ⚠⚠ esbuild 的 override：**试过、实测失败、退回了**（别再试）

esbuild 是 **Go 写的**，它那个二进制里带着 Go 运行时 ⇒ 昨天那批 Go 公告
（GO-2026-4864/4869/4871 等）落在旧版 esbuild 身上。看着该升到 0.28.x，
但 **vite 6.4.5 + esbuild 0.28.2 实测构建失败**：

```
[vite:esbuild-transpile] Transform failed with 1 error:
!~{00b}~.js:1:23: ERROR: Transforming destructuring to the configured target
environment ("chrome87", "edge88", "es2020", "firefox78", "safari14" + 2 overrides)
is not supported yet
```

⇒ **esbuild 0.28 提高了最低支持 target**，而本仓**没设 `build.target`**
⇒ 用的是 vite 6 的默认老 target ⇒ 降级转换做不了。

⚠ 要升 esbuild，就得同时抬 `build.target`（es2022 / chrome100 一类），
而那是**产品决定**：抬 target = 放弃很老的 WebView2（chromium 87 是 2021 年的）。
⇒ 不在本票里悄悄做。

⚠ 而那批 Go 漏洞的**四条路径本仓一条都不走**：
`crypto/tls`（esbuild 不做 TLS）、`crypto/x509`（不做证书校验）、
Deno 安装器（我们是 Node）、esbuild 自己的 dev server（我们用 Vite 的）
⇒ **实际暴露面接近零**。
⇒ 最终 esbuild 留在声明范围内的最新 **0.25.12**（0.25.x 线内的所有修）。
⇒ **理由已写进 `ui/pnpm-workspace.yaml`**（注释里，免得下一个人再试一遍）。

## ⚠ 系统性缺口：npm 侧**从来没有被扫过漏洞**

- Rust 侧有 `cargo deny check advisories`（门禁一步），**npm 侧一步都没有**。
- `pnpm audit` **在本机跑不起来**：默认 registry 是 npmmirror，
  而它**没有实现 audit 端点** ⇒ `ERR_PNPM_AUDIT_ENDPOINT_NOT_EXISTS`。
  ⇒ 必须显式 `--registry=https://registry.npmjs.org` 才能扫。

⇒ 这就是那 6 条能一直躺在锁文件里的原因。

## 验收项

- [x] Rust 侧扫过：`cargo deny check advisories` → **ok**
- [x] npm 侧扫过：6 条（1 high / 3 moderate / 2 low）⇒ **修完 0 条**
      ⚠ **2026-10-22 同步远程后复核**：`pnpm audit --registry=https://registry.npmjs.org`
      → **No known vulnerabilities found** ⇒ 修复在锁文件更新后**仍然成立**
- [x] `vite` 升到 6.4.5，`pnpm build` / `pnpm check` / `pnpm lint` 全过
- [x] `cookie` override 到 0.7.2，**dev server 实跑**（含畸形 cookie）不炸
- [x] `esbuild` 的 override **试过并实测失败**，原因写进配置（别再试）
- [x] Go 侧核实：仓库无 Go 代码、CI 无 Go 系 action、Go 的面只有 esbuild
- [x] CI 的 Node 运行时核实：`node-version: 22` → setup-node 取最新 22.x
      ⇒ 自动拿到 22.23.4（含昨天那批修复）⇒ **不用改**
- [x] **本机 Node**：**2026-10-22 已升到 v24.22.1**（仓库主人点头后本机实做）
      ⚠⚠ **订正（2026-10-22 实测）**：原票面写「v24.20.0 …… 自己升（fnm）」——
      **版本号与工具两处都写错了**：
      · 本机 node 原为 **v24.11.0**（不是 24.20.0）⇒ 差距比原票面写的更大；
      · 本机 node **不由 fnm 管**（`fnm list` 只有 `* system`）⇒ 实际是 **nvm-windows**
        （`C:\nvm4w\nodejs\node.exe` 是符号链接 → `G:\nvm`；原已装 24.11.0 / 20.11.1）
        ⇒ 升级命令是 **`nvm install 24.22.1` + `nvm use 24.22.1`**，**不是 fnm**。
      ✅ **实做记录（2026-10-22）**：`nvm install 24.22.1` → 下载解压到 `G:\nvm\v24.22.1`；
      `nvm use 24.22.1` → 符号链接改指 `G:\nvm\v24.22.1`，`node --version` = **v24.22.1**。
      ⚠ **pnpm 不用重装**：`pnpm.ps1` 落在 `G:\node-v19.0.0-win-x64\node_global\`，
      但那个目录**没有 node.exe** ⇒ 它走 PATH 上的 `node` ⇒ 自动跟着换版本。
      实测 `pnpm exec node --version` = **v24.22.1**（升完复核过）。
      ✅ **换完没坏**：完整门禁重跑一遍 → **ALL GREEN / exit=0**（墙钟 ~50s）；
      `pnpm audit --registry=https://registry.npmjs.org` → **No known vulnerabilities found**。
      ⚠ 影响面：本机 node 只跑 **本地 dev server**（vite）与门禁的 ui 步，
      **CI 不受影响**（CI 用 `node-version: 22` → 自动拿 22.23.4）。
- [ ] **本机 Go**：**2026-10-22 实测 go1.24.12**（`D:\Go\bin\go.exe`）< 修复版 **1.27.3**
      ⇒ 要仓库主人自己升
      ⚠ 原票面写「go1.27.0」，**实测是 1.24.12** ⇒ 差的是**一个大版本**，不是小版本落后
      ⚠ 且 Go 公告明说：**只更新环境不重新编译，程序仍曝露**
      ⚠⚠ **但对本仓而言这条的风险是零**（2026-10-22 补的推论）：
      本仓**没有 Go 代码** ⇒ 本机 go 版本**不参与**任何构建。
      唯一进仓的 Go 产物是 **esbuild 的预编译二进制**（留在 0.25.12），
      而它**不做 TLS、不做证书校验**（上面「四条路径本仓一条都不走」那段）
      ⇒ Go 那批中间证书拦截公告在本仓**没有可达路径**。
      ⇒ 升 Go 是**机器卫生**，不是本仓的欠账；**不必为它卡住本仓的活**。
- [x] **加门禁步「npm audit」**：✅ **2026-10-23 23:00 已加**（形状按仓库主人拍板）
      ⚠ 但按本仓规矩，**新增门禁步要四处一起改**
      （`gate.ps1` + `ci.yml` + `release.yml` + `AGENTS.md` + `CONTRIBUTING.md`
      + parity 见证表）⇒ 不是顺手能加的，本票不自己动
      ⚠ 判据形状要定：取不到（网络/镜像源无端点）时**判红还是跳过**——
      按票 67 的先例（「取不到就红，不让它静默变成跳过」）应是**判红**，
      但那意味着国内网络下这道步会经常红 ⇒ **要仓库主人拍板**
- [ ] **vite 8 + kit 3 升级**（另一个项目级决定）：
      vite 8 用 **Rolldown（Rust）替掉 esbuild** ⇒ **那个 Go 二进制直接消失**，
      这才是 Go 那条线的长期解。⚠ 但 kit 3 是大版本（有 breaking changes），
      且会牵动 `@sveltejs/vite-plugin-svelte` 7.x ⇒ 是一次真正的升级项目

## 2026-10-23 23:00：「npm audit」门禁步已加（形状按仓库主人拍板）

### 形状：**取不到就跳过，但必须显眼**

- 扫得到、**0 条洞** → 绿；
- 扫得到、**有洞** → **红**；
- **扫不到**（端点不存在 / 网络不通）→ **跳过 + 打横幅**
  「本次**没有**扫 npm 侧」。

⚠ **为什么不是「取不到就红」**（票 67 的先例是判红）：
票 67 禁的是「**静默**变成跳过」。而这道步的「取不到」是**环境问题**
（国内网络 / 镜像源），**不是漏洞** ⇒ 判红会让门禁在本机**天天红**
⇒ 门禁一红就没人看 ⇒ **覆盖率归零，比不扫还糟**。
⇒ 折中：**跳过但必须显眼**（横幅 + 原文），不静默。

### 落点（按本仓规矩：新增门禁步四处一起改）

| 文件 | 改了什么 |
|---|---|
| `scripts/npmaudit.ps1`（新） | 形状本体（显式 `--registry=https://registry.npmjs.org`——默认源 npmmirror **没有 audit 端点**） |
| `scripts/gate.ps1` | 步 `npm audit` + **lane 登记**（net 轨；漏登记 `Assert-Lanes` 会直接红） |
| `.github/workflows/ci.yml` | `Gate 10b | npm audit`（同一形状：`set +e` 是必需的，否则 `set -e` 会把 step 打死） |
| `scripts/check-gate-parity.ps1` | 见证表加一行（`npm audit` → `pnpm audit`） |
| `AGENTS.md` / `CONTRIBUTING.md` | 门禁步清单（两份都写） |

### 实测

```text
$ pwsh -NoProfile -File scripts\npmaudit.ps1
No known vulnerabilities found
>>> npm audit: no known vulnerabilities found
exit=0
```

### ✅ 两条分支都实跑过了（2026-10-24 00:45）

| 场景 | 实测输出 | exit |
|---|---|---|
| 扫得到（默认 npmjs） | `No known vulnerabilities found` | **0**（绿） |
| **扫不到**（指到没有 audit 端点的源） | 横幅「**没跑到**（取不到 audit 端点 / 网络不通）」+「**本次没有扫 npm 侧**」+ pnpm 原文 | **0**（跳过，**不是红**） |

⇒ 「没跑到」那条分支**不再只是纸面上的**。
⚠ 为此给 `scripts/npmaudit.ps1` 加了一个覆盖口
`CHATSERVANT_AUDIT_REGISTRY`（**只为让这条分支可被验证**）：

```powershell
$env:CHATSERVANT_AUDIT_REGISTRY='https://registry.npmmirror.com'
pwsh -NoProfile -File scripts\npmaudit.ps1
```

⚠ 默认值**不许**改：npmjs 才是有端点、真正扫得到的那一个；
而 npmmirror 正是本机的默认源 ⇒ 它**没有实现 audit 端点**
（`ERR_PNPM_AUDIT_ENDPOINT_NOT_EXISTS`）⇒ 这也是为什么必须显式 `--registry`。

⚠ 「**有洞 → 红**」那条分支**逻辑上成立**（pnpm 退出码非零、输出里没有
网络错误特征 ⇒ 直接透传非零）⇒ 但它**没法在这台机器上实跑**
（要有一份**带洞的锁文件**才能触发）⇒ 记在这里，别声称验过。
## 2026-10-23 23:40 补记：这道步在 CI 上**第一次跑就挂了**（已修）

CI run #214（提交 `1987346`）红在 `Gate 10b | npm audit`：

```text
ParserError: D:\a\_temp\…ps1:6
Line |
   6 |  if [ "$code" -eq 0 ]; then
     |  ~   Missing '(' after 'if' in if statement.
```

⇒ **本仓的 CI runner 是 Windows**（日志里 `shell: C:\Program Files\PowerShell\7\pwsh.EXE`
与 `D:\a\_temp\…ps1` 两处都写着），而我第一版把逻辑写成 **bash 段**
⇒ 等于**给 PowerShell 喂 bash** ⇒ 解析阶段就挂，**一步都没跑到**。

⇒ 改法：**CI 调与本地门禁同一份脚本**（`./scripts/npmaudit.ps1`，`shell: pwsh`）
⇒ 形状（红/跳过+横幅）**不可能两边漂**；
⇒ parity 见证子串也改成**命令**（`npmaudit.ps1`）而不是步名。

⚠ 教训（同族：「本地绿 ≠ CI 绿」的老坑，这里是**新形态**）：
**先看 runner 是什么系统、默认 shell 是什么**，再决定这段脚本写成什么语言。
本仓的 runner 是 Windows ⇒ 多行脚本一律用 `shell: pwsh`（票 13 的 autofixer 步早就是这么写的）。

## 2026-10-24 npm 侧又红了：**两条 SvelteKit 公告**（`@sveltejs/kit < 3.0.0`）

`GHSA-pfhg-q3qv-9q6r`（增强表单提交跳转）与 `GHSA-27mw-82g8-phjj`
（预渲染页读到不可信 cache-control 头）⇒ 都是 **low**，都补在 **kit 3**。
⇒ `npm audit` 门禁步按设计判红（「扫得到有洞就红」）⇒ **门禁红了**。

### 处置：**给 npm audit 加豁免表**（与 `deny.toml` 的 exceptions 同款）

Rust 侧 `cargo deny` 本来就有豁免机制，npm 侧没有 ⇒ 公告一发布就**只能整包升级**。
⇒ `scripts/npmaudit.ps1` 加 `$exempt` 表：**逐条 GHSA + 逐条理由**，
门槛三条：① 逐条列编号（不许按包名豁免）② 理由必须是
「**这条路径在本产品里不存在**」（「危害小」不算）③ **新公告照样红**。

两条的理由（**都指着代码说了**）：
- `GHSA-pfhg-q3qv-9q6r` ⇒ 本产品**没有 form action**（SPA + Tauri IPC，
  `ui/src/` 里没有 `<form>` / `use:enhance` / `export const actions`）；
- `GHSA-27mw-82g8-phjj` ⇒ 触发前提是**有 HTTP 服务器**把不可信头带进预渲染页，
  而 Tauri 走 **`file://`** ⇒ 没有响应头 ⇒ 这条路径不存在。

**反向自证**：抽掉其中一条豁免 ⇒ 同一条公告立刻变成「不在豁免表」⇒ **判红**（exit=1）。

### ⚠ 升级到 kit 3 的**迁移步骤**（试过，记下来免得再走一遍）

试装 `kit@3.1.0 + adapter-static@4 + vite-plugin-svelte@7 + vite@8 + ts@6`，
**卡在四处**，已全部退回（工作树回到已验证可用的状态）：

1. **`svelte.config.js` 不再被读**（kit 3 直接报 `config_file_unsupported`）
   ⇒ 配置要搬进 `sveltekit(...)` 插件，且**形状变扁平**（不再有 `kit: { … }` 那层）；
2. **`$lib` 被删**（`module_removed_lib`）⇒ 新写法 `#lib`，
   **但 rolldown（vite 8 的打包器）认不出 `#lib`**
   （`Rolldown failed to resolve import "#lib/PaperPanel.svelte"`）
   ⇒ 只能用 kit 给的**逃生口**：`alias: { '$lib': 'src/lib' }`；
3. **构建前必须 `svelte-kit sync`** ⇒ kit 3 不再自动 sync，
   不 sync ⇒ `.svelte-kit/` 没生成 ⇒ **虚拟 CSS 模块加载不到**
   ⇒ postcss 把 `.svelte` 当 CSS 读 ⇒ 报 `Unknown word </style>`（**极具误导性**）
   ⇒ 修法：`package.json` 加 `"prepare": "svelte-kit sync"`；
4. **`tsconfig.json` 要继承 `$app/tsconfig`**（不是 `./.svelte-kit/tsconfig.json`），
   且 `compilerOptions.types` **必须含 `$app/types`**（覆盖了它 ⇒ `tsconfig_invalid`）。

⇒ 这是一次**框架大版本迁移**（连带 rolldown 换掉 esbuild），不是「顺手升一下依赖」
⇒ 留在本票里当**独立条目**做，不塞进一次缺陷修复里。