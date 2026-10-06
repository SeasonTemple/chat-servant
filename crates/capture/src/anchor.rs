//! `PaneAnchor`：从一帧像素里推导消息列表区（`ChatPane`）的位置。
//!
//! ## 硬性要求
//!
//! - **由像素内容锚定，不写死坐标。** 同一份代码要吃深色/浅色主题、
//!   125%/150% DPI、任意窗口尺寸。
//! - **推导不出来就返回 `None`**，上层降级重试。**不猜一个默认值**——
//!   猜出来的框会让 OCR 在错误区域上工作，比报失败难查得多。
//!
//! ## 阈值从哪来
//!
//! 全部来自 `cargo xtask layout` 对**真实微信帧**的像素取证
//! （深色主题，1262×958），不是照搬参考项目的数字——那是别人机器、
//! 别人微信版本的实测值。取证结论：
//!
//! ```text
//! 面板底色 rgb(17,17,17) 占 67.4%
//! 左侧列表 rgb(34,34,51) 占 13.3%
//! 列占比：x<304 全是 0.00，x>=304 跳到 1.00（Δ=1.000）
//! 行占比：y 64~896 都在 0.88~0.99
//! 非底色细带：y 0..32（窗口标题栏）、y 684（输入框上边框）、944、956
//! ```
//!
//! 两条关键发现，都是"先看数据"省下来的：
//!
//! 1. **输入框不是靠底色区分的**——它内部与面板同色，只有 1 px 边框。
//!    所以判据是「低占比的**细**横带」，不是「整条不同色的色块」。
//! 2. **头部栏同样与面板同色**，靠文字区分，而文字属于 OCR 的活。
//!    所以 `PaneAnchor` 只把头部锚到**标题栏下沿**，精确的头部高度
//!    留给 ticket 03 用文字行去定，不在这里硬编一个比例。

use chatservant_core::frame::{Frame, Rect};

/// 列的底色占比超过这个值就算"面板列"。
/// 实测数据里面板内是 0.99 附近、面板外是 0.00，中间没有灰区，
/// 所以 0.30 有很大余量。
const COLUMN_DENSE: f32 = 0.30;

/// 行的底色占比低于这个值算"非面板行"。
const BAND_LOW: f32 = 0.35;

/// 非底色横带高于这个像素高度就不算"细带"（是色块，不是边框线）。
/// 实测边框线宽度为 0（抽样步长 4 命中 1 行），色块则是几十上百像素。
const BAND_MAX_THICKNESS: u32 = 8;

/// 输入框上边框必须落在面板高度的这个比例以下。
/// 防止把聊天区里偶尔出现的横线误判成输入框。
const INPUT_TOP_MIN_FRAC: f32 = 0.45;

/// 标题栏只能出现在顶部这个比例以内。
const TITLE_BAR_MAX_FRAC: f32 = 0.12;

/// 推导出来的锚点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneAnchor {
    /// 消息列表区。坐标以帧左上角为原点。
    pub chat: Rect,
    /// 面板底色（RGB）。
    pub panel_bg: [u8; 3],
    /// 面板左右边界（含）。用于裁剪与调试视图。
    pub panel: Rect,
    /// 窗口标题栏下沿。往下的部分才可能是聊天内容。
    pub header_top: i32,
    /// 输入框上边框。没找到就是 0，表示"没定住"。
    pub input_top: i32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AnchorError {
    #[error("帧太小（{0}x{1}），推不出消息区")]
    TooSmall(u32, u32),

    #[error("找不到面板底色：右半边没有明显的主色")]
    NoPanelBg,

    #[error("按底色找不到左右边界（首 {:?} 末 {:?}）", .0, .1)]
    NoPanelEdge(Option<usize>, Option<usize>),

    #[error("面板太窄（{0} px），不成立")]
    PanelTooNarrow(i32),

    #[error("找不到标题栏下沿")]
    NoHeaderTop,

    #[error("找不到输入框上边框")]
    NoInputTop,
}

