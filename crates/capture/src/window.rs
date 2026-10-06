//! 按进程名和标题规则找出聊天窗口。
//!
//! 用的是标准 Win32 枚举接口，不 hook、不注入、不读对方进程内存。

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, HWND_BOTTOM, IsIconic,
    IsWindowVisible, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
    ShowWindow,
};
use windows::core::{BOOL, PWSTR};

use crate::surface::ChatSurface;

/// 一个候选窗口的快照。
///
/// `hwnd_raw` 是原始句柄值。故意用 `isize` 而不是 `HWND`：
/// Win32 类型只该留在本模块里，跨 crate 传的是不透明整数。
#[derive(Debug, Clone)]
pub struct WindowHit {
    pub hwnd_raw: isize,
    pub pid: u32,
    pub exe: String,
    pub title: String,
    /// 窗口矩形面积（含边框）。只在没有标题规则时用来排序。
    pub area: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum WindowError {
    #[error("枚举顶层窗口失败：{0}")]
    Enumeration(String),

    /// 找不到时把候选带出来，否则"没找到窗口"是不可排查的。
    #[error("没找到「{label}」的窗口。{hint}")]
    NotFound { label: String, hint: String },
}

/// 枚举全部可见顶层窗口，只留下属于 `surface` 进程的。
pub fn enumerate_for(surface: &ChatSurface) -> Result<Vec<WindowHit>, WindowError> {
    struct Ctx<'a> {
        exes: &'a [&'static str],
        out: Vec<WindowHit>,
    }

    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = unsafe { &mut *(lparam.0 as *mut Ctx<'_>) };
        if let Some(hit) = unsafe { describe(hwnd) }
            && ctx.exes.iter().any(|e| *e == hit.exe)
        {
            ctx.out.push(hit);
        }
        BOOL(1) // TRUE = 继续枚举
    }

    let mut ctx = Ctx {
        exes: surface.exes,
        out: Vec::new(),
    };
    unsafe { EnumWindows(Some(cb), LPARAM(std::ptr::from_mut(&mut ctx) as isize)) }
        .map_err(|e| WindowError::Enumeration(e.to_string()))?;
    Ok(ctx.out)
}

/// 收集一个窗口的完整信息。窗口已经关了、进程名取不到就跳过它。
unsafe fn describe(hwnd: HWND) -> Option<WindowHit> {
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return None;
    }

    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };

    let exe = process_exe_name(pid)?;
    if exe.is_empty() {
        return None;
    }

    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);

    // 取不到矩形不影响识别进程，只是面积排序会不准，所以不失败。
    let mut rect = RECT::default();
    let area = if unsafe { GetWindowRect(hwnd, &raw mut rect) }.is_ok() {
        i64::from(rect.right - rect.left) * i64::from(rect.bottom - rect.top)
    } else {
        0
    };

    Some(WindowHit {
        hwnd_raw: hwnd.0 as isize,
        pid,
        exe,
        title,
        area,
    })
}

/// 进程名（小写，含扩展名）。打不开进程就返回 `None`——
/// 权限不足的进程不是我们要找的，跳过比报错合理。
fn process_exe_name(pid: u32) -> Option<String> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut size = buf.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buf.as_mut_ptr()),
            &raw mut size,
        )
    }
    .ok()?;
    let full = String::from_utf16_lossy(&buf[..size as usize]);
    std::path::Path::new(&full)
        .file_name()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
}

/// 从候选里挑主窗口。
///
/// 有 `main_title` 规则时按标题**精确**匹配；否则排除 `skip_titles`
/// 之后挑面积最大的。都不命中返回 `None`，**不猜**。
pub fn select_main(surface: &ChatSurface, candidates: &[WindowHit]) -> Option<WindowHit> {
    let mine: Vec<&WindowHit> = candidates
        .iter()
        .filter(|c| surface.matches_exe(&c.exe))
        .collect();
    if mine.is_empty() {
        return None;
    }

    if let Some(want) = surface.main_title {
        // 标题规则没命中时不退回面积：宁可报"没找到"，也不要抓一个
        // 工具窗或看图窗回来——那会让人以为程序在正常工作。
        return mine.iter().find(|c| c.title == want).map(|c| (*c).clone());
    }

    mine.iter()
        .filter(|c| !surface.skip_titles.contains(&c.title.as_str()))
        .max_by_key(|c| c.area)
        .map(|c| (*c).clone())
}

/// 找到 `surface` 的主窗口。找不到时把候选列进错误里。
pub fn find_chat_window(surface: &ChatSurface) -> Result<WindowHit, WindowError> {
    let candidates = enumerate_for(surface)?;
    if let Some(hit) = select_main(surface, &candidates) {
        return Ok(hit);
    }

    let hint = if candidates.is_empty() {
        format!(
            "该进程名（{}）下一个可见窗口都没枚举到。确认软件开着，且不是缩到托盘里。",
            surface.exes.join(" / ")
        )
    } else {
        let mut s = format!(
            "该进程下枚举到 {} 个窗口，但都不符合标题规则（期望 {:?}）：\n",
            candidates.len(),
            surface.main_title.unwrap_or("<无标题规则>")
        );
        for c in &candidates {
            s.push_str(&format!(
                "  pid={} 标题={:?} 面积={}\n",
                c.pid, c.title, c.area
            ));
        }
        s
    };
    Err(WindowError::NotFound {
        label: surface.label.to_string(),
        hint,
    })
}

/// 最小化的窗口用"不激活还原 + 压到最底"的方式恢复。
///
/// Windows 不渲染最小化的窗口，任何截图法都拿不到画面。SW_SHOWNOACTIVATE
/// 不抢焦点，HWND_BOTTOM 让它看着跟收起来一样，但 DWM 继续绘制。
///
/// 返回是否动了手。
pub fn restore_without_activating(hwnd_raw: isize) -> bool {
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if !unsafe { IsIconic(hwnd) }.as_bool() {
        return false;
    }
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_BOTTOM),
            0,
            0,
            0,
            0,
            SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE,
        );
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WECHAT;

    fn hit(exe: &str, title: &str, area: i64) -> WindowHit {
        WindowHit {
            hwnd_raw: 0,
            pid: 1,
            exe: exe.into(),
            title: title.into(),
            area,
        }
    }

    #[test]
    fn main_title_wins_over_area() {
        let cands = vec![
            hit("weixin.exe", "图片查看", 9_000_000),
            hit("weixin.exe", "微信", 1_000_000),
        ];
        assert_eq!(
            select_main(&WECHAT, &cands).map(|c| c.title),
            Some("微信".into())
        );
    }

    #[test]
    fn title_mismatch_does_not_fall_back_to_area() {
        // 宁可报"没找到"，也不要抓一个面积最大的工具窗回来。
        let cands = vec![hit("weixin.exe", "图片查看", 9_000_000)];
        assert!(select_main(&WECHAT, &cands).is_none());
    }

    #[test]
    fn other_processes_are_ignored() {
        let cands = vec![hit("chrome.exe", "微信", 1_000_000)];
        assert!(select_main(&WECHAT, &cands).is_none());
    }

    #[test]
    fn area_used_only_when_no_title_rule() {
        let surface = ChatSurface {
            key: "t",
            label: "t",
            exes: &["t.exe"],
            main_title: None,
            skip_titles: &["list"],
        };
        let cands = vec![hit("t.exe", "list", 9_000_000), hit("t.exe", "chat", 100)];
        assert_eq!(
            select_main(&surface, &cands).map(|c| c.title),
            Some("chat".into())
        );
    }
}
