//! 会话身份：**按像素指纹认，不按 OCR 文字认**。
//!
//! ## 为什么不能用文字
//!
//! 实测（`cargo xtask perceive --file target/probe/text-chat.png`）：
//! 头部那一行被正确归类成 `NoiseClass::Header`，但 OCR 把真实会话名
//! 「小号」读成了 **`G8W`**。11 px 高的中文标题，OCR 认不准是常态。
//!
//! 于是任何「拿 OCR 文字和常量比字符串」的锁定方式，在真实环境上
//! **永远是关的**——不误发，但也永远不工作。
//!
//! ## 为什么指纹比文字可靠
//!
//! 文字是 OCR 的产物；**那一小块像素**不是。换个会话，那块像素必然变；
//! 同一个会话在多帧之间，那块像素基本不变（只有未读角标之类会抖）。
//!
//! ## 怎么比：二值墨迹掩码，不是颜色直方图
//!
//! **2026-10-11 实测推翻了原来的「64 桶颜色直方图」。**
//!
//! 那天在真实微信帧上切会话，闸**一次都没关**，面板跟着切到了
//! 用户从未锁定的那个会话，还在给那个会话出候选——硬约束第 1 条直接破口。
//! 拿真实帧逐块量下来，原因有两层：
//!
//! ```text
//! 区域 x308..708 y31..71（现状用的那块，16000 px）
//!   最大桶占比 = 98.8%          ← 几乎所有颜色塌进同一个桶
//!   直方图相似度 = 0.982        ← 阈值 0.90，永远判「同一个」
//!   墨迹掩码 Dice = 0.2793       ← 换成墨迹立刻分开
//! ```
//!
//! 第一层是**量化太粗**：每通道压到 2 bit 之后，`rgb(17,17,17)`、
//! `rgb(34,34,51)`、`rgb(51,51,51)` 全落进第 0 号桶——微信深色界面
//! 九成的像素挤在一个桶里，会话名那 189 个亮像素（占 1.2%）根本推不动它。
//!
//! 第二层更要命：原来的测试 `a_different_name_prints_differently`
//! 用的是 `tint 225` 与 `tint 120`——**亮度差 105 级**。真实会话名之间的
//! 差别是**字形**，不是亮度：两个名字都是白字，亮度完全一样。
//! 那条测试一直在证明一件与真实情况无关的事，所以它一直是绿的，
//! 而闸在真实帧上从来没关过。
//!
//! 现在取的是**墨迹**：与本区域**众数亮度**差得够远的像素才算「字」。
//! 字是稀疏、位置精确的高对比像素，于是 189 个像素就足以定义身份。
//! 背景取众数而不是写死阈值，是为了深色主题（亮字暗底）与
//! 浅色主题（暗字亮底）都能用——写死阈值会在另一个主题上整个失效。
//!
//! ## 位移怎么吸收
//!
//! 掩码对位移敏感：整块挪 1 px，在掩码上就是一大片不重叠。
//! 锚点是按颜色分析推出来的，真机上不一定永远落在同一行。
//!
//! 做法是**只在「命中」这一侧放宽**：A 的某个墨像素，只要 B 在
//! **±1 px 内**有墨，就算命中。分母仍是各自的真实墨迹数。
//!
//! **为什么不先膨胀再比**：膨胀把两边的墨迹面积一起放大，位移带来的
//! 错位占比被摊薄但没消失（实测膨胀 1 px 只把 1 px 位移拉到 0.835，
//! 2 px 才 0.863，都还在 0.90 以下）；而膨胀**同时**会把两个不同名字的
//! 重叠也拉起来（实测 0.2793 → 0.5483 → 0.6354），那是往危险方向走。
//! 只放宽命中这一侧就没有这个问题：不同名字的字形位置本来就不同，
//! ±1 px 拉不动它们。
//!
//! 实测（同一对真实帧，两种量并排）：
//!
//! | 量 | 同一会话 | 不同会话 |
//! |---|---|---|
//! | 墨迹 Dice（不宽容错位） | 1.0000 | 0.2793 |
//! | **墨迹 Dice（±1 px 容错命中）** | **1.0000** | **0.2793** |
//! | 32×8 灰度格 | 1.0000 | 0.5377 |
//!
//! 两行相同 = ±1 px 的容错**没有**把不同名字拉近；而整块挪 1 px 会被
//! 完全吸收（单测 `a_shifted_box_is_still_the_same_conversation` 守着）。
//! 灰度格也分得开，但每格平均掉 25×5 像素，字被抹平；掩码保留位置信息，
//! 判别力更直接，也更好解释。

use chatservant_core::frame::{Frame, Rect};

/// 一个会话头部区域的**二值墨迹掩码**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderPrint {
    /// 逐像素 0/1。长度 = 宽 × 高。
    mask: Vec<u8>,
    /// 掩码的宽高，用来在比对时定位邻格。
    w: usize,
    h: usize,
    /// 墨像素数。Dice 的分母用它。
    ink: u32,
    /// 区域面积（宽 × 高）。**不是**墨迹数。
    total: u32,
}

impl HeaderPrint {
    /// 取一块区域的指纹。
    ///
    /// 返回 `None` 有三种情况，**含义都是「认不出」而不是「是同一个」**：
    /// 区域太小、区域内几乎没有墨（一片纯底色）、像素取不全。
    pub fn of(frame: &Frame, r: Rect) -> Option<Self> {
        let x1 = (r.x1.max(0) as u32).min(frame.width());
        let y1 = (r.y1.max(0) as u32).min(frame.height());
        let y0 = r.y0.max(0) as u32;
        let x0 = r.x0.max(0) as u32;
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let w = (x1 - x0) as usize;
        let h = (y1 - y0) as usize;
        if w * h < MIN_PIXELS as usize {
            return None;
        }

        // 一趟取亮度，顺便统计众数当背景色。
        // 裁剪过了却取不到像素**不该发生**；真发生了按「亮度 0」处理，
        //   它会让这个像素更接近亮背景而不是暗背景，即**偏向不被当成字**——
        //   宁可少认，不可多认。
        let mut lum: Vec<u8> = Vec::with_capacity(w * h);
        let mut hist = [0u32; 256];
        for y in y0..y1 {
            for x in x0..x1 {
                let l = frame.pixel(x, y).map_or(0, |[r, g, b]| luminance(r, g, b));
                hist[l as usize] += 1;
                lum.push(l);
            }
        }
        let bg = mode(&hist)?;

        let mut raw = vec![0u8; w * h];
        let mut ink = 0u32;
        for (i, &l) in lum.iter().enumerate() {
            if (l as f64 - bg as f64).abs() > INK_DELTA {
                raw[i] = 1;
                ink += 1;
            }
        }
        // 一块没有字的区域**不能**当身份：它的掩码全 0，
        // 和任何别的掩码比都是 0 分，于是「认不出」会被读成「换了人」——
        // 闸会莫名其妙地一直关着。所以这里直接返回 `None`。
        if ink < MIN_INK {
            return None;
        }

        Some(Self {
            mask: raw,
            w,
            h,
            ink,
            total: (w * h) as u32,
        })
    }

