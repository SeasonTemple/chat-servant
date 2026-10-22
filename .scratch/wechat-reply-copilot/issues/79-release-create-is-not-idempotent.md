# 79: tag 被重推时发布链路最后一跳必红，且前面的构建全白费

**What to build:** 让 `gh release create` 这一跳**幂等**——tag 已存在 release 时
覆盖资产与说明，而不是把整条流水线烧到最后一步炸掉。

**Status:** 修完 · **2026-10-22 本机验过逻辑；真机验证 = 下一个 tag 推送**

## 触发它的现象

同步远程后查 GitHub Actions，最近两个 Release run 都是红的：

| run | tag | 结果 | 时长 |
|---|---|---|---|
| `39621252744` | v0.1.1 | failure | 24m29s |
| `39621252547` | v0.1.0 | failure | 25m4s |

失败点都在最后一步，日志逐字是：

```
a release with the same tag name already exists: v0.1.1
Exception: ... 57 | … { if ($LASTEXITCODE -ne 0) { throw "gh release create 失败 (exit=$LASTEXITCODE)" } }
```

⇒ **构建二三十分钟，最后一步炸掉，什么都没发出去。**

## 根因：tag 被重推了

2026-10-21 那轮为了让票 67 的历史重写生效，`main` 与两个标签都 force-push 过
⇒ tag 指向变了 ⇒ GitHub 按 tag 触发了 Release 工作流
⇒ 而 `gh release create` 要求 release **不存在**。

⚠ 第一次重写只处理了 `main`、**漏了标签**，后来补上重指标签时又推了一次 tag
⇒ 同一个 tag 触发了两次发布尝试。

## ⚠ 顺带查出来的一件事：v0.1.0 **根本没有 release**

`gh release list` 只有一条 `v0.1.1`：

```
v0.1.1   Latest   v0.1.1   2026-10-20T07:44:44Z
```

⚠ 而 `git ls-remote --tags origin` 里 **v0.1.0 的 tag 是有的**
⇒ **v0.1.0 有 tag、没 release**：它的发布尝试（run `39621252547`）撞名失败，
而第一次（10-19 那次）的产物也没有落成 release。
⇒ ⇒ **Releases 页上现在只有 v0.1.1 一个版本**，
v0.1.0 的 tag 指向一个**从未公开发布过**的提交。

## 修法：这一跳改成幂等

判「这个 tag 的 release 在不在」：

- 在 ⇒ `gh release upload --clobber`（**按资产名替换**）+ `gh release edit --notes`
- 不在 ⇒ 原来的 `gh release create`

⚠ **不用「删了重建」**：那会**短暂把资产摘掉**，那是给用户看的窗口。
⚠ **判存在性用 `gh release list`（列表为空时照样 exit 0），不用 `gh release view`**
（不存在时它 exit 1）——这一步跑在 `$ErrorActionPreference = 'Stop'` 下，
而有些 pwsh 版本会把「原生命令非零退出」升级成终止性错误
⇒ **判存在性不该用一条注定会失败的命令**。

## 自证

- [x] **本机**：判存在性的那段代码实跑 ⇒ `v0.1.1` 走覆盖分支、
      `v0.1.2` / `v0.1.0` 走新建分支
- [x] **YAML 合法**：`yaml.safe_load` 解析通过，jobs = `verify` / `build`
- [x] `scripts/check-gate-parity.ps1` 仍 exit 0（它要解析这两个 workflow）
- [ ] **真机**：下一个 tag 推送时，Release run 走**新建**分支并出 msi+nsis
      ⇒ 且 tag 被重推时走**覆盖**分支而不是撞名失败
      → ⚠ **前半已验（2026-10-22）**：v0.1.2 真发，run `39737470359` success，
      `Publish | GitHub Release` 步**走的是新建分支**（判存在性时 v0.1.2 不在
      release 列表里）⇒ 出了 msi + nsis 两个资产。
      ⚠ **后半还没验**：tag 被重推 → 走覆盖分支那条路**至今没被真正执行过**
      ⇒ 而它正是当初两次红 run 的那条路。
      ⇒ **验它的代价很低**：对 v0.1.2 再推一次同样的 tag（内容不变），
      这一次就该走覆盖分支而不撞名。
      ⇒ ⚠ **但那会重推一个已发布的 tag** ⇒ 属于「对外可见」的动作，
      要仓库主人点头才做。
      → ✅ **2026-10-22 仓库主人点头**，**后半正在验**。
      ⚠⚠ **但「重推同一个 tag」这条路走不通**（2026-10-22 实测，**别再试**）：
      `git push --force origin v0.1.2` 推的是**同一个 tag 对象**
      （本地与远端都是 `ac7b2cf` → commit `26bfef4`）⇒ GitHub 判
      **"Everything up-to-date"**，**根本不会产生 ref 更新事件** ⇒
      实测 8 秒后查 run 列表：**一个新 run 都没有**。
      ⇒ **改用 `gh run rerun 39737470359`**（重跑 v0.1.2 那个 run）：
      同一份 workflow、**同一 commit**（`26bfef4`，含幂等修复——
      `git merge-base --is-ancestor 01f7b24 v0.1.2` 退出 0）、
      同一个 tag ⇒ **唯一变量就是「release 是否已存在」**
      ⇒ 而它现在存在 ⇒ 应当走覆盖分支。
      ⇒ 这比 force-push **更干净**：不碰远端 ref，不产生「tag 被重推」的副作用。

## ⚠ 剩下的

⚠ **v0.1.0 的 release 要不要补发**，是产品决定：
补发就得给 v0.1.0 再推一次 tag（现在这跳幂等了，推上去会**新建**它），
不补发就只剩一个 tag 悬在那儿、Releases 页上看不到它。
⇒ **这属于「对外可见」的动作，要仓库主人拍板**，本票不自己动。

✅ **2026-10-22 仓库主人拍板：不补发。**
⇒ v0.1.0 的 tag 就让它悬着——它指向的提交**从未公开发布过**，
Releases 页上只有 v0.1.1 与 v0.1.2 两版，这是**已知且接受**的状态。
⇒ 这条不再是待办，**不要**再拿它去问「要不要补发」。
