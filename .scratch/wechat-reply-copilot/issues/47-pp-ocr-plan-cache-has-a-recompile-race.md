# 47: PP-OCR 计划缓存的**重复编译竞态** —— 并发下白等十几秒

**What to build:** 让同一个宽度的计划只编一次。
现在两个线程同时要同一个 `w`，**各自编一遍**。

**Status:** done · **2026-10-17 已改已验过门禁**（Rust 1.94.1，本机跑通）
· 2026-10-18 订正标签：原先写 `ready-for-human`，但 5/5 全勾、含变红已验，
  且末节「遗留」那条（`det_plan` 持锁编译）已由**票 49** 接走并做完前半条

## ⚠ 先读这一段：这个竞态**今天在树内不可达**

2026-10-17 改之前逐个调用点核过：`BucketCache::new` 全仓只有两处
——`ocr_pp_pipeline.rs:751`（`recognize_in` 里，**每次识别新建一个**）
与一个测试。而 `recognize_with` 里的 `for (w, group) in by_w` 是**串行**的，
`by_w` 已按宽度分组 ⇒ 一轮里每个宽度本来就只取一次图。

⇒ **今天不会「两个线程同时要同一个 `w`」。** 树内没有跨线程共享同一个
`&BucketCache` 的调用者（`recognize_with` 是 `pub`，接口上开放，但无人这么用）。

⚠ **但修法仍然做**，两条理由：① 便宜；② 它把不变量**钉住**了——
把缓存提成 `static`（正如 `DET_PLAN` 已经是那样）是**一次重构之差**，
那时这个 bug 立刻变活。有守卫测试守着，它就不会悄悄回来。

⇒ **不要把它记成「现场已发生十几秒白等」。** 那是票面按「代码形状」推的，
不是实测到的现象。

⚠ 同一来源的另一个观察，**本票没动**：既然缓存每帧新建、每帧每宽度只用一次，
**这个桶缓存在当前调用形状下几乎不产生复用收益**；要真拿到收益得让缓存跨帧存活。
属另一张票的范围，按最小改动原则没碰。

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

- [x] 同一个 `w` 并发调 N 次 `get()`，**编译只被调一次**
      ——守卫测试 `one_width_is_compiled_exactly_once_under_concurrency`，
      带 `Barrier(N)`（**没有栅栏这些线程多半一个接一个跑完，这条会永远绿**），
      且断言 8 个线程拿到**同一个 `Arc`**。
      **变红已验**：把 `get_or_compile` 整段换回票面记的老形状，跑出
      `8 个线程并发取同一个宽度：编译器被调 8 次 … left: 8  right: 1`；
      32 线程那次同样红（`left: 12`）
- [x] 不同 `w` 并发时**互不阻塞** ——守卫测试 `two_widths_compile_at_the_same_time`，
      判据取**同时在编的峰值 ≥ 2**（结构事实）而**不取耗时**（会被机器快慢带偏）
- [x] 缓存上限（`max_buckets` 的「清一半」分支）行为不变 ——
      顺序用例 `the_bucket_cap_drops_half_and_keeps_the_key_just_asked_for`
      （4 桶满后再要新宽度 ⇒ `len == 3`，且本键没被自己刚做的清理带走）+
      并发用例 `eviction_under_concurrency_keeps_the_cache_consistent`
      （16 线程 × 40 次 × 12 种宽度 > 上限 4：不炸、不串、`len ≤ 上限`）
- [x] `cargo test -p chatservant-capture` 全绿 + `clippy -D warnings`
      ——**73 → 79**（+6 条新测试，**未掉任何一条**）；九步门禁 `GATE: ALL GREEN`
- [x] 报收益给**实测** ——见下

### 收益实测（机器 12 逻辑核；桩里把「编译」做成忙等 200ms，因为真 `load_rec`
是吃 CPU 的，`sleep` 桩测不出争用、会低估代价）