    /// 指纹覆盖的区域像素数。**探针用它说明取样范围**，
    /// 太小说明那块区域不足以当身份用。
    pub fn pixels(&self) -> u32 {
        self.total
    }

    /// 与另一个指纹的相似度，`0.0..=1.0`。
    ///
    /// 用 **Sørensen–Dice**（`2·重叠 / (墨A + 墨B)`）。
    ///
    /// ## 命中允许 ±[`SHIFT_TOLERANCE`] 像素的错位
    ///
    /// 锚点是按颜色分析推出来的，真机上不一定永远落在同一行；整块挪 1 px
    /// 在掩码上就是一大片不重叠。
    ///
    /// **为什么不先膨胀再比**：膨胀会把两边的墨迹面积**一起放大**，
    /// 于是 Dice 的分子分母同时变大，位移带来的那点错位占比被摊薄但没消失
    /// ——实测膨胀 1 px 只把 1 px 位移从 0.69 拉到 0.835，膨胀 2 px 才 0.863，
    /// 都还在 0.90 以下。而膨胀同时会把两个**不同**名字的重叠也拉起来
    /// （实测 0.2793 → 0.5483 → 0.6354），那是往危险方向走。
    ///
    /// 只放宽「命中」这一侧：分母仍是各自的真实墨迹数，不同名字不会因为
    /// 膨胀而「看起来更像」，而位移能被吸收。实测同一会话 1.0000、
    /// 不同会话 0.2793、整体挪 1 px 约 1.0。
    ///
    /// 区域尺寸不一致时直接给 0：尺寸变了说明锚点动了，
    /// 那是「认不出」，不该拿两片不同形状的掩码去凑一个相似度。
    pub fn similarity(&self, other: &Self) -> f32 {
        if self.ink == 0 || other.ink == 0 || self.mask.len() != other.mask.len() {
            return 0.0;
        }
        let (w, h) = (self.w, self.h);
        let b = &other.mask;
        let mut hit = 0u32;
        for y in 0..h {
            for x in 0..w {
                if self.mask[y * w + x] != 1 {
                    continue;
                }
                // 同一格，或上下左右斜角有一格墨就算命中。
                if near(b, w, h, x, y) {
                    hit += 1;
                }
            }
        }
        (2.0 * hit as f64 / (self.ink + other.ink) as f64) as f32
    }

    /// 两块区域是不是「同一个会话」。**阈值以下判为换人了。**
    pub fn is_same(&self, other: &Self) -> bool {
        self.similarity(other) >= SAME_THRESHOLD
    }

    /// **从帧里推导出头部行**，再取它的指纹。
    ///
    /// ## 这是什么
    ///
    /// [`of`] 要调用方**给一个矩形**。而那个矩形从哪来，本来一直是
    /// 调用方猜的——猜错三次（2026-10-14，见 `.scratch/…/31-*.md`）：
    /// 消息区顶部 / `chat.y0` 之上 / `chat.y0` 之下，全错，
    /// 而三次的测试都是绿的，因为夹具也按各自的错误假设画字块。
    ///
    /// 这里把「头部在哪」收进本模块：**从像素里推**，不让人告诉它。
    ///
    /// ## 怎么推
    ///
    /// 只看 `chat.x0` 右侧、消息区顶部往下 [`HEAD_SEARCH_H`] 的范围，
    /// 逐行统计「与本块众数亮度差得够远」的像素数。第一行**连续成行**
    /// 且行高在 [`MIN_ROW_H`]..=[`MAX_ROW_H`] 的，就是会话名那一行。
    ///
    /// **不用 OCR**：闸要赶在 OCR 之前跑（静默期零调用那条硬约束）。
    ///   这里只看**几何**（哪几行有墨），不读字。
    ///
    /// ## 高度必须稳定，否则闸会一直关着
    ///
    /// [`similarity`] 在两片掩码**尺寸不一致**时直接给 0。
    /// 所以推导出的高度不能逐帧乱变，否则每一帧都判成「换了人」。
    /// ⇒ 返回的高度**对齐到 [`ROW_QUANT`] 的整数倍**（行栅格），
    ///   实测真机上会话名高 11px → 落到 12px 那一档。
    ///
    /// 返回 `None` = **认不出**（顶部没找到成行的字）。调用方必须
    /// 按「认不出」处理——关闸、不起草——而不是当「是同一个」。
    ///
    /// ## 2026-10-20：只认**左对齐**的那一行
    ///
    /// 旧判据是「**最靠上**的、满足行高与墨数的那一行」。只要标题行
    /// 因为任何原因没被认出来，推导就会**一路掉到下面别的内容**上。
    /// 真机实测踩到过两次：
    ///
    /// · 「小号」（两个字）逐行墨点有 4 行低于阈值 ⇒ 标题被切碎
    ///   ⇒ 掉到了聊天区顶部**那张图片**的顶边（y 80..103）；
    /// · 于是闸认的是**图片**的像素 ⇒ 切走再切回来，闸**再也开不了**，
    ///   面板一直说「锁着别的对话，切回来才继续。」；
    /// · 名字 OCR 读的是那一行 ⇒ 面板把「小号」显示成 **「3 5」**。
    ///
    /// ⇒ 现在多一条**左对齐**判据（[`TITLE_LEFT_INK_X`]）：
    /// 会话名是消息区里**最靠左**的那行文字（实测距左沿 22px），
    /// 而图片/气泡都在 67px 以外。
    /// 认不出标题就**返回 `None`**，不许去认别的行——
    /// 「认错了」比「认不出」危险得多。
    pub fn discover(frame: &Frame, chat: Rect, width_cap: i32) -> Option<(Self, Rect)> {
        let band = head_search_rect(chat, width_cap);
        let rows = ink_rows(frame, band)?;
        // `find` 给的是 `&(i32, i32, u32, i32)`，闭包参数解构后是引用，
        // 所以要解一层。行高与墨数都要过闸，两条都要；
        // **左对齐**那条也要过（见上）。
        let &(top, bottom, _ink, _left) = rows.iter().find(|row| {
            let h = row.1 - row.0;
            (MIN_ROW_H..=MAX_ROW_H).contains(&h)
                && row.2 >= MIN_ROW_INK
                && row.3 - band.x0 <= TITLE_LEFT_INK_X
        })?;

        // 对齐到行栅格：同一行在不同帧上被推出 ±1px 时仍落到同一档，
        // 于是掩码尺寸稳定，`similarity` 不会因为高度抖动直接给 0。
        let h = quantize_row(bottom - top);
        let y0 = top;
        let rect = Rect::new(chat.x0, y0, chat.x0 + width_cap, y0 + h);
        Some((Self::of(frame, rect)?, rect))
    }
}

