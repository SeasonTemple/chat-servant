# 56: 版本号散在四个文件里，而发版闸只管其中一个——tag 打 v0.1.1 会在发版那一刻才炸

**What to build:** 让「可交付」这件事真的成立：下一个 tag 打出去能过。

**Blocked by:** 发版本身仍需授权（⚠ **打 tag 会真的发 release**，用户已定暂不发）。

**Status:** done · 2026-10-18。四处已对齐到 `0.1.1`，新增守卫 + 自证 4/4。
同日用户拍板后，把票面记着的两处未决项也改了（README 假徽章、app 壳子 license），
并顺手补出**同型的第二个缺口**：license 也有「一份真相多份副本」而 `cargo deny` 看不见，
新增 `license consistency` 一步 + 自证 5/5。

## 缺陷（一句话）

`0.1.0 → 0.1.1` 的发版决策早就定了，而 `ui/src-tauri/tauri.conf.json` 的
`version` 一直是 `0.1.0`。`release.yml` 里那道
「tag 与 `tauri.conf.json` 的 version 必须一致」的闸**要到发版那一刻才炸**——
也就是攒完所有改动、打完包，才发现版本对不上。

## 更麻烦的一半：版本号有**四份**，闸只看一份

| 文件 | 字段 | 发版闸管吗 |
|---|---|---|
| `Cargo.toml` | `[workspace.package] version` | ❌ |
| `ui/src-tauri/Cargo.toml` | `[package] version` | ❌ |
| `ui/package.json` | `.version` | ❌ |
| `ui/src-tauri/tauri.conf.json` | `.version` | ✅ |

⚠ `ui/src-tauri/Cargo.toml` 走的是**硬编码**，不是 `version.workspace = true`
（app 壳子不跟工作区继承那一套）；后两个是 **JSON，根本没有继承可借**。
⇒ 「只能手动对齐」，而「只能手动」就意味着需要一道闸。

## 已修：新增门禁一步 `version consistency`

`scripts/check-version.ps1`，挂在已有的纯读文件那几步里（排在编译前）。
`ci.yml` 加 `Gate 4 | version consistency`，`check-gate-parity.ps1` 的见证表
同步加一行——**后者是自动的**：给 `gate.ps1` 加步而不加见证，它会直接喊。

### 判据的两条形状，都是踩过的

① **不许被注释满足。** TOML 那两处**逐行剔掉 `#` 开头行**再按**节**定位，
   不做全文搜。否则我给 `[workspace.package]` 写的那段说明就能替真正的声明交账。
   （与 `check-gate-parity.ps1` 那次「注释替命令交了账」是同一条教训。）

② **解析不出来 = 红，不是跳过。** 找不到节、节里没有 `version`、JSON 挂，
   一律红并说明是哪一种。「量具失效」和「版本对不上」在屏幕上完全同形。

### ⚠ 自证时被同一个坑绊了第二次

第一版打印 `版本一致性：四处一致 = 0`——把 `0.1.1` 截成了 `0`。
原因和 `ui/scripts/autofixer.ps1` 那次**一模一样**：
`Sort-Object -Unique` 在只有一个结果时把数组**拆包成标量字符串**，
`$uniq[0]` 于是取到的是**第一个字符**。
⇒ `@($vals | Sort-Object -Unique)` 包一层。
**同一个坑在同一会话里踩两次**，所以这次直接写进脚本注释。

## 自证：`target/probe/version_selfproof.ps1` 5/5

| 组 | 注入 | 期望 | 实测 |
|---|---|---|---|
| 1 | `ui/package.json` 改成 `0.1.0` | 红，且报「四处不一致」 | `exit=1` ✓ |
| 2 | `[workspace.package]` 里插一行 `# version = "9.9.9"` 注释 | **读不到它**，仍报一致 0.1.1 | `exit=0`，输出里没有 9.9.9 ✓ |
| 3 | 把节名改成 `[workspace.pkg]` | 红，且报「读不到」 | `exit=1` ✓ |
| 4 | 从发布说明里**整行删掉** `## English` | 红 | `exit=1` ✓ |
| 5 | 还原 | 恢复一致 0.1.1 | `exit=0` ✓ |

⚠⚠ **第 4 组第一版写弱了，被自己打回来**：它把 `## English` 改成
`## ENGLISH` 企图骗过守卫，而 **PowerShell 的 `-match` 默认不区分大小写**
⇒ 守卫照样放行，用例却报了「守卫没抓住」。
⇒ 「用例失败」先问「是守卫错还是用例写弱了」。正解是整行删掉。
连带结论：`release.yml` 里那条同样的正则**对大小写是宽容的**，
本地守卫刻意**镜像**它，不单方面收紧成更严的规矩。

## 发布说明：v0.1.1 的双语稿

`docs/release-notes/v0.1.1.md` 已写好（`## 中文` / `## English` 齐全，
用 `release.yml` 第 289 行**逐字相同**的正则验过）。依据是
`v0.1.0..HEAD` 的 **203 笔提交**与其中 31 笔 `feat`/`perf` 的实际标题，
不是凭印象。

⚠ 发版闸的策略是「**存在**就必须双语，否则 `exit 1`；**不存在**只警告」——
本地守卫**镜像**这条，不自创更严的规矩。

## 原本记下的两处**没改**——现已在 2026-10-18 由用户拍板后改掉

用户 2026-10-18 的问卷结论：**两处都改**。留档是因为「当时的判断」本身有参考价值：
第一版把它们当「法律/设计范畴，不擅自改」记下来是对的（不该自己动），
但**记账不等于可以一直挂着**——挂着的那几天里没人拍板，它们就一直是一处静默的错误。

