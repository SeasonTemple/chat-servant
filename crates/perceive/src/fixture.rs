//! 夹具脱敏。
//!
//! ## 为什么必须脱敏
//!
//! 抓下来的帧里有**真实的联系人名和消息正文**。它们不能进仓库：
//! git 历史永久留存，仓库可能推远端。
//!
//! ## 踩过的两次错
//!
//! **第一次：按亮度阈值抹。** 「亮的就是文字」。错——我方气泡底色
//! `rgb(114,205,144)` 亮度 205，比任何白字都高，这条规则会连气泡一起抹掉。
//!
//! **第二次：按累计覆盖率保留主色。** 错——4 bit/通道的量化对暗色太粗，
//! `rgb(22,22,22)`（气泡内深字）与 `rgb(30,30,31)`（面板底色）落进同一个桶
//! `(17,17,17)`，深字被当成布局色留下。实测第一版夹具里「704056」
//! 「don't mind」「小号」和全部时间戳都清晰可读。
//!
//! ## 正确做法：显式白名单
//!
//! 不用任何统计去猜「什么该留」。**几何判定真正依赖的颜色是有限的三个**：
//! 面板底色、两侧气泡底色、边框线。调用方已经从锚点把它们算出来了，
//! 直接给过来，其余像素一律吸附到最近的保留色。
//!
//! ## 夹具锁什么、不锁什么
//!
//! 锁：**锚点矩形、配色、气泡/面板的归属判定**——阈值最脆弱的地方。
//! 不锁：**文字内容**。OCR 是微软的，我们的夹具不该在它升级时误报。

use std::collections::HashMap;

use chatservant_core::frame::{Frame, Rect};

pub type Rgb = [u8; 3];

/// 颜色比较容差。实测面板底色在帧内完全一致，只有抗锯齿边缘会差一点点。
pub const COLOR_TOL: i32 = 12;

/// 气泡色占消息区的最小面积比例。
///
/// 实测：单个气泡约占消息区 2% 以上；三种文字墨色各自不到 1%。
/// 取 1% 把两者分开，且不随窗口尺寸变化（用的是比例不是绝对像素数）。
pub const BUBBLE_AREA_RATIO: f64 = 0.01;

/// 侧边栏窄带里保留颜色的最小面积占比。
///
/// 实测该带有 5 种色各占 3%~60%，文字墨色都在 1% 以下。取 3%。
pub const SIDEBAR_AREA_RATIO: f64 = 0.03;

/// 局部底色图的块边长。文字笔画最粗也就两三个像素，
/// 8×8 足够分辨「这块是气泡」还是「这块是面板」，代价只有 1/64。
const BLOCK: u32 = 8;

/// 判定「这个像素就是背景色」的容差。**要非常紧。**
///
/// 实测背景区域是**精确均匀**的——面板底色在全帧每一处都是 `rgb(30,30,31)`。
/// 所以用 0 都能work。
///
/// 不能用宽松容差（原先是 12）：气泡内深字 `rgb(22,22,22)` 离面板底色只有 8，
/// 在 12 的容差内会被当成「背景色」原样留下，块主色逻辑根本没被调用——
/// 字印在绿气泡上，却因为「接近面板色」而活了下来。
///
/// 关键认知：**「这个像素是背景吗」由它所在块决定，不由它自己的颜色决定。**
/// 深字恰好接近另一个背景色，是巧合，不是它是背景的证据。
const IS_BG_TOL: i32 = 2;

pub fn near_color(a: Rgb, b: Rgb) -> bool {
    let d = |x: u8, y: u8| (x as i32 - y as i32).abs();
    d(a[0], b[0]) <= COLOR_TOL && d(a[1], b[1]) <= COLOR_TOL && d(a[2], b[2]) <= COLOR_TOL
}

fn dist(a: Rgb, b: Rgb) -> i32 {
    sq_dist(a, b)
}

/// 平方欧氏距离。比较时不开根号——省掉每像素几十次 sqrt。
fn sq_dist(a: Rgb, b: Rgb) -> i32 {
    let d = |x: u8, y: u8| (x as i32 - y as i32).pow(2);
    d(a[0], b[0]) + d(a[1], b[1]) + d(a[2], b[2])
}

fn scan(frame: &Frame, r: Rect, accept: impl Fn(Rgb) -> bool) -> HashMap<Rgb, usize> {
    let mut hist = HashMap::new();
    let x1 = (r.x1.max(0) as u32).min(frame.width());
    let y1 = (r.y1.max(0) as u32).min(frame.height());
    let mut y = r.y0.max(0) as u32;
    while y < y1 {
        let mut x = r.x0.max(0) as u32;
        while x < x1 {
            if let Some(p) = frame.pixel(x, y)
                && accept(p)
            {
                *hist.entry(p).or_insert(0) += 1;
            }
            x += 1;
        }
        y += 1;
    }
    hist
}