/// Rec.601 亮度。整数运算，返回 `0..=255`。
fn luminance(r: u8, g: u8, b: u8) -> u8 {
    ((299 * r as u32 + 587 * g as u32 + 114 * b as u32) / 1000).min(255) as u8
}

/// 推导头部时往下看的最大高度（整帧坐标，从消息区上沿起）。
///
/// 实测真机：会话名在 `chat.y0` 往下 18~28px（`chat.y0 = 31` ⇒ y 49..59），
/// 首条消息在 y 80。所以 72 够用。
///
/// ⚠⚠ **2026-10-19 订正下面那句「不会把消息行当成头部」——它成立，但靠的不是
/// 「72 < 80」这个距离，而是 `MIN_ROW_H` 那道闸。**
///
/// 实测（真实夹具 `wechat-dark-2side` / `-myside`，搜索带 y 31..103）：
///   首条消息行**确实落在带内**（`-myside` 实测 y 80..85），距离上沿只有 **49px**，
///   远小于 72。⇒ 「72 够用所以不会撞上消息行」这个推理**是错的**。
///
/// 真正挡住它的是行高：`find` 的判据要求 `MIN_ROW_H (6) <= h <= MAX_ROW_H (24)`，
/// 而那条消息行实测 **高 5px** ⇒ 不过闸 ⇒ `find` 返回 `None`。
///
/// ⇒ ⚠ **余量只有 1px。** 换一台 DPI、换一个微信版本、或消息行多一行墨迹被判成
///   一个成行块，它就可能爬过 6px 那道闸 ⇒ `discover` 认出一条**消息**当会话名，
///   而调用方会直接把它当 `peer` 显示。
///   这正是票 30「`peer` 被认成 `22：22`」那一类事故的形态。
///   ⇒ 修法不是调 `HEAD_SEARCH_H`（那会让消息行**更容易**进带），
///     而是给「头部行」加一条**它自己才有的**判据（位置上限或形态），
///     不能靠「消息行恰好不够高」这种余量 1px 的巧合。
const HEAD_SEARCH_H: i32 = 72;

/// 头部行的最小/最大行高。真机会话名实测高 **11px**。
const MIN_ROW_H: i32 = 6;
const MAX_ROW_H: i32 = 24;

/// 一行至少要有这么多墨像素才配当「一行字」。压掉了分隔线与图标边。
///
/// 取 **8** 是实测出来的：真机那行会话名每行的墨像素只有 **13~19**
/// （2026-10-14，微信 `0x3D02FC` 深色主题）。先取 20 时，笔画偏细的那类
/// 名字有整行达不到阈值，于是行段被砍断、`discover` 返回 `None`——
/// **不是「认不出」，是「行断得太碎」**。8 留得住真机的下沿，
/// 又远高于面板底色的逐行噪声（那几行实测墨点是 0）。
///
/// ⚠⚠ **2026-10-20：这条会把两个字的名字切碎。**
/// 会话名「小号」实测逐行墨点是 `12,6,12,11,6,21,8,9,17,4,4,12`——
/// 有 4 行低于 8（那是两个字各自的笔画间隙），于是标题被切成
/// 4 段（高 1/2/4/1 px），**每段都不到 [`MIN_ROW_H`]**。
/// ⇒ 标题被判废、`discover` 一路掉到下面别的行（真机上是聊天区顶部
/// 那张**图片**的顶边），把图片当成了会话身份：闸认的是图片，
/// 面板把「小号」显示成「3 5」。
/// ⇒ 修法是 [`ROW_GAP`]（允许笔画之间的空隙），**不是**把这条调小——
/// 调到 4 的话，背景噪声行也会被算成字。
const MIN_ROW_INK: u32 = 8;

/// **两段之间允许隔几行空行，仍算同一段。**
///
/// 2026-10-20 实测：两个字的名字「小号」逐行墨点
/// `12,6,12,11,6,21,8,9,17,4,4,12`，其中 4 行低于 [`MIN_ROW_INK`]（8）。
/// 那些低墨点的行**不是噪声，是字的笔画之间**——两个字的笔画交点少，
/// 有些扫描线上只擦到一两笔。
///
/// 取 2：实测最大空隙是 **2 行**（y 58..59），而「标题」与下一条内容
/// （真机上是 y 80 的图片顶边）之间隔着 **19 行**
/// ⇒ 2 足够接上笔画间隙，又绝不够把两条内容接成一段。
///
/// ⚠ 合并后的高度**不许超过 [`MAX_ROW_H`]**（见 `ink_rows`），
/// 否则标题会把下面 35px 见方的头像吸进来，整段超高而两头都认不出。
const ROW_GAP: usize = 2;

/// **会话名必须左对齐**：它的墨迹要落在消息区左沿往右这么多像素内。
///
/// 2026-10-20 实测（真机帧，消息区 x0 = 308）：
///
/// | 东西 | 最左墨迹 x | 距左沿 |
/// |---|---|---|
/// | 会话名「小号」 | 330 | **22** |
/// | 会话名「文件传输助手」 | 330 | **22** |
/// | 聊天区顶部那张图片 | 375 | 67 |
/// | 对方气泡 | 375 | 67 |
///
/// ⇒ 取 48：标题有余量（22 + 26），而图片/气泡（67）被排除在外。
///
/// ## 为什么必须有这条
///
/// 旧判据是「**最靠上**的那条满足行高与墨数的行」——只要标题行
/// 因为任何原因没被认出来（字太稀、位置变了、微信改版），
/// 推导就会**一路掉到下面别的内容**上，把图片/气泡当身份。
/// 那不是「认不出」，那是**认错了**——而闸一旦认错，
/// 就会拿别人的像素当「同一个会话」去放行。
///
/// ⇒ 认不出就**说认不出**（返回 `None` ⇒ 闸关着、面板说「没认出这是谁的对话」），
/// 这与本模块开头的立场一致：宁可「换人了但没认出来」，
/// 也不要「换了人却当成同一个」。
const TITLE_LEFT_INK_X: i32 = 48;

/// 头部高度对齐到这么粗的栅格。理由见 [`HeaderPrint::discover`]。
const ROW_QUANT: i32 = 4;

/// 推导要看的横向范围。右半边是空面板，纳入只会稀释。
fn head_search_rect(chat: Rect, width_cap: i32) -> Rect {
    let w = chat.width().min(width_cap).max(1);
    Rect::new(chat.x0, chat.y0, chat.x0 + w, chat.y0 + HEAD_SEARCH_H)
}

