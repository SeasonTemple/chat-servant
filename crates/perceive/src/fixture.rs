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
//! ## 正确做法：显式白名单 + 块级区域生长
//!
//! 不用任何统计去猜「什么该留」。**几何判定真正依赖的颜色是有限的**：
//! 面板底色、侧边栏底色、两侧气泡底色、边框线、选中行高亮。
//! 调用方已经从锚点把它们算出来了，直接给过来，其余像素一律替换。
//!
//! 替换成什么，是这个模块唯一真正难的地方。走过两条错路，都产出了
//! **看起来更糟**的夹具：
//!
//! **第三次错：普查时用宽松容差认保留色。** 普查若用 `near_color`
//! （容差 12），我方气泡里的深字 `rgb(22,22,22)` 会被算成面板底色
//! `rgb(30,30,31)`——只差 8。字形密的块于是「多数票」投给面板色，
//! 整块底色标错，深字被换成面板色压在绿气泡上：实测产出的夹具里
//! 绿气泡上的正文全是深色麻点，**仍然可读**。
//!
//! **第四次错：一个块里一个保留色都没有时退回「全局最近保留色」。**
//! 头像、表情包整块都不是布局色。浅色像素 `rgb(123,123,128)` 的全局
//! 最近是绿色，于是侧边栏的联系人名、列表项全变成绿字压在深底上——
//! **比原来更醒目**。
//!
//! 所以：**普查与渲染必须共用同一条「精确命中」判定，容差收到 2；
//! 空块向邻近块传播底色，绝不往全局找。**
//!
//! ## 三条必须同时成立的性质
//!
//! 1. **调色板封闭**：输出的每个像素都落在白名单里（容差 2 内）。
//! 2. **空间局部**：像素换成的是**它自己那一块**的底色，不是全局最近的。
//! 3. **块内一致**：同一块里所有非保留像素颜色完全相同——这是
//!    「字形形状被抹掉」的充分条件。字只有 2~3 px 宽，块是 8×8，
//!    字形绝无可能填满整块，所以它一定被抹平。
//!
//! ## 夹具锁什么、不锁什么
//!
//! 锁：**锚点矩形、配色、气泡/面板的归属判定**——阈值最脆弱的地方。
//! 不锁：**文字内容**。OCR 是微软的，我们的夹具不该在它升级时误报。

use std::collections::{HashMap, VecDeque};

use chatservant_core::frame::{Frame, Rect};

/// 一个像素的 RGB。用定长数组而不是自定义类型，是因为所有判据都是
/// 逐通道比数值，没有需要挂在它上面的行为。
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

/// 判定「这个像素精确就是某个保留色」的容差。**必须极紧。**
///
/// 普查与渲染共用这一条，它是脱敏唯一的一道闸：凡是没能精确命中的像素，
/// 一律换成它所在块的底色。
///
/// 实测背景区域是**精确均匀**的——面板底色在全帧每一处都是 `rgb(30,30,31)`。
/// 所以 2 已经很宽裕。
///
/// 不能用宽松容差（原先普查阶段用的是 `near_color` 的 12）：气泡内深字
/// `rgb(22,22,22)` 离面板底色只有 8，会被当成「这个块是面板色」
/// 投票给它，整块底色就标错，深字被换成面板色压在绿气泡上照样可读。
///
/// 关键认知：**「这个像素是背景吗」由它所在块决定，不由它自己的颜色决定。**
/// 深字恰好接近另一个背景色，是巧合，不是它是背景的证据。
const IS_BG_TOL: i32 = 2;

/// 判定「这个像素就是所在块那片表面」的容差。**必须比 COLOR_TOL 紧得多。**
///
/// 实测背景区域是**精确均匀**的——面板底色在全帧每一处都是 `rgb(30,30,31)`。
/// 同一片表面内部的像素就是精确同色，4 已经远远够用。
///
/// 为什么不用 `COLOR_TOL`(12)：**抗锯齿的字形边缘会趁虚而入。**
/// 实测侧边栏一行联系人里，光是字形边缘的过渡像素 `rgb(55,55,56)` 就有
/// 342 个（占该行 1.78%）——它离侧边栏底色 `rgb(47,47,48)` 只有 8，
/// 在 12 之内就被原样留下，于是在夹具上留下一层能看出字形的麻点。
/// 收紧到 4 之后这些边缘像素一律刷成块底色。
const SURFACE_TOL: i32 = 4;

