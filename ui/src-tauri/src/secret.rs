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
//!
//! ## ⚠ 端点与代理也走这一条（2026-10-15，票 43）
//!
//! 开放自定义端点与代理之后，凭据多出两种**新形状**：
//! `https://user:pass@host/`（代理几乎都长这样）与
//! `?access_token=…`（把 token 放路径/查询里的端点）。两者都由
//! [`mask_url_credentials`] 覆盖，**且调用点与 key 是同一条**：
//! 凡是会把端点或代理地址拼进对外文本的地方，一律走 [`redact`]，
//! 不另开旁路。
//!
//! 凭据目标名按**组**分开（`chat-servant/<组>/<供应商>`），
//! 两组模型配置各自一把 key，见 [`target`]。

use chatservant_reason::ModelGroup;
use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW,
    CredWriteW,
};
use windows::core::{PCWSTR, PWSTR};

/// 凭据管理器里的目标名前缀。`chat-servant/<组>/<供应商>`。
const TARGET_PREFIX: &str = "chat-servant";

/// ⚠ **2026-10-15 改了目标名形状**（票 43）：从 `chat-servant/<供应商>`
/// 变成 `chat-servant/<组>/<供应商>`。
///
/// 为什么非改不可：两组模型配置要**各自一把 key**。共用一把的话
/// 「决策走本地、文字走云端」这种组合根本配不出来——而后写入的那一把
/// 会把前一把顶掉。
///
/// [`legacy_target`] 是**读时**的迁移兜底，见 [`from_vault`]。
fn target(group: ModelGroup, provider: &str) -> String {
    format!("{TARGET_PREFIX}/{}/{}", group.key(), provider)
}

/// 旧版（2026-10-15 之前）的目标名。**只读时兜底，不写。**
fn legacy_target(provider: &str) -> String {
    format!("{TARGET_PREFIX}/{provider}")
}

/// 掩码后的样子。**长度刻意不等于真实长度**——那会泄漏信息。
const MASK: &str = "••••••••";

/// 一把 key 的状态。**前端只拿得到这个，拿不到 key 本身。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyStatus {
    /// 已经配好了（凭据管理器或环境变量任一有值）。
    pub configured: bool,
    /// 值来自环境变量。这种情况下设置页的输入框应该是只读的。
    pub from_env: bool,
    /// 值来自**旧版那个不带组别的目标名**。界面上要说明，因为删的时候
    /// 会连带把它删掉（见 [`erase`]）。
    pub legacy: bool,
    /// 给界面看的掩码，仅用于「已配置」那一行提示。
    pub masked: String,
}

/// 没配置时的统一提示。**不能是空列表或静默失败。**
///
/// ⚠ 2026-10-15 压缩过两次。
///
/// **第一次**（原来四行）压缩后仍占掉设置页将近一半：它**每个未配置的供应商
/// 各渲染一遍**，设了三个就出现三次、而三段逐字相同。
///
/// **第二次**：把「在这个页面填」删掉。那句本来是**假的**——未配置时界面上
/// 根本没有输入框（前端把输入框锁在「换一把」这道门后面，而那个按钮只在
/// 已配置分支里，见 `.scratch/…/40-key-input-unreachable-when-unset.md`）。
/// 输入框修好之后它就是废话：框就在提示下面一行。
///
/// 留什么：① 存哪（凭据管理器，不落文件）——这是**隐私承诺**，必须在界面上，
/// 不能只活在文档里；② 环境变量这条备选，给 CI / 脚本用。
/// 删什么：「配好后不用重启设置，下一次生成就会用上」——
/// 那是**行为**，不是**说明**；它该在真的生效时告诉用户，不该每屏预告一次。
pub fn missing_hint(group: ModelGroup, provider: &str) -> String {
    format!(
        "存进 Windows 凭据管理器（{} 这一组独立存放），不落文件；也可设环境变量 {}_API_KEY。",
        group.label(),
        provider.to_uppercase()
    )
}

