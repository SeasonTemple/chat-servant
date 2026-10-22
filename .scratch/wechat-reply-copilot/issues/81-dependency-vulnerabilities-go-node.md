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
- [ ] **加门禁步「npm audit」**：现在 npm 侧零覆盖 ⇒ 要加
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
