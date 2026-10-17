# 53: `dev.ps1` 的 `Run` 把**输出和退出码一起返回**，四个动词永远 exit 1

**What to build:** 让本仓的**内循环入口**能报出真实的红绿。

**Blocked by:** 无。

**Status:** done · 2026-10-18。根因与实测证据见下；四个动词已改，绿红两档都自证过。

## 根因（一句话）

PowerShell 里**函数的全部输出都是返回值**。`Run` 里那几行给人看的
`Write-Output "========== …"` / `">>> … OK"` 和 `return $code`
走的是**同一条管道**，于是调用方 `$a = Run …` 把它们一起收走。
两件事同时坏：

1. **那些判定行从来没打印过。** 实测整段输出只有 **73 个字符**，
   只剩 cargo 自己那行 `Finished`。
2. **`$a` 变成字符串数组，`$a -ne 0` 是「过滤」比较**，
   返回所有不等于 0 的元素 ⇒ 恒为真 ⇒
   **`check` / `test` / `ui-check` / `ui` 永远 `exit 1`。**

## 实测证据

改之前跑 `dev.ps1 check`（当时代码确实是绿的）：

```
$ o = pwsh -NoProfile -File scripts\dev.ps1 check 2>&1 | Out-String
总长度=73
含 ========== -> False
含 >>> -> False
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.71s
EXIT=1
```

**clippy 打了 `Finished`、一个 warning 都没有，脚本仍然 exit 1。**

## 为什么一直没被发现

⚠ 因为**人读的是输出，机器读的是退出码，而两者现在说的是两件事**：

- 人看到的输出里只有 cargo 的 `Finished` ⇒ 读成「过了」。
- 机器看到的退出码是 1 ⇒ 读成「没过」。

本仓 `AGENTS.md` 里恰好有一条「别只信门禁的退出码，要看它打印的那两行」——
那条纪律本来是对的（针对 `gate.ps1` 早先 `exit` 漏写），
但它**恰好会把 `dev.ps1` 这种真坏掉的情况也解释成「大概没问题」**。
⇒ 纪律不能替代量具正确。

## 已修：照抄 `gate.ps1` 的纪律

`gate.ps1` 的 `Step` **根本不返回值**，直接读 `$LASTEXITCODE`，
所以它从来没有这个毛病。`dev.ps1` 改成同一套：

```powershell
function Run([string]$label, [scriptblock]$body) {
  …
  & $body
  $script:runCode = $LASTEXITCODE    # 不 return
}
```

四个调用点改成「先 `Run`，再读 `$script:runCode`」。

⚠ 顺带把 `'test'` 里的 `$args` 改名成 `$targs`——`$args` 是 PowerShell
**自动变量**，在脚本作用域里赋值虽然能跑，但它同时也是脚本的未绑定参数，
拿它当普通局部变量迟早出事。

## 自证（绿红两档都跑过，判据取退出码）

| 档 | 做法 | 期望 | 实测 |
|---|---|---|---|
| 绿 | 正常代码 | exit 0 **且**打印 `>>> … OK` | `EXIT=0`，两行都在 |
| 红 | 往 `crates/core/src/lib.rs` 塞一行 fmt 违规 | exit 1 **且**打印 `>>> … FAILED` | `RED_EXIT=1`，`fmt FAILED (exit=1)` + `clippy FAILED (exit=101)` |
| 还原 | `finally` 写回原内容 | exit 0，文件无 diff | `GREEN_EXIT=0`，`git diff --stat` 空 |

⚠ 绿档这一条**才是这次修好坏的证明**：只验红档的话，
「恒红」和「真判红」长得一模一样——它两档都红。只有绿档 exit 0 才算修好。

## 附带：`shotpanel.ps1` 的默认输出路径写死了

```powershell
[string]$Out = 'E:\workspace\chat-servant\target\probe\v11_full.png'
```

那是本仓**换过位置之前**的路径。`E:` 盘在，所以 `New-Item -Force`
会把那个目录**凭空建出来**，脚本照样打印 `saved`，而截图躺在仓库外面没人看得见。
已改成从 `$PSScriptRoot` 算：只跳**一层** `Split-Path -Parent`
（`Split-Path` 自己就去掉叶子，套两层会退到仓库的上一级去，
我第一版就是这么写到 `E:\devSpace\target\` 的），实测落在
`E:\devSpace\chat-servant-m31f\target\probe\panel.png`。

## 验收

- [x] `Run` 不再返回值，退出码走 `$script:runCode`
- [x] `check` / `test` / `ui-check` / `ui` 四个调用点全改
- [x] `$args` 改名 `$targs`
- [x] 绿档自证：exit 0 + 打印 `>>> … OK`
- [x] 红档自证：exit 1 + 打印 `>>> … FAILED`（注入 fmt 违规）
- [x] 还原后文件无 diff
- [x] `shotpanel.ps1` 默认路径实测落在仓库内
- [ ] **下一轮改动后留意**：`dev.ps1` 现在真的会红了，
      以前「一直红但没人看」的状态结束 ⇒ 内循环里若有脚本红，
      要当真去查而不是忽略