/// 一块里「出现最多的精确色」占多少像素，才算**平直表面**。
///
/// 一块表面是平的（面板底、气泡底、边框线）；一块内容是花的（头像、照片、
/// 字形笔画）。这个比值就是「平 vs 花」的度量。
///
/// 取 0.35：布局块最坏的情况是**对半分**（标题栏/面板、边框线/面板的交界块），
/// 正好 0.50，留 40% 余量；而照片块里同色像素成片的情况远达不到 0.35。
const SURFACE_PURITY_MIN: f32 = 0.35;

/// 脱敏失败时（白名单里一个色都没命中）用的兜底色。
/// 整帧被刷成同一个白名单色不泄漏任何内容。
fn fallback_color(keep: &[Rgb]) -> Rgb {
    keep[0]
}

/// 每个块的判定结果。
struct BlockInfo {
    /// 块内出现最多的**精确命中**保留色。没有则由传播填。
    surface: Option<Rgb>,
    /// 出现最多的精确色占块内像素的比例。≥ `SURFACE_PURITY_MIN` 即平直表面。
    purity: f32,
}

/// 两个颜色是否近到可以当同一个（每个通道差 ≤ [`COLOR_TOL`]）。
///
/// 这是**宽松**比较，只在「这块区域大致是什么底色」这类粗判据上用。
/// ⚠ 脱敏的判定闸是 [`IS_BG_TOL`]（容差 2）而不是这里——宽松容差会让气泡内深字
/// `rgb(22,22,22)` 被当成面板底色，正是「绿气泡上正文仍是深色麻点」的成因。
pub fn near_color(a: Rgb, b: Rgb) -> bool {
    near_with(a, b, COLOR_TOL)
}

fn near_with(a: Rgb, b: Rgb, tol: i32) -> bool {
    let d = |x: u8, y: u8| (x as i32 - y as i32).abs();
    d(a[0], b[0]) <= tol && d(a[1], b[1]) <= tol && d(a[2], b[2]) <= tol
}

/// 平方欧氏距离。比较时不开根号——省掉每像素几十次 sqrt。
fn sq_dist(a: Rgb, b: Rgb) -> i32 {
    let d = |x: u8, y: u8| (x as i32 - y as i32).pow(2);
    d(a[0], b[0]) + d(a[1], b[1]) + d(a[2], b[2])
}

