//! `Frame` 是一次窗口捕获得到的完整位图。
//!
//! 硬约束：常驻内存，**永不落盘**。唯一的例外是 `xtask probe` 显式写盘，
//! ��那条路径要打���提示。

/// 矩形。坐标原点在左上角，右下开区间（`x1`/`y1` 是**不含**的）。
///
/// `Serialize` 让它能直接发给 UI——**几何判定仍然只在 Rust 侧**，
/// 前端拿到的坐标只用来渲染，不用来重算。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Rect {
    /// `const`：夹具的黄金基线要把它写进常量里。
    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    pub fn width(self) -> i32 {
        self.x1.saturating_sub(self.x0).max(0)
    }

    pub fn height(self) -> i32 {
        self.y1.saturating_sub(self.y0).max(0)
    }

    pub fn area(self) -> i64 {
        i64::from(self.width()) * i64::from(self.height())
    }

    /// 换个高度，**保持上沿不变**。给「同一个上沿、但要更高那条带子」
    /// 的场景用（见 `watch.rs` 里读会话名那条）。
    pub fn with_height(self, height: i32) -> Self {
        Rect::new(self.x0, self.y0, self.x1, self.y0 + height)
    }

    pub fn is_empty(self) -> bool {
        self.width() == 0 || self.height() == 0
    }

    /// 平移到以 `(origin_x, origin_y)` 为左上角的坐标系。
    ///
    /// 完全落在原点之外的矩形返回 `None`：OCR 框整体跑到气泡左边或上边，
    /// 拿它算裁剪区只会得到一个无意义的负偏移。
    pub fn translate_to(&self, origin_x: i32, origin_y: i32) -> Option<Rect> {
        let r = Rect::new(
            self.x0 - origin_x,
            self.y0 - origin_y,
            self.x1 - origin_x,
            self.y1 - origin_y,
        );
        if r.is_empty() || r.x1 <= 0 || r.y1 <= 0 {
            None
        } else {
            Some(r)
        }
    }
}

/// 一帧 RGB8 画面，通道紧凑排列（row-major，每像素 3 字节）。
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl Frame {
    /// 缓冲区长度与尺寸不符时返回 `None`——捕获层拿到的原始缓冲区
    /// 有可能因为窗口中途改尺寸而与声明尺寸对不上，这种情况必须显式失败，
    /// 不能让读像素时越界。
    pub fn from_rgb(width: u32, height: u32, rgb: Vec<u8>) -> Option<Self> {
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(3)?;
        if rgb.len() != expected {
            return None;
        }
        Some(Self { width, height, rgb })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.rgb
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 3]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y as usize) * (self.width as usize) + (x as usize)) * 3;
        Some([self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]])
    }

    /// 全黑检测。
    ///
    /// GPU 合成窗口用错误的捕获路径会稳定返回全黑帧，这是已知失败模式，
    /// 必须当成错误报出来，而不是把黑图当成"成功抓到了一帧"。
    /// 阈值取"任一通道 > 8"——容忍极轻的压缩/偏色噪声，但纯黑一定判黑。
    pub fn is_black(&self) -> bool {
        self.rgb
            .chunks_exact(3)
            .all(|p| p[0] <= 8 && p[1] <= 8 && p[2] <= 8)
    }

    /// 截取一块并平移到新原点。越界会裁剪；完全落在外面返回 `None`。
    pub fn crop(&self, r: Rect) -> Option<Self> {
        let r = Rect::new(
            r.x0.clamp(0, self.width as i32),
            r.y0.clamp(0, self.height as i32),
            r.x1.clamp(0, self.width as i32),
            r.y1.clamp(0, self.height as i32),
        );
        if r.is_empty() {
            return None;
        }
        let (w, h) = (r.width() as usize, r.height() as usize);
        let mut out = Vec::with_capacity(w * h * 3);
        for y in r.y0..r.y1 {
            let start = ((y as usize) * (self.width as usize) + r.x0 as usize) * 3;
            let end = start + w * 3;
            out.extend_from_slice(&self.rgb[start..end]);
        }
        Frame::from_rgb(w as u32, h as u32, out)
    }

    /// 最近邻放大 `n` 倍。
    ///
    /// ## 为什么必须是最近邻
    ///
    /// 这个方法存在的唯一目的是喂给 OCR 的**诊断与补救**。
    /// 如果用双线性/双三次，字形边缘会被插值抹平——那样即使 OCR
    /// 「变好了」，也说不清是字号问题还是插值把笔画糊掉了。
    /// 最近邻只复制像素，放大前后字形完全一致，结论才站得住。
    ///
    /// `n <= 1` 原样返回自己。
    pub fn upscaled_nearest(&self, n: u32) -> Option<Self> {
        if n <= 1 {
            return Some(self.clone());
        }
        let (w, h) = (self.width.checked_mul(n)?, self.height.checked_mul(n)?);
        let mut out = Vec::with_capacity((w as usize).checked_mul(h as usize)?.checked_mul(3)?);
        for y in 0..h {
            let src_row = ((y / n) as usize) * (self.width as usize) * 3;
            for x in 0..w {
                let sx = (x / n) as usize;
                let s = src_row + sx * 3;
                out.extend_from_slice(&self.rgb[s..s + 3]);
            }
        }
        Frame::from_rgb(w, h, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, v: [u8; 3]) -> Frame {
        Frame::from_rgb(w, h, [v[0], v[1], v[2]].repeat((w * h) as usize)).expect("solid")
    }

    #[test]
    fn buffer_len_must_match_dimensions() {
        assert!(Frame::from_rgb(2, 2, vec![0; 11]).is_none());
        assert!(Frame::from_rgb(2, 2, vec![0; 12]).is_some());
    }

    #[test]
    fn all_zero_is_black() {
        assert!(solid(4, 4, [0, 0, 0]).is_black());
    }

    #[test]
    fn any_bright_pixel_defeats_black() {
        let mut f = solid(4, 4, [0, 0, 0]);
        f.rgb[0] = 9;
        assert!(!f.is_black());
    }

    #[test]
    fn pixel_out_of_range_is_none() {
        let f = solid(2, 2, [10, 20, 30]);
        assert_eq!(f.pixel(1, 1), Some([10, 20, 30]));
        assert_eq!(f.pixel(2, 0), None);
    }

    #[test]
    fn crop_moves_origin_to_zero() {
        let f = solid(4, 4, [10, 20, 30]);
        let c = f.crop(Rect::new(1, 1, 3, 3)).expect("crop");
        assert_eq!((c.width(), c.height()), (2, 2));
        assert_eq!(c.pixel(0, 0), Some([10, 20, 30]));
    }

    #[test]
    fn crop_clamps_out_of_bounds_and_rejects_full_outside() {
        let f = solid(4, 4, [10, 20, 30]);
        assert_eq!(
            f.crop(Rect::new(0, 0, 99, 99))
                .map(|c| (c.width(), c.height())),
            Some((4, 4))
        );
        assert!(f.crop(Rect::new(10, 10, 20, 20)).is_none());
    }

    #[test]
    fn empty_rect_reports_empty() {
        let r = Rect::new(5, 5, 5, 9);
        assert!(r.is_empty());
        assert_eq!(r.width(), 0);
        assert_eq!(r.height(), 4);
    }

    #[test]
    fn translate_to_rejects_fully_negative() {
        let r = Rect::new(0, 0, 10, 10);
        assert!(r.translate_to(50, 50).is_none());
        assert_eq!(r.translate_to(4, 3), Some(Rect::new(-4, -3, 6, 7)));
    }
}
