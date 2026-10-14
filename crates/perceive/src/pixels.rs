//! 像素原语。归类规则全靠这些，全是纯读，不改任何像素。

use std::collections::HashMap;

use chatservant_core::frame::{Frame, Rect};

/// 一个像素的 RGB，三个通道各 0~255。
pub type Rgb = [u8; 3];

/// 两个颜色的容差。实测面板底色在帧内**完全一致**（`rgb(30,30,31)` 到处一样），
/// 只有文字抗锯齿边缘会差一点点，所以 12 有很大余量。
pub const COLOR_TOL: i32 = 12;

/// 两个颜色是否近到可以当同一个（**逐通道**比，容差 [`COLOR_TOL`]）。
///
/// 注意这是逐通道而非欧氏距离：面板底色的判据要的是「同一个颜色到处都一模一样」，
/// 抗锯齿边缘只会让某一个通道差一点点，逐通道比才不会把它们误算成不同底色。
pub fn near(a: Rgb, b: Rgb) -> bool {
    let d = |x: u8, y: u8| (x as i32 - y as i32).abs();
    d(a[0], b[0]) <= COLOR_TOL && d(a[1], b[1]) <= COLOR_TOL && d(a[2], b[2]) <= COLOR_TOL
}

/// 判定「这块底色**就是**面板底色」的容差。**必须比 [`COLOR_TOL`] 紧得多。**
///
/// 2026-10-14 真机**浅色主题**实测（`target/probe/frame-1791974144.png`）：
/// 对方气泡 `rgb(238,238,240)` 与面板 `rgb(250,250,250)` 逐通道只差
/// **12, 12, 10**，而 [`COLOR_TOL`] 恰好就是 12 —— `12 <= 12` 成立，
/// 对方气泡被判成面板底色，那句话**不会变成消息**，系统等于认为对方没说话。
///
/// 为什么不给它那个宽松度：[`COLOR_TOL`] 之所以是 12，是留给**抗锯齿边缘像素**的
/// （面板底色在帧内精确均匀，只有字边缘会差一点点）。而判「在不在气泡里」时
/// 比的是 `probe.bg`——那是文字框外一圈采样出来的**成片底色**，不吃这个宽松度。
/// 仓里 `fixture.rs` 的 `IS_BG_TOL` 已经是 2，它注释里记的正是同一类成因。
pub const PANEL_BG_TOL: i32 = 2;

/// 「这两块底色是同一个面板底色吗」。比 [`near`] 紧，**只**用于「在不在气泡里」。
///
/// 收紧之后两个主题都成立（2026-10-14 实测）：
///
/// | 主题 | 对方气泡 | 面板 | 逐通道差 | 结果 |
/// |---|---|---|---|---|
/// | 深色 | `rgb(47,47,48)` | `rgb(30,30,31)` | 17, 17, 17 | 在气泡里 ✓ |
/// | 浅色 | `rgb(238,238,240)` | `rgb(250,250,250)` | 12, 12, 10 | 在气泡里 ✓ |
///
/// 面板上的灰字/时间戳/会话名底色**就是**面板底色（差 0），仍然判「在面板上」。
pub fn is_panel_bg(a: Rgb, b: Rgb) -> bool {
    let d = |x: u8, y: u8| (x as i32 - y as i32).abs();
    d(a[0], b[0]) <= PANEL_BG_TOL && d(a[1], b[1]) <= PANEL_BG_TOL && d(a[2], b[2]) <= PANEL_BG_TOL
}

/// 感知亮度。用于「文字比底色亮还是暗」。
pub fn luma(c: Rgb) -> f32 {
    0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32
}

