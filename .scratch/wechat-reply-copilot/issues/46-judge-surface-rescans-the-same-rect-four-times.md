# 46: `judge_surface` 把同一块矩形每帧每框扫 4~5 遍

**What to build:** 让「判一块区域是不是平表面」只扫一次像素。
现在它扫 4~5 遍，其中一次还是**逐像素**（STEP=1），而这条路径
**每个框每帧都走**。

**Status:** done · **2026-10-17 已改已验过门禁**（Rust 1.94.1，本机跑通）
· 2026-10-18 订正标签：原先写 `ready-for-human` 并留着一段
  「本票三条修法我都没验过」——**那是云端 Linux 沙箱的说法，已作废**，
  与它正上方 5/5 的验收项直接矛盾。

## ⚠ 票面前提被实测推翻，先读这一段

票面原写「扫 4~5 遍…**最坏 3 次重复**」。**实测不是这样**：

- `:100` 在 `colors > FLAT_MAX_COLORS` 的**早返回分支**里，与 `:107`/`:108`
  **互斥**——一趟走不到它们。一趟最多重复 **2 次**，不是 3 次。
- 守卫测试变红实测：平表面分支 `left: 2 / right: 1`，早返回分支 **通过**（本来就是 1）。

⚠ 更要紧的是**收益量级**：STEP=4 那一遍的遍数确实从 2 降到 1，但一次 STEP=4
只采 STEP=1 的 **1/16** 点数，所以总采样点只降 **5.6%**；而早返回分支 **0%**
（本来就只编一次，无可省）。剩下的 94% 是逐像素色种数那一遍，红线不许动。

| 用例 | 改前采样点 | 改后 | 降幅 |
|---|---|---|---|
| 气泡内文字行 120×16（平表面） | 2160 | 2040 | **5.6%** |
| 面板上文字行 100×12（平表面） | 1350 | 1275 | **5.6%** |
| 整块图片 240×120（早返回） | 30600 | 30600 | **0%** |

⇒ **这个改动是对的（去掉一次纯重复扫描、且判据逐例不变），但它不是「4 遍→1 遍」
那个量级。** 别拿它当性能优化的主要收益来汇报。

## 位置

`crates/perceive/src/mask.rs:90` `judge_surface`

| # | 行 | 干了什么 | 采样密度 |
|---|---|---|---|
| 1 | `:91` | `distinct_colors(frame, r)` | **逐像素**（`x += 1`，`mask.rs:129`）+ 建 `HashSet` |
| 2 | `:100` | `dominant_ratio(frame, r)` | STEP=4（`pixels.rs:85`）+ 建 `HashMap` |
| 3 | `:107` | `dominant_ratio(frame, r) >= FLAT_MIN_PURITY` | 同上，**又一次** |
| 4 | `:108` | `dominant_ratio(frame, r)` | 同上，**又一次** |

⚠ **`:107` 与 `:108` 把同一个 `purity` 算了两次**，纯粹是为了填两个字段
（一个进 `is_flat` 的判定，一个进 `purity` 字段本身）。
⇒ 块全平的情况下走 1/2/3 三条 = **3 次**；色种数超限时走 1/2 = **2 次**。
⇒ **最坏 3 次重复**。

## 为什么它是热的

`judge_surface` 是**每个文本框的必经之路**（`mask.rs` 负责给每个框分类
`ImageText` / `Fragment` / 消息行），而 `perceive` 跑在**观察循环**里，
每帧对当帧所有框各走一次。
⚠ 硬约束第 6 条「静默期零调用」管的是**模型调用**，像素扫描不受它约束，
所以这条路径是**无条件每帧执行**的。

## 修法（⚠ 有一个需要人判断的点）

最小改法：把 `dominant_ratio` 拆一个吃 `&HashMap<Rgb, usize>` 的版本，
`judge_surface` 里**建一次直方图**，三处共用。

```rust
// 现在的形状（每次都重扫）
pub fn dominant_ratio(frame: &Frame, r: Rect) -> f32 {
    let hist = exact_hist(frame, r);
    ratio_of(&hist)
}

/// 建议拆成两个
fn ratio_of(hist: &HashMap<Rgb, usize>) -> f32 { /* 现有后两行 */ }
pub fn dominant_ratio(frame: &Frame, r: Rect) -> f32 {
    ratio_of(&exact_hist(frame, r))
}
```

