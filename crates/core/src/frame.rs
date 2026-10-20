//! `Frame` 是一次窗口捕获得到的完整位图。
//!
//! 硬约束：常驻内存，**永不落盘**。唯一的例外是 `xtask probe` 显式写盘，
//! 那条路径要打印提示。〔按上下文复原，非原文〕

/// 矩形。坐标原点在左上角，右下开区间（`x1`/`y1` 是**不含**的）。
///
/// `Serialize` 让它能直接发给 UI——**几何判定仍然只在 Rust 侧**，
/// 前端拿到的坐标只用来渲染，不用来重算。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Rect {
    /// 左边界，含。
    pub x0: i32,
    /// 上边界，含。
    pub y0: i32,
    /// 右边界，**不含**。
    pub x1: i32,
    /// 下边界，**不含**。
    pub y1: i32,
}

impl Rect {
    /// `const`：夹具的黄金基线要把它写进常量里。
    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// 宽。**饱和减法**——负尺寸（`x1 < x0`）读成 0 而不是绕回一大片。
    pub fn width(self) -> i32 {
        self.x1.saturating_sub(self.x0).max(0)
    }

    /// 高。同 [`Rect::width`] 的饱和语义。
    pub fn height(self) -> i32 {
        self.y1.saturating_sub(self.y0).max(0)
    }

    /// 面积。走 `i64`——4K 全屏 `1920*1080*4` 级别的乘法在 `i32` 里是安全的，
    /// 但夹具里有更大的合成图，别让面积成为溢出的来源。
    pub fn area(self) -> i64 {
        i64::from(self.width()) * i64::from(self.height())
    }

    /// 换个高度，**保持上沿不变**。给「同一个上沿、但要更高那条带子」
    /// 的场景用（见 `watch.rs` 里读会话名那条）。
    pub fn with_height(self, height: i32) -> Self {
        Rect::new(self.x0, self.y0, self.x1, self.y0 + height)
    }

    /// 空矩形。宽或高为 0 就算空——**不检查 x0/x1 的先后**，
    /// 因为退化矩形在 OCR 里是常态（气泡被裁掉一半）。
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

    /// 宽（像素）。
    pub fn width(&self) -> u32 {
        self.width
    }

    /// 高（像素）。
    pub fn height(&self) -> u32 {
        self.height
    }

    /// 原始 RGB 缓冲，行主序、3 字节/像素。**只读借用**——
    /// 这帧永不落盘，所以这里没有 `&mut` 版本是刻意的。
    pub fn as_bytes(&self) -> &[u8] {
        &self.rgb
    }

    /// 取一个像素。越界返回 `None` 而不是 panic——
    /// OCR 框会给出退化坐标，调用方不该为此崩。
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

    /// 双线性放大 `n` 倍。
    ///
    /// ## 它和 [`upscaled_nearest`] 是两件事，不是同一个开关的两档
    ///
    /// 最近邻那份的职责是**诊断**：只复制像素，字形一字不差，
    /// 所以「放大 N 倍还是乱码」能推导出关于**字号**的结论。
    /// 它写明了这个用途，也写明了「换成插值就说不清了」——
    /// 那句话约束的是**诊断**，不是**生产**。
    ///
    /// 生产要的是**读对**。小字（10~12px）在最近邻放大下，笔画边缘是
    /// 锯齿块，识别器会把字形结构读崩；双线性把边缘抹平之后，
    /// 字反而更像它本来的样子（票 41 的 ③ 实测，2026-10-21）。
    ///
    /// ⚠ **它只许喂给 OCR，不许喂给任何「按像素做判定」的东西。**
    /// 插值会造出源图里不存在的颜色（两色之间），于是取色、对比度、
    /// 纯度、填满度这些阈值全部会漂。当前架构里这件事是安全的：
    /// perceive 归类用的是**未放大的原始裁剪件**，OCR 用的是放大副本，
    /// 两份数据不同源（`perceive::pipeline::read` 与 `watch::read_round`）。
    /// ⇒ 换插值之前，先确认那条「测量用原始像素」的性质还在。
    ///
    /// `n <= 1` 原样返回自己。
    pub fn upscaled_bilinear(&self, n: u32) -> Option<Self> {
        if n <= 1 {
            return Some(self.clone());
        }
        let (w, h) = (self.width.checked_mul(n)?, self.height.checked_mul(n)?);
        let mut out = Vec::with_capacity((w as usize).checked_mul(h as usize)?.checked_mul(3)?);
        let n_f = n as f32;
        for y in 0..h {
            // 源坐标按**像素中心**对齐：目标像素 (x,y) 的中心落在源图的
            // (x+0.5)/n - 0.5 处。少这半像素，整幅图会整体偏半像素，
            // 而字形边缘恰好在最要命的位置上。
            let sy = (y as f32 + 0.5) / n_f - 0.5;
            let (ya, yb, fy) = span(sy, self.height as i32);
            for x in 0..w {
                let sx = (x as f32 + 0.5) / n_f - 0.5;
                let (xa, xb, fx) = span(sx, self.width as i32);
                let w00 = (1.0 - fx) * (1.0 - fy);
                let w10 = fx * (1.0 - fy);
                let w01 = (1.0 - fx) * fy;
                let w11 = fx * fy;
                for c in 0..3 {
                    let p00 = f32::from(self.rgb[((ya * self.width as usize) + xa) * 3 + c]);
                    let p10 = f32::from(self.rgb[((ya * self.width as usize) + xb) * 3 + c]);
                    let p01 = f32::from(self.rgb[((yb * self.width as usize) + xa) * 3 + c]);
                    let p11 = f32::from(self.rgb[((yb * self.width as usize) + xb) * 3 + c]);
                    let v = p00 * w00 + p10 * w10 + p01 * w01 + p11 * w11;
                    out.push(v.round().clamp(0.0, 255.0) as u8);
                }
            }
        }
        Frame::from_rgb(w, h, out)
    }
}

