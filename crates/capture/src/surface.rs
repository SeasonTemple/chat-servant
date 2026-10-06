//! 目标聊天软件的一份适配描述。
//!
//! 这里只放"怎么认出这个软件的窗口"和"这个软件自己的气泡长什么样"这类
//! **领域知识**。采集、调理、识别的算法在别处，不随软件变化。
//!
//! 接一个新软件 = 新增一份 `ChatSurface`，不改任何引擎代码。

/// 窗口层面的适配信息。
///
/// 挑窗口的规则是**进程名 + 标题规则**，不是面积。
/// 微信同进程还开着工具窗和看图窗，面积可能比主窗口还大，
/// 按面积挑会挑错。
#[derive(Debug, Clone, Copy)]
pub struct ChatSurface {
    /// 稳定标识，用于配置和会话路由。
    pub key: &'static str,
    /// 界面上显示的名字。
    pub label: &'static str,
    /// 进程名（小写，含扩展名）。任一匹配即认为是这个软件。
    pub exes: &'static [&'static str],
    /// 首选的顶层窗口标题。`None` 表示"排除 skip_titles 后挑面积最大的"。
    pub main_title: Option<&'static str>,
    /// 永远不是会话窗口的标题（联系人列表、通知气泡等）。
    pub skip_titles: &'static [&'static str],
}

impl ChatSurface {
    /// 这个进程名是不是本软件。
    pub fn matches_exe(&self, exe: &str) -> bool {
        let exe = exe.to_ascii_lowercase();
        self.exes.iter().any(|e| *e == exe)
    }
}

/// 微信。v1 只做这一个。
///
/// 4.x 的窗口标题是「微信」；3.x 时代是「微信」但气泡色不同，
/// 换成别的值前先跑 `cargo xtask probe frame` 看一眼实际窗口。
pub const WECHAT: ChatSurface = ChatSurface {
    key: "wechat",
    label: "微信",
    exes: &["weixin.exe", "wechat.exe"],
    main_title: Some("微信"),
    skip_titles: &[],
};

/// 当前支持的全部软件。接入新软件往这里加。
pub const ALL: &[ChatSurface] = &[WECHAT];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_match_is_case_insensitive() {
        assert!(WECHAT.matches_exe("Weixin.exe"));
        assert!(WECHAT.matches_exe("WEIXIN.EXE"));
    }

    #[test]
    fn exe_match_rejects_other_apps() {
        assert!(!WECHAT.matches_exe("chrome.exe"));
        assert!(!WECHAT.matches_exe("weixin.ex"));
        assert!(!WECHAT.matches_exe(""));
    }

    #[test]
    fn surface_keys_are_unique() {
        for (i, a) in ALL.iter().enumerate() {
            for b in &ALL[i + 1..] {
                assert_ne!(a.key, b.key, "key 重复: {}", a.key);
            }
        }
    }

    #[test]
    fn every_surface_lists_at_least_one_exe() {
        for s in ALL {
            assert!(!s.exes.is_empty(), "{} 没有配置进程名", s.key);
        }
    }
}
