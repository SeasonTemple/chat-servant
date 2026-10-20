# 74: 本机另外两个 cargo 入口没有那份覆盖 ⇒ 清理脚本永远不删任何东西

**What to build:** 凡在本机跑 cargo 的入口，都必须带着与门禁**逐字相同**的
`TAURI_CONFIG` 覆盖；而且这件事由**机器**守住（改一处就要改三处）。

**Status:** done · **2026-10-20 实测：targetgc 拿到活产物清单并回收了 2.4 GiB
（8.8 GiB → 6.4 GiB）；守卫注入失败后确实报红**

## 起因：本轮「清理垃圾」清不动，而它**没报错**

2026-10-20 跑 `python scripts/targetgc.py --mode superseded --dry-run`：

```text
target = 8.8 GiB  (8.80 GiB)
>>> live_artifacts: `cargo test --no-run` 退出 101，不据此删除任何文件
>>> [superseded] 拿不到 cargo 的活产物清单 ⇒ **本轮不删任何文件**。
>>> target now 8.8 GiB
```

⇒ 表面看是「没什么可清」，实际是**清理工具在这台机器上完全动不了**：
它把「判据拿不到」翻译成「不删」，而**判据永远拿不到**。

## 根因：同一份覆盖只有门禁有

`ui/src-tauri/tauri.conf.json` 的 `bundle.resources` 指向

```json
"../../target/models/ppocrv6-small/*": "models/ppocrv6-small/"
```

而 `target/` 是 **gitignored** 的 ⇒ **干净检出上那个目录不存在**。
tauri-build 的 build script 在 `cargo check/clippy/test` 时就遍历 resources，
glob 匹配为空**无条件**抛 `GlobPathNotFound`（tauri-utils `resources.rs:289`）：

```text
error: failed to run custom build command for `chat-servant-app v0.1.1`
Caused by:
  glob pattern ../../target/models/ppocrv6-small/* path not found or didn't match any files.
```

⇒ **每个会编 `chat-servant-app` 的 cargo 步骤都挂**。

门禁早就知道这件事：`scripts/gate.ps1:75` 设了

```powershell
$env:TAURI_CONFIG = '{"bundle":{"resources":null}}'
```

（`null` 而不是 `{}`：`TAURI_CONFIG` 走 `json_patch::merge`，
**空对象是空操作**；必须给 `null` 才真的删掉那个键。）

⚠ **而本机还有另外两个入口会直接跑 cargo，它们都没有这份覆盖：**

| 入口 | 跑什么 | 没有覆盖的后果 |
|---|---|---|
| `scripts/targetgc.py` | `cargo test --workspace --all-targets --no-run` | 拿不到活产物清单 ⇒ **清理永远不删东西**，且不报错 |
| `scripts/dev.ps1` | `check` / `test` 两个动词 | `dev.ps1 check` / `test` 恒红，而红的原因与代码无关 |

## 修法

1. `scripts/targetgc.py`：列举活产物时给 cargo 带上**同一份**覆盖。
2. `scripts/dev.ps1`：`check` / `test` 两个动词带上同一份覆盖。
3. `scripts/check-gate-parity.ps1` 加守卫 ⑧：**逐字比对**三处
   （`gate.ps1` / `dev.ps1` / `targetgc.py`）必须一模一样。

⚠ **只给跑 cargo 的动词设，`'app'` 那条不给。**
真机要带着权重才跑，缺了应当**响**（编不过），
而不是悄悄回落到 Windows OCR（票 39 / 41 记的就是那个回落）。

## 守卫 ⑧

`scripts/check-gate-parity.ps1` 第 ⑧ 条：扫三个入口，剥掉**整行注释**后
取那串 JSON，去掉空白再与门禁那份逐字比。

⚠ **匹配前必须剥注释**（与该文件上方那条同理由）：
注释里写着那串 JSON，不等于代码里真的设了它。

### 自证（注入一次已知失败 ⇒ 守卫必须红）

把 `dev.ps1` 的覆盖改成 `{"bundle":{}}`（正是文档里点名的那个错法）：

```text
⑧ scripts/dev.ps1 的 TAURI_CONFIG 覆盖与门禁那份不一致
   它是：{"bundle":{}}
   门禁那份必须是：{"bundle":{"resources":null}}
   ⇒ 必须是 null：TAURI_CONFIG 走 json_patch::merge，空对象是空操作。
exit=1
```

⇒ 守卫不是「永远绿着的假判据」。改回后 exit=0。

## 验收项

- [x] `targetgc.py` 的活产物列举带上与门禁逐字相同的 `TAURI_CONFIG` 覆盖
- [x] `dev.ps1` 的 `check` / `test` 两个动词带上同一份覆盖
- [x] `'app'` 动词**不**带覆盖（真机缺权重仍然要响）
- [x] 守卫 ⑧ 逐字比对三个入口，剥注释后再匹配
- [x] 守卫量具自检：一个入口都没扫到 ⇒ 直接红
- [x] 自证：注入 `{"bundle":{}}` ⇒ 守卫红（exit=1）；改回 ⇒ 绿（exit=0）
- [x] 实测：改完后 `targetgc.py` 拿到活产物清单并真的回收了空间
