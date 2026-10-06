//! 第 ③ 级兜底：从屏幕 DC 上按窗口矩形抠一块。
//!
//! ## 为什么需要这一级
//!
//! 实机结论（见 docs/adr/0004-capture-strategy.md）：
//! 本机 WGC **系统层面支持**（`IsSupported()` 返回 true，Win10 22631），
//! 但 `windows-capture` 2.0 的会话路径对**微信窗口、记事本窗口、主显示器**
//! 三者表现完全一致：会话线程无错退出，一帧不来。
//! `PrintWindow` + `PW_RENDERFULL_CONTENT` 则被微信直接拒绝（返回 0），
//! 正是 GPU 合成自绘窗口的典型症状。
//!
//! 所以三级分级实际落地成：
//!
//! ```text
//! ① WGC         首选（能抓被遮挡）      本机不工作，原因待查
//! ② PrintWindow 退路                     微信拒绝（预期内）
//! ③ 区域抓屏    最后一级                 本文件
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
