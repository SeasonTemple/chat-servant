# 69: CI 从三个提交前就红了 —— 守卫要求了一个**永远满足不了**的前置

**What to build:** 让 `main` 的 CI 恢复绿，且**不把真人聊天内容放进仓库**；
同时把「哪些不变量在 CI 上没验」**写成字**而不是让人从绿灯里猜。

**Status:** done · **2026-10-19 修完并验证：CI run `39263318318` 结论 success。**
根因不是代码，是 `d67918f` 留下的一个**永远满足不了的前置**（见下）。

## 现象：连续三个 run 全红

```text
failure  fix(tools): 订正 ort 联接的假陈述、回收 2.3 GiB、修 targetgc 的假报告  39249556236
failure  perf(gate): 门禁分四轨并行，229.3s → 81.4s，并修掉五个静默失效        39246826894
failure  fix(perceive): 黄金基线守卫在 CI 上从未真跑过，却一直报通过            39236999113
```

红的都是同一条：

```text
thread 'tests::a_real_bubble_never_has_a_busy_ring' panicked at crates\perceive\src\tests.rs:737:
两份夹具都不在本机 ⇒ `a_real_bubble_never_has_a_busy_ring` **一条都没验**。
test result: FAILED. 92 passed; 1 failed; 0 ignored; ...
```

## 根因：`d67918f` 那一轮留了一个不可能的前置

`d67918f`（2026-10-19 18:21）把「缺夹具即跳过」改成**硬失败**，理由是对的：

- `libtest` **丢弃通过测试的 stdout/stderr**，`ci.yml` 又不带 `--nocapture`
  ⇒ 那条「跳过」提示在 CI 日志里**一个字都看不到**，只有 `test result: ok`
  ⇒ 干净 runner 上黄金基线**从未真的验过，却报通过**。

但它同时写下了一句：

> ⇒ **后果**：Gate 8 的 `cargo test --workspace` 现在**要求夹具在场**。
>   CI 若要缓存/准备夹具，在 Gate 8 之前加一步。

⚠⚠ **而那一步永远备不出来。** 夹具是 `crates/capture/fixtures/` 下的
**真实会话脱敏帧**，硬约束第 3 条写着「测试夹具里不写真实聊天内容」
⇒ 它们**不能**进公开仓库 ⇒ CI 上**永远**是零夹具。

⇒ 「硬失败」+「夹具永远不在」= **CI 永远红**。
⇒ 而且 `d67918f` 漏掉了同族的另外三处（见下），
所以「一个改法覆盖全族」这件事当时**就没成立**。

## 同族四处的策略当时是乱的

| 位置 | 当时的行为 | 在 CI 上 |
|---|---|---|
| `perceive/src/tests.rs::a_real_bubble_never_has_a_busy_ring` | 硬失败 | **红** |
| `perceive/tests/golden.rs`（3 条） | 硬失败 | **红**（被 fail-fast 挡住，没跑到） |
| `perceive/src/media.rs::a_real_text_only_frame_finds_nothing` | `eprintln!` + `return` | **绿着，但没验** ⚠ |
| `ocr_accuracy.rs`（合成语料） | `CI` ⇒ panic | 绿（**CI 能现造语料**，这个 panic 是对的） |

⚠ 第三行就是 `d67918f` 要根除的那个坑（「`ok` 与跳过逐字同形」）——
**它至今还活着**，那一轮漏了。

## 修法：真帧测试 = **本机专属**（`#[ignore]`），不变量由合成替身在 CI 上守

三件事同时成立才算修完：

1. **依赖真帧的 `#[test]` 一律标 `#[ignore]`。**
   `cargo test --workspace`（CI 与本机门禁跑的是同一条命令）不跑它们，
   但它们出现在**汇总行**里：

   ```text
   test result: ok. 91 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
   test result: ok.  0 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out
   ```

   ⇒ **「没跑」是机器可读的**，与「ok」不同形。
   ⚠ 这正是 `d67918f` 想要的东西，只是用 `#[ignore]` 拿到，
   而不是用「硬失败」拿到（后者把 CI 永久钉红）。

