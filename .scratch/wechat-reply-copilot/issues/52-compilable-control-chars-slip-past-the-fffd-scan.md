# 52: 源码里混进了**能编译的控制符**，而门禁那道扫描只查 U+FFFD

**What to build:** 让「源码里的字符损坏」这件事有机器闸，而不是靠人眼扫注释。

**Blocked by:** 无。

**Status:** done · 2026-10-18。抓到 5 处、修掉 5 处，`scan-mojibake.ps1` 加了三条判据，自证四档全过。

⚠ 这行 Status **刻意不写 `N/M`**：`ticketstatus.py` 的漂移检测用
`(\d+)\s*/\s*(\d+)` 在整行里搜，而我原来写的「自证 4/4 会响」是散文，
结果被判成「Status 自己写的进度数」，凭空报了一条漂移。
⇒ 那个正则扫的是**整行**，所以 Status 行里任何分数形状都会被当成进度声明。

## 根因（一句话）

编码往返 / 转义误处理把**控制符写进了源码**，而它们**编译得过、测试跑得过**——
受损的是注释和 SAFETY 说明，`rustc` / `clippy` / `svelte-check` 一个都不管。

## 实测在 `crates/capture/src/inject.rs` 抓到五处

| # | 位置 | 损坏形态 | 读出来看着像 |
|---|---|---|---|
| 1 | SAFETY 注释 | `bytes` 的 `b` → **U+0008**（退格符） | `ytes` |
| 2 | SAFETY 注释 | `focus` / `fill_input_box` 的 `f` → **U+000C**（换页符） | `ocus` |
| 3 | SAFETY 注释 | `type_unicode` 的 `t` → **TAB** | `` ype_unicode`` |
| 4 | SAFETY 注释 | 行尾留了字面反引号 + `r` | ``只会造 `r`` |
| 5 | 测试辅助 `seq()` 的 SAFETY 注释 | 同 #4 | ``只会造 `r`` |

⚠ 第 1 条**只有 dump 字符码才看得见**：U+0008 是不可见控制符，
`read` / `grep` / 大多数编辑器都不显示它，于是「bytes」直接显示成「ytes」——
而 `ytes` 看起来像个正常的英文词根，肉眼扫注释**扫不出来**。

**只有第 5 处是新加的守卫自己抓到的**，前四处是我用一条临时脚本
（`git ls-files` 全量扫控制符）抓的。⇒ 那条临时脚本的逻辑必须进正式门禁，
否则下一次同样的损坏还是靠运气。

## 门禁缺口（为什么 5 处能躺着）

`scripts/scan-mojibake.ps1` 只查 `U+FFFD`。U+0008 / U+000C / TAB
**都不是 U+FFFD**，所以那道闸对上面五处**全部漏判**。

## 已修：三条新判据

```powershell
$ctrlRe   = [regex]"[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]"   # 任何源码里都没有合法用途
$midTabRe = [regex]"[\p{L}\p{N}]\t[\p{L}\p{N}]"          # 词中间的 TAB = 字母被吃了
$litEscRe = [regex]'`[a-zA-Z][ \t]*$'                     # 行尾字面 `r / `n
```

TAB 与 LF/CR 从 `ctrlRe` 里**排除**——TAB 本身合法，判据要落在
「字母 夹 TAB 夹 字母」这个形状上，否则会变成缩进风格之争。

### ⚠ 第三条判据第一版写错了，被自证打回来

v1 判据是「反引号紧跟在 `*` 后面」。第一跑就在三张票里各误报一次：

```
30-real-frame-ocr-accuracy.md:117   … **`y 51..63`（高 12）** …
34-tray-...-dark-taskbar.md:107     … **`L ∈ [0.1411, 0.2654]`** …
40-key-input-unreachable...md:40    … **`p.configured === false`** …
```

markdown 的**加粗 + 行内代码**长这样。⇒ 收窄成「行尾是 反引号 + 一个字母」，
**并且只对源码扫，`.md` 一律跳过**。收窄后在 186 个源码文件里命中 1 条，
而那 1 条正是第五处真损坏——**零误报，且仍然有效**。

⚠ 「不再误报」和「仍然有效」是两件事，只有注入损伤能分开 ⇒ 见「自证」。

## 自证：`target/probe/scan_ctrl_selfproof.ps1` 4/4

对三个形状各注入一次损伤 + 一次还原后基线，**判据取退出码**：

```
  PASS  exit=1  U+0008 控制符（bytes 的 b 变成退格符）
  PASS  exit=1  词中间的 TAB（type_unicode 的 t 变成制表符）
  PASS  exit=1  行尾字面转义（反引号 + r）
  PASS  exit=0  还原后恢复干净
SELFPROOF: ALL GREEN (4/4)
```

台架用 `scripts/dev.ps1` 当受试文件（本轮它没有未提交改动），
`finally` 里从内存原样写回。⚠ 这台架自己有个坑：早期版本用
`Select-Object -First N` 截断管道，把 pwsh 子进程杀掉 → **拿到假 exit=0**。
去掉截断后真实退出码才露出来。**量具自己坏掉时，报出来的红绿都是反的。**

## 验收

- [x] 五处损坏全部定位并修掉（字符码 dump 确认，不是靠「看着像」）
- [x] `scan-mojibake.ps1` 加三条判据
- [x] 第三条判据收窄到零误报（markdown 明确排除）
- [x] 自证 4/4 会响（注入 + 还原基线）
- [x] 基线跑全仓 255 个被扫文件：干净
- [x] **没有新增门禁步**——挂在已有的 `mojibake scan` 那一步上，
      所以 `ci.yml` 与 `check-gate-parity.ps1` 都不受影响
