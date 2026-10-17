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

## ⚠⚠ 缺陷③（dry run 第二次抓出）：Windows 上 Python 打印中文会直接崩

dry run `39037571268`：`verify` job ✅（缺陷②的修法在真 runner 上生效），
`pip install rapidocr omegaconf` ✅（替代 uv 的方案成立），
然后 `Fetch PP-OCR weights` ❌：

```
UnicodeEncodeError: 'charmap' codec can't encode characters in position 0-1
  File "...\Lib\encodings\cp1252.py", line 19, in encode
    print(f"下载 {name} …")
```

`actions/setup-python` 装的 Python 3.12 在 Windows 上 **stdout 是传统代码页 cp1252**，
而本仓的脚本输出全是中文。与 AGENTS.md 里那条 PS 5.1 编码陷阱同族，
但 **Python 这侧当时没有任何闸门**。

已修 `scripts/fetch_models.py`（模块顶层 reconfigure stdout+stderr，
`errors="replace"` 兜底：有字符编不出来时降级成 `?`，而不是让**打印失败**变成栈回溯）。
顺带补上同样缺的 `ui/scripts/targetaudit.py` 与 `scripts/ticketstatus.py`
——`scripts/targetgc.py` 早就有了，说明这个坑在别处已咬过一次，只有那一个脚本幸免。

### 守卫第三条不变量

> **`gate.ps1` 用 python 调起的脚本，若含非 ASCII，就必须自己 reconfigure。**

为什么值得机器查：这一类错**只在调用方显形**（CI 的 runner、本机 bench 台架），
本地「记得设 `$env:PYTHONIOENCODING='utf-8'`」就能糊过去——本会话我糊了好几次。
`errors` 也一并记：守卫查的是「有没有 `.reconfigure(`」这个形态，
不是「跑起来有没有崩」。

## ⚠⚠ 缺陷④（不用 runner 就能发现）：装包产物路径写错了，而且**同一份错路径重复了三遍**

`release.yml` 里三处（核对装包、publish 资产、dry run 收尾）都写：

```
ui/src-tauri/target/release/bundle
```

**三处都错。** `ui/src-tauri` 是**根工作区的成员**，所以 cargo 的 target 目录
在工作区根。本机实测（tier 3，不是推断）：

| 路径 | `Test-Path` |
|---|---|
| `target/release/bundle` | **True**（`msi/chat-servant_0.1.0_x64_en-US.msi` 65.4 MB + `nsis/…_x64-setup.exe` 58.6 MB） |
| `ui/src-tauri/target/release/bundle` | **False** |

⇒ 即使 `tauri build` 成功，「核对装包内有权重」会 throw「没找到打包产物目录」，
publish 的资产 glob 会拿到空再 throw。**这一步是必然失败的。**

### 顺带订正一个我自己搞错的结论

我一度报「本机这个 bundle 里没有权重」。**那是查错了地方**：
`bundle.resources` 的文件是**进安装包内部**（`<install>/models/...`），
不会以散文件形式躺在 `target/release/bundle/` 下。

正确查法是看 build script 拷到 target 目录的那一份（tauri-build `lib.rs:745-763`
就是把 resources 拷进 target dir）：

- `target/release/models/ppocrv6-small/`：det 9696.9 KiB / rec 20736.7 KiB / dict 73.2 KiB
- `target/debug/models/ppocrv6-small/`：同样三个

⇒ **`bundle.resources` 是生效的，装包里确实有权重。**

### 修法

抽成 job 级 `BUNDLE_DIR: target/release/bundle`，三处都改用它。
抽 env 的理由和抽不抽无关：**三份副本里的任何一份漂了，两份还对的读者不会察觉**
——这与「三处文档都说对应并不让那句话更真一点」是同一个陷阱。


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

## ⚠⚠ 缺陷①的**第二个现场**：在 `release.yml` 的 `verify` job 上原样复发

dry run `39036210343`（2026-10-18 13:21）第一次派出去，**5 分钟就红**，
报错与 `ci.yml` 当时逐字相同：

```
glob pattern ../../target/models/ppocrv6-small/* path not found or didn't match any files.
failed to run custom build command for `chat-servant-app`
```

原因：`verify` job 跑 cargo 时**权重还没取**（取权重在 `build` job 里，晚得多），
而 `release.yml` 里**压根没有 `TAURI_CONFIG` 覆盖**——我先前只修了 `ci.yml`。

