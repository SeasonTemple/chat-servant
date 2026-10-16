# 46: `judge_surface` 把同一块矩形每帧每框扫 4~5 遍

**What to build:** 让「判一块区域是不是平表面」只扫一次像素。
现在它扫 4~5 遍，其中一次还是**逐像素**（STEP=1），而这条路径
**每个框每帧都走**。

**Status:** ready-for-agent · **2026-10-16 静态审查定位，未改未验——本沙箱装不上 Rust 工具链（`static.rust-lang.org` 不通、Debian bookworm 无 `rustc` 包且版本不支持 edition 2024），改不动也验不了**

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

- [ ] `judge_surface` 一次 `judge_surface` 调用里 `exact_hist` **只被调一次**
      （建议加一条守卫测试：在 `exact_hist` 里数调用次数，或直接断言
      `judge_surface` 的总耗时/采样点数不随重复调用线性增长）
- [ ] 判据结果**逐例不变** —— `SurfaceVerdict` 的 `is_flat` / `purity` / `colors`
      在改动前后对同一批输入完全相同（`mask.rs` 已有测试集，拿它当基线）
- [ ] `cargo clippy -p chatservant_perceive -- -D warnings` 通过
- [ ] `cargo fmt --all` 后 `cargo test -p chatservant_perceive` 全绿
- [ ] 报收益时给**实测数**：改前/改后每帧 `judge_surface` 的总采样点个数
      （不要只说「更快」）

⚠⚠ **本票三条修法我都没验过**（连 `cargo fmt` 都跑不了），
上面每一条都只是**读代码推导出来的**，不是实测。
⚠ 改之前先在能编译的机器上跑一遍 `cargo test -p chatservant_perceive`
拿基线，否则「判据结果不变」这条无从谈起。

## 为什么不是 agent 今晚改的

本沙箱**装不上 Rust 工具链**：
- `static.rust-lang.org` = 000（rustup 下不来）
- Debian bookworm 源里**没有 `rustc` 包**（只有 `cargo`），且版本不支持
  edition 2024（需 1.85+）
- `crates.io` = 403

⇒ 改一行 Rust 就交不出「验过的东西」，而本项目的纪律是
「**别只信门禁的退出码**」+「全绿 ≠ 验过」。
⚠ 宁可记成待办，也不落地一份**没编译过、没格式化过、没跑过测试**的 diff。