/// 感知距离：亮度项 + 色度项。
///
/// 单纯的欧氏距离会把「深灰 vs 近黑」判得过近，而那恰恰是我们要区分的
/// （气泡正文墨色 `rgb(22,22,22)` vs 面板灰字 `rgb(123,123,128)`）。
/// 所以把亮度分量剥掉，剩下的色度单独算。
pub fn dist(a: Rgb, b: Rgb) -> f32 {
    let dl = luma(a) - luma(b);
    let c = [
        a[0] as f32 - b[0] as f32,
        a[1] as f32 - b[1] as f32,
        a[2] as f32 - b[2] as f32,
    ];
    let mean = (c[0] + c[1] + c[2]) / 3.0;
    let sat = (c[0] - mean).powi(2) + (c[1] - mean).powi(2) + (c[2] - mean).powi(2);
    (dl * dl + sat).sqrt()
}

/// 彩度 = 最大通道减最小通道。灰的颜色彩度为 0。
///
/// 实测：我方微信气泡底色 `rgb(114,205,144)` 彩度 **91**；
/// 面板底色 `rgb(30,30,31)` 彩度 **1**。这个量级差就是判据的立足点。
pub fn chroma(c: Rgb) -> i32 {
    c[0].max(c[1]).max(c[2]) as i32 - c[0].min(c[1]).min(c[2]) as i32
}

const STEP: u32 = 4;

/// 采样网格全量化，直接用精确色值，避免把相近色并成一个。
fn exact_hist(frame: &Frame, r: Rect) -> HashMap<Rgb, usize> {
    let mut hist = HashMap::new();
    let x0 = r.x0.max(0) as u32;
    let y0 = r.y0.max(0) as u32;
    let x1 = (r.x1.max(0) as u32).min(frame.width());
    let y1 = (r.y1.max(0) as u32).min(frame.height());
    let mut y = y0;
    while y < y1 {
        let mut x = x0;
        while x < x1 {
            if let Some(p) = frame.pixel(x, y) {
                *hist.entry(p).or_insert(0) += 1;
            }
            x += STEP;
        }
        y += STEP;
    }
    hist
}

/// 区域里出现最多的颜色。
pub fn dominant_color(frame: &Frame, r: Rect) -> Option<Rgb> {
    exact_hist(frame, r)
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(c, _)| c)
}

/// 区域里众数颜色的占比。用来判断「这片区域是不是平的」。
///
/// 实测依据：气泡内部底色均匀，占比高；照片/头像区域颜色杂，占比低。
pub fn dominant_ratio(frame: &Frame, r: Rect) -> f32 {
    let hist = exact_hist(frame, r);
    let total: usize = hist.values().sum();
    if total == 0 {
        return 0.0;
    }
    hist.values().copied().max().unwrap_or(0) as f32 / total as f32
}

/// 区域内所有像素的平均色。
pub fn mean_color(frame: &Frame, r: Rect) -> Option<Rgb> {
    let mut acc = [0f64; 3];
    let mut n = 0f64;
    let x1 = (r.x1.max(0) as u32).min(frame.width());
    let y1 = (r.y1.max(0) as u32).min(frame.height());
    for y in (r.y0.max(0) as u32)..y1 {
        for x in (r.x0.max(0) as u32)..x1 {
            if let Some(p) = frame.pixel(x, y) {
                acc[0] += p[0] as f64;
                acc[1] += p[1] as f64;
                acc[2] += p[2] as f64;
                n += 1.0;
            }
        }
    }
    if n == 0.0 {
        return None;
    }
    Some([(acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8])
}

/// 一行文字**周围一圈**的底色探测结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BubbleHit {
    /// 文字框外圈的主色 = 气泡底色（框内被字形污染，不能用）。
    pub bg: Rgb,
    /// 外圈众数占比。很低说明这片区域是「图」不是「底色」。
    pub purity: f32,
    /// 框内与 bg 距离最大的像素 = 文字墨色。
    pub ink: Rgb,
    /// ink 与 bg 的感知距离。
    pub contrast: f32,
}