/// 把高度对齐到 [`ROW_QUANT`] 的整数倍，最小一档。
fn quantize_row(h: i32) -> i32 {
    // 2026-10-14：这里原先写的是 `div_euclid`，即**向下**取整。
    //   真机会话名高 11px ⇒ 11.div_euclid(4)*4 = **8**，把字形下沿裁掉 3px。
    //   而同一函数的文档注释明写「11px → 落到 12px 那一档」——
    //   **代码和注释对不上**，裁掉的正是指纹最该看的那几行笔画。
    //   改成向上取整：宁可多留 1px 底色，也不要切字。
    //   手写而不是 `div_ceil`：当前 toolchain 的 `int_roundings` 还不稳定
    //   （实测 `error[E0658]: use of unstable library feature`）。
    let q = (h.max(1) + ROW_QUANT - 1).div_euclid(ROW_QUANT) * ROW_QUANT;
    q.clamp(MIN_ROW_H, MAX_ROW_H)
}

/// 逐行数「与本块众数亮度差得够远」的像素，返回**连续成行**的
/// `(y0, y1, 墨像素数)`。
///
/// 背景取**整块**的众数而不是每行各自取：整块里绝大多数是面板底色，
/// 每行取自己的众数会把「一行稀疏的字」当成背景（字比底多不了几个）。
///
/// ## 2026-10-20 改：允许笔画之间的**空隙**（`ROW_GAP`）
///
/// ### 起因（真机实测，不是推断）
///
/// 会话名「**小号**」只有两个字，它的**逐行墨点数**实测是：
///
/// ```text
/// y 49..61： 12, 6, 12, 11, 6, 21, 8, 9, 17, 4, 4, 12
/// ```
///
/// 有 **4 行低于 [`MIN_ROW_INK`]（8）** —— 那是两个字各自的**笔画之间**。
/// 原来的判据是「**每一行**都要有墨才算成行」，于是标题被切成
/// 4 段（高 1 / 2 / 4 / 1 px），**每段都不到 [`MIN_ROW_H`]（6）**
/// ⇒ 标题被判废 ⇒ [`HeaderPrint::discover`] 一路掉到下面**别的行**
/// （真机上是聊天区顶部那张**图片**的顶边，y 80..103），
/// 把**图片**当成了会话身份。
///
/// 后果有两条，都实测到了：
/// · 闸认的是图片的像素 ⇒ 切走再切回来，闸**再也开不了**
///   （面板一直说「锁着别的对话，切回来才继续。」）；
/// · 名字 OCR 读的是那一行 ⇒ 面板把「小号」显示成 **「3 5」**。
///
/// ### 修法
///
/// 允许**跳过 ≤ [`ROW_GAP`] 行的空行**再接上下一段——
/// 「字有笔画间隙」是字形的常态，不是噪声。
/// ⚠ 但**合并后的高度不许超过 [`MAX_ROW_H`]**：否则标题会把它下面的
/// 头像（35px 见方）吸进来，整段变得超高而**两头都认不出**。
///
/// 6 个字的「文件传输助手」实测每行 23~54 墨，本来就连成一段，
/// 这条改动对它**零影响**（回归见同名测试）。
fn ink_rows(frame: &Frame, band: Rect) -> Option<Vec<(i32, i32, u32, i32)>> {
    let x0 = band.x0.max(0) as u32;
    let y0 = band.y0.max(0) as u32;
    let x1 = (band.x1.max(0) as u32).min(frame.width());
    let y1 = (band.y1.max(0) as u32).min(frame.height());
    if x1 <= x0 || y1 <= y0 {
        return None;
    }

    // 一趟取亮度 + 直方图。
    let mut lum = Vec::with_capacity((x1 - x0) as usize * (y1 - y0) as usize);
    let mut hist = [0u32; 256];
    for y in y0..y1 {
        for x in x0..x1 {
            let l = frame.pixel(x, y).map_or(0, |[r, g, b]| luminance(r, g, b));
            hist[l as usize] += 1;
            lum.push(l);
        }
    }
    let bg = mode(&hist)?;

    let w = (x1 - x0) as usize;
    let h = (y1 - y0) as usize;
    let mut out: Vec<(i32, i32, u32, i32)> = Vec::new();
    // 行段 = 一串「有墨的行」。两段之间若只隔着 ≤ `ROW_GAP` 行的空行，
    // **算同一段**（笔画间隙，见上面的实测）。
    //
    // 段的范围是 `[seg_start, last_ink + 1)`：段尾是**最后一行有墨的行**，
    // 不含它后面的空行。
    let mut seg_start: Option<usize> = None;
    let mut last_ink: usize = 0;
    let mut seg_ink: u32 = 0;
    // 段内**最靠左**的墨迹 x（帧坐标）。左对齐判据要用它。
    let mut seg_left: i32 = 0;
    for row in 0..h {
        let mut line_ink: u32 = 0;
        let mut line_left: i32 = i32::MAX;
        for c in 0..w {
            let l = lum[row * w + c];
            if (l as f64 - bg as f64).abs() > INK_DELTA {
                line_ink += 1;
                if line_left == i32::MAX {
                    line_left = (x0 as usize + c) as i32;
                }
            }
        }
        if line_ink < MIN_ROW_INK {
            continue;
        }
        if let Some(s) = seg_start {
            let gap = row - last_ink - 1;
            let merged_h = (row - s) as i32;
            // 接上：空行在允许范围内，且合并后仍然不超高。
            if gap <= ROW_GAP && merged_h <= MAX_ROW_H {
                seg_ink += line_ink;
                seg_left = seg_left.min(line_left);
                last_ink = row;
                continue;
            }
            // 接不上：上一段到此为止，另起一段。
            out.push((
                (y0 as usize + s) as i32,
                ((y0 as usize + last_ink + 1) as i32),
                seg_ink,
                seg_left,
            ));
        }
        seg_start = Some(row);
        last_ink = row;
        seg_ink = line_ink;
        seg_left = line_left;
    }
    if let Some(s) = seg_start {
        out.push((
            (y0 as usize + s) as i32,
            ((y0 as usize + last_ink + 1) as i32),
            seg_ink,
            seg_left,
        ));
    }
    out.retain(|&(a, b, _, _)| b > a);
    (!out.is_empty()).then_some(out)
}

/// 直方图的众数。整片全 0（不该发生）返回 `None`。
fn mode(hist: &[u32; 256]) -> Option<u8> {
    let mut best = 0u32;
    let mut at = 0usize;
    for (i, &v) in hist.iter().enumerate() {
        if v > best {
            best = v;
            at = i;
        }
    }
    (best > 0).then_some(at as u8)
}

/// `(x,y)` 自身或周围 [`SHIFT_TOLERANCE`] 像素内有没有墨。
fn near(mask: &[u8], w: usize, h: usize, x: usize, y: usize) -> bool {
    let r = SHIFT_TOLERANCE;
    let y0 = y.saturating_sub(r);
    let y1 = (y + r).min(h - 1);
    let x0 = x.saturating_sub(r);
    let x1 = (x + r).min(w - 1);
    for ny in y0..=y1 {
        for nx in x0..=x1 {
            if mask[ny * w + nx] == 1 {
                return true;
            }
        }
    }
    false
}