| 场景 | 改前（老双重检查） | 改后（每键 OnceLock） |
|---|---|---|
| 8 线程同宽 | **8 次编译** / 墙钟 201.9ms | **1 次** / 墙钟 201.0ms |
| 32 线程同宽（> 核数） | **12 次编译**（= 核数，其余线程撞上了已完成的缓存）/ 216.6ms | **1 次** / 202.1ms |
| 编译总工作量 | 8×200 = **1600ms**；32 线程档 12×200 = **2400ms** | **200ms**（降 87.5% / 91.7%） |

⚠ **墙钟几乎没动，必须说清楚而不是包装成「更快」**：这台机器 12 核，
老形状下多个编译**并行**跑完，所以省下的是 **CPU 工作量**与并发编译带来的
内存/带宽争用，**不是墙钟延迟**。墙钟要出明显收益，得是核数少于并发线程数、
或机器已被采集线程占住的情形——那种情形本机这次**没测到**，没测到的不算。

## 实际采用的形状（票面没写到的两个障碍）

用**方案 2（每键 `OnceLock`）**，标准库，无新增依赖。锁只护「取/建 cell」一下，
**编译在锁外**，同宽后来的人 `get_or_init` 等第一个人编完。

⚠ **障碍一：`get_or_init` 的闭包不能返回 `Result`**（`FnOnce() -> T`），
而 `load_rec` 是 `Result<Plan, PpError>`。若 cell 存 `OnceLock<Plan>`，
失败时闭包**没法把失败告诉等待者** ⇒ 等待者会永远挂在 `get_or_init` 上，
**比重复编译更糟的死锁**。所以引入 `enum Slot<P> { Ready(Arc<P>), Failed }`。

⚠ **障碍二：失败会不会被永久缓存** ——`get_or_init` **会**缓存闭包的值。
留在 cell 里就是**永久毒化**（「模型文件后来修好了也永远起不来」）。处理：
① cell 里只记 `Failed` 这个事实、**不存错误本身**（`PpError` 不可克隆，
且错误文本要显示在面板上，给后来者一个占位等于对它撒谎）；
② 看到 `Failed` 的线程把该键从表里摘掉，**且只摘同一个 cell**（`Arc::ptr_eq` 判定——
中途可能已有别人为该宽度建了新 cell，摘掉新的会连带作废人家正在等的编译结果）；
③ 再自己编一趟只为拿一个**真的** `Err`，顺带覆盖「权重刚被修好」。
守卫测试 `a_failed_compile_is_not_cached_forever` 钉住。

⚠ **实测到的 std 语义（决定了要不要 `catch_unwind`）**：Rust 1.94.1 上
`get_or_init` 的闭包 panic 时，panic 线程展开、**并发等待线程不会挂死**
（实测它改为执行自己的闭包）⇒ **不需要** `catch_unwind`，包装只会把 panic 藏起来。
若 std 哪天改成「等待者永久挂起」，这里才会变成真问题，注释里留了这条判断的前提。

⚠ **清桶与 `get_or_init` 的竞态**：`Arc` 是这里安全性的全部来源——摘掉只是从表里
移除引用，cell 活到最后一个使用者手里 ⇒ 不悬垂、不 panic。已知且**接受**的代价：
清桶正好插在编译中间时，那个宽度可能被并行的另一个线程**再编一遍**，
比悬垂小得多。`len()` 的语义也变了：现在数的是 **cell**（含「正在编」那个），
编译窗口内比早先大 1，已在文档里写明。

⚠⚠ **`capture` 依赖 `windows` / `windows-capture`**
（`crates/capture/Cargo.toml`）⇒ 确实**只能在 Windows 上编**。这条成立，
但 2026-10-17 已在本机验完（Rust 1.94.1）。

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

### 实际处理：统一 `lock_recover(m)`，但**逐处算过**而不是照抄

新增 `lock_recover<T>` = `m.lock().unwrap_or_else(|e| e.into_inner())`，四处共用。