/// 一个矩形里出现最多的若干个精确色。
pub fn dominant_in(frame: &Frame, r: Rect, limit: usize) -> Vec<Rgb> {
    let mut v: Vec<(Rgb, usize)> = scan(frame, r, |_| true).into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v.into_iter().take(limit).map(|(c, _)| c).collect()
}

/// 消息区里「不是面板底色、但成片出现」的颜色 = 气泡色。
pub fn find_bubble_colors(frame: &Frame, chat: Rect, panel_bg: Rgb) -> Vec<Rgb> {
    let hist = scan(frame, chat, |p| !near_color(p, panel_bg));
    let area = ((chat.x1 - chat.x0).max(0) * (chat.y1 - chat.y0).max(0)) as f64;
    if area <= 0.0 {
        return Vec::new();
    }
    let min = (area * BUBBLE_AREA_RATIO) as usize;
    hist.into_iter()
        .filter(|(_, n)| *n >= min)
        .map(|(c, _)| c)
        .collect()
}

/// 按显式白名单脱敏：不在 `keep` 里的像素吸附到最近的保留色。
///
/// `keep` 为空时原样返回——没有白名单就等于什么都没抹，
/// 与其产出一张全被吸成纯色的废图，不如让调用方的守卫报出来。
pub fn redact_with(frame: &Frame, keep: &[Rgb]) -> Frame {
    if keep.is_empty() {
        return frame.clone();
    }
    let (w, h) = (frame.width(), frame.height());
    let bw = w.div_ceil(BLOCK) as usize;
    let bh = h.div_ceil(BLOCK) as usize;

    // 第一遍：每块取「块内出现最多的保留色」作为该块的底色。
    //
    // 这一步是为了修一个真实的泄漏：气泡内深字 rgb(22,22,22) 到面板底色
    // rgb(30,30,31) 的距离只有 192，到气泡底色 rgb(114,205,144) 的距离
    // 是 56837。按全局最近吸附，这块字会变成面板色——在绿气泡上反而
    // 更显眼，文字照样能读。文字的语义是「印在某个底色上」，
    // 所以它该变成**所在块的底色**，不是全局最近的保留色。
    let mut block_bg: Vec<Option<Rgb>> = vec![None; bw * bh];
    for by in 0..bh {
        for bx in 0..bw {
            let mut counts: HashMap<Rgb, usize> = HashMap::new();
            for y in (by as u32 * BLOCK)..((by as u32 + 1) * BLOCK).min(h) {
                for x in (bx as u32 * BLOCK)..((bx as u32 + 1) * BLOCK).min(w) {
                    if let Some(p) = frame.pixel(x, y)
                        && let Some(k) = keep.iter().find(|k| near_color(p, **k))
                    {
                        *counts.entry(*k).or_insert(0) += 1;
                    }
                }
            }
            block_bg[by * bw + bx] = counts.into_iter().max_by_key(|(_, n)| *n).map(|(c, _)| c);
        }
    }

    // 第二遍：本身是保留色的原样留；否则继承所在块的底色。
    // 块里一个保留色都没有（纯头像/图片区）才退回全局最近色。
    let mut out = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            let p = frame.pixel(x, y).unwrap_or([0, 0, 0]);
            let snapped = if keep.iter().any(|k| sq_dist(p, *k) <= IS_BG_TOL * IS_BG_TOL) {
                // 它就是背景本身，原样留
                p
            } else {
                // 它是被印在某个底色上的内容，继承所在块的底色
                let block = (y / BLOCK) as usize * bw + (x / BLOCK) as usize;
                block_bg[block].unwrap_or_else(|| {
                    keep.iter()
                        .min_by_key(|k| dist(p, **k))
                        .copied()
                        .unwrap_or(p)
                })
            };
            out.extend_from_slice(&snapped);
        }
    }
    Frame::from_rgb(w, h, out).expect("输出尺寸与输入一致")
}

/// 矩形里占比达标的**全部**颜色。
///
/// 不能只取 top-1：实测侧边栏有 5 种色各占 3%~60%，
/// 只留最多的那个会把次要的几万像素吸走，列占比剖面一变，
/// `PaneAnchor` 的边界就漂。
pub fn dominant_above(frame: &Frame, r: Rect, min_ratio: f64) -> Vec<Rgb> {
    let hist = scan(frame, r, |_| true);
    let total: usize = hist.values().sum();
    if total == 0 {
        return Vec::new();
    }
    let min = (total as f64 * min_ratio) as usize;
    hist.into_iter()
        .filter(|(_, n)| *n >= min)
        .map(|(c, _)| c)
        .collect()
}

