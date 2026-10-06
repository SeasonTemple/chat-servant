//! `PaneAnchor` 的像素取证。
//!
//! ## 为什么先做这个
//!
//! 锚定规则里的每一个阈值（底色怎么取、列占比要多少、分隔线多"平"才算）
//! 都必须从**真实微信帧**的像素分布里推出来，而不是照着印象填。
//! 参考项目 `app/capture.py` 里的 `chat_area()` 给过一组实测值，但那是
//! 它的机器、它的微信版本；照搬等于把别人的数字当自己的事实。
//!
//! 所以这里只做一件事：**把一帧的像素结构如实打出来**，供设计规则使用。
//! 它不进产品链路，只给 `xtask probe layout` 和人看。

use std::collections::HashMap;

use chatservant_core::frame::Frame;

/// 颜色量化：把每个通道压到 4 bit（16 级）。这样抗轻微抗锯齿噪声，
/// 又不会把两个相近的深灰并成一个。
fn quantize(px: [u8; 3]) -> u32 {
    let r = (px[0] >> 4) as u32;
    let g = (px[1] >> 4) as u32;
    let b = (px[2] >> 4) as u32;
    (r << 8) | (g << 4) | b
}

fn unquantize(q: u32) -> [u8; 3] {
    [
        ((q >> 8) & 0xF) as u8 * 17,
        ((q >> 4) & 0xF) as u8 * 17,
        (q & 0xF) as u8 * 17,
    ]
}

/// 一帧的像素取证结果。
pub struct Diagnostics {
    pub width: u32,
    pub height: u32,
    /// 出现最多的 N 个量化颜色，按占比降序
    pub top_colors: Vec<(u32, usize, f64)>,
    /// 采样网格的步长（全量统计太慢，诊断够用）
    pub step: u32,
    /// 采样网格算出的列底色占比、峰值位置与转折位置
    pub columns: ColumnProfile,
    /// 行底色占比与"整行单色"的候选分隔线
    pub rows: RowProfile,
}

pub struct ColumnProfile {
    /// 每列等于面板底色的比例（0..1）
    pub dominance: Vec<f32>,
    /// 面板底色（量化后）
    pub panel_bg: u32,
    /// 第一根占比 > 阈值的列
    pub first_dense: Option<usize>,
    /// 最后一根占比 > 阈值的列
    pub last_dense: Option<usize>,
    /// 占比曲线上的最大跳变点——面板左边界通常在这里
    pub biggest_jump_at: Option<usize>,
    pub biggest_jump_delta: f32,
}

pub struct RowProfile {
    /// 每行等于面板底色的比例
    pub dominance: Vec<f32>,
    /// 候选横向分隔线：整行颜色高度一致且不等于面板底色
    pub separators: Vec<Separator>,
}

pub struct Separator {
    pub y: u32,
    /// 该行最常见色与次常见色的占比差——越小说明越"平"
    pub flatness: f64,
    pub color: u32,
}

const SAMPLE_STEP: u32 = 4;

