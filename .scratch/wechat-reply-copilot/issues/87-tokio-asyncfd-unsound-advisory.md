# 87: tokio RUSTSEC-2026-0386（`AsyncFd` 不健全）——门禁自己抓到的

**Status:** done（2026-10-23 门禁当场抓出；升 tokio 1.53.2 → 1.54.1；advisories 复跑 ok）

## 一、怎么发现的

加「npm audit」门禁步那一轮，**全门禁第一次红了**，而红的不是新加的那步：

```text
[net] advisories FAILED, bans ok, licenses ok, sources ok
>>> cargo deny check FAILED [5.1s] (exit=1)
```

⇒ `cargo deny check advisories` 报出一条**当天新出的** RustSec 公告。

## 二、公告内容

**RUSTSEC-2026-0386**（<https://rustsec.org/advisories/RUSTSEC-2026-0386>）：

`tokio::io::unix::AsyncFd` 的安全构造器（`new` / `with_interest` / `try_new` /
`try_with_interest`）接受任何 `T: AsRawFd`，**不要求**调用方证明文件描述符
的所有权或有效性 ⇒ 安全代码可以从任意/已关闭的 fd 构造 `AsyncFd`，
或在其存活期间关掉/替换底层 fd。

两处 I/O 安全性问题：

1. `AsyncFd<T>` 对所有 `T: AsRawFd` 实现 `AsFd`（内部调
   `BorrowedFd::borrow_raw`）⇒ 安全代码能拿到一个并不拥有该 fd 的 `BorrowedFd`；
2. Tokio 的 I/O driver 在 `epoll`/`kqueue` 的 user data 里存着内部分配的指针；
   若底层 fd 被复制后关闭、且 fd 号被同一 runtime 上的另一个 IO 资源复用，
   第一个 `AsyncFd` 被 drop 时会把**复用的** fd 号注销 ⇒ use-after-free。

⇒ tokio 1.51.6 / 1.53.3 / 1.54.0 起，这些构造器被**弃用**，
改用 `unsafe` 的 `AsyncFd::register` / `register_with_interest`
（调用方自负 fd 在 `AsyncFd` 存活期间保持有效、打开、不变）。

## 三、受影响的版本与修法

- 本仓锁的是 **tokio 1.53.2** ⇒ 落在受影响区间。
- 公告给的解法：**升到 >=1.51.6, <1.52.0 或 >=1.53.3**。
- 实做：`cargo update -p tokio` ⇒ **1.53.2 → 1.54.1**（Rust 1.85 兼容的最新）。
- 复跑 `cargo deny check` ⇒ **`advisories ok, bans ok, licenses ok, sources ok`** ✓。

## 四、⚠ 这条公告对本仓的实际暴露面

**几乎为零，但门禁照样红**——这点要说清楚，免得下一个人以为我们差点出事：

- 公告的触发面是 **`tokio::io::unix::AsyncFd`**，即 **unix 专属**（`epoll`/`kqueue`）。
- 本仓是 **Windows 桌面应用** ⇒ 这段代码在目标平台上**根本不参与编译**。
- ⇒ 升它的理由是「**门禁要绿**」+「不把一个不健全的依赖留在锁文件里」，
  **不是**「我们被打了」。

## 五、教训

- ⚠ **「门禁红了」不一定是「我刚改坏了」**：这一轮的红来自
  **上游今天新发的公告**（与本轮改动毫无关系）。
  ⇒ 判据：先看红的是**哪一步**（`cargo deny` 而不是我动的那几处），
  再看公告的**受影响版本**与**暴露面**，最后才决定升还是豁免。
- ⇒ 顺带证明了 `cargo deny` 这道步**不是摆设**：它今天真的抓到了一条新的不健全依赖。
  （这也是为什么本轮同时把 **npm 侧**也补上了覆盖——票 81 的那半。）

## 六、验收项

- [x] 公告内容与受影响版本核清（tokio 1.53.2 ⇒ 受影响）
- [x] 按公告给的区间升到 1.54.1
- [x] `cargo deny check` 复跑 ⇒ advisories ok
- [x] 记下暴露面（unix 专属；本仓 Windows ⇒ 实际为零）
- [x] 全门禁绿

## 七、仍然没做的

- 锁文件里的**其它**依赖是否还有「公告已出但还没进 advisory-db」的：
  那要等 advisory-db 更新，仓库这边做不了。
- npm 侧那道步的「**没跑到**」分支本机没被触发过（端点可达）⇒
  要验它得把 registry 指到没有 audit 端点的源（见票 81）。