/// 小于这个像素数的区域不配当身份。太小的块连字都装不下。
const MIN_PIXELS: u32 = 200;

/// 与背景亮度差过这个值就算「字」。
///
/// 取 60 是因为实测那块区域里正文是 lum≈200、背景是 lum≈30，
/// 中间没有渐变层；60 卡在两侧中间，对主题整体深浅不敏感
/// （只要字与底仍差得开）。
const INK_DELTA: f64 = 60.0;

/// 墨迹少于这个像素数就当「这块里没有身份」。
const MIN_INK: u32 = 40;

/// 命中判定允许的错位半径（像素）。见 [`HeaderPrint::similarity`]。
const SHIFT_TOLERANCE: usize = 1;

/// 相似度阈值。
///
/// **这一条现在是实测出来的，不是猜的**（2026-10-11，真实微信帧）：
///
/// | 量 | 同一会话 | 不同会话 |
/// |---|---|---|
/// | 墨迹 Dice（不宽容错位） | 1.0000 | 0.2793 |
/// | **墨迹 Dice（±1 px 容错命中）** | **1.0000** | **0.2793** |
///
/// 不同名字的字形位置本来就不同，±1 px 的容错**不会**把它们拉近
/// （上表两行相同就是这个意思）；而整块挪 1 px 会被完全吸收。
/// 0.90 落在 0.28 与 1.00 之间，两侧都留了很宽的余量。
///
/// 方向仍然是保守的：宁可「换人了但没认出来」（闸多关一次，用户看得见），
/// 也不要「换了人但没认出来」（对着别人的聊天起草）。
pub const SAME_THRESHOLD: f32 = 0.90;

#[cfg(test)]
mod tests {
    use super::*;

    /// **消息行在 `HEAD_SEARCH_H` 带内，但它**过不了行高闸** ⇒ 认不出。**
    ///
    /// 这条锁的是 2026-10-19 实测出的形状（真实夹具 `-myside`）：
    /// 首条消息行在 `y 80..85`，搜索带是 `y 31..103` ⇒ **它在带内**，
    /// 距上沿只有 49px。所以「72 够用 ⇒ 不会撞上消息行」那个推理是**错的**。
    ///
    /// 真正挡住它的是 `MIN_ROW_H = 6` —— 那条消息行实测**高 5px**。
    ///
    /// ⚠⚠ **余量只有 1px。** 本测试的作用不是证明现在安全，而是：
    ///   哪天消息行变成 6px（换 DPI、换微信版本、墨迹多一行），
    ///   **这条会立刻红**，提醒该给「头部行」加一条它自己才有的判据，
    ///   而不是继续靠「恰好不够高」这种巧合。
    ///
    /// ⇒ 用**真实夹具的几何**（y 80、高 5），不凭空编数字：
    ///   编出来的场景只能证明编出来的场景。
    #[test]
    fn a_message_row_inside_the_head_band_is_rejected_by_row_height() {
        let w = 400u32;
        let h = 200u32;
        // 消息区上沿在 y=31，首条消息行在 y=80（真机实测），
        // 高 5px —— 比 `MIN_ROW_H` 少 1。
        let chat_top = 31i32;
        let msg_top = 80i32;
        let msg_h = 5i32;

        let f = frame_of(
            |x, y| {
                let y = y as i32;
                // 背景
                if (msg_top..msg_top + msg_h).contains(&y) && (20..300).contains(&x) {
                    [225, 225, 229]
                } else {
                    [30, 30, 31]
                }
            },
            w,
            h,
        );

        // 前提：这条消息行**确实在搜索带内** —— 若哪天 HEAD_SEARCH_H 被改小，
        // 本测试的前提就不再成立，那时它会安静地测另一件事。
        assert!(
            msg_top < chat_top + HEAD_SEARCH_H,
            "前提：消息行必须落在搜索带内（y {msg_top} < {}），否则本测试失去意义",
            chat_top + HEAD_SEARCH_H
        );

        // 前提：它确实只有 5px —— 余量只有 1px 这件事本身要被断言住。
        assert_eq!(
            msg_h,
            MIN_ROW_H - 1,
            "前提：消息行恰好比 MIN_ROW_H 少 1px。若夹具几何变了，本测试要重写"
        );

        let chat = Rect::new(0, chat_top, w as i32, h as i32);
        assert!(
            HeaderPrint::discover(&f, chat, w as i32).is_none(),
            "带内的消息行**被认成了会话名** ⇒ 界面会把消息正文当 peer 显示。\
             这是票 30「peer 被认成 22：22」那一类事故的形态。\n\
             修法不是调 HEAD_SEARCH_H（那只会让消息行更容易进带），\
             而是给「头部行」加一条它自己才有的判据。"
        );
    }

    /// **两个字的名字也要认得出**（2026-10-20 真机实测的形状）。
    ///
    /// ## 这条锁的是哪个事故
    ///
    /// 真机上会话名是「小号」（两个字），实测**逐行墨点数**是
    /// `12,6,12,11,6,21,8,9,17,4,4,12` —— 有 4 行低于 `MIN_ROW_INK`(8)。
    /// 那些行不是噪声，是**两个字的笔画之间**。
    ///
    /// 旧判据要求「每一行都有墨」，于是标题被切成 4 段（高 1/2/4/1 px），
    /// 每段都不到 `MIN_ROW_H`(6) ⇒ 标题被判废 ⇒ `discover` 一路掉到
    /// 聊天区顶部**那张图片**的顶边（y 80..103），把图片当成了会话身份。
    ///
    /// ## 后果（真机上都看到了）
    ///
    /// · 闸认的是**图片** ⇒ 切走再切回来，闸**再也开不了**，
    ///   面板一直说「锁着别的对话，切回来才继续。」；
    /// · 名字 OCR 读的是那一行 ⇒ 面板把「小号」显示成 **「3 5」**。
    ///
    /// ⇒ 夹具用**实测数字**：逐行墨点数、标题位置（y 49..61，最左墨迹
    /// x=330）、图片块位置（x 375 起、y 80..103）都照抄真机帧。
    /// 断言推出来的是**标题那一行**，不是下面那张图片。
    #[test]
    fn a_two_character_title_is_still_found_and_the_image_below_it_is_not() {
        let f = two_char_title_frame(true);
        let chat = Rect::new(
            REAL_CHAT_X0,
            REAL_CHAT_Y0,
            REAL_CHAT_X0 + 400,
            REAL_CHAT_Y0 + HEAD_SEARCH_H,
        );
        let (print, rect) = HeaderPrint::discover(&f, chat, 400)
            .expect("两个字的名字必须认得出来——它被切碎了就只剩下面的图片了");
        assert_eq!(
            rect.y0, 49,
            "推出来的不是标题那一行（y 49..61），而是 y {}..{}",
            rect.y0, rect.y1
        );
        assert_eq!(rect.y1, 61, "标题行的高度变了：{:?}", rect);
        assert!(print.pixels() > 0, "推出的那一块没有墨像素（{:?}）", rect);
    }

