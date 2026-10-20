# 75: Python 侧依赖浮着版本 ⇒ 每轮重下 27 MB，且发版结果不可复现

**What to build:** Python 侧的依赖（`rapidocr` / `omegaconf`）钉死版本，
装它的地方走同一份要求文件并开 pip 缓存；由机器守。

**Status:** done · **2026-10-20 落地并自证：四处注入全部报红；门禁全绿**

## 起因：问了一句「每次都要装？」

2026-10-20 真发 v0.1.1 时看到 verify job 第一段是
`Install OCR model-table dependencies`。查下来：

| 项 | 实测 |
|---|---|
| `rapidocr-3.10.0` wheel | **27.3 MB** |
| `omegaconf-2.4.0` | 254 kB |
| 连带装上 | 15 个：`numpy-2.5.4`、`opencv-python-headless-5.0.0.93`、`Pillow-13.0.0`、`Shapely`、`pyclipper`、`requests`… |
| 单步耗时 | **19s**（release 的 verify job）/ 16s（ci.yml 那次日志时间戳差） |
| 装了几处 | **三处**：`ci.yml` 的 Gate 7b-2、`release.yml` 的 verify job、`release.yml` 的 build job |
| pip 缓存 | **无** |

## ⚠⚠ 更要紧的不是耗时，是**权重来源浮着**

`scripts/fetch_models.py` 刻意**不硬编**权重 URL 与 SHA256，而是从
`rapidocr` 包自带的模型表里读（`rapidocr.inference_engine.base.MODEL_URL_PATH`）
—— 理由是「硬编 URL 的坏处是它会悄悄指到别的东西」。

⇒ **换 rapidocr 版本 = 换权重来源。**

而词典的 SHA256 是**本地钉死**的（脚本里的 `DICT_SHA256`，上游不给这个校验值）
⇒ 上游一换版就对不上，脚本报错退出。

这是设计意图（宁可发版失败，也不要装一个来路不明的权重），但**浮着装**意味着：
**同一个 tag 隔两周重跑可能失败** ⇒ 发版结果不可复现。

而 Rust 侧有 `Cargo.lock` 钉死，**Python 侧没有锁文件** —— 与 ADR-0003 的依赖
纪律同族，只是这一族只长到了 Python 这边。

## 刺眼的不对称：同一份 workflow 里别的都缓存了

| 依赖 | 缓存 |
|---|---|
| node | `actions/setup-node@v5` + `cache: pnpm` + `cache-dependency-path` ✅ |
| Rust | `dtolnay/rust-toolchain@stable`（自带 toolchain cache）✅ |
| 权重（30 MB） | `actions/cache@v4` path=`target/models` ✅（命中时「Fetch OCR weights」只花 2s） |
| **pip 依赖（27 MB）** | **无** ❌ |

## 修法

1. 新增仓库根的 `requirements-ocr.txt`：
   - 钉死 `rapidocr==3.10.0` / `omegaconf==2.4.0`
   - ⚠ **只钉这两个顶层包**：影响行为的只有它们（模型表的来源 + 它的 YAML 解析器）；
     传递依赖仍随解析浮动，那影响的是「装出来的东西」，不影响「权重从哪来」。
2. 三处 `pip install rapidocr omegaconf` → `pip install -r requirements-ocr.txt`
3. 三处 `actions/setup-python@v5` 加 `cache: 'pip'` + `cache-dependency-path: requirements-ocr.txt`

### ⚠ `cache-dependency-path` 是**必须**的，不是可选

`actions/setup-python` 的 `cache: 'pip'` 默认 glob 找的是 `**/requirements.txt`
⇒ 本仓**没有**那个名字的文件，而 `.agents/skills/ui-styling/scripts/requirements.txt`
是**第三方 skill 的内容** ⇒ 不显式指定的话，**缓存 key 由别人的文件决定**。

## 守卫 ⑨（`scripts/check-gate-parity.ps1`）

规则（缺一不可）：

1. 仓库里有一份 `requirements-ocr.txt`，顶层包**必须钉版本**（`==`）；
2. workflow 里的 `pip install` 必须指向那份文件（不许裸写包名）；
3. 装它的那**个 job** 必须开 `cache: 'pip'`，且**必须显式**写
   `cache-dependency-path`（理由同上）。

⚠ 判据从**不变量**推，不从「哪几个文件出过事」推（与 ④⑥⑦ 同一纪律）：
以后新加一个 workflow 装 Python 依赖，这三条照样适用。

### 顺带：把 job 切分抽成一份

新增 ⑨ 要判「某个 job 里有没有装依赖」，原先 ④⑥ 已经有同一套切法。
⇒ 抽成 `Get-JobSpans`，两处共用（⚠ 写第二份切法，两份迟早漂，
而漂了没人看得出来 —— 与「三处文档都说对应并不让那句话更真一点」是同一个陷阱）。

**回归判据**：守卫自己打的那几个数字**一个都不能变**（16 步 / 2 workflow 3 job /
2 个跑测试的 job / 5 条真帧测试 / 3 个本机 cargo 入口），变一个就是重构改坏了。

### 自证（注入四处失败 ⇒ 守卫必须报红）

| 注入 | 守卫报什么 |
|---|---|
| `requirements-ocr.txt` 里 `rapidocr` 去掉 `==` | ⑨ 里的 `rapidocr` 没有钉版本 |
| `ci.yml` 的 pip install 改回裸写包名 | ⑨ ci:192 的 pip install 没有指向 requirements-ocr.txt（**带行号**） |
| `release.yml` verify job 去掉 `cache: 'pip'` | ⑨ release 的 job verify：装了 Python 依赖却没开 pip 缓存 |
| `release.yml` build job 去掉 `cache-dependency-path` | ⑨ release 的 job build：开了 pip 缓存却没显式写 cache-dependency-path |

⇒ 四条**各报各的**，`exit=1`；恢复后 `exit=0`、数字不变。

## 验收项

- [x] 新增 `requirements-ocr.txt`，顶层包钉死版本
- [x] 三处 `pip install` 都指向它
- [x] 三处 `setup-python` 都开 `cache: 'pip'` 且**显式**写 `cache-dependency-path`
- [x] 守卫 ⑨ 逐 job 判（不是逐文件）
- [x] job 切分抽成 `Get-JobSpans`，与 ④⑥ 共用一份
- [x] 重构后守卫自报的那些数字**一个都没变**
- [x] 自证：四处注入全部报红（exit=1），恢复后转绿（exit=0）
- [x] 门禁十六步全绿
