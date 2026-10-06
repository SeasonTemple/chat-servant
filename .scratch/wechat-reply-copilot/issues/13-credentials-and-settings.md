# 13: 凭据与设置——key 不落文件、不进日志

**What to build:** 用户在设置页填模型 key，程序能从凭据处读到它去调模型，
但**任何文件里都不出现 key，任何日志里都不出现 key**。

设置改动即时生效不用重启。已配置的 key 输入框留空 = 保留，
用户不想每次改别的设置都重新粘一遍。

前端拿不到 key，也不需要知道模型路由——所有网络调用在 Rust 侧。

**Blocked by:** 12

**Status:** done

- [x] key 存进 Windows 凭据管理器（或环境变量），仓库和配置目录里 grep 不到
- [x] 所有错误文本经过脱敏，含 key 的上游报错不会原样显示或写入日志
- [x] 设置改动下一次生成立即生效，不用重启
- [x] 已配置时 key 输入框留空 = 保留原值，不清空
- [x] 前端拿不到 key：前端代码里没有任何一处能读到 key 本身
- [x] 前端不发起网络调用，所有出网都在 Rust 侧
- [x] 配置里切换模型供应商时，只需填一把新 key，不需重配整个应用
- [x] 缺少 key 时给出明确的下一步指引，不是一个空列表或静默失败

## 落在哪

`ui/src-tauri/src/secret.rs` —— 凭据存取 + 脱敏，纯 Rust，不经前端。

key 只去两处：**Windows 凭据管理器**（`chat-servant/<provider>`，
`CRED_PERSIST_LOCAL_MACHINE` 不漫游到域里其它机器）与
**环境变量** `<PROVIDER>_API_KEY`（给 CI / 脚本）。

## 三个关键决定

**已配置时设置页不显示 key 输入框。** 要改才点「换一把」。
这样明文 key 不会因为打开设置页就躺在界面上。提交后立刻清空输入框。

**`load()` 读到 key 就登记进内存表。** 这是 redact 能抹掉报错里 key 的前提——
读取的那一刻就是它进入本进程内存的那一刻，也是唯一需要登记的时刻。

**留空 = 保留原值，删除走单独的 `forget_key`。**
用户不想每次改别的设置都重新粘一遍 key。把「留空」做成删除，
用户会莫名其妙丢 key。

## 脱敏的三层

`redact()` 依次处理：① 已登记的 key 值本身 ② `sk-` 开头的 token 形状
（上游可能回显**没加载过**的 token）③ 短于 8 字节的串不登记，
否则会把正常文本也误伤。

有测试钉住「没可疑内容时不能动原文」——日志里全是乱码就没法看了。

## 泄露扫描的自检

`cargo xtask secret-scan` 扫仓库里有没有 key。key 从
`CHATSERVANT_PROBE_KEY` 环境变量读，**绝不能进命令行**——
命令行会留在 shell 历史和进程列表里，比日志泄露更隐蔽。

**这个扫描器自己也做了自检**（`scripts/secret-scan-selfcheck.ps1`）：
故意往仓库埋一个假 key，确认它抓得到，再撤掉确认它放得过。
理由：扫不到东西时，「没有输出」和「扫描器坏了」在屏幕上长得一模一样，
不证明它抓得到，它的「没抓到」就不可信。实测通过。

门禁在 `CHATSERVANT_PROBE_KEY` 有值时自动加一道 secret scan。

## Windows 凭据 API 的坑

- `CREDENTIALW` 要的是 `PWSTR`（可写），不是 `PCWSTR`。
- `CredReadW` 的错误码是 `HRESULT`，比 `ERROR_NOT_FOUND` 要 `.into()`。
- 释放内存的函数叫 **`CredFree`** 不是 `CredFreeMemory`，且**没有返回值**，
  所以不能 `let _ =`（clippy 会报 `let_unit_value`）。

## 未完成

`probe_key` 现在返回 `false` 而不是 `true`——网络层是 ticket 10 的事，
**不假装能连通**。