    /// **认不出标题就认不出，不许去认下面那块图片。**
    ///
    /// 这是上一条的**安全侧**：上一条保证「两个字的名字能认出来」，
    /// 这一条保证「**万一认不出来**」时不会退化成「认了别的行」。
    ///
    /// 旧判据是「最靠上的、满足行高与墨数的行」——标题一被判废，
    /// 推导就一路掉到图片上。图片的像素**会变**（滚动、重绘），
    /// 于是闸一会儿开一会儿关；更糟的是两个会话都带图片时，
    /// 闸会把**不同**的会话判成同一个。
    ///
    /// ⇒ 图像块在 x=375（距左沿 67px）⇒ 不是左对齐 ⇒ **不许**被当成会话名。
    /// 认不出就返回 `None`（闸关着、面板说「没认出这是谁的对话」）。
    #[test]
    fn a_block_that_is_not_left_aligned_is_never_taken_as_the_title() {
        let f = two_char_title_frame(false);
        let chat = Rect::new(
            REAL_CHAT_X0,
            REAL_CHAT_Y0,
            REAL_CHAT_X0 + 400,
            REAL_CHAT_Y0 + HEAD_SEARCH_H,
        );
        assert!(
            HeaderPrint::discover(&f, chat, 400).is_none(),
            "没有标题时，把下面那块图片当成了会话名 ⇒ 闸会拿图片的像素当身份，\
             换个会话它就认不出来了（这是「认错了」，比「认不出」危险得多）"
        );
    }

    /// 真机实测的「小号」**逐行墨点数**（2026-10-20，微信 4.0 深色主题）。
    ///
    /// 帧 1175x900，消息区 x0=308。标题在 y 49..61，最左墨迹 x=330。
    const TWO_CHAR_TITLE_INK: [(i32, u32); 12] = [
        (49, 12),
        (50, 6),
        (51, 12),
        (52, 11),
        (53, 6),
        (54, 21),
        (55, 8),
        (56, 9),
        (57, 17),
        (58, 4),
        (59, 4),
        (60, 12),
    ];
    const REAL_CHAT_X0: i32 = 308;
    const REAL_CHAT_Y0: i32 = 31;
    const REAL_TITLE_LEFT_X: i32 = 330;
    const REAL_IMAGE_X0: i32 = 375;
    const REAL_IMAGE_Y0: i32 = 80;
    const REAL_IMAGE_Y1: i32 = 103;

    /// 按实测数字造一帧：底色深、标题浅（逐行墨点数照抄），
    /// 聊天区顶部再放一块**右移的亮块**（真机上那张图片的位置）。
    ///
    /// `with_title = false` 时**不画标题**——用来验「认不出就不认」。
    fn two_char_title_frame(with_title: bool) -> Frame {
        let w = 800u32;
        let h = 200u32;
        frame_of(
            |x, y| {
                let x = x as i32;
                let y = y as i32;
                // 聊天区顶部那张图片：整块亮 ⇒ 每一行都有很多墨。
                if (REAL_IMAGE_Y0..REAL_IMAGE_Y1).contains(&y) && (REAL_IMAGE_X0..517).contains(&x)
                {
                    return [190, 190, 194];
                }
                if with_title {
                    for (ty, n) in TWO_CHAR_TITLE_INK {
                        if ty == y && x >= REAL_TITLE_LEFT_X && x < REAL_TITLE_LEFT_X + n as i32 {
                            return [225, 225, 229];
                        }
                    }
                }
                [30, 30, 31]
            },
            w,
            h,
        )
    }