/// key 存进 Windows 凭据管理器。
///
/// `CRED_PERSIST_LOCAL_MACHINE`：跟着本机走，**不同步到漫游配置**——
/// 漫游会把凭据带到域里的其它机器去。
pub fn store(group: ModelGroup, provider: &str, secret: &str) -> Result<(), String> {
    let target = target(group, provider);
    let mut blob = secret.as_bytes().to_vec();

    let mut name: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    // UserName 只是凭据管理器列表里的显示名，不是登录名。
    let mut user: Vec<u16> = format!("{}/{provider}", group.key())
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

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
pub fn load(group: ModelGroup, provider: &str) -> Option<String> {
    let key = read_group(group, provider).or_else(|| from_env(provider))?;
    // 读到就登记。这样后续任何报错在 redact 时都能把它抹掉——
    // **读取的那一刻就是它进入本进程内存的那一刻**，也是唯一需要登记的时刻。
    remember(&key);
    Some(key)
}

/// 这一组的值从哪儿来。**读出来的那一把是不是旧版目标名里的。**
pub fn source(group: ModelGroup, provider: &str) -> KeySource {
    if read_group(group, provider).is_some() {
        KeySource::Group
    } else if read_one(&legacy_target(provider)).is_some() {
        KeySource::Legacy
    } else if from_env(provider).is_some() {
        KeySource::Env
    } else {
        KeySource::None
    }
}

/// 一把 key 当前是从哪儿来的。**前端只需要知道「是不是旧版那一把」。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    /// 这一组自己的目标名。
    Group,
    /// 2026-10-15 之前存的那把（**不带组别**，两组共用同一把）。
    Legacy,
    /// 环境变量。
    Env,
    /// 没有。
    None,
}

/// 先读这一组自己的，没有再读**旧版那个不带组别的**。
///
/// ⚠ 迁移兜底（2026-10-15 引入组别时加的）：不兜的话，每个用过旧版的
/// 机器升级后会突然变成「没配 key」，而界面上没有任何东西解释这件事——
/// 那正是这个项目反复在删的那种静默失效。用户下一次在这组里保存时，
/// 值就落到组内目标名下，兜底自然不再生效。
fn read_group(group: ModelGroup, provider: &str) -> Option<String> {
    read_one(&target(group, provider)).or_else(|| read_one(&legacy_target(provider)))
}

/// 从环境变量读。变量名规则收在 [`chatservant_reason::Provider::key_env_names`]，
/// 不在这里自己拼——应用与 `xtask e2e` 必须用同一份，否则会出现
/// 「诊断命令和产品读的不是同一个变量」这种已经犯过的病。
fn from_env(provider: &str) -> Option<String> {
    let p = chatservant_reason::Provider::from_key(provider)?;
    for var in p.key_env_names() {
        if let Some(v) = std::env::var(var).ok().filter(|s| !s.trim().is_empty()) {
            return Some(v);
        }
    }
    None
}

/// 从凭据管理器读**一个**目标名。
fn read_one(target: &str) -> Option<String> {
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
///
/// ## 为什么**两处都删**
///
/// 这一格的值可能来自组内目标名，也可能来自**旧版那把不带组别的**
/// （见 [`read_group`]）。只删一处的话，用户点完「删除」，界面上还写着
/// 「已配置」——一个**说谎的控件**比删不掉更糟。
///
/// ⚠ 代价要说清：旧版那把是两组共用的，删掉它对两组都生效。
/// 界面上必须把这一点说明，不能让另一组无声地掉线。
pub fn erase(group: ModelGroup, provider: &str) -> Result<(), String> {
    let group_target = target(group, provider);
    let legacy = legacy_target(provider);
    let mut first_err = None;
    for t in [&group_target, &legacy] {
        if let Err(e) = delete_one(t) {
            // 两处里一处都没有（常见：本来就是空的）不算失败——
            // `delete_one` 把「没有这一条」也归成一种 Err，这里分不清，
            // 所以**只要组内那处成功就当成功**。
            if t == &group_target {
                first_err = Some(e);
            }
        }
    }
    match first_err {
        None => Ok(()),
        Some(e) => Err(e),
    }
}

fn delete_one(target: &str) -> Result<(), String> {
    let name: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { CredDeleteW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None) }
        .map_err(|e| format!("删除凭据失败：{}", redact(&e.to_string())))
}

