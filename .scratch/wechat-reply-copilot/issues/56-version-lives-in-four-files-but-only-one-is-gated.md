# 56: 版本号散在四个文件里，而发版闸只管其中一个——tag 打 v0.1.1 会在发版那一刻才炸

**What to build:** 让「可交付」这件事真的成立：下一个 tag 打出去能过。

**Blocked by:** 发版本身仍需授权（⚠ **打 tag 会真的发 release**，用户已定暂不发）。

**Status:** done · 2026-10-18。四处已对齐到 `0.1.1`，新增守卫 + 自证 4/4。

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

## 顺带记下的两处**没改**（都要用户拍板）

1. **`README.md:13` / `README.zh-CN.md:13` 的 release 徽章写着 `v0.1.0`，
   而仓库从来没有任何 release**（`gh release list` 为空；`v0.1.0` 那个 tag 指向
   2026-10-12 的初始提交、落后 HEAD 190 个提交）。
   改成 `v0.1.1` 等于**宣称一个不存在的发布**；去掉徽章是设计决定。
   ⇒ 留着，但**它是假声明**，发布那天要一起改。
2. **`ui/src-tauri/Cargo.toml` 写 `license = "MIT OR Apache-2.0"`，
   而工作区与 `LICENSE` 是 BUSL-1.1。** 许可声明属法律范畴，不擅自改。
   （该包 `publish = false`，不会作为 crate 上传。）

## 验收

- [x] 四处版本号一起改到 `0.1.1`
- [x] `scripts/check-version.ps1` 写成（剔注释 / 按节定位 / 解析失败即红）
- [x] 挂进 `gate.ps1`（纯读那几步里）+ `ci.yml` + parity 见证表
- [x] 自证 4/4 会响（含「注释冒充」与「结构变了」两组）
- [x] 门禁 12 步全绿（新增一步后 `TOTAL（12 步）`）
- [x] `CONTRIBUTING.md` 的门禁表与「改版本号要改四处」写进去
- [x] README 徽章与 app 壳子的 license —— **记下来，不擅自改**
