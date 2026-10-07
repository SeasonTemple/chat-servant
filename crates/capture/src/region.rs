//! 第 ③ 级兜底：从屏幕 DC 上按窗口矩形抠一块。
//!
//! ## 为什么需要这一级
//!
//! 它是**最后一道**，不是默认路径。产品默认走 `wgc_alt`（`wgc` crate），
//! 实测 342 ms 抓到被遮挡的微信窗口、帧内 OCR 可读，见 ADR-0004。
//!
//! 但 `windows-capture` 2.0.1 在本机对**微信窗口、记事本窗口、主显示器**
//! 三者表现一致地失败：会话送来第一帧之前就关闭、一帧不来（**不是** WGC
//! 本身的问题——同一个 API 换 `wgc` crate 就正常）。它降级为诊断实现，
//! 不作产品默认，所以这一级留给「WGC 与 PrintWindow 都不可用」的场合。
//!
//! `PrintWindow` + `PW_RENDERFULL_CONTENT` 则被微信直接拒绝（返回 0），
//! 正是 GPU 合成自绘窗口的典型症状。
//!
//! 所以三级分级实际落地成：
//!
//! ```text
//! ① WGC(wgc crate) 首选（能抓被遮挡）      本机可用，342 ms
//! ② PrintWindow    退路                    微信拒绝（预期内）
//! ③ 区域抓屏       最后一级                本文件
//! ```
//!
//! ## 代价，必须写明
//!
//! 这一级读的是**屏幕像素**，所以要求目标窗口**可见且未被遮挡**。
//! 被别的窗口盖住时抓到的是盖在上面的东西。
//!
//! 这与产品承诺的「被其他窗口盖住也能读」冲突。所以：
//! 抓到非黑帧**不等于**抓到的是目标窗口——调用方必须自己校验
//! （例如比对 `Frame` 内容与预期布局），不能因为"没报错"就当成有效帧。

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HDC, RGBQUAD, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsIconic, IsWindowVisible};

#[derive(Debug, thiserror::Error)]
pub enum RegionError {
    #[error("窗口不可见或已最小化。第 ③ 级读的是屏幕像素，看不见就抓不到")]
    NotVisible,
    #[error("取不到窗口矩形")]
    NoRect,
    #[error("创建 GDI 绘图上下文失败（{0}）")]
    Gdi(&'static str),
    #[error("屏幕抓取返回 0（BitBlt 失败）")]
    BitBltFailed,
    #[error("读回位图失败（GetDIBits 返回 0）")]
    GetDIBitsFailed,
    #[error("抓到的画面是纯黑")]
    Black,
}

/// 按窗口在屏幕上的位置，抠一块出来。
///
/// 返回的画面是**屏幕坐标**下的内容——`Frame` 的原点是屏幕左上角。
pub fn capture(hwnd_raw: isize) -> Result<chatservant_core::frame::Frame, RegionError> {
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() || unsafe { IsIconic(hwnd) }.as_bool() {
        return Err(RegionError::NotVisible);
    }
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &raw mut rect) }.map_err(|_| RegionError::NoRect)?;
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    if w <= 0 || h <= 0 {
        return Err(RegionError::NoRect);
    }
    let (w, h) = (w as u32, h as u32);

    // 整块都在屏幕外（多显示器拔了、窗口挪走了）也抓不到。
    let (screen_w, screen_h) = {
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        (unsafe { GetSystemMetrics(SM_CXSCREEN) }, unsafe {
            GetSystemMetrics(SM_CYSCREEN)
        })
    };
    if rect.left >= screen_w || rect.top >= screen_h || rect.right <= 0 || rect.bottom <= 0 {
        return Err(RegionError::NoRect);
    }

    let frame = {
        let screen_dc = unsafe { GetDC(None) };
        if screen_dc.is_invalid() {
            return Err(RegionError::Gdi("GetDC"));
        }
        let mem_dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
        if mem_dc.is_invalid() {
            unsafe {
                let _ = ReleaseDC(None, screen_dc);
            }
            return Err(RegionError::Gdi("CreateCompatibleDC"));
        }
        let bitmap = unsafe { CreateCompatibleBitmap(screen_dc, w as i32, h as i32) };
        if bitmap.is_invalid() {
            unsafe {
                let _ = DeleteDC(mem_dc);
                let _ = ReleaseDC(None, screen_dc);
            }
            return Err(RegionError::Gdi("CreateCompatibleBitmap"));
        }

        unsafe {
            let _old = SelectObject(mem_dc, bitmap.into());

            // SRCCOPY：整块搬过来，不需要遮罩。
            let result = if BitBlt(
                mem_dc,
                0,
                0,
                w as i32,
                h as i32,
                Some(screen_dc),
                rect.left,
                rect.top,
                SRCCOPY,
            )
            .is_ok()
            {
                read_rgb(mem_dc, bitmap, w, h)
            } else {
                Err(RegionError::BitBltFailed)
            };

            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(None, screen_dc);
            result
        }
    }?;
    Ok(frame)
}