⇒ **修了 A 没修隔壁文件，它就原样复发。** 这次 dry run 的价值就在这：
它在发布任何东西之前，把「本项目从来没跑通过的发版链路」的真实断点指出来了。

已修：`verify` job 加同样的 `TAURI_CONFIG` 覆盖（verify 不打包，处理方式与 CI 相同）。

## ⚠ `release.yml` 用的是 `uv`，而 **runner 上没有 uv**（2026-10-18 查实）

原先那一步是：

```yaml
run: uv run --with rapidocr --with omegaconf python scripts/fetch_models.py
```

查官方 runner 镜像清单（`actions/runner-images`，
`images/windows/Windows2025-Readme.md`，**image `20261013.288.2`**）：
Tools 与 Package Management 两段共列 60 多个工具
（Python 3.12.10 / pip 26.2.1 / Pipx / Chocolatey / Miniconda / jq / cmake …），
**没有 `uv`**。Linux 镜像有，Windows 没有 ⇒ build job 第一步就
`uv: command not found`。

⚠ **查之前先自检了量具**：那份 readme 只有 35.7 KB，小到可疑，
所以先确认它确实是那份清单（Python/Node/Rust/Cargo/CMake/Git/pip 都在，
「7-Zip 缺」只是它写作 `7zip`），**再**拿它的「没有 uv」当结论。
「清单里没有」只有在清单是真的时才成立。

⚠ `rapidocr 3.10.0` 的 `Requires-Python` 是 `>=3.8,<4`，
所以 `actions/setup-python` 钉 `'3.12'` 不会卡住 pip 解析（已查本机 dist-info）。

已改：`actions/setup-python@v5`（first-party）+ `python -m pip install`。
PyPI 来源没变、供应链面没变，去掉的只是「假定有个工具」。

## 守卫因此从「只看 ci.yml」扩到「所有 workflow × 逐 job」

同一条不变量现在扫 `.github/workflows/*.yml` 的**每一个 job**：

> 要么自己在跑 cargo 之前把权重取回来，要么声明 null 覆盖。

逐 job 而不是逐文件，因为同一个文件里可以同时存在「需要权重的 build job」
与「不需要的 verify job」，文件级判断会把它们混成一句。

⚠ **`pnpm tauri build` 本身就是 cargo 步骤**（CLI 去调 `cargo build`）。
只认字面 `cargo` 的话，那个**唯一真正需要权重**的 build job 会被整段跳过。
这条是被自证组 L 打出来的。

⚠ **「量具失效」必须与「量具没找到东西」分开**：现在若 workflow 里有 `jobs:`
却一个 job 都切不出来，直接报红。来源是一次自证事故：台架正则末尾写成
`.*\n` 配 `(?s)`，贪婪地把整个 workflow 吃掉只剩 26 行，
守卫对着残文件报「合规」，我差点反过来去改一个本来正确的守卫。

自证 **15/15**（`parity_selfproof.py`，真仓库文件未触碰，每组还打印该棵树的形状）。
其中三格是本轮新增、也是本轮教训：

| 组 | 期望 | 抓到了什么 |
|---|---|---|
| K 去掉 `release.yml` verify 的覆盖 | 红 | 就是 dry run 那个缺陷 |
| L 连 `run:` 一起删掉取权重整步 | 红 | **第一版只删了 `- name:`**，留下孤儿 `run:` 行，守卫判绿是**对的**——那一格是我用例写弱了，不是守卫漏检 |
| O 有 `jobs:` 却切不出 job | 红 | 量具失效不许当合规 |


原先那一步是：

```yaml
run: uv run --with rapidocr --with omegaconf python scripts/fetch_models.py
```

查官方 runner 镜像清单（`actions/runner-images`，
`images/windows/Windows2025-Readme.md`，**image `20261013.288.2`**）：
Tools 与 Package Management 两段共列 60 多个工具
（Python 3.12.10 / pip 26.2.1 / Pipx / Chocolatey / Miniconda / jq / cmake …），
**没有 `uv`**。Linux 镜像有，Windows 没有 ⇒ build job 第一步就
`uv: command not found`。

⚠ **查之前先自检了量具**：那份 readme 只有 35.7 KB，小到可疑，
所以先确认它确实是那份清单（Python/Node/Rust/Cargo/CMake/Git/pip 都在，
「7-Zip 缺」只是它写作 `7zip`），**再**拿它的「没有 uv」当结论。
「清单里没有」只有在清单是真的时才成立。