/// 精确命中白名单则返回该保留色。
///
/// 普查与渲染**必须**都走这里。两处用不同的容差，正是「绿气泡上的正文
/// 仍是深色麻点」那个夹具的成因。
fn exact_keep(p: Rgb, keep: &[Rgb]) -> Option<Rgb> {
    let lim = IS_BG_TOL * IS_BG_TOL;
    keep.iter().copied().find(|k| sq_dist(p, *k) <= lim)
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
    v.sort_by_key(|a| std::cmp::Reverse(a.1));
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

/// 按显式白名单脱敏：不在 `keep` 里的像素替换成**它所在块的底色**。
///
/// ## 两阶段
///
/// 1. **普查**：把帧切成 `BLOCK`×`BLOCK` 的块，每块取块内出现最多的
///    **精确命中**的保留色作为该块底色。一个保留色都没有的块留空。
/// 2. **传播**：块网格上的多源 BFS，把留空的块填成 8 邻域里最近的
///    已定底色。网格连通，所以除「白名单一个都没命中」外必然填满。
///
/// 渲染时：**与所在块底色相差在 `COLOR_TOL` 内的像素原样保留，其余一律
/// 取块底色**——于是每个块在输出里是一���纯表面，字形被彻底抹平。
///
/// ## 为什么判据是「与块底色相近」而不是「在白名单里」
///
/// 走过第五次错：**「这个像素是保留色」不能推出「这个像素是这块的表面」。**
/// 实测头像照片的暗部里有一大片像素恰好等于 `rgb(31,31,32)`——那是当初
/// 从边框两行取 top-2 众色时混进来的抗锯齿过渡色，本来就在白名单里。
/// 于是照片里的暗部被当成侧边栏的表面原样留下，夹具上留下一块
/// 与头像宫格同形的深色剪影。
///
/// 白名单是**跨整帧**的颜色集合，内容（头像/图片/表情包）与布局完全可能
/// 用上同一个色值；只有「这块的底色」才是有空间含义的东西。
///
/// ## 为什么不能一刀切成「每个块纯色」
///
/// 输入框上边框只有 3 px，落在 8×8 的块里必然被面板底色压过多数。
/// 一刀切会把边框抹掉，`PaneAnchor` 的 `input_top` 就塌了。
/// 边框色 `rgb(39,39,40)` 离面板底色只有 9，在 `COLOR_TOL` 内，
/// 所以「与块底色相近就保留」既留住了边框，也挡住了照片里的同值像素。
///
/// `keep` 为空时原样返回——没有白名单就等于什么都没抹，
/// 与其产出一张全被刷成纯色的废图，不如让调用方的守卫报出来。
///
/// ## 为什么不能退回「全局最近保留色」
///
/// 头像、表情包整块都不是布局色，空块不少。浅色像素 `rgb(123,123,128)`
/// 的全局最近是绿色，于是侧边栏的联系人名、日文全文、URL、时间戳
/// 全变成绿字压在深底上——**比原来更醒目**。向邻近块传播才是对的：
/// 像素的语义是「印在某个底色上」，那个底色就在它旁边。
///
/// `keep` 为空时原样返回——没有白名单就等于什么都没抹，
/// 与其产出一张全被刷成纯色的废图，不如让调用方的守卫报出来。
pub fn redact_with(frame: &Frame, keep: &[Rgb]) -> Frame {
    if keep.is_empty() {
        return frame.clone();
    }
    let (w, h) = (frame.width(), frame.height());
    let bw = w.div_ceil(BLOCK) as usize;
    let bh = h.div_ceil(BLOCK) as usize;
    let mut blocks = block_infos(frame, keep, bw, bh);
    spread_surfaces(&mut blocks, bw, bh);

    let fallback = fallback_color(keep);
    let mut out = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for y in 0..h {
        let row = (y / BLOCK) as usize * bw;
        for x in 0..w {
            let p = frame.pixel(x, y).unwrap_or([0, 0, 0]);
            let info = &blocks[row + (x / BLOCK) as usize];
            let c = match info.surface {
                // ① 与所在块底色同一片表面 —— 原样保留。
                Some(s) if near_with(p, s, SURFACE_TOL) => p,
                // ② 平直表面上，白名单里的精确布局色也原样保留。
                //    交界块（边框线穿过面板、标题栏挨着面板）必须靠这一条
                //    才不会把边界挪走：8×8 的块必然被多数那侧的颜色盖过，
                //    只按 ① 渲染会让 band 边界漂掉最多 BLOCK-1 像素。
                _ if info.purity >= SURFACE_PURITY_MIN && exact_keep(p, keep).is_some() => p,
                Some(s) => s,
                None => fallback,
            };
            out.extend_from_slice(&c);
        }
    }
    Frame::from_rgb(w, h, out).expect("输出尺寸与输入一致")
}

/// 阶段一：逐块取「块内出现最多的**精确命中**保留色」，并量出块的平直度。
fn block_infos(frame: &Frame, keep: &[Rgb], bw: usize, bh: usize) -> Vec<BlockInfo> {
    let (w, h) = (frame.width(), frame.height());
    let mut blocks = Vec::with_capacity(bw * bh);
    for by in 0..bh {
        let y_end = ((by as u32 + 1) * BLOCK).min(h);
        for bx in 0..bw {
            let x_end = ((bx as u32 + 1) * BLOCK).min(w);
            let mut all: HashMap<Rgb, usize> = HashMap::new();
            let mut kept: HashMap<Rgb, usize> = HashMap::new();
            for y in (by as u32 * BLOCK)..y_end {
                for x in (bx as u32 * BLOCK)..x_end {
                    if let Some(p) = frame.pixel(x, y) {
                        *all.entry(p).or_insert(0) += 1;
                        if let Some(k) = exact_keep(p, keep) {
                            *kept.entry(k).or_insert(0) += 1;
                        }
                    }
                }
            }
            let total: usize = all.values().sum();
            let top = all.values().copied().max().unwrap_or(0);
            blocks.push(BlockInfo {
                // 并列时取色值更小的一个，保证结果与遍历顺序无关、可复现。
                surface: kept
                    .into_iter()
                    .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
                    .map(|(c, _)| c),
                purity: top as f32 / total.max(1) as f32,
            });
        }
    }
    blocks
}

/// 阶段二：块网格上的多源 BFS，把空块填成 8 邻域里最近的已定底色。
///
/// 网格是连通的，只要有一个块定了底色就必然能填满——不需要兜底分支。
fn spread_surfaces(blocks: &mut [BlockInfo], bw: usize, bh: usize) {
    let mut queue: VecDeque<usize> = blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| b.surface.map(|c| (i, c)))
        .map(|(i, _)| i)
        .collect();
    while let Some(i) = queue.pop_front() {
        let Some(s) = blocks[i].surface else { continue };
        let (bx, by) = (i % bw, i / bw);
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let nx = bx as i32 + dx;
                let ny = by as i32 + dy;
                if nx < 0 || ny < 0 || nx >= bw as i32 || ny >= bh as i32 {
                    continue;
                }
                let j = ny as usize * bw + nx as usize;
                if blocks[j].surface.is_none() {
                    blocks[j].surface = Some(s);
                    queue.push_back(j);
                }
            }
        }
    }
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

    // 边框线：只取 `edge` 与 `edge+1` 两行的**主色**，且要求它占比达标。
    //
    // 实测（1262×958 的小号会话帧，x 310..1254）：
    //   y=683   rgb(39,39,40)  占 97.9%
    //   y=684   rgb(44,44,45)
    //   y=685   rgb(47,47,48)
    // 输入框上边框是**抗锯齿的圆角矩形描边**，三层渐变，不是单色线。
    //
    // 关键是**不能排除面板底色**：rgb(39,39,40) 离面板色 rgb(30,30,31) 只有 9，
    // 在 COLOR_TOL=12 内会被当成「就是面板色」滤掉——那样就把占 97.9% 的
    // 真边框扔了，只剩 1.8% 的杂色，边框等于完全没采到，input_top 就塌。
    //
    // 但反过来**也不能取 top-2**：那 1.8% 里就有 rgb(44,44,45)、rgb(31,31,32)
    // 这类抗锯齿过渡色。它们进了白名单之后，头像照片暗部里**恰好等于它们的
    // 像素**就会被当成表面原样留下——实测夹具上留下一块与头像宫格同形的
    // 深色剪影。过渡色零散只占 1%~2%，主色却横跨整个面板宽度、占 97.9%，
    // 80% 的达标线把两者分得很开。
    for edge in [header_top, input_top] {
        // **只取那一行**。取两行会把主色占比腰斩到 49%：实测 y=683 的
        // rgb(39,39,40) 占 97.9%，y=684 是另一种颜色 rgb(44,44,45)，
        // 两行合起来谁也到不了 80%，边框色就被整条漏掉了。
        let line = Rect::new(chat.x0, edge, chat.x1, edge + 1);
        keep.extend(band_surface(frame, line));
    }
    keep
}

