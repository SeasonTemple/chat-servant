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
use windows::core::{BOOL, Owned, PWSTR};

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

    /// `EnumWindows` 的回调。**由 Windows 在本线程上同步调用**，
    /// 不跨线程、不延迟——所以它拿到的 `lparam` 指向的 `ctx` 一定还活着。
    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` 是 `enumerate_for` 用 `std::ptr::from_mut(&mut ctx)`
        // 传进来的**栈上** `ctx`；`EnumWindows` 是同步枚举，返回前不会撤销，
        // 期间 `ctx` 一直不可移动、不可再借用。这正是把裸指针包成 LPARAM
        // 再取回来的标准做法。
        let ctx = unsafe { &mut *(lparam.0 as *mut Ctx<'_>) };
        // SAFETY: `describe` 只读窗口信息、不改全局状态；下面已给出它的前提。
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
    // SAFETY: `cb` 是同步回调，返回前 `ctx` 不动（见上面的说明）。
    unsafe { EnumWindows(Some(cb), LPARAM(std::ptr::from_mut(&mut ctx) as isize)) }
        .map_err(|e| WindowError::Enumeration(e.to_string()))?;
    Ok(ctx.out)
}

/// 收集一个窗口的完整信息。窗口已经关了、进程名取不到就跳过它。
///
/// # Safety
///
/// `hwnd` 必须是 `EnumWindows` 正在枚举的**活窗口句柄**。
/// 函数只查询（可见性 / 进程 / 标题 / 矩形），不改动窗口状态，
/// 所以不要求调用方持有任何额外锁；但传一个已销毁的 HWND
/// 仍会读到未定义内容。
unsafe fn describe(hwnd: HWND) -> Option<WindowHit> {
    // SAFETY: 只读窗口可见性，不解引用指针、不写内存。
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return None;
    }

    let mut pid: u32 = 0;
    // SAFETY: `pid` 是栈上 u32，`&raw mut` 有效且生命周期覆盖调用。
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };

    let exe = process_exe_name(pid)?;
    if exe.is_empty() {
        return None;
    }

    // 缓冲区长度必须如实传给 Win32。`GetWindowTextW` 返回的是**实际写入的
    // 字符数（不含终止 NUL）**，最大不超过 `buf.len()-1`——`n` 越界的话
    // 下面 `&buf[..n]` 就会 panic。
    let mut buf = [0u16; 512];
    // SAFETY: `buf` 是 512 个 u16 的栈数组，`&mut buf` 自动带上长度，
    // Win32 只往这块内存里写。
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);

    // 取不到矩形不影响识别进程，只是面积排序会不准，所以不失败。
    let mut rect = RECT::default();
    // SAFETY: `rect` 是栈变量，`GetWindowRect` 只写 4 个 i32。
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
    // SAFETY: 只申请进程查询权限（`PROCESS_QUERY_LIMITED_INFORMATION`），
    // 不申请读写内存——这里只读可执行文件的路径。
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    // `Owned` 在 drop 时调 `CloseHandle`。这里用 guard 而不是在末尾手动关：
    // 后面还有提前返回的路（下面 `QueryFullProcessImageNameW` 失败的 `?`，
    // 以及 `file_name()` 取不到时的 `None`），逐条补 `CloseHandle` 迟早会漏一条。
    //
    // 实测（2026-10-19）：改成 guard 之前**每次调用泄漏一个句柄**，
    // 500 次调用把本进程句柄数从 683 抬到 1183。这条在热路径上——
    // `describe` 对**每个可见顶层窗口**调一次，`load_frame` 又每个观察 tick
    // 跑一遍，所以泄漏速率是「可见窗口数 × tick 数」，常驻数小时就是数万个。
    //
    // SAFETY: `raw` 是上面 `OpenProcess` 成功返回的、**本函数独占**的进程句柄，
    // 还没交给过别人；`Owned` 接管它并在 drop 时关闭，正是它该去的所有权终点。
    let handle = unsafe { Owned::new(raw) };
    let mut buf = [0u16; 1024];
    // `size` 是**入参也是出参**：入参写容量，出参写实际字符数。
    // 必须先填上真实容量，否则 Win32 认为缓冲区是 0 字节、直接失败。
    let mut size = buf.len() as u32;
    // SAFETY: `buf` 是 1024 个 u16 的栈数组、容量已如实写进 `size`；
    // `size` 是栈上 u32。Win32 只往这块内存里写，不多写。
    unsafe {
        QueryFullProcessImageNameW(
            *handle,
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
///
/// ## 同标题时为什么还要比面积
///
/// 精确匹配的标题可能命中**不止一个**窗口（多开实例、
/// 主窗口 + 通知窗）。枚举顺序是 z-order，不是「哪个是主窗口」，
/// 所以取 `find()` 的第一个等于让 z-order 决定。
/// 同标题之间取面积最大的，标题规则判定「是哪一类」，面积只定主次。
///
/// ## 别拿最小化窗口的 `GetWindowRect` 判大小
///
/// 实测（2026-10-07）：微信主窗口最小化后 `GetWindowRect` 返回
/// **160x28**，还原后是 **1175x900**——最小化时给的是图标矩形，
/// 不是窗口真实尺寸。曾据此误判「选中的是托盘气泡窗口」，
/// 并加了一道尺寸下限，结果把最小化的主窗口一起挡掉，
/// 害得 `restore_without_activating` 的还原路径永远走不到。
/// 要判大小必须先 `GetWindowPlacement` 取还原尺寸。
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
        return mine
            .iter()
            .filter(|c| c.title == want)
            .max_by_key(|c| c.area)
            .map(|c| (*c).clone());
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
    // SAFETY: 只读窗口状态，不解引用指针、不写内存。
    if !unsafe { IsIconic(hwnd) }.as_bool() {
        return false;
    }
    // SAFETY: 两个调用都只**改窗口的显示状态**，不解引用任何我们持有的指针；
    // 参数全是立即值。`SW_SHOWNOACTIVATE` + `SWP_NOACTIVATE` 保证不抢焦点。
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

    #[test]
    fn same_title_picks_largest_not_first_enumerated() {
        // 精确匹配可能命中多个窗口。枚举顺序是 z-order，不能让它决定主次。
        let big = hit("weixin.exe", "微信", 1_057_500);
        let small = hit("weixin.exe", "微信", 100);
        for cands in [vec![big.clone(), small.clone()], vec![small, big.clone()]] {
            let picked = select_main(&WECHAT, &cands).expect("应当选中");
            assert_eq!(picked.area, 1_057_500, "同标题里应当取面积最大的");
        }
    }
}