/// 取证一帧。纯读，不改任何像素。
pub fn analyze(frame: &Frame) -> Diagnostics {
    let (w, h) = (frame.width(), frame.height());
    let step = SAMPLE_STEP.max(1);

    // --- 直方图：面板底色是全图最常见的颜色 ---
    let mut hist: HashMap<u32, usize> = HashMap::new();
    let mut y = 0u32;
    while y < h {
        let mut x = 0u32;
        while x < w {
            if let Some(p) = frame.pixel(x, y) {
                *hist.entry(quantize(p)).or_insert(0) += 1;
            }
            x += step;
        }
        y += step;
    }
    let total: usize = hist.values().sum();
    let mut top_colors: Vec<(u32, usize, f64)> = hist
        .iter()
        .map(|(&q, &n)| (q, n, n as f64 / total as f64))
        .collect();
    top_colors.sort_by(|a, b| b.1.cmp(&a.1));
    top_colors.truncate(8);

    // --- 面板底色：右半边最常见的颜色 ---
    // 右边是聊天区，联系人列表在左边，两者底色不同。
    let panel_bg = dominant_in_band(frame, w / 2, w, 0, h, step).unwrap_or(0);

    // --- 列占比：聊天区底色在每列的纵向覆盖率 ---
    let (y_lo, y_hi) = (h / 4, h * 3 / 4);
    let cols = (w as usize).div_ceil(step as usize);
    let mut dominance = vec![0f32; cols];
    let mut yy = y_lo;
    while yy < y_hi {
        let mut c = 0usize;
        while c < cols {
            let x = (c as u32) * step;
            if let Some(p) = frame.pixel(x, yy)
                && quantize(p) == panel_bg
            {
                dominance[c] += 1.0;
            }
            c += 1;
        }
        yy += step;
    }
    let rows_sampled = (y_hi - y_lo).div_ceil(step).max(1) as f32;
    for d in dominance.iter_mut() {
        *d /= rows_sampled;
    }

    let (first_dense, last_dense) = {
        let dense: Vec<usize> = (0..cols).filter(|&c| dominance[c] > 0.30).collect();
        (dense.first().copied(), dense.last().copied())
    };
    let mut biggest_jump_at = None;
    let mut biggest_jump_delta = 0f32;
    for c in 1..cols {
        let d = (dominance[c] - dominance[c - 1]).abs();
        if d > biggest_jump_delta {
            biggest_jump_delta = d;
            biggest_jump_at = Some(c);
        }
    }

    // --- 行：底色占比 + 候选分隔线 ---
    // 只在面板的横向范围内统计，否则联系人列表的分割线会混进来。
    let (x_lo, x_hi) = match (first_dense, last_dense) {
        (Some(a), Some(b)) if b > a => (a as u32 * step, (b as u32 + 1) * step),
        _ => (w / 2, w),
    };
    let rows = (h as usize).div_ceil(step as usize);
    let mut dominance_r = vec![0f32; rows];
    let mut separators = Vec::new();
    let mut r = 0usize;
    while r < rows {
        let y = (r as u32) * step;
        let mut row_hist: HashMap<u32, usize> = HashMap::new();
        let mut x = x_lo;
        let mut sampled = 0usize;
        while x < x_hi {
            if let Some(p) = frame.pixel(x, y) {
                *row_hist.entry(quantize(p)).or_insert(0) += 1;
                sampled += 1;
                if quantize(p) == panel_bg {
                    dominance_r[r] += 1.0;
                }
            }
            x += step;
        }
        if sampled > 0 {
            dominance_r[r] /= sampled as f32;
            // flatness = 众数占比 - 次众数占比。越小越"平"。
            let mut counts: Vec<usize> = row_hist.values().copied().collect();
            counts.sort_unstable_by(|a, b| b.cmp(a));
            let top = *counts.first().unwrap_or(&0) as f64 / sampled as f64;
            let second = *counts.get(1).unwrap_or(&0) as f64 / sampled as f64;
            let flatness = top - second;
            let color = row_hist
                .iter()
                .max_by_key(|&(_, &n)| n)
                .map(|(&q, _)| q)
                .unwrap_or(0);
            if flatness < 0.15 && color != panel_bg {
                separators.push(Separator { y, flatness, color });
            }
        }
        r += 1;
    }

    Diagnostics {
        width: w,
        height: h,
        top_colors,
        step,
        columns: ColumnProfile {
            dominance,
            panel_bg,
            first_dense,
            last_dense,
            biggest_jump_at,
            biggest_jump_delta,
        },
        rows: RowProfile {
            dominance: dominance_r,
            separators,
        },
    }
}

/// 某个矩形范围内出现最多的量化颜色。
fn dominant_in_band(frame: &Frame, x0: u32, x1: u32, y0: u32, y1: u32, step: u32) -> Option<u32> {
    let mut hist: HashMap<u32, usize> = HashMap::new();
    let mut y = y0;
    while y < y1 {
        let mut x = x0;
        while x < x1 {
            if let Some(p) = frame.pixel(x, y) {
                *hist.entry(quantize(p)).or_insert(0) += 1;
            }
            x += step;
        }
        y += step;
    }
    hist.into_iter().max_by_key(|(_, n)| *n).map(|(q, _)| q)
}

