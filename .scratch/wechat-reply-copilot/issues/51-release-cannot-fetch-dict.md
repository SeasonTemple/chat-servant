# 51: 权重不在仓库里，而 build script 与取权重的脚本都假设它在——CI 连红 28 次、发版取不到词典

**What to build:** 让「干净机器上能从零构建」这件事成立。
现在它**不成立，而且是静默的**：本机有权重，于是本地门禁全绿；CI 与发版机器上没有，于是它们红。

**Blocked by:** 发版那一跳需要你授权打 tag（⚠ **打 tag 会真的发一个 release**）。

**Status:** ready-for-human

## 根因（一句话）

`ab8831d` 把权重接进了装包（`tauri.conf.json` 的 `bundle.resources`），
而权重在 `target/models/` 下——`.gitignore:1` 的 `/target` 让它**永远不在仓库里**。
于是「构建需要某个不在仓库里的文件」这件事，被本机的存在掩盖了。

## 缺陷 ①：CI 连红 28 次（本地看不见）

`tauri-build` 的 build script 在 `cargo check/clippy/test` 时就遍历 `bundle.resources`，
glob 匹配为空**无条件**抛错（tauri-utils 2.10.1 `resources.rs:289` `GlobPathNotFound`）。
`target/models/` 在干净 runner 上不存在 ⇒ **每个 cargo 步骤全挂**。

边界是实测的，不是推的：

| commit | 时间 | CI |
|---|---|---|
| `acf3d0f` | 2026-10-17 07:57Z | **GREEN** |
| `ab8831d` | 2026-10-17 08:07Z | failure |

`ab8831d` 之后 28 次推送全红，**中间夹着纯文档提交**——
文档改不动 Rust，所以只可能是 build script。这条排除法比读日志更硬。

### 已修：`ci.yml` 用 `TAURI_CONFIG` 把它 null 掉

```yaml
TAURI_CONFIG: '{"bundle":{"resources":null}}'
```

链路（tauri-build 2.7.1 `lib.rs:684`）：`json_patch::merge`（RFC 7396）删掉该键
⇒ `config.bundle.resources` 是 `None`
⇒ `lib.rs:745-749` 的 `unwrap_or(BundleResources::List(Vec::new()))`
⇒ 不迭代 glob ⇒ 不报错。

⚠ **必须是 `null`，不能是 `{}`**：merge patch 里空对象是**空操作**。
这一格是守卫自证出来的（见下），不是想当然。

**为什么 CI 可以不打包**：CI 只跑 clippy/test/lint，从不跑 `tauri build`，
那个 resources 映射在这里本来就不会被用到。**发版那条路要权重**，走 `fetch_models.py`。

## 缺陷 ②：`release.yml` 取不到词典（从未成功过）

`release.yml` 在 `tauri build` 之前跑 `scripts/fetch_models.py`，
而那个脚本**只校验词典、不取词典**，缺了就 `exit 1` 并叫人「从本机 wheel 复制」。

`release.yml` 历史上只跑过**一次**，结论是 `cancelled`——**从未成功过**。
所以「`release.yml` 里写了取权重」只是配置层面的断言，不是它能跑通的证据。

### 调研结论：词典**取得到**，之前是找错分支了

2026-10-18 实测（`target/probe/dict_probe.py`，自带下载计数）：

| 问题 | 答案 |
|---|---|
| `dict_url` 在模型表里吗？ | 在，共 78 条 |
| 在**我们用的** `onnxruntime` 那一支吗？ | **不在**。该支每个节点只有 `model_dir` + `SHA256` |
| 那在哪？ | 挂在 `mnn` / `paddle` / `torch` 三支的同一个键下，三支指向**同一个 URL** |
| 取回来的就是本机在用的那份吗？ | **是**。`rec_small` 与 `rec_medium` 与本机那份**逐字节相同**（18708 行，sha256 `b5f2bfe2…`）；`rec_tiny` 是 6904 行，不同文件 |