/// 推导一帧的消息区。
pub fn derive(frame: &Frame) -> Result<PaneAnchor, AnchorError> {
    let (w, h) = (frame.width(), frame.height());
    if w < 300 || h < 300 {
        return Err(AnchorError::TooSmall(w, h));
    }

    // --- 1) 面板底色：右半边最常见的颜色 ---
    let panel_bg = dominant_in_band(frame, w / 2, w, 0, h, 4).ok_or(AnchorError::NoPanelBg)?;

    // --- 2) 左右边界：列底色占比的起止 ---
    // 只看中段（25%~75%），避开顶部标题栏和底部输入框——它们底色不同，
    // 参与统计会把边界糊掉。
    let (y_lo, y_hi) = (h / 4, h * 3 / 4);
    let step = 4u32;
    let col_w = ((w as usize).div_ceil(step as usize)).max(1);
    let mut col_hits = vec![0u32; col_w];
    let mut y = y_lo;
    while y < y_hi {
        let mut c = 0usize;
        while c < col_w {
            let x = (c as u32) * step;
            if x < w
                && let Some(p) = frame.pixel(x, y)
                && near(p, panel_bg)
            {
                col_hits[c] += 1;
            }
            c += 1;
        }
        y += step;
    }
    let rows = (((y_hi - y_lo) as usize).div_ceil(step as usize)).max(1) as f32;
    let density: Vec<f32> = col_hits.iter().map(|&n| n as f32 / rows).collect();

    let first = density.iter().position(|&d| d > COLUMN_DENSE);
    let last = density.iter().rposition(|&d| d > COLUMN_DENSE);
    let (first, last) = match (first, last) {
        (Some(a), Some(b)) => (a, b),
        other => return Err(AnchorError::NoPanelEdge(other.0, other.1)),
    };
    let x0 = (first as u32) * step;
    // +1 再乘 step 会向上取整越过帧宽（1262 的帧算出 1264），必须夹住。
    let x1 = (((last as u32) + 1) * step).min(w);
    if ((x1 - x0) as i32) < 100 {
        return Err(AnchorError::PanelTooNarrow((x1 - x0) as i32));
    }

    // --- 3) 标题栏下沿：顶部一小条非面板底色的横带 ---
    // 取 band 的**下沿**而不是上沿：band 是「非面板底色」的那一段，
    // 聊天内容在它**下面**才开始。标题栏 band 实测 y 0..32，
    // 取上沿会得到 y=0，把标题栏也划进消息区。
    let bands = low_dominance_bands(frame, panel_bg, x0, x1);
    let header_top = bands
        .iter()
        .find(|e| e.0 as f32 <= h as f32 * TITLE_BAR_MAX_FRAC)
        .map(|e| e.1 as i32)
        .ok_or(AnchorError::NoHeaderTop)?;

    // --- 4) 输入框上边框：中下部第一条细带 ---
    // 细带：高 ≤ BAND_MAX_THICKNESS。色块（几十像素）会被排除。
    let input_top = bands
        .iter()
        .find(|e| {
            let thickness = e.1 - e.0;
            e.0 as f32 >= h as f32 * INPUT_TOP_MIN_FRAC && thickness <= BAND_MAX_THICKNESS
        })
        .map(|e| e.0 as i32)
        .ok_or(AnchorError::NoInputTop)?;
    if input_top <= header_top {
        return Err(AnchorError::NoInputTop);
    }

    Ok(PaneAnchor {
        chat: Rect::new(x0 as i32, header_top, x1 as i32, input_top),
        panel_bg: panel_bg.0,
        panel: Rect::new(x0 as i32, 0, x1 as i32, h as i32),
        header_top,
        input_top,
    })
}