/// 把取证结果渲染成给人看的报告。
pub fn render(d: &Diagnostics) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "帧 = {}x{}   采样步长 = {}\n\n",
        d.width, d.height, d.step
    ));

    s.push_str("== 全图主色（前 8）==\n");
    for &(q, n, frac) in d.top_colors.iter().take(8) {
        let c = unquantize(q);
        s.push_str(&format!(
            "  rgb({:>3},{:>3},{:>3})   {:>8} px   {:>5.1}%\n",
            c[0],
            c[1],
            c[2],
            n,
            frac * 100.0
        ));
    }

    let c = &d.columns;
    let pc = unquantize(c.panel_bg);
    s.push_str(&format!(
        "\n== 面板底色（右侧最常见）==\n  rgb({:>3},{:>3},{:>3})  (量化 {})\n",
        pc[0], pc[1], pc[2], c.panel_bg
    ));
    s.push_str(&format!(
        "  首根占比>30% 的列 = {:?}    最后一根 = {:?}\n",
        c.first_dense, c.last_dense
    ));
    s.push_str(&format!(
        "  最大跳变 = 列 {:?}  Δ={:.3}   （面板左边界通常在这里）\n",
        c.biggest_jump_at, c.biggest_jump_delta
    ));

    s.push_str("\n== 列底色占比（每 16 列取一个采样打印）==\n  ");
    let show = 16usize;
    for i in 0..d.columns.dominance.len().div_ceil(show) {
        let x0 = i * show;
        let x1 = ((i + 1) * show).min(d.columns.dominance.len());
        let peak = d.columns.dominance[x0..x1]
            .iter()
            .cloned()
            .fold(0f32, f32::max);
        let bar = "#".repeat((peak * 40.0) as usize);
        s.push_str(&format!(
            "  {:>5}|{:<42}{:.2}\n",
            x0 * d.step as usize,
            bar,
            peak
        ));
    }

    s.push_str("\n== 行底色占比（每 16 行取一个采样打印）==\n  ");
    let rstep = 16usize;
    for i in 0..d.rows.dominance.len().div_ceil(rstep) {
        let a = i * rstep;
        let b = ((i + 1) * rstep).min(d.rows.dominance.len());
        let mean = d.rows.dominance[a..b].iter().sum::<f32>() / (b - a) as f32;
        let bar = "#".repeat((mean * 40.0) as usize);
        s.push_str(&format!(
            "  y={:>5}|{:<42}{:.2}\n",
            a * d.step as usize,
            bar,
            mean
        ));
    }

    // 低占比的连续行 = 一整条"非面板底色"横带。头部栏、输入框都在这里。
    // 早先用「整行单色」判据是错的：这两条带里都有文字，顶众数与次众数
    // 的差不会小，于是一条都检不出来。看行底色占比才看得见。
    let mut bands: Vec<(u32, u32)> = Vec::new();
    let mut cur: Option<(u32, u32)> = None;
    for (r, &v) in d.rows.dominance.iter().enumerate() {
        if v < 0.35 {
            match &mut cur {
                Some((_, end)) => *end = r as u32 * d.step,
                None => cur = Some((r as u32 * d.step, r as u32 * d.step)),
            }
        } else if let Some(b) = cur.take() {
            bands.push(b);
        }
    }
    if let Some(b) = cur {
        bands.push(b);
    }
    s.push_str(&format!(
        "\n== 非面板底色横带（行占比 < 0.35 的连续段）==\n  共 {} 条：\n",
        bands.len()
    ));
    for (y0, y1) in bands.iter().take(16) {
        s.push_str(&format!("  y {:>4} .. {:>4}   高 {:>3}\n", y0, y1, y1 - y0));
    }

    s.push_str("\n== 候选横向分隔线（整行单色且非面板底色）==\n");
    if d.rows.separators.is_empty() {
        s.push_str("  （没有）\n");
    } else {
        s.push_str(&format!("  共 {} 条：\n", d.rows.separators.len()));
        for sep in d.rows.separators.iter().take(24) {
            let c = unquantize(sep.color);
            s.push_str(&format!(
                "  y={:>4}  平坦度={:.3}  rgb({:>3},{:>3},{:>3})\n",
                sep.y, sep.flatness, c[0], c[1], c[2]
            ));
        }
    }
    s
}