2. **缺件路径的 `panic!` 保留着。**
   `cargo test --workspace -- --include-ignored` 才会真跑它们，
   而那时缺夹具仍**硬失败**（附恢复命令）⇒
   「跑起来却没验」仍然是红的理由，守卫没有被削弱成空气。

3. **那条不变量在 CI 上有替身。**
   `perceive/src/tests.rs::real_text_lines_stay_below_both_icon_thresholds`
   用**合成帧**守同一条（气泡里的字不会被头像闸误伤），
   合成帧在 Rust 里画出来 ⇒ **CI 上真跑**。
   ⇒ 我们没有因为「真帧跑不了」就把守卫删掉，只是把**保真度**那部分挪到本机。

⇒ 顺带把「这些测试在 CI 上没验」**写成字**：`ci.yml` 的 Note 步现在逐条说清，
而不是让人从一片绿灯里猜（`d67918f` 那一版的 Note 写的是「Gate 8 要求夹具在场」，
那是个**假陈述**——CI 永远备不出来）。

## 机器守卫：`check-gate-parity.ps1` 的 ⑦

范围**按函数体推，不靠名单**（名单会漂——上一轮就漏了一个）：

1. 凡是**函数体**（或它调用的、同文件的辅助函数）碰到 `capture/fixtures` 的
   `#[test]`，**必须**标 `#[ignore]`。
2. 合成替身 `real_text_lines_stay_below_both_icon_thresholds` 必须在、且**不能**被 ignore
   （否则等于把守卫整个删掉）。
3. `ci.yml` 必须含 `--include-ignored`（本机怎么真跑它们）。
4. **量具自检**：一条都没扫到 ⇒ 直接红（「没查到」不等于「没问题」）。

⚠ 这条守卫抓的正是**复发**：同族四处当时三种策略，其中一处至今是静默跳过。

## 验收项

- [x] 根因定位到具体那一步（`d67918f` 的 ci.yml Note + 硬失败路径）
- [x] 同族四处策略统一（`tests.rs` / `golden.rs` ×3 / `media.rs`）
- [x] 汇总行**看得见** `ignored` 计数（不是被 libtest 吞掉的 stderr）
- [x] 缺件路径仍是**硬失败**（`--include-ignored` 缺夹具 ⇒ exit=101）
- [x] 合成替身在、且未被 ignore
- [x] 机器守卫 ⑦ 接进 `check-gate-parity.ps1`（已在 `gate.ps1` 与 `ci.yml` 里）
- [x] `ci.yml` 的 Note 改成如实陈述（旧的「要求夹具在场」是假陈述）
- [x] **自证 S1**：去掉一个 `#[ignore]` ⇒ 守卫红（exit=1）
- [x] **自证 S2**：藏起夹具 + `--include-ignored` ⇒ 硬失败（exit=101）
- [x] 本机门禁全绿
- [x] **CI 上真跑一次绿** —— run `39263318318` 结论 **success**
      （Gate 8 过了 ⇒ 那一步确实是红因；整轮 16 步全绿）

## 附：一次偶发红，未复现（诚实记档）

本轮第一次跑门禁是 **FAILED**，第二、三次都是 `GATE: ALL GREEN`，
**工作树在三次之间没有任何变化** ⇒ 那是一次**偶发**。

⚠⚠ **失败步的输出没留下来**，所以**至今不知道是哪一步红的**——
原因在**我这一侧**：我用 `pwsh -File gate.ps1 | Select-Object -Last 60` 收输出，
把失败步那段截掉了。

⇒ 教训（已落成做法）：**跑门禁用 `scripts/run-gate.ps1`**，它会把完整输出
Tee 到 `target/gate.log`；**别自己套管道截断**——
门禁红的那一刻正是最需要看全文的时候，而截断把它拿走了。
⇒ 这也正是本仓那条老教训的又一次：*「一次没跑的步骤，和一次跑了但报 0 的步骤，
在观测上一样」* 的近亲——**一次红了的门禁，和一次没被记下来的红，在证据上一样**。

