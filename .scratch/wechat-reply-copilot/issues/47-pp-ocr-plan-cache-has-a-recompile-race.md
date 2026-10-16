# 47: PP-OCR 计划缓存的**重复编译竞态** —— 并发下白等十几秒

**What to build:** 让同一个宽度的计划只编一次。
现在两个线程同时要同一个 `w`，**各自编一遍**。

**Status:** ready-for-agent · **2026-10-16 静态审查定位，未改未验——同票 46 的环境限制；且 `capture` 依赖 `windows`，只能在 Windows 上编**

## 位置

`crates/capture/src/ocr_pp_pipeline.rs:577-591`（`PpPlanCache::get`）

```rust
pub fn get(&self, w: usize) -> Result<Plan, PpError> {
    if let Some(p) = self.buckets.lock().unwrap().get(&w) {   // ① 拿锁、放锁
        return Ok(Arc::clone(p));
    }
    let plan = ocr_pp::load_rec(&self.plan_path(), w)?;          // ② 编译（锁外）
    let mut m = self.buckets.lock().unwrap();                   // ③ 再拿锁
    if m.len() >= self.max_buckets && !m.contains_key(&w) { /* 清一半 */ }
    m.insert(w, Arc::clone(&plan));                             // ④ 无条件覆盖
    Ok(plan)
}
```

## 竞态怎么发生

```text
线程 A: ① 查 → 没有 → 放锁
线程 B: ① 查 → 也没有（A 还没放进去）→ 放锁
线程 A: ② 编（几十~几百 ms）
线程 B: ② 编（同样长）   ← ⚠ 与 A 完全重复
线程 A: ③ 存入
线程 B: ③ 存入 → 覆盖 A 的（内容等价，浪费已发生）
```

⚠ **代码自己写着代价**：`ocr_pp_pipeline.rs` 上方那段注释记的是
「det 白跑一遍（**十几秒**）」。⇒ 并发命中时不是多几十毫秒，是**多十几秒**。

## 修法（三选一，从窄到宽）

1. **持锁编译**：把 ② 挪进锁内。⚠ 代价是**一个 width 在编译时阻塞其它
   width** —— 而 `det` 与 `rec` 的输入形状不同，本来也不会同时命中同一个桶。
   最小改动，但要先确认没有「A 编 1 号形状、B 急着要 2 号形状」这种并发模式。
2. **每键 `OnceCell`**：`HashMap<usize, Arc<OnceCell<Plan>>>`，
   第一次进来的人 `get_or_init` 编译，后来的 `get()` 等同一个 cell。
   ⇒ 不阻塞不同 width，也不会重复编译。**推荐这条。**
3. 用 `dashmap` 之类的分片 map。⚠ **新增依赖要先过 `deny.toml`**（AGENTS.md：
   新增工作区成员/依赖都要补，且要确认许可证是 permissive）——
   为这一处引入并发容器不值当，**优先 2**。

## 验收项

- [ ] 同一个 `w` 并发调 N 次 `get()`，**`load_rec` 只被调一次**
      （守卫：用一个计数用的桩或 `AtomicUsize` 计调用次数）
- [ ] 不同 `w` 并发时**互不阻塞**（若选了方案 1，验收项要写明
      「已接受这个阻塞」并给出实测）
- [ ] 缓存上限（`max_buckets` 那个「清一半」分支）行为不变 ——
      现有测试若有覆盖，补一条并发下的覆盖
- [ ] `cargo test -p chatservant_capture` 全绿 + `clippy -D warnings`
- [ ] 报收益给**实测**：并发场景下改前/改后的 `load_rec` 调用次数与总耗时

⚠⚠ **`capture` 依赖 `windows` / `windows-capture`**
（`crates/capture/Cargo.toml`）⇒ **只能在 Windows 上编**，
本沙箱连 `cargo check` 都过不了，更别说验。
⚠ 这也意味着：**本票的修法在 Linux 上无法做任何形式的验证**，
必须交到 Windows 机器上。

## 顺带：同一文件 4 处 `lock().unwrap()` 的中毒问题

`:578` / `:582` / `:593` / `:617` 全部是 `lock().unwrap()`。

⚠ `std::sync::Mutex` 在**持锁线程 panic 后会中毒**，此后每次 `lock()`
都返回 `Err`，而 `unwrap()` 直接 **panic** ⇒
**一次 panic 之后，这个缓存的每次访问都 panic**。
而这是 OCR 流水线的**共享缓存**（`buckets` / `DET_PLAN`），
⇒ 中毒一次，OCR 路径整个挂死。

`rust-skills` 的 `own-mutex-interior.md` 明写要这样处理：

```rust
// 或忽略中毒（谨慎使用）
let guard = mutex.lock().unwrap_or_else(|e| e.into_inner());
```

⚠ 本项目 `AGENTS.md` 把 `rust-skills` 列为**唯一可信的 6 个 skill 之一**，
这是它第 47 条（Mutex Poisoning）说的原话。
⇒ **要么按它改成 `into_inner()` 恢复，要么明写「这里中毒就崩」是有意为之**
—— 现在是**默认 `unwrap()`，等于没表态**。
⚠ 同族问题：`crates/reason/src/mock_upstream.rs:101/124` 也是
`.expect("锁")`（那两处在 mock 里，风险低，但同样没表态）。

## 为什么不是 agent 今晚改的

同票 46：沙箱装不上 Rust 工具链；且本 crate 依赖 Windows 专属 crate，
**连编译都不可能**。改完交不出验过的结果。