    fn frame_of(f: impl Fn(u32, u32) -> [u8; 3], w: u32, h: u32) -> Frame {
        let mut px = vec![0u8; (w * h * 3) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&f(x, y));
            }
        }
        Frame::from_rgb(w, h, px).expect("尺寸一致")
    }

    /// 帧要比取样区**宽出余量**：早先把帧设成正好 300 宽，于是「往右挪 1 px」
    /// 的取样框被裁到帧边（299 宽），掩码长度与原框不同，相似度直接给 0——
    /// 位移测试假失败，而原因在夹具不在被测代码。
    const W: u32 = 400;
    const H: u32 = 200;
    const R: Rect = Rect::new(0, 0, 300, 40);

    /// 一块「写着名字」的头部。**字的比例照实测来。**
    ///
    /// 真实帧上那块区域 16000 px 里只有 **189 个墨迹像素（1.2%）**。
    /// 早先这里画的是 `x % 7 < 4` —— 那是在字行里铺满 4/7 的**条纹**，
    /// 覆盖率高出真实值几十倍。拿条纹当名字，会得到一个比现实更容易
    /// 分开的假象，也会把「膨胀」该有的作用掩盖掉。
    fn header(tint: u8) -> Frame {
        named(tint, 0)
    }

    /// 三个「字」块，宽 13 px。`style` 决定笔画怎么排 ——
    /// **真实会话名之间的差别就是这个，不是亮度。**
    ///
    /// 两个 style 是**同一个框里笔画落在不同位置**（左重 / 右重），
    /// 不是两套花纹：花纹之间会共享笔画，`±1 px` 的容错会把它们拉近，
    /// 那时候测的就不是「名字不同」而是「花纹相似」了。
    fn named(tint: u8, style: u32) -> Frame {
        frame_of(
            |x, y| {
                for sx in [24u32, 42, 60] {
                    // 必须用 checked 且**落在块内**：早先写 `saturating_sub`，
                    // 字块上方的每一行都被折成 cy=0，于是取样框往上挪一像素
                    // 就会凭空多出一片墨，位移测试因此假失败。
                    let Some(cx) = x.checked_sub(sx).filter(|v| *v < 13) else {
                        continue;
                    };
                    let Some(cy) = y.checked_sub(12).filter(|v| *v < 13) else {
                        continue;
                    };
                    let on = match style {
                        0 => cx < 7,  // 笔画压在左半
                        1 => cx >= 7, // 笔画压在右半
                        2 => cy < 7,  // 上重
                        _ => false,
                    };
                    if on {
                        return [tint, tint, tint];
                    }
                }
                [30, 30, 31]
            },
            W,
            H,
        )
    }

    /// **同一块区域，两次取指纹必须一致。** 这是整条路的地基。
    #[test]
    fn the_same_region_always_prints_the_same() {
        let f = header(225);
        let a = HeaderPrint::of(&f, R).expect("取得到");
        let b = HeaderPrint::of(&f, R).expect("取得到");
        assert_eq!(a, b);
        assert!(a.is_same(&b), "同一帧同一区域必须判为同一个");
    }

    /// **换个名字，指纹要明显不同 —— 而且名字的差别是「字形」不是「亮度」。**
    ///
    /// **这条测试在 2026-10-11 被整个换掉了。**
    ///
    /// 旧版是 `header(225)` 对 `header(120)`——亮度差 105 级。
    /// 真实会话名之间**亮度完全一样**（都是白字），差的是笔画布局；
    /// 于是旧版那条测试证明的全是与真实情况无关的东西，
    /// 一直是绿的，而闸在真实帧上从来没关过。
    ///
    /// 现在两边的字**亮度相同、形状不同**，这才是真实名字的差别。
    #[test]
    fn a_different_name_of_the_same_brightness_is_a_different_conversation() {
        let a = HeaderPrint::of(&named(225, 0), R).expect("取得到");
        let b = HeaderPrint::of(&named(225, 1), R).expect("取得到");
        assert!(
            !a.is_same(&b),
            "同亮度、不同字形的两个名字被判成同一个了：相似度 {}\
             （这是 2026-10-11 那个破口在单测里的样子）",
            a.similarity(&b)
        );
    }

    /// 整体亮度平移（未读角标、主题微调）**不该**被当成换人。
    ///
    /// 掩码只看「与背景差多少」，字整体变暗一点仍远高于 `INK_DELTA`。
    #[test]
    fn a_small_change_is_not_a_conversation_change() {
        let a = HeaderPrint::of(&header(225), R).expect("取得到");
        let b = HeaderPrint::of(&header(200), R).expect("取得到");
        assert!(
            a.is_same(&b),
            "只差 25 级灰不该判为换人，相似度 {}",
            a.similarity(&b)
        );
    }

    /// **框抖一两个像素不该判为换人。**
    ///
    /// 膨胀 1 px 就是为这件事存在的：掩码本身对平移敏感，
    /// 不膨胀的话字块平移 1 px 的重叠率会掉一半上下。
    ///
    /// 这里刻意**保持区域尺寸不变**（宽 300、高 40），只挪取样起点。
    /// 尺寸变了是另一回事，见下面那条。
    #[test]
    fn a_shifted_box_is_still_the_same_conversation() {
        let f = header(225);
        let a = HeaderPrint::of(&f, R).expect("取得到");
        let b = HeaderPrint::of(&f, Rect::new(1, 1, 301, 41)).expect("取得到");
        assert!(
            a.is_same(&b),
            "框抖 1px 被误判成换人了：{}",
            a.similarity(&b)
        );
    }

    /// **区域尺寸变了就该判为「认不出」**，不能拿两片不同形状的掩码凑相似度。
    ///
    /// 真机上尺寸会变只有一个原因：锚点动了（窗口改了大小）。
    /// 那时候闸关上是**对的**——宁可多关一次，也不要拿着旧形状去比。
    #[test]
    fn a_resized_box_is_not_the_same_conversation() {
        let f = header(225);
        let a = HeaderPrint::of(&f, R).expect("取得到");
        let b = HeaderPrint::of(&f, Rect::new(0, 0, W as i32, 41)).expect("取得到");
        assert!(
            !a.is_same(&b),
            "尺寸都变了还判成同一个：{}",
            a.similarity(&b)
        );
    }

    /// **浅色主题下也要能用**：暗字亮底，墨迹照样取得到。
    ///
    /// 背景取众数就是为了这条——写死亮度阈值的话整个浅色主题会失效，
    /// 而那种失效是**静默**的：掩码全 0 → 和谁都不像 → 闸一直关着。
    #[test]
    fn a_light_theme_header_still_prints() {
        let f = frame_of(
            |x, y| {
                if (12..=24).contains(&y) && x % 7 < 4 {
                    [40, 40, 45]
                } else {
                    [240, 240, 240]
                }
            },
            W,
            H,
        );
        let p = HeaderPrint::of(&f, R).expect("浅色主题也要取得到");
        assert!(p.ink > 0, "浅色主题下没取到任何墨迹——背景取法有问题");
    }

    /// **一片纯底色必须返回 `None`，不能给一个「空指纹」。**
    ///
    /// 空指纹会让「认不出」看起来像「换了人」，闸会莫名其妙一直关着。
    #[test]
    fn a_blank_region_prints_nothing() {
        let blank = frame_of(|_, _| [30, 30, 31], W, H);
        assert!(
            HeaderPrint::of(&blank, R).is_none(),
            "一片纯底色也给了指纹：它和任何区域比都是 0 分"
        );
    }

    /// **取不到指纹必须返回 `None`，不能给一个「空指纹」。**
    #[test]
    fn too_small_a_region_prints_nothing() {
        let f = header(225);
        assert!(HeaderPrint::of(&f, Rect::new(0, 0, 4, 4)).is_none());
        assert!(HeaderPrint::of(&f, Rect::new(0, 0, 0, 0)).is_none());
    }

    /// 越界要被裁到帧内，而不是 panic 或读到垃圾。
    #[test]
    fn an_out_of_bounds_rect_is_clamped_not_trusted() {
        let f = header(225);
        let p = HeaderPrint::of(&f, Rect::new(0, 0, 10_000, 10_000)).expect("应裁剪后取得到");
        assert!(p.pixels() > 0);
    }

    // ─────────────── discover（票 31：从像素推头部，不让人告诉它）───────────────

    /// 按**真机几何**造的帧：消息区上沿 `CHAT_Y0`，会话名行在其下 18px、高 11px。
    ///
    /// 那两个数来自 2026-10-14 真机测量（微信 `0x3D02FC`，深色主题）：
    /// `chat.y0 = 31`、会话名行 y 49..59、首条消息 y 80..97。
    /// 写在夹具里是**为了让测试接近现实**，不是把「头部在哪」写进代码——
    /// 推导的那一侧不知道这两个数。
    const CHAT_Y0: i32 = 31;

    /// 一帧：聊天区从 `chat.y0` 起，头部行画在 `chat.y0 + row_off`。
    /// `row_off` 变 ⇒ 模拟微信改版 / 布局微调。
    fn real_frame(row_off: i32, style: u32) -> Frame {
        frame_of(
            |x, y| {
                let y = y as i32;
                let top = CHAT_Y0 + row_off;
                if y >= top && y < top + 11 {
                    for sx in [24u32, 42, 60] {
                        let Some(cx) = x.checked_sub(sx).filter(|v| *v < 13) else {
                            continue;
                        };
                        let cy = (y - top) as u32;
                        let on = match style {
                            0 => cx < 7,
                            1 => cx >= 7,
                            _ => cy < 7,
                        };
                        if on {
                            return [225, 225, 225];
                        }
                    }
                }
                [30, 30, 31]
            },
            W,
            H,
        )
    }

    fn chat_rect() -> Rect {
        Rect::new(0, CHAT_Y0, 300, 200)
    }

    /// **头部行挪了位置，推导要跟着走。**
    ///
    /// 这条是票 31 的核心判据。它证明「头部在哪」是**推出来的**：
    /// 夹具把字块放在三个不同位置，三次都要推对。
    /// 改前两个位置都能过 ⇒ `discover` 里没有写死 y。
    ///
    /// 改前若 `discover` 用的是常量（比如「上沿往下 40px」），
    /// 第二个位置（18）和第三个（30）**至少有一个会推错或推不到**。
    #[test]
    fn discover_follows_the_header_line_wherever_it_moves() {
        for off in [18, 30, 8] {
            let f = real_frame(off, 0);
            let rows = ink_rows(&f, head_search_rect(chat_rect(), 300));
            println!("off=+{off} rows={rows:?}");
            let (print, rect) = HeaderPrint::discover(&f, chat_rect(), 300)
                .unwrap_or_else(|| panic!("头部行在 +{off} 时没推出来——它把位置写死了"));
            assert_eq!(
                rect.y0,
                CHAT_Y0 + off,
                "推出来的上边不对：头部行在 +{off}，推到 {}",
                rect.y0 - CHAT_Y0
            );
            assert!(print.pixels() > 0, "+{off} 那一行推出了一块没墨的区域");
        }
    }

    /// **推出来的高度要对齐栅格**，否则 `similarity` 会因尺寸不一致给 0，
    /// 闸会每帧都判成「换了人」。
    #[test]
    fn the_discovered_height_is_quantized_so_the_mask_size_is_stable() {
        // 两个高度差 1px 的头部行，推出来必须是**同一个**尺寸。
        let (a, ra) = HeaderPrint::discover(&real_frame(18, 0), chat_rect(), 300).unwrap();
        let (b, rb) = HeaderPrint::discover(&real_frame(19, 0), chat_rect(), 300).unwrap();
        assert_eq!(
            ra.height(),
            rb.height(),
            "行高差 1px 就推出不同尺寸 ⇒ 闸会每帧判成换人（{} vs {}）",
            ra.height(),
            rb.height()
        );
        // 高度差 1px 不该被当成换了会话
        assert!(
            a.is_same(&b),
            "头部只挪了 1px 却被判成换了会话，相似度 {}",
            a.similarity(&b)
        );
    }

    /// **量化只许向上，不许裁字。**
    ///
    ///这条是 2026-10-14 实测出来的：`quantize_row` 原先写 `div_euclid`，
    /// 即**向下**取整。真机会话名高 11px ⇒ 落到 **8px**，把字形下沿裁掉 3px。
    /// 而它的文档注释明写「11px → 落到 12px 那一档」——**代码和注释对不上**。
    ///
    /// **为什么裁字没被别的判据抓住**：实测把 `quantize_row` 退回 `div_euclid`
    ///   之后，`chat-servant-app` 侧 73 条测试**全绿**（含专门盯着像素量的那条）。
    ///   闸仍然关得对、仍然分得开两个会话——所以「少了 3px 笔画」在行为上
    ///   几乎看不出来。**判别力退化是静默的**，只能靠断言把它钉住。
    ///
    /// ⇒ 这里直接断言「推出的行高 ≥ 实际行高」，不钉具体数值：
    ///   钉死了改 `ROW_QUANT` 就得改测试，而那个量本来就该能调。
    #[test]
    fn quantization_may_round_up_but_must_never_crop_the_glyph() {
        // 真机行高 11（`real_frame` 里画的就是 `top..top+11`）。
        const REAL_ROW_H: i32 = 11;
        for off in [18, 30, 8] {
            let f = real_frame(off, 0);
            // 先从像素里量出**没量化之前**的真实行高。
            let raw = ink_rows(&f, head_search_rect(chat_rect(), 300))
                .expect("量得到行段")
                .iter()
                .find(|row| {
                    let h = row.1 - row.0;
                    (MIN_ROW_H..=MAX_ROW_H).contains(&h) && row.2 >= MIN_ROW_INK
                })
                .map(|row| row.1 - row.0)
                .expect("量得到头部那一行");
            assert_eq!(
                raw, REAL_ROW_H,
                "夹具画的是 {REAL_ROW_H}px，量出来却是 {raw}——量具本身偏了"
            );

            let (_print, rect) = HeaderPrint::discover(&f, chat_rect(), 300).expect("推出头部");
            assert!(
                rect.height() >= raw,
                "+{off}：行高 {raw} 被量化成 {} —— 向下取整把字形裁掉了\
                 （宁可多留 1px 底色，也不要切字）",
                rect.height()
            );
            // 向上取整 ⇒ 最多多留 ROW_QUANT-1 px，不能多到把下一行也吃进来。
            assert!(
                rect.height() < raw + ROW_QUANT,
                "+{off}：行高 {raw} 被量化成 {} —— 多留了太多，下一行的笔画会被吃进来",
                rect.height()
            );
        }
    }

    /// **认不出必须返回 `None`**，而不是随便给一块指纹。
    ///
    /// 这是**失败方向**：返回 `None` ⇒ 闸按「认不出」处理 ⇒ 关闸不起草。
    /// 若这里给了一个空/错指纹，闸会以为「还是同一个人」而继续动手——
    /// 那就正好撞在硬约束第 1 条上。
    #[test]
    fn discover_returns_none_when_there_is_no_header_line() {
        let blank = frame_of(|_, _| [30, 30, 31], W, H);
        assert!(
            HeaderPrint::discover(&blank, chat_rect(), 300).is_none(),
            "一片纯底色也推出了头部——认不出时必须返回 None，让闸关着"
        );
    }

    /// **同一会话、头部位置微移 ⇒ 仍然判为同一个**（闸要能扛布局抖动）。
    #[test]
    fn discover_is_stable_across_a_small_layout_shift() {
        let a = HeaderPrint::discover(&real_frame(18, 0), chat_rect(), 300)
            .unwrap()
            .0;
        let b = HeaderPrint::discover(&real_frame(21, 0), chat_rect(), 300)
            .unwrap()
            .0;
        assert!(
            a.is_same(&b),
            "布局挪了 3px 就判成换人了，相似度 {}",
            a.similarity(&b)
        );
    }

    /// **换个会话，推出来的指纹必须不同**——推导对了，判别力不能丢。
    #[test]
    fn discover_still_tells_two_conversations_apart() {
        let a = HeaderPrint::discover(&real_frame(18, 0), chat_rect(), 300)
            .unwrap()
            .0;
        let b = HeaderPrint::discover(&real_frame(18, 1), chat_rect(), 300)
            .unwrap()
            .0;
        assert!(
            !a.is_same(&b),
            "两个字形不同的会话被判成同一个——推导对了但判别力丢了，相似度 {}",
            a.similarity(&b)
        );
    }
}