⚠ **可疑但未证实的方向**：第一次跑是**冷缓存**那轮（clippy 69.4s、test 37.8s，
而热缓存时 2.8s / 30.5s），且**紧接在删掉 5.11 GiB `target/` 之后**；
同轮里两个 git 类脚本（version consistency 12.6s / gate-CI parity 10.8s）
比热缓存时慢了 5 倍 ⇒ 与「文件系统还在刷那一大坨删除」相符。
⇒ **不写成结论**（没复现、没证据），只记下「冷缓存 + 大批量删除之后」这个**线索**。

## 附：为什么不是「把夹具放进 CI 缓存」

看着像那么回事，但**不成立**：

- 缓存里也得**有源**。源在开发者本机 ⇒ 需要一条「把本机文件传到 CI」的路
  ⇒ 那条路本身就是把真实会话内容搬出本机。
- 硬约束第 3 条管的是「**测试夹具**里不写真实聊天内容」，而 CI 缓存里的
  msi/夹具**同样会被人下载**（runner 的 cache 在公开仓库上就是公开的）。
- ⇒ 唯一合法的路是**合成语料**，而合成语料只能守**机制**，
  守不了**真像素**（这也是票 30/39 一路在处理的事）。

⇒ 所以「本机专属 + 合成替身 + 如实写明」不是妥协，是硬约束下唯一诚实的形状。

## ⚠ 别拿「权重」那套来类比（两者的差别是硬的）

| | OCR **权重**（31 MB ONNX） | **真帧夹具**（`crates/capture/fixtures/`） |
|---|---|---|
| 来源 | 公开的模型权重，可下载 | **用户自己的真实聊天** |
| 能进公开仓库吗 | 能（`scripts/fetch_models.py` 拉） | **不能**（硬约束第 3 条） |
| CI 能备出来吗 | **能** ⇒ 所以 CI **必须**备，且缺件要 panic | **不能** ⇒ 只能本机专属 |
| 现在的机制 | `fetch_models.py` + parity 守卫 ⑥ 要求「跑测试前取权重、造合成语料」 | `#[ignore]` + parity 守卫 ⑦ 要求「必须标本机专属、合成替身必须在」 |

⚠ 两边的**判据方向是相反的**：
权重那侧「缺件 = 守卫没跑 = 红」，真帧这侧「缺件 = 守卫跑不了 = 本机专属」。
**共同原则是「判不出来时不做不可逆的事」**，但代码上两处写法相反 ——
因为「能不能拿到材料」这一条就不同。
⇒ **别看到「ocr_accuracy 缺件 panic」就以为真帧那侧也该 panic**：
那是把 CI 永久钉红的做法（实测三个 run 全红）。

## 自证怎么复现（两条，都不需要额外工具）

**S1 · 守卫会不会响**

```powershell
# 去掉 perceive/src/tests.rs 里那行 #[ignore = "local-only: ..."]
pwsh -NoProfile -File scripts\check-gate-parity.ps1   # ⇒ exit=1，报「依赖 gitignored 真帧，却没标 #[ignore]」
# 改回去后同一命令 ⇒ exit=0
```

⚠ 恢复文件时**重写内容**而不是复制备份（`File.Copy` 会把 mtime 设成源文件的，
cargo 按 mtime 判新鲜度 ⇒ 后面几轮验的是陈旧产物）。
本轮的做法：`[IO.File]::WriteAllText($p, $orig, $enc)`。

**S2 · 守卫还是不是真的**

```powershell
Move-Item crates\capture\fixtures crates\capture\fixtures.__stashed__
cargo test -p chatservant-perceive -- --include-ignored   # ⇒ exit=101，缺夹具硬失败
Move-Item crates\capture\fixtures.__stashed__ crates\capture\fixtures
```

⇒ 两条合起来说明：**跑起来就必须验到**，而**没跑时机器看得见**。
