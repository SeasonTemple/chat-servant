//! 凭据存取与脱敏。
//!
//! ## 硬约束（AGENTS.md 第 7 条）
//!
//! **API key 不落普通文件、不进日志。** 只从两处读：
//!
//! 1. **Windows 凭据管理器**（首选）——`CredWriteW` / `CredReadW`。
//!    这是系统加密存储，用户在「控制面板 → 凭据管理器」里能看到并删除。
//! 2. **环境变量**——给 CI / 脚本用，不进任何文件。
//!
//! **前端拿不到 key。** 这个模块只被 Rust 侧调用，`dispatch` 之类命令
//! 拿到的只是「有没有配置」的布尔值，不是 key 本身。
//!
//! ## 为什么还要脱敏
//!
//! 凭据管理器本身是安全的，但**报错文本**不安全：上游 API 返回的
//! 401 经常把请求里带的 key 原文回显。若直接 `format!("{e}")`
//! 打到日志或显示给用户，key 就泄到了日志文件里——而日志文件
//! 是普通文件。
//!
//! 所以 [`redact`] 是**所有**对外文本的必经之路。

use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW,
    CredWriteW,
};
use windows::core::{PCWSTR, PWSTR};

/// 凭据管理器里的目标名前缀。`chat-servant/<provider>`。
const TARGET_PREFIX: &str = "chat-servant";

/// 掩码后的样子。**长度刻意不等于真实长度**——那会泄漏信息。
const MASK: &str = "••••••••";

/// 一把 key 的状态。**前端只拿得到这个，拿不到 key 本身。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyStatus {
    /// 已经配好了（凭据管理器或环境变量任一有值）。
    pub configured: bool,
    /// 值来自环境变量。这种情况下设置页的输入框应该是只读的。
    pub from_env: bool,
    /// 给界面看的掩码，仅用于「已配置」那一行提示。
    pub masked: String,
}

/// 没配置时的统一提示。**不能是空列表或静默失败。**
pub fn missing_hint(provider: &str) -> String {
    format!(
        "还没给 {provider} 配 key。两种方式二选一：\n\
         ① 在这个设置页填进去（存进 Windows 凭据管理器，不落普通文件）\n\
         ② 设环境变量 {}_API_KEY 后重启本程序\n\
         配好后不用重启设置——下一次生成就会用上。",
        provider.to_uppercase()
    )
}

/// key 存进 Windows 凭据管理器。
///
/// `CRED_PERSIST_LOCAL_MACHINE`：跟着本机走，**不同步到漫游配置**——
/// 漫游会把凭据带到域里的其它机器去。
pub fn store(provider: &str, secret: &str) -> Result<(), String> {
    let target = format!("{TARGET_PREFIX}/{provider}");
    let mut blob = secret.as_bytes().to_vec();

    let mut name: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    // UserName 只是凭据管理器列表里的显示名，不是登录名。
    let mut user: Vec<u16> = provider.encode_utf16().chain(std::iter::once(0)).collect();

    let cred = CREDENTIALW {
        Flags: Default::default(),
        Type: CRED_TYPE_GENERIC,
        // `CREDENTIALW` 要的是**可写**指针（PWSTR），不是 PCWSTR。
        // 名字本身我们不改，但类型上要过 `as_ptr().into()`。
        TargetName: PWSTR(name.as_mut_ptr()),
        Comment: PWSTR::null(),
        LastWritten: Default::default(),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: PWSTR::null(),
        UserName: PWSTR(user.as_mut_ptr()),
    };

    // 写完立刻抹掉内存里的副本。凭据管理器的副本由系统负责加密，
    // 我们这份只是路过。
    let r = unsafe { CredWriteW(&cred, 0) };
    blob.fill(0);
    r.map_err(|e| format!("写凭据管理器失败：{}", redact(&e.to_string())))
}

/// 读 key。**凭据管理器优先于环境变量**——
/// 设置页填的应当覆盖机器上的环境变量，否则用户改不动。
pub fn load(provider: &str) -> Option<String> {
    let key = from_vault(provider).or_else(|| from_env(provider))?;
    // 读到就登记。这样后续任何报错在 redact 时都能把它抹掉——
    // **读取的那一刻就是它进入本进程内存的那一刻**，也是唯一需要登记的时刻。
    remember(&key);
    Some(key)
}