1. **`README.md:13` / `README.zh-CN.md:13` 的 release 徽章写着 `v0.1.0`，
   而仓库从来没有任何 release**（`gh release list` 为空；`v0.1.0` 那个 tag 指向
   2026-10-12 的初始提交、落后 HEAD 190 个提交）。
   ⇒ **已删**。改成 `v0.1.1` 等于宣称一个不存在的发布；徽章删掉后发版那天再加回来。
   ⚠ 这个仓库的「版本一致性」只有四处版本号，**徽章不在其中**——
     所以删掉之后没有任何守卫会喊，真发版时要记得手动加回。
2. **`ui/src-tauri/Cargo.toml` 写 `license = "MIT OR Apache-2.0"`，
   而工作区与 `LICENSE` 是 BUSL-1.1。** ⇒ **已改成 `BUSL-1.1`**。
   连带发现 `deny.toml:63` 还有一句「本项目源码是 MIT/Apache-2.0」是**同一错误的第二份副本**，
   一并订正（只改读到的那一份是这类问题的标准失败方式）。

### ⚠ 改完顺手验出来的真缺口：`cargo deny check` 在这个方向上**抓不到**

上面第 2 条改完，我去问「谁在守着它」。实测（把 license 改回 `MIT OR Apache-2.0`）：

| | 结果 |
|---|---|
| `cargo deny check` 退出码 | **0**（绿） |
| 它给的信号 | `warning[license-exception-not-encountered]` 一条 |

原因：MIT / Apache-2.0 **都在 `deny.toml` 的白名单里** ⇒ 写错成 permissive
不触发 "not explicitly allowed"；而 `deny.toml` 里那条
`{ allow = ["BUSL-1.1"], crate = "chat-servant-app" }` 只是变成一条**用不上的例外**，
warning 不影响退出码，而门禁只看退出码。

⚠ **反方向是有效的**：写成 BUSL-1.1 而没进 `exceptions` ⇒ `cargo deny` 会 exit 1。
   所以这道新守卫守的是**白名单方向**，不是「许可没被允许」。
   ⚠ 与版本号那条是**同一种病**：一份真相写在 N 个文件里，而现有闸只看其中 1 个
     （版本是 4 选 1，license 是「根 vs 其余 5 个」）。

## 已修：新增门禁一步 `license consistency`

`scripts/check-license.ps1`，同样挂在纯读文件那几步里（编译前）。
`ci.yml` 加 `Gate 5 | license consistency`（后续步号顺延到 14），
`check-gate-parity.ps1` 见证表同步加一行。

判据三条：

1. **唯一出处 = 根 `[workspace.package] license`**，剔 `#` 注释后按节定位。
2. **成员清单由 `cargo metadata --no-deps` 给，不硬编码包名** ⇒ 新增 crate 自动被覆盖。
   硬编码就等于「按出事的那个文件选范围」——下一个新增 crate 的人不会想起来改守卫。
   （`deny.toml` 的注释里本来就写着「补的时候让 `cargo metadata` 当裁判」。）
3. **比对的是 metadata 的解析后值**，不是逐文件 grep `license = "…"`：
   成员可以用 `license.workspace = true` 继承，字面写法与继承写法在文件里**长得不一样**，
   但实际许可必须相同。

⚠ 读不到（根没 license / 成员既没写也没继承 ⇒ metadata 里是 null）**一律红**，
  不当「跳过」——量具坏了和许可不一致在输出上同形。

### 自证 5/5（`target/probe/license_selfproof.ps1`）

| 组 | 注入 | 期望 |
|---|---|---|
| 基线 | 不动 | 绿 `exit=0`，6 个成员逐个点名 ✓ |
| 1 | app 壳子改回 `MIT OR Apache-2.0` | 红 `exit=1` ✓（cargo deny 在这里是瞎的） |
| 2 | 根 manifest 删掉 `license` | 红 `exit=1` ✓ |
| 3 | `license` 只留在注释里 | 红 `exit=1` ✓（不许被注释满足） |
| 4 | 还原 | 绿 `exit=0` ✓ |

⚠ 自证台架**不用 `git checkout` 还原**：工作区里有本次的未提交改动，
  `git checkout --` 会把它们一起吃掉。一律「读原值 → 改 → 验 → 写回」，
  `finally` 里**逐字比对**，不一致就 `exit 2`。
  ⚠ 第一版还在 finally 里打了「逐字一致 = `$LASTEXITCODE -eq 0`」——
    那是**上一个命令**的退出码，不是比对结果。真正的比对在函数内部完成。

## 验收

- [x] 四处版本号一起改到 `0.1.1`
- [x] `scripts/check-version.ps1` 写成（剔注释 / 按节定位 / 解析失败即红）
- [x] 挂进 `gate.ps1`（纯读那几步里）+ `ci.yml` + parity 见证表
- [x] 自证 4/4 会响（含「注释冒充」与「结构变了」两组）
- [x] 门禁 12 步全绿（新增一步后 `TOTAL（12 步）`）
- [x] `CONTRIBUTING.md` 的门禁表与「改版本号要改四处」写进去
- [x] README 徽章与 app 壳子的 license —— **已由用户拍板后改掉**（徽章删、license 改 BUSL-1.1）
- [x] 全量扫同名的错误副本（`deny.toml:63` 那句「本项目源码是 MIT/Apache-2.0」）
- [x] 新增 `scripts/check-license.ps1` + 挂 `gate.ps1` / `ci.yml`(Gate 5) / parity 见证表
- [x] 自证 5/5（含「cargo deny 抓得到的方向」与「读不到即红」两组）
- [x] 实测记档：改回错误 license 时 `cargo deny check` **exit=0**（它在这个方向是瞎的）