/// 组出该保留的颜色集合：面板底色 + 侧边栏色 + 气泡色 + 两处边框色。
///
/// ## 四样都必须留，缺一个锚点就塌
///
/// - **面板底色**：配色基准。
/// - **侧边栏色**：`PaneAnchor` 靠「面板 vs 侧边栏」的列占比跳变定 `panel.x0`。
///   实测漏掉它会让左边界从 192 漂到 0——侧边栏被吸成面板色，分界就消失了。
/// - **气泡色**：归类要判 me/them。
/// - **边框色**：从锚点的两条边界线上取，抹掉就推不出 header/input 边界。
///
/// 边框色取边界线**上方两行**（线本身 + 紧邻一行），那一带是纯边框色。
pub fn keep_colors(
    frame: &Frame,
    chat: Rect,
    panel_bg: Rgb,
    header_top: i32,
    input_top: i32,
) -> Vec<Rgb> {
    let mut keep = vec![panel_bg];

    // 侧边栏色：**扫整条侧边栏**，取面积占比达标的全部颜色。
    //
    // 走过两条错路，都不行：
    // ① top-1 —— 侧边栏有 5 种色各占 3%~48%，只留一种会让其余几万像素
    //    被吸走，列占比剖面一变，panel.x0 就漂。
    // ② 只扫锚点左界左侧 24px 窄带 —— `rgb(37,37,38)` 占整条侧边栏 18.3%，
    //    但它主要住在 x<280，窄带只覆盖右侧一小条，根本扫不到。
    //
    // 实测整条侧边栏的分布（1262×958 的小号会话帧）：
    //   rgb(47,47,48)   48.4%  侧边栏底色（与对方气泡同色！）
    //   rgb(37,37,38)   18.3%  次级底色
    //   rgb(59,59,61)    9.8%
    //   rgb(84,164,108)  7.0%  当前会话的绿色高亮行
    //   文字墨色合计     0.7%  ← 取 3% 阈值正好把它们排除
    if chat.x0 > 0 {
        let sidebar = Rect::new(0, chat.y0, chat.x0, chat.y1);
        keep.extend(dominant_above(frame, sidebar, SIDEBAR_AREA_RATIO));
    }

    keep.extend(find_bubble_colors(frame, chat, panel_bg));

    // 边框线：只取 `edge` **那一行**，按面积阈值取、且排除面板底色。
    //
    // 边框只有 1 px。按 `edge-2 .. edge+1` 取 4 行的众色是错的——
    // 里面 3 行是面板底色，取到的就是面板底色，边框色根本没被采到
    // （保留色里 rgb(30,30,31) 重复出现三次就是证据）。
    for edge in [header_top, input_top] {
        let line = Rect::new(chat.x0, edge, chat.x1, edge + 1);
        let h = scan(frame, line, |p| !near_color(p, panel_bg));
        let total: usize = h.values().sum();
        let min = (total as f64 * BORDER_AREA_RATIO) as usize;
        if let Some((c, _)) = h
            .into_iter()
            .filter(|(_, n)| *n >= min)
            .max_by_key(|(_, n)| *n)
        {
            keep.push(c);
        }
    }
    keep
}