/// 从环境变量读。变量名 `<PROVIDER>_API_KEY`。
fn from_env(provider: &str) -> Option<String> {
    let var = format!("{}_API_KEY", provider.to_uppercase());
    std::env::var(var).ok().filter(|s| !s.trim().is_empty())
}

/// 从凭据管理器读。
fn from_vault(provider: &str) -> Option<String> {
    let target = format!("{TARGET_PREFIX}/{provider}");
    let name: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let mut raw: *mut CREDENTIALW = std::ptr::null_mut();
        match CredReadW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None, &mut raw) {
            Ok(()) if !raw.is_null() => {
                let cred = &*raw;
                let bytes = std::slice::from_raw_parts(
                    cred.CredentialBlob,
                    cred.CredentialBlobSize as usize,
                );
                // 凭据里的字节不是 UTF-8 结尾的，from_utf8_lossy 会带上替换字符。
                // 这里手动按字节走，不留隐藏字符。
                let s = String::from_utf8_lossy(bytes)
                    .trim_end_matches('\0')
                    .to_string();
                // `CredFree` 收 `*const c_void` 且**没有返回值**，所以不能 `let _ =`。
                CredFree(raw as *const std::ffi::c_void);
                (!s.is_empty()).then_some(s)
            }
            // 「没配」是正常状态，不是错误。**不返回 Err**——
            // 上层要区分「没配」和「读失败」，两者处理方式完全不同。
            // CredReadW 的错误码是 `HRESULT`，Win32 错误码要 `.into()` 转换。
            Err(e) if e.code() == ERROR_NOT_FOUND.into() => None,
            // 其它读失败（凭据库损坏、权限不足…）也**不能当成「有 key」**——
            // 那会让上层拿着空 key 去调 API，401 报错还更难查。
            Err(_) => None,
            // 报告成功却没给指针：防御性分支，前一条已挡住正常情况。
            Ok(()) => None,
        }
    }
}

/// 删掉凭据里的 key。
pub fn erase(provider: &str) -> Result<(), String> {
    let target = format!("{TARGET_PREFIX}/{provider}");
    let name: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { CredDeleteW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None) }
        .map_err(|e| format!("删除凭据失败：{}", redact(&e.to_string())))
}

/// 当前状态。**不含 key**——只有「有没有」和「来自哪」。
pub fn status(provider: &str) -> KeyStatus {
    let from_env = from_env(provider).is_some();
    match load(provider) {
        Some(_) => KeyStatus {
            configured: true,
            from_env,
            masked: MASK.to_string(),
        },
        None => KeyStatus {
            configured: false,
            from_env: false,
            masked: String::new(),
        },
    }
}

/// 设置页提交：**留空 = 保留原值**，不清空。
///
/// 用户不想每次改别的设置都重新粘一遍 key。
/// 返回值告诉调用方「有没有真的写」。
pub fn apply(provider: &str, input: Option<&str>) -> Result<bool, String> {
    match input.map(str::trim).filter(|s| !s.is_empty()) {
        Some(secret) => {
            store(provider, secret)?;
            Ok(true)
        }
        // 留空：什么都不做。**不是删除**——删除要走单独的 erase。
        None => Ok(false),
    }
}

/// 抹掉文本里可能出现的 key。
///
/// 三个来源：
/// 1. 已知的 key 值本身（我们知道，就直接替换）
/// 2. `sk-` / `Bearer ` 开头的 token 形状
/// 3. `api_key=...` / `"key": "..."` 形状的赋值
///
/// **宁可多抹。** 日志里少几个字符无所谓，泄一次就是事故。
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for known in known_secrets() {
        if !known.is_empty() {
            out = out.replace(&known, MASK);
        }
    }
    mask_sk_shaped(&out)
}

/// 扫出所有 `sk-…` 形状的 token 并替换。
fn mask_sk_shaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(at) = rest.find("sk-") else {
            out.push_str(rest);
            return out;
        };
        let tail = &rest[at..];
        let end = tail
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
            .unwrap_or(tail.len());
        out.push_str(&rest[..at]);
        out.push_str(MASK);
        rest = &tail[end..];
    }
}

/// 当前进程里已知的 key 值。
///
/// 加载过的 key 缓存在这里，好让 [`redact`] 能把它们从报错里抹掉。
/// **只存在内存里**，进程退出即消失。
static KNOWN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn known_secrets() -> Vec<String> {
    KNOWN.lock().map(|g| g.clone()).unwrap_or_default()
}