/// 一条细带里占比达标的主色。没有达标色返回 `None`。
///
/// 达标线 80%：布局细带（边框线）横跨整个面板宽度，主色占 97.9%；
/// 抗锯齿过渡色只占 1%~2%。中间没有灰区。
const BORDER_COLOR_SHARE: f64 = 0.80;

fn band_surface(frame: &Frame, r: Rect) -> Option<Rgb> {
    let hist = scan(frame, r, |_| true);
    let total: usize = hist.values().sum();
    let ((c, n), _) = hist.into_iter().max_by_key(|(_, n)| *n).map(|e| (e, ()))?;
    (n as f64 >= total as f64 * BORDER_COLOR_SHARE).then_some(c)
}

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

    /// **回归测试（第四次错）：一个块里一个保留色都没有时，必须向邻近块
    /// 传播底色，绝不退回「全局最近保留色」。**
    ///
    /// 这是**头像**的形状。头像整块都不是布局色，8×8 的块一块保留色都没有。
    /// 旧实现退回全局最近，于是浅色头像像素 `rgb(90,190,130)` 被换成绿色
    /// ——它离绿色 `(84,164,108)` 明显比离侧边栏色近。整帧头像变成绿块，
    /// 侧边栏的联系人名、列表项同理。
    #[test]
    fn a_block_with_no_keep_color_inherits_from_its_neighbours_not_globally() {
        const SIDEBAR: Rgb = [47, 47, 48];
        /// 刻意挑一个**全局最近是绿色**的头像色：
        /// 到 `GREEN` 的平方距离 1196，到 `SIDEBAR` 是 29022。
        const AVATAR: Rgb = [90, 190, 130];
        const GREEN: Rgb = [84, 164, 108];

        let (w, h) = (240u32, 120u32);
        let mut px = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                // 左边 96 px 是侧边栏，右边是聊天面板
                let mut c = if x < 96 { SIDEBAR } else { PANEL };
                // 侧边栏中间摆一个 32×32 的「头像」——整块都不是布局色
                if (32..64).contains(&x) && (40..72).contains(&y) {
                    c = AVATAR;
                }
                px.extend_from_slice(&c);
            }
        }
        // 远处放一块绿色，充当「全局最近」的诱饵
        for y in 0..40u32 {
            for x in 160..240u32 {
                let i = ((y * w + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&GREEN);
            }
        }
        let f = Frame::from_rgb(w, h, px).unwrap();

        let r = redact_with(&f, &[PANEL, SIDEBAR, GREEN]);
        assert_eq!(
            r.pixel(48, 56),
            Some(SIDEBAR),
            "头像必须溶进它旁边的侧边栏底色，而不是全局最近的绿色"
        );
        assert_eq!(r.pixel(0, 0), Some(SIDEBAR), "侧边栏底色原样保留");
        assert_eq!(r.pixel(200, 10), Some(GREEN), "绿色块原样保留");
    }

    /// **回归测试（第三次错）：普查必须用**精确**命中，不能用宽松容差。**
    ///
    /// 我方气泡里的正文墨色是 `rgb(22,22,22)`，离面板底色 `rgb(30,30,31)`
    /// 只有 8——在 `near_color` 的容差 12 之内。普查若用它，字形密的块会
    /// 把票投给面板色，整块底色标错，深字被换成面板色压在绿气泡上，
    /// 仍然可读（实测夹具里绿气泡上的正文就是一片深色麻点）。
    ///
    /// 这里直接画一个**铺满整块的 8×8 实心墨色块**——那是密笔画汉字
    /// 字腔的真实情形，不是人为夸张。
    #[test]
    fn ink_near_a_keep_color_cannot_hijack_the_block_census() {
        let (w, h) = (200u32, 200u32);
        let mut px = Vec::with_capacity((w * h * 3) as usize);
        for _ in 0..(w * h) {
            px.extend_from_slice(&PANEL);
        }
        for y in 40..72u32 {
            for x in 40..160u32 {
                let i = ((y * w + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&MY_BUBBLE);
            }
        }
        // x 96..104 / y 48..56 —— 恰好铺满 (bx=12, by=6) 这一个块
        for y in 48..56u32 {
            for x in 96..104u32 {
                let i = ((y * w + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&DARK_INK);
            }
        }
        let f = Frame::from_rgb(w, h, px).unwrap();

        let r = redact_with(&f, &[PANEL, MY_BUBBLE]);
        assert_eq!(
            r.pixel(100, 52),
            Some(MY_BUBBLE),
            "整块铺满的墨色也必须溶成气泡底色；变成面板色就仍压在绿底上可读"
        );
    }

    /// 输出的调色板必须是白名单的子集——这是脱敏的底线不变量。
    ///
    /// 任何在白名单之外的颜色出现在产物里，都意味着某个原始像素
    /// 原样漏了过去（文字、头像、图片都可能）。
    #[test]
    fn output_palette_is_closed_over_the_keepset() {
        const SIDEBAR: Rgb = [47, 47, 48];
        const GREEN: Rgb = [84, 164, 108];
        let (w, h) = (240u32, 160u32);
        let mut px = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                // 掺入大量「内容」：头像、字形、渐变过渡
                let c = if x < 96 {
                    SIDEBAR
                } else if y >= 100 {
                    GREEN
                } else {
                    PANEL
                };
                px.extend_from_slice(&c);
            }
        }
        let put = |px: &mut [u8], x: u32, y: u32, c: Rgb| {
            let i = ((y * w + x) * 3) as usize;
            px[i..i + 3].copy_from_slice(&c);
        };
        for y in 20..60u32 {
            for x in 20..60u32 {
                // 一张「照片」：颜色杂乱，全都不是保留色
                put(&mut px, x, y, [(x % 200) as u8, (y % 190) as u8, 90]);
            }
        }
        for y in 70..76u32 {
            for x in 120..220u32 {
                put(&mut px, x, y, [225, 225, 229]);
            }
        }
        let f = Frame::from_rgb(w, h, px).unwrap();

        let keep = [PANEL, SIDEBAR, GREEN];
        let r = redact_with(&f, &keep);
        for y in 0..h {
            for x in 0..w {
                let p = r.pixel(x, y).expect("在范围内");
                assert!(
                    keep.iter().any(|k| near_color(p, *k)),
                    "({x},{y}) 的输出色 rgb({:?}) 不在白名单内",
                    p
                );
            }
        }
    }

    /// **回归测试（第五次错）：「是保留色」不等于「是这块的表面」。**
    ///
    /// 实测头像照片的暗部里有一大片像素**恰好等于 `rgb(31,31,32)`**——
    /// 那是当初从边框两行取 top-2 众色时混进来的抗锯齿过渡色，本来就在
    /// 白名单里。按「在白名单里就原样保留」渲染，照片暗部被当成侧边栏
    /// 的表面留下，夹具上出现一块与头像宫格**同形**的深色剪影。
    ///
    /// 这里造一张「照片」：整体杂乱，但**故意掺进与面板底色完全相同的像素**。
    /// 断言：每个块的输出像素都必须落在**该块自己的底色**容差内——
    /// 白名单里的外来同值像素必须被刷掉。
    #[test]
    fn a_keep_colored_pixel_still_must_match_its_own_block_surface() {
        const SIDEBAR: Rgb = [47, 47, 48];
        /// 实测头像照片暗部的真实取值，恰好等于边框过渡色 rgb(31,31,32)。
        const PHOTO_DARK: Rgb = [31, 31, 32];

        let (w, h) = (240u32, 160u32);
        let mut px = Vec::with_capacity((w * h * 3) as usize);
        for _ in 0..(w * h) {
            px.extend_from_slice(&SIDEBAR);
        }
        let put = |px: &mut [u8], x: u32, y: u32, c: Rgb| {
            let i = ((y * w + x) * 3) as usize;
            px[i..i + 3].copy_from_slice(&c);
        };
        // 一张 64×64 的「照片」：杂乱配色，其中一条竖带**恰好是面板底色**。
        for y in 40..104u32 {
            for x in 60..124u32 {
                let c = if x == 80 || x == 81 {
                    PANEL // 与侧边栏底色无关，却是一个「保留色」
                } else {
                    [(x % 200) as u8, (y % 190) as u8, 90]
                };
                put(&mut px, x, y, c);
            }
        }
        for y in 60..72u32 {
            for x in 70..110u32 {
                put(&mut px, x, y, PHOTO_DARK);
            }
        }
        let f = Frame::from_rgb(w, h, px).unwrap();

        let keep = [PANEL, SIDEBAR];
        let r = redact_with(&f, &keep);

        // 自己重算一遍块底色，逐块核对输出的表面一致性。
        let bw = w.div_ceil(BLOCK) as usize;
        let bh = h.div_ceil(BLOCK) as usize;
        let mut blocks = block_infos(&f, &keep, bw, bh);
        spread_surfaces(&mut blocks, bw, bh);

        for by in 0..bh {
            for bx in 0..bw {
                let Some(surface) = blocks[by * bw + bx].surface else {
                    continue;
                };
                for y in (by as u32 * BLOCK)..((by as u32 + 1) * BLOCK).min(h) {
                    for x in (bx as u32 * BLOCK)..((bx as u32 + 1) * BLOCK).min(w) {
                        let p = r.pixel(x, y).expect("在范围内");
                        assert!(
                            near_color(p, surface),
                            "块 ({bx},{by}) 底色 rgb({surface:?})，但 ({x},{y}) 的输出是 rgb({p:?})——\
                             白名单里的外来同值像素没被刷掉"
                        );
                    }
                }
            }
        }
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