已改：`actions/setup-python@v5`（first-party）+ `python -m pip install`。
PyPI 来源没变、供应链面没变，去掉的只是「假定有个工具」。

## dry run：唯一能不出东西就跑完整条链路的入口

`workflow_dispatch` 多一个 `dry_run` 布尔输入。开着时
取权重 → `tauri build` → **核对装包内真有权重** 全部照跑，
只把 `gh release create` 那一发跳过，并显式打印
「没有创建 release、没有上传任何资产、没有打 tag」。

那个收尾步骤是刻意出声的：不写它，这次跑动的产物**确实存在**在 runner 上，
只是没离开 runner，而后来的读法会是「跑完了 ⇒ 发出去了」。

`tag` 输入的说明也改了：dry_run 时可填 `main` 这类任意 ref
（publish 已跳过，`checkout ref:` 只是个 ref）。


## ✅ 已实测：CI 转绿（缺陷①的正向判据）

本地门禁**不含** build script 那条路径，所以本地全绿证明不了任何事。
正向判据只能是 runner：

| | |
|---|---|
| run | `39035163536`（sha `8261f44`） |
| 时间 | 2026-10-18 13:05:53 → 13:14:51（8m58s） |
| 结论 | **success** |
| 12 步 | Gate 1~12 全 success |

关键那一步：`Gate 5 | cargo clippy -D warnings` = success
——它就是连红 28 次的那一步。

顺带证实了两件本来只是「配置层面看着对」的事：

- `Gate 2 | sheet pack in sync` success ⇒ `shell: pwsh` + `run: ./xxx.ps1`
  这条路在 `windows-latest` 上真的能跑（本票新增的那一步）。
- `Gate 3 | gate/CI parity` success ⇒ 新守卫在 runner 的文件副本上
  不会误报（剥注释那段在那里同样成立）。
- `Gate 7 | secret scan` success ⇒ `TAURI_CONFIG` 没有影响到
  `cargo run -p xtask` 那条路。


## 验收项

- [x] 缺陷①的根因定位到具体代码位置与 commit 边界（不是「CI 红了」）
- [x] 缺陷①已修（`TAURI_CONFIG` null override），并有守卫覆盖该形状
- [x] 守卫自证会响，判据用退出码；`{}` 空操作那一格单独验过
- [x] 缺陷②的根因定位（词典在别的 backend 分支，不是「上游没有」）
- [x] 缺陷②已修（`find_dict_url` + 钉死 SHA256），端到端取回并逐字节比对过
- [x] ModelSet 文件名对齐变成机器检查（不需要权重在场）
- [x] **推送后确认 CI 转绿**（run `39035163536`，sha `8261f44`，12/12 success）
- [x] **查实 `uv` 不在 runner 镜像清单里**（image `20261013.288.2`，60+ 工具中无它），
      并改用 `actions/setup-python` + `pip`
- [x] 加 `dry_run` 开关，让整条发版链路能在**不出东西**的前提下被验证
- [x] `release.yml` 的闸门自证（`check_release_yml.py` 8/8，两条 `if:` 严格互斥）
- [x] **缺陷①在 `release.yml` verify 上复发**：dry run `39036210343` 抓到并修掉
- [x] 守卫从「只看 ci.yml」扩到「所有 workflow × 逐 job」，自证 15/15
- [x] **缺陷③ Python 打印中文在 cp1252 上崩**：dry run `39037571268` 抓到并修掉，
      三个脚本补齐 reconfigure，并加第三条守卫（自证 17/17）
- [x] **缺陷④ 装包产物路径三处全错**：`ui/src-tauri/target/...` 不存在，
      真实是工作区根的 `target/release/bundle`（本机 `Test-Path` 实测）⇒ 抽成
      job 级 `BUNDLE_DIR`，路径只存一处
- [x] 顺带订正我自己搞错的一处结论：「装包里没有权重」是查错了地方——
      正确查法是 `target/{release,debug}/models/ppocrv6-small/`，三个文件都在，
      **`bundle.resources` 是生效的**
- [ ] **dry run 跑通到底**（缺陷④修完后重派；`tauri build` 那一段尚未在任何 runner 上跑过）
- [ ] **push 触发时 `inputs.dry_run` 为空 ⇒ 照常发布**（语义推理，验证它要打 tag）
- [ ] **跑通一次真实发版**（需授权打 tag）

