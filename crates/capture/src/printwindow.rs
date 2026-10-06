//! `PrintWindow` 兜底捕获路径。
//!
//! WGC 是首选（能抓被遮挡和 GPU 合成窗口），但在这台机器上实测：
//! WGC 会话起来后会**立刻关闭且不报错**，微信和记事本表现一致。
//! 见 docs/adr/0004-capture-strategy.md。
//!
//! 按分级往下走：`PrintWindow` + `PW_RENDERFULL_CONTENT`。
//! 这个标志让窗口自己重绘进我们给的 DC，对 DirectComposition / Direct2D
//! 自绘窗口是唯一能出画的路子。代价：**要求目标窗口可见**。

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GetDIBits, GetWindowDC, HBITMAP, HDC, RGBQUAD, ReleaseDC, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

/// `PW_RENDERFULL_CONTENT`。让窗口重绘而不是让 DWM 回放合成结果，
/// 对 DirectComposition / Direct2D 自绘窗口是唯一能出画的路子。
const RENDER_FULL_CONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(2);

#[derive(Debug, thiserror::Error)]
pub enum PrintWindowError {
    #[error("取不到窗口矩形，或窗口尺寸无效")]
    BadSize,
    #[error("创建 GDI 绘图上下文失败（{0}）")]
    Gdi(&'static str),
    #[error("PrintWindow 返回 0：目标窗口拒绝重绘。这类窗口通常只能靠 WGC")]
    Rejected,
    #[error("PrintWindow 成功但 GetDIBits 读回失败")]
    GetDIBitsFailed,
    #[error("取到位图但缓冲区对不上尺寸 {0}x{1}")]
    SizeMismatch(u32, u32),
    #[error("抓到的画面是纯黑")]
    Black,
}

/// 抓一帧。返回 RGB8 紧凑排列，**只在内存里**。
pub fn capture(hwnd_raw: isize) -> Result<chatservant_core::frame::Frame, PrintWindowError> {
    let hwnd = HWND(hwnd_raw as *mut c_void);
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &raw mut rect) }.map_err(|_| PrintWindowError::BadSize)?;
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    if w <= 0 || h <= 0 {
        return Err(PrintWindowError::BadSize);
    }
    let (w, h) = (w as u32, h as u32);

    // GDI 对象必须在所有提前返回之前清理，否则句柄泄漏。
    // 整个过程跑在一个闭包里，拿到结果再往外传。
    let frame = {
        let window_dc = unsafe { GetWindowDC(Some(hwnd)) };
        if window_dc.is_invalid() {
            return Err(PrintWindowError::Gdi("GetWindowDC"));
        }
        let mem_dc = unsafe { CreateCompatibleDC(Some(window_dc)) };
        if mem_dc.is_invalid() {
            unsafe {
                let _ = ReleaseDC(Some(hwnd), window_dc);
            }
            return Err(PrintWindowError::Gdi("CreateCompatibleDC"));
        }
        // 0,0 = 与 window_dc 同尺寸
        let bitmap = unsafe { CreateCompatibleBitmap(window_dc, 0, 0) };
        if bitmap.is_invalid() {
            unsafe {
                let _ = DeleteDC(mem_dc);
                let _ = ReleaseDC(Some(hwnd), window_dc);
            }
            return Err(PrintWindowError::Gdi("CreateCompatibleBitmap"));
        }

        unsafe {
            let _old = SelectObject(mem_dc, bitmap.into());

            let result = if PrintWindow(hwnd, mem_dc, RENDER_FULL_CONTENT).as_bool() {
                read_rgb(mem_dc, bitmap, w, h)
            } else {
                Err(PrintWindowError::Rejected)
            };

            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(Some(hwnd), window_dc);
            result
        }
    }?;
    Ok(frame)
}

/// 从内存 DC 读回 BGRA，转成 RGB8。
unsafe fn read_rgb(
    mem_dc: HDC,
    bitmap: HBITMAP,
    w: u32,
    h: u32,
) -> Result<chatservant_core::frame::Frame, PrintWindowError> {
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            // 负高度 = 从上到下，省一次翻转
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
        return Err(PrintWindowError::GetDIBitsFailed);
    }

    let mut rgb = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for px in bgra.chunks_exact(4) {
        rgb.extend_from_slice(&px[2..=3]); // R, G
        rgb.push(px[0]); // B
    }
    let frame = chatservant_core::frame::Frame::from_rgb(w, h, rgb)
        .ok_or(PrintWindowError::SizeMismatch(w, h))?;
    if frame.is_black() {
        return Err(PrintWindowError::Black);
    }
    Ok(frame)
}