/// 双线性采样在**一个轴**上的取值：返回 `(下标0, 下标1, 权重)`。
///
/// 越界时**延拓边缘像素**，不回绕、也不补黑——补黑会在画面边上造出一条
/// 黑边，而黑边会被 OCR 当成字读。
fn span(v: f32, limit: i32) -> (usize, usize, f32) {
    let i0 = v.floor();
    let f = v - i0;
    let a = (i0 as i32).clamp(0, limit - 1) as usize;
    let b = (i0 as i32 + 1).clamp(0, limit - 1) as usize;
    (a, b, f)
}

/// 放大时用哪种重采样。
///
/// **为什么不是一个整数开关**：最近邻与双线性在同一个倍数下**不可比**——
/// 一个只复制像素、一个造出源图里没有的颜色。混在一个开关里，
/// 「只换重采样方式」的对照实验就做不出来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resample {
    /// 只复制像素，字形一字不差（**诊断**口径）。
    Nearest,
    /// 边缘按邻域插值（**生产**口径）。
    Bilinear,
}

impl Frame {
    /// 按 `resample` 放大 `n` 倍。
    ///
    /// 这是**唯一的放大入口**：调用方不许自己去挑重采样方式，
    /// 因为「用哪种」是**引擎的属性**（见 `chatservant_capture::ocr`）。
    pub fn upscaled(&self, n: u32, resample: Resample) -> Option<Self> {
        match resample {
            Resample::Nearest => self.upscaled_nearest(n),
            Resample::Bilinear => self.upscaled_bilinear(n),
        }
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

    /// 双线性放大必须**真的插值**——不然它就是最近邻换了个名字，
    /// 而「换插值能让小字读对」这个结论会落空。
    #[test]
    fn bilinear_blends_the_middle_pixels_and_keeps_the_edges_exact() {
        let f = Frame::from_rgb(2, 1, vec![0, 0, 0, 255, 255, 255]).expect("frame");
        let up = f.upscaled_bilinear(2).expect("upscale");
        assert_eq!((up.width(), up.height()), (4, 2));
        let row: Vec<u8> = (0..4).map(|x| up.pixel(x, 0).expect("px")[0]).collect();
        // 像素中心对齐 ⇒ 0.25/0.75 处各留一个混合像素，两端仍是原值。
        assert_eq!(row, vec![0, 64, 191, 255]);
    }

    /// 纯色放大后必须**还是那个纯色**——插值不许造出源图里没有的颜色。
    /// 这条是「这份画面只许喂给 OCR」的底线：一旦有人拿它去做取色，
    /// 纯色变出第二种颜色，整条阈值链都会漂。
    #[test]
    fn bilinear_keeps_a_solid_color_exactly() {
        let f = solid(3, 2, [100, 120, 140]);
        let up = f.upscaled_bilinear(4).expect("upscale");
        assert_eq!((up.width(), up.height()), (12, 8));
        for y in 0..8 {
            for x in 0..12 {
                assert_eq!(up.pixel(x, y), Some([100, 120, 140]), "({x},{y}) 漂了");
            }
        }
    }

    #[test]
    fn both_upscalers_are_identity_below_two() {
        let f = solid(2, 2, [7, 8, 9]);
        assert_eq!(f.upscaled_nearest(1).expect("n"), f);
        assert_eq!(f.upscaled_bilinear(1).expect("b"), f);
    }
}