⚠ 旧脚本用 `find_entry` 找词典，而它要的是「带 `model_dir` 的那个节点」——
恰好是我们**没有** `dict_url` 的那一支。于是报出来的话非常像「上游没有词典」，
而真相是词典在别的 backend 那一支。**这是个「搜不到 ⇒ 所以没有」的经典误判。**

### 已修

- 新增 `find_dict_url()`：要的是「带 `dict_url` 的那个节点」，不设 `model_dir` 的前提。
- 词典现在会被下载并校验。
- ⚠ **上游不给词典的 SHA256**（同节点里那个 `SHA256` 管的是 .mnn），所以
  `DICT_SHA256` **本地钉死**（2026-10-18 实测值）。
  与本文件「URL 不硬编」的立场不冲突：URL 从上游表来（跟得上换版），
  哈希本地钉（挡得住**静默**换内容）。上游合法更新时会响并要求人复核——那正是想要的。

## 守卫（两处，都自证过）

### `scripts/check-gate-parity.ps1` 的第二条不变量

`bundle.resources` 里只要有一条指向仓库之外，`ci.yml` 就**必须**把它 null 掉，否则红。
守的是「本机能编 ≠ 干净机器能编」这个形状。

### `fetch_models.py --check` 的 ModelSet 对齐

两边各写各的文件名（Python 字面量 vs Rust const `V6_SMALL`），漏一边 ⇒
产品去读一个不存在的文件，而**编译与测试都照绿**。
`--check` 现在**不需要权重在场也不需要网络**就能查这条，`release.yml` 已经调用它。

## 自证记录

判据一律是**退出码**（`parity_selfproof.py` 10/10、`fetch_selfproof.py` 10/10，
真仓库文件未被触碰）：

| 组 | 期望 | 实测 |
|---|---|---|
| `TAURI_CONFIG` 写成 `{}`（merge patch 空操作） | 红 | 红 |
| 整行删掉 `TAURI_CONFIG` | 红 | 红 |
| resources 改成指向仓库内 | 绿 | 绿 |
| ModelSet.rec 改名 | 报出来 | 报出来 |
| 空目录下跑下载分支 | exit 0 | exit 0，三个文件全部取回并校验通过 |
| 词典被替换后 `--check` | 非 0 | exit 1 |

## ⚠ 仍未证实的（不要当成已验证）

1. **CI 是否真的绿了**：本地门禁不含 build script 这条路径，
   正向判据是「推送后看 runner 的结果」，不是本机读数。
2. **`uv` 在 `windows-latest` 上是否可用**：`release.yml` 用
   `uv run --with rapidocr --with omegaconf`，但**没有任何一次成功运行**，
   所以这是配置层面的断言（tier 1），不是实测（tier 3）。
   本机走的是 `target/bench-venv`，**完全没有验证过 runner 上这条**。
3. **从未有过一次成功的发版**：`Verify weights landed in the bundle` 等步骤
   在真实 runner 上一次都没跑过，它们的正确性是纸面推理。
4. **tag 我不会自己打**：打 tag 会真的发一个 release，那是要授权的动作。

## 验收项

- [x] 缺陷①的根因定位到具体代码位置与 commit 边界（不是「CI 红了」）
- [x] 缺陷①已修（`TAURI_CONFIG` null override），并有守卫覆盖该形状
- [x] 守卫自证会响，判据用退出码；`{}` 空操作那一格单独验过
- [x] 缺陷②的根因定位（词典在别的 backend 分支，不是「上游没有」）
- [x] 缺陷②已修（`find_dict_url` + 钉死 SHA256），端到端取回并逐字节比对过
- [x] ModelSet 文件名对齐变成机器检查（不需要权重在场）
- [ ] **推送后确认 CI 转绿**（需看 runner 实测结果）
- [ ] **`uv` 在 `windows-latest` 可用**（tier 1 断言，需实测）
- [ ] **跑通一次真实发版**（需授权打 tag）