| 处 | 临界区里有什么 | 判断 |
|---|---|---|
| `buckets` 的「取/建 cell」 | 只有 `HashMap` 的 `get`/`entry`/`retain`/`remove` | **恢复是对的**：这些要么整体生效、要么分配时直接 abort，**不存在改了一半的中间态** |
| `buckets` 的 `len` | 同上，只读 | 同上 |
| `get_or_compile` 的「摘掉失败键」 | `get` + `remove` + `ptr_eq` | 同上 |
| `det_plan` 的 `DET_PLAN` | **有第三方代码**（tract 的 `load_det`），它 panic 是有可能的 | **恢复依然是对的**，但理由不同：`*slot = Some(..)` 排在 `load_det` **之后**，炸的时候 slot 里仍是上一份**形状已校验过**的旧值 |

⚠ **没有写成无条件 `into_inner()` 就完事**：`lock_recover` 的文档注释明确写了
「哪天这里不再成立」（往临界区里加了会 panic 的用户代码、或改成「先改一半再编译」，
恢复就变危险，那时该改成重建缓存）。默认 `unwrap()` 等于没表态——这一点确实修掉了。

⚠ `crates/reason/src/mock_upstream.rs:101/124` **本轮未动**（不在范围）。
判断：那两处在 mock 里，锁的内容是请求槽位，**同样不存在半改状态**（取出后立刻释放/
整体替换），所以 `into_inner()` 也安全；但更值得做的是**换成命名锁 helper**——
`.expect("锁")` 和 `.unwrap()` 是同一个毛病：既没表态，也让 panic 消息永远只有
「锁」两个字，排查时看不出是哪个锁。优先级低（mock 路径，且 mock 上 panic 本来就该炸）。

### 守卫测试（6 条，**全部不依赖权重**）

⚠ 关键约束：**这台机器上当时没装权重**（票面记的是 `ch_PP-OCRv4_rec_mobile.onnx` 不存在），
而真跑模型的测试在无权重时是**直接 return 跳过**的 ⇒ 用真图写并发守卫 =
**在 CI 上永远绿 = 没守**。所以把「怎么保证只编一次」抽成**与图无关的泛型函数**
`get_or_compile<P, E, F>`，测试注入计数桩（`Slot<u32>`），守的**就是
`BucketCache::get` 走的那一段同步逻辑**，不是另写一份假的。

⚠ **2026-10-18 订正**：权重**现在装了**，而且换成了 v6 small
（`<仓库>/target/models/ppocrv6-small/`，票 39）。**但这不改变本票的结论**——
CI 上仍然没有权重（`target/` 是 gitignored 的 31 MB），所以「这条守卫必须
与权重无关」这个判断**至今成立**，只是它成立的理由从「本机没装」变成了
「**别人与 CI 都没装**」。别因为本机现在能跑真推理，就把守卫改回依赖权重。

除上面两条，还有 `a_poisoned_lock_does_not_make_every_later_call_panic`
（故意持锁 panic 之后仍能取图，且**没被当成命中缓存**）与
`a_failed_compile_is_not_cached_forever`。

⚠ 如实留白：`two_widths_compile_at_the_same_time` 换成票面**方案 1（持锁编译）**
时峰值恒等于 1 → 会红；`a_poisoned_lock_…` 把 `lock_recover` 改回 `unwrap()` → 会红。
**这两种反向形状只做了推理，没有实跑**，实跑过的只有「退回老双重检查」那一种。

## 当时为什么记成待办

同票 46：云端沙箱装不上 Rust 工具链；且本 crate 依赖 Windows 专属 crate，
**连编译都不可能**，改完交不出验过的结果。
⚠ 而 2026-10-17 在季寺的 Windows 机器上照着票面跑，第一条命令就通了。

## 遗留（不在本票范围，建议单开）

⚠ **`det_plan` 仍在持锁期间编译 det 图**（十几秒级）。它不会重复编译，
但会**堵住所有其它 det 调用者**——这正是票 46 方案 1 明确不接受的形状。
按同样的 `OnceLock` 思路可治。