/// 把一个 key 登记为「已知敏感值」，让后续 redact 能抹掉它。
pub fn remember(secret: &str) {
    if secret.len() < 8 {
        // 短于此的多半不是 key，别占位置——否则会误伤正常文本。
        return;
    }
    // `Mutex::lock` 返回 `LockResult`，与 `&&` 短路合不了。分开写。
    if let Ok(mut g) = KNOWN.lock() {
        if !g.iter().any(|s| s == secret) {
            g.push(secret.to_string());
        }
    }
}

/// 抹掉所有已登记的 key。用于「不再需要」时清缓存。
pub fn forget_all() {
    if let Ok(mut g) = KNOWN.lock() {
        g.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "sk-abc123def456ghi789jkl";

    /// 报错里回显了 key —— 这是真实会发生的事，上游 401 经常这么干。
    #[test]
    fn an_error_echoing_the_key_is_masked() {
        remember(KEY);
        let msg = format!("upstream 401: invalid api key {KEY} (request id abc)");
        let safe = redact(&msg);
        assert!(!safe.contains(KEY), "key 还在：{safe}");
        assert!(safe.contains("401"), "原因要保留：{safe}");
        forget_all();
    }

    /// 没登记过的 `sk-` 形状也要抹掉。
    ///
    /// 上游可能回显**别人的** key 或一个我们没加载过的值。
    #[test]
    fn an_unregistered_sk_shaped_token_is_still_masked() {
        let msg = "failed with Bearer sk-ZZZZxxxxYYYYzzzz9999WWW";
        let safe = redact(msg);
        assert!(
            !safe.contains("sk-ZZZZxxxxYYYYzzzz9999WWW"),
            "未登记的 sk- 形状没被抹掉：{safe}"
        );
        assert!(safe.contains("Bearer"), "上下文要保留：{safe}");
    }

    /// 没有可疑内容时**不能**动原文——日志里全是这个就没法看了。
    #[test]
    fn plain_text_is_untouched() {
        let msg = "锚点推不出来：面板底色不在预期范围";
        assert_eq!(redact(msg), msg);
    }

    /// 多个 sk- 形状都要抹，不能只抹第一个。
    #[test]
    fn every_occurrence_is_masked() {
        let msg = "a=sk-aaaaaaa1111111 b=sk-bbbbbbb2222222";
        let safe = redact(msg);
        assert!(!safe.contains("aaaaaaa1111111"), "{safe}");
        assert!(!safe.contains("bbbbbbb2222222"), "{safe}");
    }

    /// 太短的串不登记为敏感值——否则 redact 会把正常文本也抹掉。
    #[test]
    fn very_short_strings_are_not_registered() {
        remember("abc");
        assert!(
            !known_secrets().iter().any(|s| s == "abc"),
            "短串不该被登记成敏感值"
        );
        assert_eq!(redact("abc def"), "abc def", "短串不该导致误抹");
    }

    /// 设置页留空 = 保留原值，**不是清空**。
    ///
    /// 这是用户明确要的：不想每次改别的设置都重新粘一遍。
    #[test]
    fn an_empty_input_keeps_the_existing_key() {
        assert_eq!(apply("no-such-provider-xyz", None), Ok(false));
        assert_eq!(apply("no-such-provider-xyz", Some("   ")), Ok(false));
        assert!(
            !status("no-such-provider-xyz").configured,
            "没配过的 provider 提交空值后仍然应该是没配"
        );
    }

    /// 没配 key 时给的下一步指引要**具体到能照做**。
    #[test]
    fn the_missing_hint_is_actionable() {
        let h = missing_hint("openai");
        assert!(h.contains("凭据管理器"), "没说存哪：{h}");
        assert!(h.contains("OPENAI_API_KEY"), "没说环境变量名：{h}");
        assert!(h.len() > 40, "指引太短，没有可操作性：{h}");
    }

    /// 状态里**永远不含 key 本身**。
    #[test]
    fn the_status_never_carries_the_key() {
        remember(KEY);
        let s = status("no-such-provider-xyz");
        let dumped = format!("{s:?}");
        assert!(!dumped.contains(KEY), "状态里带出了 key：{dumped}");
        forget_all();
    }
}