/// 边框行里保留颜色的最小面积占比。输入框上边框是圆角矩形的一线，
/// 该行大部分仍是面板底色，所以阈值要低。
const BORDER_AREA_RATIO: f64 = 0.02;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attribute::Palette;
    use chatservant_capture::anchor;

    const PANEL: Rgb = [30, 30, 31];
    const MY_BUBBLE: Rgb = [114, 205, 144];
    const DARK_INK: Rgb = [22, 22, 22];

    fn frame_with_text() -> Frame {
        let (w, h) = (200u32, 200u32);
        let mut px = Vec::with_capacity((w * h * 3) as usize);
        for _ in 0..(w * h) {
            px.extend_from_slice(&PANEL);
        }
        // 白字小块——必须被抹掉
        for y in 80..90u32 {
            for x in 40..160u32 {
                let i = ((y * w + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&[225, 225, 229]);
            }
        }
        Frame::from_rgb(w, h, px).unwrap()
    }

    /// **回归测试**：气泡内深字 `rgb(22,22,22)` 与面板底色 `rgb(30,30,31)`
    /// 只差 8。它印在**绿气泡**上，必须变成气泡色——变成面板色就仍然
    /// 看得见（在绿底上面板色是深灰）。
    ///
    /// 笔画画成 **2 px 细条纹**：真实文字笔画就这个粗细。画成实心块的话，
    /// 一个 8×8 的块里墨色反而占多数，块主色就取不到气泡了——那是夹具
    /// 不真实，不是算法不对。
    #[test]
    fn dark_ink_inherits_its_local_bubble_background() {
        let (w, h) = (200u32, 200u32);
        let mut px = Vec::with_capacity((w * h * 3) as usize);
        for _ in 0..(w * h) {
            px.extend_from_slice(&PANEL);
        }
        for y in 40..70u32 {
            for x in 40..160u32 {
                let i = ((y * w + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&MY_BUBBLE);
            }
        }
        // 两道 2px 的笔画
        for y in [50u32, 58] {
            for x in 60..140u32 {
                for dy in 0..2u32 {
                    let i = (((y + dy) * w + x) * 3) as usize;
                    px[i..i + 3].copy_from_slice(&DARK_INK);
                }
            }
        }
        let f = Frame::from_rgb(w, h, px).unwrap();

        // 白名单里只有面板色与气泡色——深字不在其中
        let r = redact_with(&f, &[PANEL, MY_BUBBLE]);
        assert_eq!(
            r.pixel(100, 50),
            Some(MY_BUBBLE),
            "深字应继承所在气泡的底色"
        );
        assert_eq!(r.pixel(100, 45), Some(MY_BUBBLE), "气泡底色必须原样保留");
        assert_eq!(r.pixel(10, 10), Some(PANEL), "面板底色必须原样保留");
        assert_eq!(r.pixel(10, 10), Some(PANEL), "面板底色必须原样保留");
    }

    #[test]
    fn keeps_explicit_colors_untouched() {
        let f = frame_with_text();
        let r = redact_with(&f, &[PANEL]);
        assert_eq!(r.pixel(10, 10), Some(PANEL));
        // 白字被吸附成面板色
        assert_eq!(r.pixel(100, 85), Some(PANEL), "白字应被抹平");
    }

    #[test]
    fn empty_keeplist_is_a_noop() {
        let f = frame_with_text();
        let r = redact_with(&f, &[]);
        assert_eq!(
            r.as_bytes(),
            f.as_bytes(),
            "空白名单应原样返回，由调用方守卫报错"
        );
    }

    /// 脱敏不得改变锚点——否则这张夹具锁不住几何。
    #[test]
    fn redaction_does_not_move_the_anchor() {
        let (w, h) = (800u32, 600u32);
        let side = [34u8, 34, 51];
        let mut px = Vec::with_capacity((w * h * 3) as usize);
        for _ in 0..h {
            for x in 0..w {
                let c = if x < 192 { side } else { PANEL };
                px.extend_from_slice(&c);
            }
        }
        let put = |px: &mut [u8], x: u32, y: u32, c: [u8; 3]| {
            let i = ((y * w + x) * 3) as usize;
            px[i..i + 3].copy_from_slice(&c);
        };
        for x in 192..w {
            for y in 0..28u32 {
                put(&mut px, x, y, [51, 51, 51]);
            }
            for y in 426..430u32 {
                put(&mut px, x, y, [51, 51, 51]);
            }
        }
        let f = Frame::from_rgb(w, h, px).unwrap();

        let before = anchor::derive(&f).expect("脱敏前应能推导");
        let keep = keep_colors(
            &f,
            before.chat,
            before.panel_bg,
            before.header_top,
            before.input_top,
        );
        let after = anchor::derive(&redact_with(&f, &keep)).expect("脱敏后应能推导");
        assert_eq!(before.chat, after.chat, "脱敏不得移动锚点");
    }

    #[test]
    fn palette_survives_redaction() {
        let (w, h) = (400u32, 300u32);
        let mut px = Vec::with_capacity((w * h * 3) as usize);
        for _ in 0..(w * h) {
            px.extend_from_slice(&PANEL);
        }
        for y in 40..90u32 {
            for x in 240..380u32 {
                let i = ((y * w + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&MY_BUBBLE);
            }
        }
        let f = Frame::from_rgb(w, h, px).unwrap();
        let chat = Rect::new(0, 0, w as i32, h as i32);

        let p1 = Palette::learn(&f, chat).expect("配色");
        let keep = keep_colors(&f, chat, p1.panel_bg, 0, 0);
        let p2 = Palette::learn(&redact_with(&f, &keep), chat).expect("配色");
        assert_eq!(p1.panel_bg, p2.panel_bg, "脱敏不得改变学到的面板底色");
    }
}
