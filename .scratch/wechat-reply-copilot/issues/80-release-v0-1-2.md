# 80: 发 v0.1.2

**What to build:** 把 v0.1.1 之后累计的修复发成 v0.1.2——版本号四处对齐、
双语发布说明、tag 推送、Release 产出 msi + nsis。

**Status:** done · **2026-10-22 真发 v0.1.2：run `39737470359` success，msi + nsis 两个资产落地**

## 为什么要发这一版

v0.1.1 是 2026-10-20 发的。之后主线累计 20 个提交，其中**两处用户能看见**：

| 改动 | 票据 | 用户看到的现象 |
|---|---|---|
| 名字带子 2× → **3×** | 票 77 | 面板顶部**读不出会话名** → 现在读得出 |
| 生产 OCR 视图改**双线性** | 票 41 ③ | WinRT 回退路径整行小字读成乱码 → 没了 |

⇒ 已发出去的 v0.1.1 安装包**不含**这两处 ⇒ 不发这一版，用户拿到的就是旧的。

## 这次要动的四样

- [x] **版本四处**一起改到 `0.1.2`：
      `Cargo.toml`（`[workspace.package]`）/ `ui/src-tauri/Cargo.toml`（`[package]`）/
      `ui/package.json` / `ui/src-tauri/tauri.conf.json`
      ⇒ 门禁那一步 `version consistency`（`scripts/check-version.ps1`）会逐个点名
- [x] **`Cargo.lock`** 里 5 个工作区成员的版本同步到 `0.1.2`
- [x] **双语发布说明** `docs/release-notes/v0.1.2.md`（`## 中文` + `## English`）
      ⇒ 缺任一段，**发版那一步会 exit 1**（不是建议，是门禁）
- [x] 完整门禁绿（`version consistency` 步会检查说明存在且双语）

## 发出去之后要验的

- [x] tag `v0.1.2` 推送 ⇒ Release workflow 绿
      → run `39737470359` conclusion=success
- [x] **Releases 页上出现 v0.1.2**，资产是**两个**：msi + nsis
      → `chat-servant_0.1.2_x64-setup.exe`（38.4 MB）+
      `chat-servant_0.1.2_x64_en-US.msi`（45.2 MB）
- [x] 资产名里的版本是 **0.1.2**
      → ⚠ 这条不是形式主义：tag 与 `tauri.conf.json` 对不上时，
      发出去的是「标题 v0.1.2、附件叫 0.1.1」的错标 release，
      而**没有任何一步会报错**（release.yml 的「Report publish gate」就是拦这个的）
      → 实测该步 **completed success**（tag v0.1.2 ↔ version 0.1.2）

### 发版链路这一步真跑通了（值得记一笔）

- `verify` job（mojibake → 取权重 → 造语料 → fmt+clippy+test）**绿**
- `Build installers` job：取权重 → 打包 → **核对装包内有权重** → 发布
  - `Report publish gate` success
  - `Verify weights before bundling` success
  - `Build msi + nsis` success
  - `Verify weights landed in the bundle` success
  - `Publish | GitHub Release` success
    ⇒ **这一步正是票 79 改幂等的那一步**（之前 tag 重推会在这里撞名失败）
- ⇒ 发布页：https://github.com/SeasonTemple/chat-servant/releases/tag/v0.1.2

## ⚠ 连带决定：v0.1.0 **不补发**

`gh release list` 只有 v0.1.1 ⇒ **v0.1.0 有 tag、没 release**
（它的发布尝试撞名失败，见票 79）。

**决定不补**：v0.1.0 是历史版本、没人用它的包，补上去反而多一个
「看起来可下载、实际没人维护」的版本。
⚠ 若后来要补：现在发布链路幂等了，重推 v0.1.0 的 tag 即可。

## ⚠ 这次特意没动的东西

- **发送档位**：仍是两档，`Auto` 依然不二次确认 ⇒ **本版不含安全模型变更**，
  发布说明里显式写明了这一句。
- **`v0.1.0` 的 tag 与那个空 release**：留着，不清理。