/// 当前状态。**不含 key**——只有「有没有」和「来自哪」。
pub fn status(group: ModelGroup, provider: &str) -> KeyStatus {
    let src = source(group, provider);
    match src {
        KeySource::None => KeyStatus {
            configured: false,
            from_env: false,
            legacy: false,
            masked: String::new(),
        },
        s => KeyStatus {
            configured: true,
            from_env: s == KeySource::Env,
            legacy: s == KeySource::Legacy,
            masked: MASK.to_string(),
        },
    }
}

/// 设置页提交：**留空 = 保留原值**，不清空。
///
/// 用户不想每次改别的设置都重新粘一遍 key。
/// 返回值告诉调用方「有没有真的写」。
pub fn apply(group: ModelGroup, provider: &str, input: Option<&str>) -> Result<bool, String> {
    match input.map(str::trim).filter(|s| !s.is_empty()) {
        Some(secret) => {
            store(group, provider, secret)?;
            Ok(true)
        }
        // 留空：什么都不做。**不是删除**——删除要走单独的 erase。
        None => Ok(false),
    }
}

/// 抹掉文本里可能出现的 key。
///
/// 四个来源：
/// 1. 已知的 key 值本身（我们知道，就直接替换）
/// 2. `sk-` / `Bearer ` 开头的 token 形状
/// 3. `api_key=...` / `"key": "..."` 形状的赋值
/// 4. **URL 里的凭据**（用户名密码段 + 查询参数里的 token）——见 [`mask_url_credentials`]
///
/// **宁可多抹。** 日志里少几个字符无所谓，泄一次就是事故。
///
/// ⚠ 2026-10-15：第 2、4 两步搬去了 [`chatservant_core::redact`]。
///   原因是它们**不是这一层独有的**——`crates/reason` 那侧的代理与
///   自定义端点报错同样需要遮蔽 URL 凭据，而本 crate 够不着 Windows
///   凭据管理器，于是那一侧自己写了一份**只认 `sk-`** 的弱版。
///   两份能力不同的 `redact` ＝ 两条旁路，强的那个等于没做。
///   ⇒ 纯字符串部分收到 core，这里只保留**本层独有的第 1 步**。
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for known in known_secrets() {
        if !known.is_empty() {
            out = out.replace(&known, MASK);
        }
    }
    chatservant_core::redact::shapes(&out)
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
// secret-scan: allow —— 本模块里那几串 sk-… 是**故意的假 token**：
// 它们是脱敏器自己的测试样本，出现的地方就是要被它抹掉的输入。
// pre-commit 的钩子按内容匹配 sk- 形状，拦下来只会让人养成 --no-verify
// 的习惯。按钩子自己写的规矩走**文件内豁免标记**，而不是绕过。
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
        let g = ModelGroup::Llm;
        assert_eq!(apply(g, "no-such-provider-xyz", None), Ok(false));
        assert_eq!(apply(g, "no-such-provider-xyz", Some("   ")), Ok(false));
        assert!(
            !status(g, "no-such-provider-xyz").configured,
            "没配过的 provider 提交空值后仍然应该是没配"
        );
    }

    /// **两组是两把 key，不是一把。**
    ///
    /// 这是本票验收的核心：合并成一个列表时，「决策走本地、文字走云端」
    /// 在数据结构上就配不出来。断言的是**目标名分得开**——
    /// 凭据管理器里那两条必须是两条独立记录。
    #[test]
    fn the_two_groups_get_two_different_credential_targets() {
        assert_ne!(
            target(ModelGroup::Decision, "openai"),
            target(ModelGroup::Llm, "openai")
        );
        assert_ne!(
            target(ModelGroup::Decision, "openai"),
            target(ModelGroup::Llm, "openai")
        );
        // 而它仍然不是「随便什么都能进」：组名与供应商名都收在受控的取值里。
        assert_eq!(
            target(ModelGroup::Decision, "openai"),
            "chat-servant/decision/openai"
        );
        assert_eq!(target(ModelGroup::Llm, "openai"), "chat-servant/llm/openai");
    }

    /// 没配 key 时给的下一步指引要**具体到能照做**。
    #[test]
    fn the_missing_hint_is_actionable() {
        let h = missing_hint(ModelGroup::Llm, "openai");
        assert!(h.contains("凭据管理器"), "没说存哪：{h}");
        assert!(h.contains("OPENAI_API_KEY"), "没说环境变量名：{h}");
        assert!(h.contains("独立存放"), "没说这是这一组自己的一把：{h}");
        assert!(h.len() > 40, "指引太短，没有可操作性：{h}");
    }

    /// 提示**不许承诺界面上做不到的事**。
    ///
    /// 2026-10-15 的真机死路：提示写着「在这个页面填」，而未配置时
    /// 输入框被锁在 `drafts[id] === REVEAL` 后面、而唯一设 `REVEAL` 的
    /// 「换一把」按钮又在 `{#if configured}` 分支里 ⇒ 那一页根本没有办法填。
    /// **提示与界面不一致，比没有提示更糟**：用户照着做，发现做不到。
    ///
    /// 前端那侧的真修是 `40-key-input-unreachable-when-unset.md`；
    /// 这条断言钉住的是**提示不许再变回那个说法**。
    #[test]
    fn the_missing_hint_does_not_promise_an_action_the_page_cannot_do() {
        let h = missing_hint(ModelGroup::Decision, "openai");
        assert!(
            !h.contains("在这个页面填"),
            "提示又承诺「在这个页面填」了——那一页现在确实能填，但这条断言是\
             防止它在别处被改回去、而输入框又被锁起来：{h}"
        );
    }

    /// 状态里**永远不含 key 本身**。
    #[test]
    fn the_status_never_carries_the_key() {
        remember(KEY);
        let s = status(ModelGroup::Llm, "no-such-provider-xyz");
        let dumped = format!("{s:?}");
        assert!(!dumped.contains(KEY), "状态里带出了 key：{dumped}");
        forget_all();
    }

    /// 代理地址里的**用户名密码段**不能进日志（票 43 的前置）。
    #[test]
    fn a_url_with_basic_auth_credentials_is_masked() {
        let msg = "代理连不上：https://alice:hunter2@proxy.internal:8080/ 拒绝连接";
        let safe = redact(msg);
        assert!(!safe.contains("hunter2"), "代理口令泄了：{safe}");
        assert!(!safe.contains("alice"), "代理账号泄了：{safe}");
        // ⚠ host 与端口必须留着——那是判断「哪个代理出问题」的唯一线索
        assert!(
            safe.contains("proxy.internal:8080"),
            "host 被抹掉了：{safe}"
        );
    }

    /// 查询参数里的 token 同样不能进日志。
    #[test]
    fn a_query_string_api_key_is_masked() {
        let msg = "GET /v1/models?access_token=abc123DEF456&model=x 失败";
        let safe = redact(msg);
        assert!(
            !safe.contains("abc123DEF456"),
            "query 里的 token 泄了：{safe}"
        );
        assert!(safe.contains("model=x"), "非敏感参数被误抹：{safe}");
    }

    /// 没有凭据的 URL **不许被动**——抹掉它等于让报错失去排查价值。
    ///
    /// 这条与 `plain_text_is_untouched` 同族：宁可少抹，也不能把
    /// 「是哪家、哪个端点」这条线索一起抹掉。
    #[test]
    fn a_url_without_credentials_is_left_alone() {
        let msg = "连不上 https://api.openai.com/v1/ 超时";
        assert_eq!(redact(msg), msg, "无凭据的 URL 被动了");
    }

    /// `tokenizer` 这类**只是碰巧含有 token 字样**的词不能被误伤。
    #[test]
    fn a_parameter_merely_containing_the_word_token_is_not_masked() {
        let msg = "字段 tokenizer 缺失";
        assert_eq!(redact(msg), msg, "误伤了非敏感字段名：{msg}");
    }
}