/// 面板内「底色占比低于 BAND_LOW」的连续行段。
///
/// 只在 `x0..x1` 这段横向范围里统计，否则左侧列表的分隔线会混进来。
///
/// **行方向逐行扫，不抽样。** 抽着扫会漏掉输入框的边框——它是 1 px 的，
/// 抽样格点不一定落在它上面。真实帧上 iy=684 恰好能被 4 整除所以"能用"，
/// 换个窗口尺寸（iy=426）就直接漏检。**在原尺寸下碰巧成立是最危险的一种通过。**
/// 横向仍然按 step 抽样以控制总采样量。
fn low_dominance_bands(frame: &Frame, bg: Rgb, x0: u32, x1: u32) -> Vec<(u32, u32)> {
    let h = frame.height();
    let col_step = 4u32;
    let cols = (((x1 - x0) as usize).div_ceil(col_step as usize)).max(1);

    let mut bands: Vec<(u32, u32)> = Vec::new();
    let mut open: Option<(u32, u32)> = None;

    for y in 0..h {
        let mut hits = 0u32;
        let mut c = 0usize;
        while c < cols {
            let x = x0 + (c as u32) * col_step;
            if x < x1
                && let Some(p) = frame.pixel(x, y)
                && near(p, bg)
            {
                hits += 1;
            }
            c += 1;
        }
        let frac = hits as f32 / cols as f32;
        if frac < BAND_LOW {
            match &mut open {
                Some((_, end)) => *end = y,
                None => open = Some((y, y)),
            }
        } else if let Some(b) = open.take() {
            bands.push(b);
        }
    }
    if let Some(b) = open {
        bands.push(b);
    }
    bands
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANEL_BG: [u8; 3] = [30, 30, 31];
    const SIDEBAR: [u8; 3] = [34, 34, 51];
    const CHROME: [u8; 3] = [51, 51, 51];

    /// 造一个「微信式」帧：整幅面板底色，左边一条侧边栏，
    /// 顶部一条标题栏，输入框只画一圈 1 px 边框（**内部同色**——
    /// 真实帧就是这样，取证时发现的）。
    fn synthetic(w: u32, h: u32, sidebar_frac: f32, title_frac: f32, input_frac: f32) -> Frame {
        let sx = (w as f32 * sidebar_frac) as u32;
        let ty = (h as f32 * title_frac) as u32;
        let iy = (h as f32 * input_frac) as u32;
        let mut buf = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                let mut p = PANEL_BG;
                if x < sx {
                    p = SIDEBAR;
                }
                if y < ty || y == iy {
                    p = CHROME;
                }
                buf.extend_from_slice(&p);
            }
        }
        Frame::from_rgb(w, h, buf).expect("synthetic")
    }

    /// 窗口尺寸可变。锚点必须**跟着尺寸走**，而不是停在某组固定坐标上。
    /// 这条是本模块最重要的不变量：任何写死的坐标都会在这里露馅。
    #[test]
    fn anchor_tracks_window_size() {
        // 侧边栏占比、标题栏高度、输入框位置都按比例给，
        // 模拟「同一个布局在不同尺寸窗口下的表现」。
        let cases = [(800u32, 600u32), (1262, 958), (1920, 1200), (2560, 1440)];
        for (w, h) in cases {
            let f = synthetic(w, h, 0.24, 0.045, 0.71);
            let a = derive(&f).unwrap_or_else(|e| panic!("{w}x{h} 推导失败：{e}"));

            let want_sx = (w as f32 * 0.24) as i32;
            let want_ty = (h as f32 * 0.045) as i32;
            let want_iy = (h as f32 * 0.71) as i32;

            // 采样步长是 4，边界允许几个像素的误差。
            assert!(
                (a.panel.x0 - want_sx).abs() <= 8,
                "{w}x{h}: 面板左边 {} 偏离预期 {want_sx}",
                a.panel.x0
            );
            assert!(
                (a.header_top - want_ty).abs() <= 8,
                "{w}x{h}: 标题栏下沿 {} 偏离预期 {want_ty}",
                a.header_top
            );
            assert!(
                (a.input_top - want_iy).abs() <= 8,
                "{w}x{h}: 输入框上沿 {} 偏离预期 {want_iy}",
                a.input_top
            );
            assert_eq!(a.panel.x1, w as i32, "{w}x{h}: 右边界应贴到窗口右缘");
        }
    }

    /// 侧边栏宽度是用户可拖的，同一尺寸下换占比也要跟上。
    #[test]
    fn anchor_follows_sidebar_width() {
        for frac in [0.15f32, 0.24, 0.35] {
            let f = synthetic(1200, 900, frac, 0.045, 0.71);
            let a = derive(&f).expect("推导");
            let want = (1200.0 * frac) as i32;
            assert!(
                (a.panel.x0 - want).abs() <= 8,
                "侧边栏占比 {frac}: 得到 {} 预期 {want}",
                a.panel.x0
            );
        }
    }

    /// 输入框拉得更高/更低都要跟上。
    #[test]
    fn anchor_follows_input_height() {
        for frac in [0.55f32, 0.71, 0.85] {
            let f = synthetic(1200, 900, 0.24, 0.045, frac);
            let a = derive(&f).expect("推导");
            let want = (900.0 * frac) as i32;
            assert!(
                (a.input_top - want).abs() <= 8,
                "输入框占比 {frac}: 得到 {} 预期 {want}",
                a.input_top
            );
        }
    }

    /// 纯面板底色的帧推不出标题栏与输入框——必须报失败，不许猜。
    /// 猜出来的框会让 OCR 在错误区域上工作，比报失败难查得多。
    #[test]
    fn blank_frame_fails_loudly() {
        let px: Vec<u8> = [30u8, 30, 31].repeat(1000 * 800);
        let f = Frame::from_rgb(1000, 800, px).unwrap();
        let err = derive(&f).expect_err("纯色帧应推不出");
        assert_eq!(err, AnchorError::NoHeaderTop);
    }

    #[test]
    fn tiny_frame_rejected() {
        let f = synthetic(120, 100, 0.24, 0.045, 0.71);
        assert_eq!(derive(&f), Err(AnchorError::TooSmall(120, 100)));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Rgb([u8; 3]);

/// 底色比较留 4 级余量（每级 17）。实测面板底色内部完全一致，
/// 但抗锯齿文字边缘会差一点点，太紧会把文字像素算成"非底色"。
fn near(a: [u8; 3], b: Rgb) -> bool {
    let d = |x: u8, y: u8| (x as i32 - y as i32).abs();
    d(a[0], b.0[0]) <= 4 && d(a[1], b.0[1]) <= 4 && d(a[2], b.0[2]) <= 4
}

fn dominant_in_band(frame: &Frame, x0: u32, x1: u32, y0: u32, y1: u32, step: u32) -> Option<Rgb> {
    let mut hist: std::collections::HashMap<Rgb, usize> = std::collections::HashMap::new();
    let mut y = y0;
    while y < y1 {
        let mut x = x0;
        while x < x1 {
            if let Some(p) = frame.pixel(x, y) {
                *hist.entry(Rgb(p)).or_insert(0) += 1;
            }
            x += step;
        }
        y += step;
    }
    hist.into_iter().max_by_key(|(_, n)| *n).map(|(c, _)| c)
}