/// 抓之前先问一下：这个窗口现在能不能用第 ③ 级抓。
///
/// UI 上要据此禁用/提示「请把聊天窗口露出来」，比抓完再报错友好。
pub fn is_capturable(hwnd_raw: isize) -> bool {
    capture(hwnd_raw).is_ok()
}

/// 目标点是否被别的窗口盖着。
///
/// WGC 不可用时，这是唯一能诚实告诉用户「你这条读不到」的手段。
/// 用 `WindowFromPoint` 拿命中的顶层窗口，跟目标 HWND 比一下。
pub fn is_occluded(hwnd_raw: isize, sample_points: &[(i32, i32)]) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::WindowFromPoint;
    let hwnd = HWND(hwnd_raw as *mut c_void);
    sample_points.iter().any(|&(x, y)| {
        let hit = unsafe { WindowFromPoint(POINT { x, y }) };
        !hit.is_invalid() && hit != hwnd
    })
}

/// 窗口**屏幕**矩形上的 3×3 采样点。给 [`is_occluded`] 用。
///
/// 单独抽出来是因为调用方不该自己算——它得是**屏幕坐标**，
/// 用窗口内的坐标去 `WindowFromPoint` 会命中窗口自己，永远返回 false，
/// 于是那道闸形同虚设却看不出来。算错一次就会得到一个永远为假的检查。
///
/// 窗口不可见或取不到矩形时返回空 `Vec`——那时 `is_occluded` 天然返回
/// `false`，而**真实的**失败原因由 `capture` 报出来。
pub fn occlusion_samples(hwnd_raw: isize) -> Vec<(i32, i32)> {
    let hwnd = HWND(hwnd_raw as *mut c_void);
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &raw mut rect) }.is_err() {
        return Vec::new();
    }
    let (l, t, r, b) = (rect.left, rect.top, rect.right, rect.bottom);
    if r <= l || b <= t {
        return Vec::new();
    }
    // 取边长 1/4 与 3/4 处，避开标题栏这类「本来就是目标窗口」的边条。
    let xs = [l + (r - l) / 4, l + (r - l) / 2, l + (r - l) * 3 / 4];
    let ys = [t + (b - t) / 4, t + (b - t) / 2, t + (b - t) * 3 / 4];
    let mut out = Vec::with_capacity(9);
    for &y in &ys {
        for &x in &xs {
            out.push((x, y));
        }
    }
    out
}

/// 窗口是不是被 DWM **cloaked**（藏起来）了。
///
/// ## 为什么要单独查这一项
///
/// `IsWindowVisible` 为 true、`IsIconic` 为 false、`WindowFromPoint` 也
/// 命中它——**但它照样可能抓不到**。cloaked 的窗口不参与 DWM 合成，
/// WGC 拿不到帧、PrintWindow 也拒绝重绘。触发条件之一是窗口在
/// **另一个虚拟桌面**上。
///
/// 光靠「可见 / 未遮挡」两个维度会把这种状态**误判成应该能抓**。
/// 它排在遮挡检查**前面**就是为此：被藏起来的窗口在屏幕坐标上仍然占位，
/// `WindowFromPoint` 命中的是它自己，遮挡检查会说「没被挡」。
///
/// ⚠ **本机实测它是 false**，而 WGC 仍然失败——所以它是**候选原因之一**，
/// 不是已确认的原因。别把它当成对当前故障的诊断结论。
pub fn is_cloaked(hwnd_raw: isize) -> bool {
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    let hwnd = HWND(hwnd_raw as *mut c_void);
    let mut cloaked: i32 = 0;
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            std::mem::size_of::<i32>() as u32,
        )
    };
    ok.is_ok() && cloaked != 0
}