/// 探测一行文字的气泡底色与墨色。
///
/// **为什么测外圈而不是框内**：框内被字形覆盖，众色会被拉偏。
/// 实测把外圈和框内都量过，外圈给出的就是气泡真实底色。
pub fn probe_bubble_bg(frame: &Frame, text_rect: Rect, pad: i32) -> Option<BubbleHit> {
    let outer = Rect::new(
        text_rect.x0 - pad,
        text_rect.y0 - pad,
        text_rect.x1 + pad,
        text_rect.y1 + pad,
    );
    let ring = Ring::new(frame, text_rect, pad);
    let hist = ring.hist();
    let (bg, hits) = hist.into_iter().max_by_key(|(_, n)| *n)?;
    let purity = hits as f32 / ring.sampled().max(1) as f32;

    let mut ink = bg;
    let mut contrast = 0f32;
    let x1 = (text_rect.x1.max(0) as u32).min(frame.width());
    let y1 = (text_rect.y1.max(0) as u32).min(frame.height());
    for y in (text_rect.y0.max(0) as u32)..y1 {
        for x in (text_rect.x0.max(0) as u32)..x1 {
            if let Some(p) = frame.pixel(x, y) {
                let d = dist(p, bg);
                if d > contrast {
                    contrast = d;
                    ink = p;
                }
            }
        }
    }

    let _ = outer;
    Some(BubbleHit {
        bg,
        purity,
        ink,
        contrast,
    })
}

/// 文字框外围的一圈像素（不含框内）。
struct Ring<'a> {
    frame: &'a Frame,
    inner: Rect,
    pad: i32,
}

impl<'a> Ring<'a> {
    fn new(frame: &'a Frame, inner: Rect, pad: i32) -> Self {
        Self { frame, inner, pad }
    }

    fn bounds(&self) -> Rect {
        Rect::new(
            self.inner.x0 - self.pad,
            self.inner.y0 - self.pad,
            self.inner.x1 + self.pad,
            self.inner.y1 + self.pad,
        )
    }

    fn hist(&self) -> HashMap<Rgb, usize> {
        let b = self.bounds();
        let x1 = (b.x1.max(0) as u32).min(self.frame.width());
        let y1 = (b.y1.max(0) as u32).min(self.frame.height());
        let mut hist = HashMap::new();
        let mut y = b.y0.max(0) as u32;
        while y < y1 {
            let mut x = b.x0.max(0) as u32;
            while x < x1 {
                let inside = x >= self.inner.x0 as u32
                    && x < self.inner.x1 as u32
                    && y >= self.inner.y0 as u32
                    && y < self.inner.y1 as u32;
                if !inside && let Some(p) = self.frame.pixel(x, y) {
                    *hist.entry(p).or_insert(0) += 1;
                }
                x += 1;
            }
            y += 1;
        }
        hist
    }

    /// 外圈采样点总数。用于算 purity = 众数命中数 / 总采样点。
    fn sampled(&self) -> usize {
        let b = self.bounds();
        let w = (b.x1 - b.x0).max(0) as usize;
        let h = (b.y1 - b.y0).max(0) as usize;
        let inner = (self.inner.width() as usize) * (self.inner.height() as usize);
        w.saturating_mul(h).saturating_sub(inner).max(1)
    }
}

/// 区域里「墨」的比例——与 `bg` 差别很大的像素占比。
///
/// 实测依据：气泡正文很高，照片区域也高但纯度低；灰色时间戳很低。
pub fn ink_ratio(frame: &Frame, r: Rect, bg: Rgb) -> f32 {
    let x1 = (r.x1.max(0) as u32).min(frame.width());
    let y1 = (r.y1.max(0) as u32).min(frame.height());
    let mut inked = 0usize;
    let mut total = 0usize;
    for y in (r.y0.max(0) as u32)..y1 {
        for x in (r.x0.max(0) as u32)..x1 {
            if let Some(p) = frame.pixel(x, y) {
                total += 1;
                if dist(p, bg) > 40.0 {
                    inked += 1;
                }
            }
        }
    }
    if total == 0 {
        return 0.0;
    }
    inked as f32 / total as f32
}