⇒ `judge_surface` 里 `let hist = exact_hist(frame, r);` 建一次，
三次判定全用 `ratio_of(&hist)`。
⚠ `dominant_color`（`pixels.rs:109`）也是 `exact_hist` 的一次性用法，
**可以不动** —— 它在 `attribute.rs:239` 单独调，与本票无关。

### ⚠⚠ 需要人拍：两支函数的**采样密度不同，不能直接合并**

| | 步长 | 用途 |
|---|---|---|
| `distinct_colors` | **STEP = 1**（`mask.rs:129`） | 精确色种数。注释明写「不做量化合并」 |
| `exact_hist` | **STEP = 4**（`pixels.rs:85`） | 众数占比 |

⇒ `hist.len()` 与 `set.len()` **语义不同**：前者只数采到的样点，
后者数全区域。**色种数是 `judge_surface` 的主判据**
（`mask.rs:95-97`：「图片区实测众数占比高达 0.51，单看占比**一定**会漏判，
所以色种数是主判据」）—— **改采样密度会直接改判据本身**。

⇒ **不要**图省事把 `distinct_colors` 也换成 STEP=4。
只能合并**同样 STEP=4 的那三次**。

## 验收项

- [x] `judge_surface` 一次调用里 `exact_hist` **只被调一次**
      ——守卫测试 `one_judge_surface_call_builds_exactly_one_histogram`，
      **用线程局部计数器**（`thread_local!` + `Cell`，在 `pixels.rs`）而不是全局静态：
      `cargo test` 多线程跑同一进程，全局计数会把别的测试算进来，守卫就会时红时绿。
      **变红已验**：把 `judge_surface` 逐字改回优化前的样子，跑出
      `实测建了 2 张   left: 2  right: 1`，exit=101
- [x] 判据结果**逐例不变** ——守卫测试
      `sharing_one_histogram_gives_the_same_verdict_as_recscanning`：**保持原样的**
      `distinct_colors`(STEP=1) + `dominant_ratio`(STEP=4，自建图) 在测试里把
      优化前公式原样复刻成 `before()`，对 9 组用例逐字段 `assert_eq!` 整个
      `SurfaceVerdict`（`purity` 是 f32 精确比较）。覆盖平表面分支、早返回分支、
      退化框（`x1<x0`）、整块框在帧外（`None`）
- [x] `cargo clippy -p chatservant-perceive --all-targets -- -D warnings` 通过
      （`cargo clean -p chatservant-perceive` 强制重 lint 后 exit=0）
- [x] `cargo fmt --all` 后 `cargo test -p chatservant-perceive` 全绿
      ——lib **86 → 88**（+2 新增）、集成 3，failed 0
- [x] 报收益时给**实测数** ——见开头那张表（5.6% / 5.6% / 0%）

⚠⚠ ~~**本票三条修法我都没验过**（连 `cargo fmt` 都跑不了），
上面每一条都只是**读代码推导出来的**，不是实测。
⚠ 改之前先在能编译的机器上跑一遍 `cargo test -p chatservant-perceive`
拿基线，否则「判据结果不变」这条无从谈起。~~

> ⚠ **2026-10-18 订正：上面这段作废。** 它是 2026-10-16 云端 Linux 沙箱
> 的处境记述，而**那台机器编不了不是本票的阻碍**——2026-10-17 在本机
> （Rust 1.94.1）照票面跑，第一条命令就通。
> 现在的事实以本票正上方的验收项为准：5/5 全勾，含「变红已验」
> （把 `judge_surface` 改回优化前的样子 ⇒ `left: 2 right: 1`、exit=101）
> 与 `cargo test -p chatservant-perceive` 全绿（lib 86 → 88）。
> ⇒ **别再照着这段去重跑基线**，它描述的世界已经不存在。

## 当时的记录为什么是「待办」

2026-10-16 云端会话**装不上 Rust 工具链**：
- `static.rust-lang.org` = 000（rustup 下不来）
- Debian bookworm 源里**没有 `rustc` 包**（只有 `cargo`），且版本不支持
  edition 2024（需 1.85+）
- `crates.io` = 403

⇒ 那时改一行 Rust 就交不出「验过的东西」，而本项目的纪律是
「**别只信门禁的退出码**」+「全绿 ≠ 验过」，
所以宁可记成待办，也不落地一份没编译过的 diff。

⚠⚠ **这个「装不上」只对那台云端 Linux 成立。** 2026-10-17 在季寺的 Windows
机器上（Rust 1.94.1）照着票面跑，第一条命令就通了。
⇒ **「我这里编不了」是环境事实，不是这张票的阻碍**，别让它挂成待办。