/// WGC 的前置判据：**只查真正挡住 WGC 的东西**。
///
/// 不可见 / 最小化 / 被 DWM 藏起来（多半在别的虚拟桌面）——
/// 这三种情况下 WGC 确实拿不到帧。
///
/// ## 刻意**不查遮挡**
///
/// 遮挡是第 ③ 级区域抓屏的死穴，但对 WGC 不是——**WGC 就是为抓被遮挡
/// 窗口而存在的**。实测（2026-10-07）6/9 采样点被盖住时，
/// e2e 照样读出了对方消息，八段全通。
///
/// 把遮挡当挡箭牌的后果是实际的：观察循环曾用区域抓屏的判据去挡 WGC，
/// 结果**只要有任何东西盖住微信就直接返回错误**——而那永远是常态。
/// 表现为面板上写着「抓不到：窗口被别的窗口盖住了」，一帧都没抓到过。
pub fn why_wgc_unreadable(hwnd_raw: isize) -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible};
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return Some("窗口不可见（可能在别的虚拟桌面或已最小化到托盘）".into());
    }
    if unsafe { IsIconic(hwnd) }.as_bool() {
        return Some("窗口已最小化——还原它才能抓到".into());
    }
    // cloaked 要排在遮挡**前面**：被藏起来的窗口在屏幕坐标上仍然占位，
    // `WindowFromPoint` 会命中它自己，于是遮挡检查说「没被挡」——
    // 而真正的原因是它根本不参与合成。两个维度都过才谈得上「能抓」。
    if is_cloaked(hwnd_raw) {
        return Some("窗口被 DWM 藏起来了（多半在另一个虚拟桌面上）——切回它所在的桌面".into());
    }
    None
}

/// 一句话说明「为什么现在**用区域抓屏**读不到」，给日志和界面用。
///
/// 存在的理由：区域抓屏失败时，错误信息只有「抓帧失败」四个字，
/// 用户看不出是自己把窗口压在别的窗口底下了。这句提示比重试一千次有用。
///
/// ## ⚠ **它不适用于 WGC**
///
/// 遮挡对区域抓屏是致命的，但 **WGC 恰恰就是为抓被遮挡窗口设计的**。
/// 拿它当 WGC 的前置判据，会让**只要有任何东西盖住目标窗口就直接失败**，
/// 而那是常态。WGC 的判据是 [`why_wgc_unreadable`]。
pub fn why_unreadable(hwnd_raw: isize) -> Option<String> {
    if let Some(why) = why_wgc_unreadable(hwnd_raw) {
        return Some(why);
    }
    use windows::Win32::UI::WindowsAndMessaging::WindowFromPoint;
    let hwnd = HWND(hwnd_raw as *mut c_void);
    let pts = occlusion_samples(hwnd_raw);
    let covered = pts
        .iter()
        .filter(|&&(x, y)| {
            let hit = unsafe { WindowFromPoint(POINT { x, y }) };
            !hit.is_invalid() && hit != hwnd
        })
        .count();
    if covered == 0 {
        return None;
    }
    Some(format!(
        "窗口被别的窗口盖住了（{}/{} 个采样点命中的是别的窗口）——把它露出来",
        covered,
        pts.len()
    ))
}

unsafe fn read_rgb(
    mem_dc: HDC,
    bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
    w: u32,
    h: u32,
) -> Result<chatservant_core::frame::Frame, RegionError> {
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: windows::Win32::Graphics::Gdi::BI_RGB.0,
            ..Default::default()
        },
        bmiColors: [RGBQUAD::default(); 1],
    };
    let mut bgra = vec![0u8; (w as usize) * (h as usize) * 4];
    let lines = unsafe {
        GetDIBits(
            mem_dc,
            bitmap,
            0,
            h,
            Some(bgra.as_mut_ptr().cast()),
            &raw mut info,
            DIB_RGB_COLORS,
        )
    };
    if lines == 0 {
        return Err(RegionError::GetDIBitsFailed);
    }

    let mut rgb = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for px in bgra.chunks_exact(4) {
        rgb.extend_from_slice(&px[2..=3]);
        rgb.push(px[0]);
    }
    let frame =
        chatservant_core::frame::Frame::from_rgb(w, h, rgb).ok_or(RegionError::GetDIBitsFailed)?;
    if frame.is_black() {
        return Err(RegionError::Black);
    }
    Ok(frame)
}
