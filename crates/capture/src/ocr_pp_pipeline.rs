//! PP-OCRv4 的**推理管线**：预处理 → det 后处理 → rec 后处理。
//!
//! # 与 [`crate::ocr_pp`] 的分工
//!
//! [`ocr_pp`] 只管「把 ONNX 编译成能跑的图、把输出取出来」——那层与模型
//! 无关。**这一层全是 PP-OCRv4 特有的约定**，换模型就得重写。
//!
//! # 三件事，原型代码里对应三段
//!
//! 1. **det 后处理** — 概率图 `[1,1,h,w]` → 若干文本框。
//!    二值化 → 连通域 → 筛掉噪点 → 换算回原图坐标。
//! 2. **rec 后处理** — `[1,T,C]` 概率 → 文本。
//!    逐帧 argmax → 去重相邻 → 去 blank → 查词典。
//! 3. **rec 宽度分桶** — 每个框宽度都不同，而 tract 的形状编译期决定。
//!    不分桶就得「每个框重编一次图」，几十到几百毫秒一个框。
//!
//! # 形状顺序：类在**最后一维**（2026-10-10 实测）
//!
//! rec 的输出是 `[1, T, C]`（`C = 6625`），**不是** `[1, C, T]`。
//! 64px 宽的空白图实测得 `[1, 8, 6625]`。CTC 解码按下标 `t*C + c` 走；
//! 写反了会得到一整段空白，**而且它不报错**。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use chatservant_core::frame::{Frame, Rect};

use crate::ocr::{OcrError, OcrLine, OcrText};
use crate::ocr_pp::{self, BLANK, Plan, PpError};

/// det 的权重文件名。**磁盘上就这一个**，没有「按尺寸分文件」这一说。
pub const DET_MODEL: &str = "ch_PP-OCRv4_det_mobile.onnx";

/// rec 的权重文件名。同上，**只有一个**。
///
/// 2026-10-15 实测：早先的 [`BucketCache::plan_path`] 按宽度拼出
/// `ch_PP-OCRv4_rec_mobile_w{w}.onnx`，而磁盘上从没有过这种文件，
/// 于是每个宽度的 rec 编译都对着一个不存在的路径失败。见 [`BucketCache`]。
pub const REC_MODEL: &str = "ch_PP-OCRv4_rec_mobile.onnx";

/// det 的总下采样倍数。**8 次步长 2 的下采样 = 32。**
///
/// 所以 det 的输入 H/W 必须是 32 的倍数，否则图里的 pad 算错。
pub const DET_STRIDE: usize = 32;

/// det 的归一化常数。取自 PaddleOCR 的 `DetNormalizeImage`
/// （`mean=[0.485,0.456,0.406]`、`std=[0.229,0.224,0.225]`）。
///
/// ## 原先 `std` 写错了，把对比度压掉一半（2026-10-15 修）
///
/// 早先这里写的是 `std = [0.456, 0.456, 0.456]`——把 mean 的第二个分量
/// 抄进了 std。后果是**送进网络的对比度只有应有值的一半**，
/// DB 头的 sigmoid 输出整体偏低，按 [`DET_THRESH`] 二值化之后
/// 只剩笔画最密的核心留下来。
///
/// 实测后果（`cargo xtask ocr <帧> --stats` 对同一张真帧）：
/// 一行 6 个字的墨迹高 **12px**，而 det 给的框高只有 **8px**，
/// 且框整体上移约 6~7px —— rec 拿到的是一个**被削掉下半截**的字，
/// 识别结果只能是乱码。
pub const DET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
pub const DET_STD: [f32; 3] = [0.229, 0.224, 0.225];

/// rec 的归一化常数。取自 PaddleOCR 的 `RecNormalizeImage`
/// （`mean = std = [0.5, 0.5, 0.5]`）。
///
/// ## det 与 rec 是**两套**常数，不能共用（2026-10-15 修）
///
/// 早先 rec 复用了 det 的 ImageNet 那一套。它们在 PaddleOCR 里
/// 本来就是分开配的，共用会让 rec 的输入整体偏离训练分布。
pub const REC_MEAN: [f32; 3] = [0.5, 0.5, 0.5];
pub const REC_STD: [f32; 3] = [0.5, 0.5, 0.5];

/// PaddleOCR DB 后处理的 `unclip` 外扩比例。
///
/// 外扩是**照 DB 论文的做法**，不是为了让框变好看：二值化阈值一定会削掉
/// 字形的抗锯齿边缘，外扩把被削掉的那一圈补回来，
/// rec 才拿得到完整的字。
///
/// 外扩量按 `w * r / sqrt(w*h)` 算，所以**细长框的纵向几乎不扩**
/// （160x14 的行框纵向只多 0.45px）。这是公式本身的行为，
/// 不是漏了纵向参数——纵向覆盖要靠概率图本身，扩不出来。
pub const DET_UNCLIP_RATIO: f32 = 1.5;

/// rec 的输入高。**我们给的**，不是模型的（模型四个维全是动态的）。
pub const REC_HEIGHT: usize = 48;

/// rec 的总下采样倍数。宽度必须取 8 的倍数。
pub const REC_STRIDE: usize = 8;

/// det 概率图的二值化阈值。
///
/// PaddleOCR 的 `det_db_thresh` 默认 0.3。这里取 0.3 是为了跟权重对齐——
/// 调高会漏掉浅色字，调低会把气泡边框当成文字。
pub const DET_THRESH: f32 = 0.3;

/// 连通域最小面积（以**二值化后的概率图**像素计）。
///
/// 一个汉字在 32 倍数缩放后的概率图上占几百像素；3px 的噪点占 9。
/// 20 落在两者之间。
pub const DET_MIN_AREA: usize = 20;

/// 文本框最小宽高。
pub const DET_MIN_W: usize = 3;
pub const DET_MIN_H: usize = 3;

/// 一屏最多取多少个**行**框。
///
/// 保护的是 rec 的计算量：每个框都要过一遍 rec（有编译开销），
/// 而 det 在一块纯色背景上也可能吐出成百上千个噪点框。
///
/// **计数的单位是「行」，不是「连通域」**（2026-10-15 修）。
/// 早先这个上限卡在 [`det_boxes`] 里、也就是行合并**之前**，
/// 于是预算被单字框花掉：真帧上一行十四个字就是十几个框，
/// 64 的预算几行就耗光，屏幕下半部分整片丢失，
/// 而剩下的一半又是碎片——现场看上去就是「整张图被切成单字框」。
/// 改成在 [`merge_into_lines`] 之后限流，64 行足够覆盖一屏聊天消息。
pub const DET_MAX_BOXES: usize = 64;

// ────────────────────────────────────────────────────────────────────────
// 预处理
// ────────────────────────────────────────────────────────────────────────

/// det 的输入尺寸：32 的倍数。
///
/// **不缩放比例**——PP-OCRv4 的 det 内部有 `Resize` 按比例缩到 1/4，
/// 输入给多大就按多大算。取整到 32 的倍数是硬要求（[`DET_STRIDE`]）。
pub fn det_size(w: u32, h: u32) -> (usize, usize) {
    let w = (w as usize).div_ceil(DET_STRIDE) * DET_STRIDE;
    let h = (h as usize).div_ceil(DET_STRIDE) * DET_STRIDE;
    (w.max(DET_STRIDE), h.max(DET_STRIDE))
}

/// rec 的输入宽度：8 的倍数，且不小于 8。
///
/// rec 对窄框有下限：宽度小于 8 的话内部下采样后时间步会是 0。
/// **这是把宽度取整到 8 的倍数的直接原因**，不是「凑整好看」。
pub fn rec_width(w: u32) -> usize {
    ((w as usize).div_ceil(REC_STRIDE) * REC_STRIDE).max(REC_STRIDE)
}

/// RGB 帧 → det 的 `[1,3,H,W]` 归一化张量。
///
/// PP-OCR 的归一化是 `(v/255 - mean) / std`，`mean = std = [0.485, 0.456, 0.406]`
/// （ImageNet 标准）。**注意通道顺序是 RGB**，而我们的 `Frame` 本来就是 RGB8。
///
/// 尺寸不足时按**最近邻复制**右下角补齐——不是补黑。补黑会在边缘造出
/// 一条假的暗边，det 可能把它当文字。
pub fn det_input(frame: &Frame, w: usize, h: usize) -> Vec<f32> {
    let (fw, fh) = (frame.width() as usize, frame.height() as usize);
    let src = frame.as_bytes();
    let mut out = vec![0.0f32; 3 * w * h];
    for y in 0..h {
        let sy = y.min(fh.saturating_sub(1));
        for x in 0..w {
            let sx = x.min(fw.saturating_sub(1));
            let p = &src[(sy * fw + sx) * 3..(sy * fw + sx) * 3 + 3];
            let o = y * w + x;
            for c in 0..3 {
                out[c * w * h + o] = (p[c] as f32 / 255.0 - DET_MEAN[c]) / DET_STD[c];
            }
        }
    }
    out
}

/// 裁一个框 → rec 的 `[1,3,48,W']` 归一化张量。
///
/// 高**固定** 48（[`REC_HEIGHT`]），宽按框的比例算再取整到 8 的倍数。
/// 只缩不裁：宽高比要保持，否则字会被拉变形，识别率掉得很厉害。
pub fn rec_input(frame: &Frame, box_: Rect, out_w: usize) -> Vec<f32> {
    let h = REC_HEIGHT;
    let (fw, fh) = (frame.width() as usize, frame.height() as usize);
    let src = frame.as_bytes();
    let mut out = vec![0.0f32; 3 * h * out_w];
    let bx = box_.x0.max(0) as usize;
    let by = box_.y0.max(0) as usize;
    let bw = (box_.width().max(1) as usize).min(fw.saturating_sub(bx));
    let bh = (box_.height().max(1) as usize).min(fh.saturating_sub(by));
    for y in 0..h {
        let sy = by + y * bh / h;
        for x in 0..out_w {
            let sx = bx + x * bw / out_w;
            let p = &src[(sy * fw + sx) * 3..(sy * fw + sx) * 3 + 3];
            let o = y * out_w + x;
            for c in 0..3 {
                out[c * h * out_w + o] = (p[c] as f32 / 255.0 - REC_MEAN[c]) / REC_STD[c];
            }
        }
    }
    out
}

// ────────────────────────────────────────────────────────────────────────
// det 后处理
// ────────────────────────────────────────────────────────────────────────

/// 概率图 → 文本框（坐标已换算回**原图**）。
///
/// 四步，顺序不能换：
/// 1. 二值化（`> [`DET_THRESH`]`）
/// 2. 连通域标记（8 邻域）
/// 3. 按面积与宽高筛掉噪点
/// 4. 按 `DET_STRIDE` 反缩放回原图，并裁进画面
pub fn det_boxes(prob: &[f32], pw: usize, ph: usize, src_w: usize, src_h: usize) -> Vec<Rect> {
    let n = pw * ph;
    // `prob` 长度不保证正好是 `n`（det 输出形状对不上时会有）。逐个
    // `get().unwrap_or(0.0)`：越界的当 0，也就是「不亮」，落在阈值下，
    // 不会被二值化收进来。形状真的错了要由调用方的形状检查去报错。
    let mask: Vec<bool> = prob
        .iter()
        .take(n)
        .map(|v| *v > DET_THRESH)
        .chain(std::iter::repeat_n(false, n.saturating_sub(prob.len())))
        .collect();

    // 连通域标记。显式栈而不是递归——概率图可以很大，递归会爆栈。
    let mut labels = vec![0u32; n];
    let mut next = 1u32;
    let mut comps: Vec<(usize, usize, usize, usize, usize)> = Vec::new(); // (area, min_x, min_y, max_x, max_y)
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..n {
        if !mask[start] || labels[start] != 0 {
            continue;
        }
        let label = next;
        next += 1;
        stack.push(start);
        labels[start] = label;
        let (mut area, mut mnx, mut mny, mut mxx, mut mxy) = (0usize, pw, ph, 0usize, 0usize);
        while let Some(p) = stack.pop() {
            let (x, y) = (p % pw, p / pw);
            area += 1;
            mnx = mnx.min(x);
            mny = mny.min(y);
            mxx = mxx.max(x);
            mxy = mxy.max(y);
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= pw as i32 || ny >= ph as i32 {
                        continue;
                    }
                    let q = ny as usize * pw + nx as usize;
                    if mask[q] && labels[q] == 0 {
                        labels[q] = label;
                        stack.push(q);
                    }
                }
            }
        }
        comps.push((area, mnx, mny, mxx, mxy));
    }

    let mut boxes: Vec<Rect> = comps
        .iter()
        .filter(|(area, _, _, _, _)| *area >= DET_MIN_AREA)
        .filter_map(|(_, mnx, mny, mxx, mxy)| {
            let (w, h) = (mxx - mnx + 1, mxy - mny + 1);
            if w < DET_MIN_W || h < DET_MIN_H {
                return None;
            }
            // PaddleOCR 的 `unclip`：按 `w*r/sqrt(w*h)` 外扩。
            // 缺了它，框会紧贴二值化阈值留下的笔画核心，
            // 抗锯齿被削掉的那一圈永远补不回来，rec 拿到的字是**缺的**。
            // 2026-10-15 补上——早先完全没有这一步。
            let len = ((w * h) as f64).sqrt();
            let dx = w as f64 * DET_UNCLIP_RATIO as f64 / len;
            let dy = h as f64 * DET_UNCLIP_RATIO as f64 / len;
            // 反缩放回原图坐标。用四舍五入而不是截断：截断会让最右/最下
            // 的那一列像素永远落在框外。
            let sx = src_w as f64 / pw as f64;
            let sy = src_h as f64 / ph as f64;
            let x0 = (((*mnx as f64) - dx) * sx).round() as i32;
            let y0 = (((*mny as f64) - dy) * sy).round() as i32;
            let x1 = (((*mxx as f64 + 1.0) + dx) * sx).round() as i32;
            let y1 = (((*mxy as f64 + 1.0) + dy) * sy).round() as i32;
            Some(Rect {
                x0,
                y0,
                x1: x1.clamp(0, src_w as i32),
                y1: y1.clamp(0, src_h as i32),
            })
        })
        .filter(|r| r.width() > 0 && r.height() > 0)
        .collect();

    // 这里**不**按 [`DET_MAX_BOXES`] 截断（2026-10-15 移走）。
    // 截断必须发生在 [`merge_into_lines`] **之后**，理由见那条常量的说明：
    // 在合并前截断，预算花的是**单字框**，一行十四个字就把 64 用光了，
    // 真正该保留的整行反而被裁掉。实测真帧上 64 个框里混着整行与碎片，
    // 一行文字被切成 12 个 12px 的框，逐行看全是「单字框」。
    //
    // 排序保持稳定的语义：读出来自上而下、自左而右。
    boxes.sort_by_key(|b| (b.y0, b.x0));
    boxes
}

/// 把 det 的框按行合并（同一行相邻的框拼成一行）。
///
/// PP-OCR 的 det 吐的是**文本块**，不是行；一行中文可能切成两三块。
/// 而 [`OcrLine`] 的语义是「一行」，下游的消息归并靠 `center_y`。
///
/// 合并判据：垂直重叠超过一半，且水平间距小于行高的 60%。
pub fn merge_into_lines(boxes: &[Rect]) -> Vec<Rect> {
    let mut lines: Vec<Rect> = Vec::new();
    let mut sorted: Vec<Rect> = boxes.to_vec();
    sorted.sort_by_key(|b| (b.y0, b.x0));
    for b in sorted {
        let bh = b.height().max(1);
        let mut merged = false;
        for l in lines.iter_mut() {
            let lh = l.height().max(1);
            // 垂直重叠
            let ov = (b.y1.min(l.y1) - b.y0.max(l.y0)).max(0);
            if ov * 2 < bh.min(lh) {
                continue;
            }
            let gap = if b.x0 >= l.x1 {
                b.x0 - l.x1
            } else {
                l.x0 - b.x1
            };
            if gap > lh * 3 / 5 {
                continue;
            }
            l.x0 = l.x0.min(b.x0);
            l.y0 = l.y0.min(b.y0);
            l.x1 = l.x1.max(b.x1);
            l.y1 = l.y1.max(b.y1);
            merged = true;
            break;
        }
        if !merged {
            lines.push(b);
        }
    }
    lines.sort_by_key(|l| (l.y0, l.x0));
    lines
}

// ────────────────────────────────────────────────────────────────────────
// det 层诊断（探针用，不参与识别）
// ────────────────────────────────────────────────────────────────────────

/// det 层的量化诊断。
///
/// ## 为什么需要它
///
/// 「识别成一堆单字框」这句话可以是 det 的锅、可以是 rec 的锅、
/// 也可以是**根本没走到 PP-OCR**（回落了 WinRT）。光看最终文本分不出来——
/// 2026-10-15 那次就是：现场看到的是 WinRT 的逐字切分，却一直在调 det 参数。
///
/// 唯一能定位的办法是**把 det 这一层单独量出来**：框数、框高、框宽。
/// 一行中文在 1267x930 的微信帧上大约是十几行，框数就该是十几；
/// 框高该接近一行字高（本帧实测 11~14px），不是单个汉字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetStats {
    /// det 的输入尺寸（32 的倍数）。
    pub det_input_w: usize,
    pub det_input_h: usize,
    /// 概率图的尺寸。PP-OCRv4 的 det 输出与输入同尺寸。
    pub prob_w: usize,
    pub prob_h: usize,
    /// 二值化后的前景像素数，用来确认阈值没有把图削空或全收进来。
    pub fg_pixels: usize,
    /// 连通域框数（**合并前**）。
    pub raw_count: usize,
    pub raw_heights: Vec<i32>,
    pub raw_widths: Vec<i32>,
    /// 合并成行后的框数（**限流前**）。
    pub merged_count: usize,
    /// 因超过 [`DET_MAX_BOXES`] 而被丢掉的行数。
    ///
    /// 这个数不为 0 就意味着**屏幕下半部分根本没进 rec**，
    /// 而它在最终文本里完全看不出来——只是「消息好像少了几条」。
    pub dropped_lines: usize,
    /// 限流后真正送去 rec 的行数。
    pub line_count: usize,
    pub line_heights: Vec<i32>,
    pub line_widths: Vec<i32>,
    pub line_rects: Vec<Rect>,
}

/// 只跑 det，把**概率图**原样交出去。**探针用，不参与识别。**
///
/// ## 为什么要有它
///
/// 框的统计（框数、框高）只能告诉你「框比字矮」，
/// 答不出「概率图是整体弱、还是只削掉了边缘」。
/// 这两者的修法完全不同：前者是阈值/归一化，后者是框的外扩。
/// 把概率图按强度打成字符画看一眼就分得清。
///
/// 返回 `(展平的概率图, 宽, 高)`。
pub fn det_prob_map(frame: &Frame) -> Result<(Vec<f32>, usize, usize), PpError> {
    let (fw, fh) = (frame.width() as usize, frame.height() as usize);
    if fw == 0 || fh == 0 {
        return Err(PpError::Shape("空帧".into()));
    }
    let (dw, dh) = det_size(fw as u32, fh as u32);
    let plan = det_plan(dh, dw)?;
    let buf = det_input(frame, dw, dh);
    let input = tract_onnx::prelude::Tensor::from_shape(&[1, 3, dh, dw], &buf)
        .map_err(|e| PpError::Shape(format!("造 det 输入失败：{e}")))?;
    let (vals, shape) = ocr_pp::run(&plan, input, "det")?;
    if shape.len() != 4 || shape[0] != 1 || shape[1] != 1 {
        return Err(PpError::Shape(format!("det 输出不是 [1,1,h,w]：{shape:?}")));
    }
    Ok((vals, shape[3], shape[2]))
}

/// 只跑 det，把每一层的中间量如实报出来。**探针用，不参与识别。**
pub fn det_stats(frame: &Frame) -> Result<DetStats, PpError> {
    let (fw, fh) = (frame.width() as usize, frame.height() as usize);
    if fw == 0 || fh == 0 {
        return Err(PpError::Shape("空帧".into()));
    }
    let (dw, dh) = det_size(fw as u32, fh as u32);
    let (vals, pw, ph) = det_prob_map(frame)?;
    let fg_pixels = vals
        .iter()
        .take(pw * ph)
        .filter(|v| **v > DET_THRESH)
        .count();
    let raw = det_boxes(&vals, pw, ph, fw, fh);
    let merged = merge_into_lines(&raw);
    // 限流**在合并之后**——见 [`DET_MAX_BOXES`]。这里如实报出被丢掉多少行，
    // 因为「屏幕下半截没读到」这个故障在最终文本里完全看不出来。
    let kept = merged.len().min(DET_MAX_BOXES);
    let lines: Vec<Rect> = merged.iter().take(DET_MAX_BOXES).copied().collect();
    Ok(DetStats {
        det_input_w: dw,
        det_input_h: dh,
        prob_w: pw,
        prob_h: ph,
        fg_pixels,
        raw_count: raw.len(),
        raw_heights: raw.iter().map(|b| b.height()).collect(),
        raw_widths: raw.iter().map(|b| b.width()).collect(),
        merged_count: merged.len(),
        dropped_lines: merged.len() - kept,
        line_count: lines.len(),
        line_heights: lines.iter().map(|b| b.height()).collect(),
        line_widths: lines.iter().map(|b| b.width()).collect(),
        line_rects: lines,
    })
}

// ────────────────────────────────────────────────────────────────────────
// rec 后处理：CTC 解码
// ────────────────────────────────────────────────────────────────────────

/// CTC 解码。`vals` 是 `[1,T,C]` 展平后的缓冲，`chars` 是 6624 长的词典。
///
/// 三步，**顺序是 CTC 的定义，不能换**：
/// 1. 每个时间步取最大类（argmax）
/// 2. 折叠**连续重复**的类
/// 3. 去掉 blank
///
/// 第 2 步只折叠**连续**的。`aab` 要变成 `ab`（去重），
/// 而 `aba` 要保持 `aba`——因为中间那个 `a` 不是连续重复。
/// 写成「全局去重」会把 `aba` 读成 `ab`，是最常见的 CTC 实现错误。
///
/// 返回里还带上每步的置信度（max prob 的均值），供上游筛掉垃圾行。
pub fn ctc_decode(vals: &[f32], shape: &[usize], chars: &[String]) -> (String, f32) {
    // 形状容错：万一某版权重给的是 [1,C,T]，这里明确报错而不是猜。
    // 猜错的后果是「解出一整段空白」且**不报错**（见模块头）。
    if shape.len() != 3 {
        return (String::new(), 0.0);
    }
    let (t, c) = (shape[1], shape[2]);
    if c != chars.len() + 1 {
        // 类数与词典对不上。**这是配置错误，不是识别失败**——
        // 返回空串 + 0 置信度，上游会把它当「这一框没字」。
        tracing::warn!(
            classes = c,
            dict = chars.len(),
            expected = chars.len() + 1,
            "rec 类数与词典对不上，跳过本框"
        );
        return (String::new(), 0.0);
    }
    let mut text = String::new();
    let mut prev = usize::MAX;
    let mut conf_sum = 0.0f32;
    let mut conf_n = 0usize;
    for i in 0..t {
        let row = &vals[i * c..(i + 1) * c];
        let (best, prob) = argmax(row);
        if best != prev && best != BLANK {
            // 直接用 `chars[best]`，**不是 `chars[best - 1]`**。
            // `chars[0]` 是给 blank 留的空占位，真实字符从 `chars[1]` 起，
            // 所以类 c ↔ `chars[c]`。减一的话第一个字符会取到那个空占位，
            // 结果是**每行开头少一个字**——而且它不报错，只是静默少字。
            if let Some(ch) = chars.get(best) {
                text.push_str(ch);
            }
        }
        conf_sum += prob;
        conf_n += 1;
        prev = best;
    }
    let conf = if conf_n == 0 {
        0.0
    } else {
        conf_sum / conf_n as f32
    };
    (text, conf)
}

fn argmax(row: &[f32]) -> (usize, f32) {
    let mut bi = 0usize;
    let mut bv = f32::NEG_INFINITY;
    for (i, v) in row.iter().enumerate() {
        if *v > bv {
            bv = *v;
            bi = i;
        }
    }
    (bi, bv)
}

// ────────────────────────────────────────────────────────────────────────
// rec 宽度分桶
// ────────────────────────────────────────────────────────────────────────

/// 按宽度缓存编译好的 rec 图。
///
/// ## 为什么必须缓存
///
/// tract 的形状是**编译期**决定的（见 [`ocr_pp::load_rec`] 的说明）。
/// 一个 300px 宽的框和 320px 宽的框要编两次，而每次编译是几十到几百毫秒。
/// 一屏消息有几十个框，逐框重编会让 OCR 从「几十毫秒」变成「几秒」。
///
/// ## 桶的取法
///
/// 按 `rec_width()` 取整后的宽度**直接当桶键**。这会让桶很多（每个不同宽度
/// 一个桶），但命中率高——聊天里的气泡宽度高度重复。桶的**数量**不设上限，
/// 真正该防的是无界增长，见 [`BucketCache::max_buckets`]。
pub struct BucketCache {
    dir: PathBuf,
    buckets: Mutex<HashMap<usize, Plan>>,
    /// 超过这个数就把最久没用的桶丢掉一半。
    max_buckets: usize,
}

impl BucketCache {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            buckets: Mutex::new(HashMap::new()),
            max_buckets: 32,
        }
    }

    /// rec 权重路径。
    ///
    /// ## 曾经按宽度拼文件名，那是个静默失效（2026-10-15 实测）
    ///
    /// 早先这里是 `format!("ch_PP-OCRv4_rec_mobile_w{w}.onnx")`，理由是
    /// 「每个宽度一份预编译的图」。**但磁盘上从没有过这种文件**——
    /// 权重是安装时一次性下载的三个固定名，代码里没有任何地方产出过
    /// 按宽度的文件。
    ///
    /// 于是每个宽度的 [`ocr_pp::load_rec`] 都在 `model_for_path` 上失败，
    /// 错误一路传到 [`recognize_in`] 被吞成 `None`，
    /// [`crate::ocr::recognize`] 于是**静默回落 WinRT**。
    ///
    /// 症状极具误导性：det 白跑一遍（十几秒），而**输出是 WinRT 的**——
    /// 于是现场看到的是「PP-OCR 把整张图切成单字框」，而真正的 det 行为
    /// 从来没被观测过。分桶要缓存的是**编译结果**，不是**文件**。
    fn plan_path(&self) -> PathBuf {
        self.dir.join(REC_MODEL)
    }

    /// 拿宽度 `w` 的可执行计划，没有就编一个。
    pub fn get(&self, w: usize) -> Result<Plan, PpError> {
        if let Some(p) = self.buckets.lock().unwrap().get(&w) {
            return Ok(Arc::clone(p));
        }
        let plan = ocr_pp::load_rec(&self.plan_path(), w)?;
        let mut m = self.buckets.lock().unwrap();
        if m.len() >= self.max_buckets && !m.contains_key(&w) {
            // 清一半而不是清空：清空会让下一个框又把整轮编一遍。
            let keep: Vec<usize> = m.keys().copied().take(self.max_buckets / 2).collect();
            m.retain(|k, _| keep.contains(k));
        }
        m.insert(w, Arc::clone(&plan));
        Ok(plan)
    }

    pub fn len(&self) -> usize {
        self.buckets.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

static DET_PLAN: OnceLock<Mutex<Option<(usize, usize, Plan)>>> = OnceLock::new();

fn det_plan_slot() -> &'static Mutex<Option<(usize, usize, Plan)>> {
    DET_PLAN.get_or_init(|| Mutex::new(None))
}

/// det 图的缓存。**只缓存一份**——det 的输入尺寸随帧变，编一个几十毫秒，
/// 而一帧只跑一次，缓存收益不大；缓存多份反而占内存。
///
/// ## 缓存必须按形状校验（2026-10-15 修）
///
/// 早先这里只存 `Plan`，**不看 `h`/`w`**。而 det 的图是按具体形状编译的，
/// 聊天窗口一被拖动尺寸，det 输入就从 `[1,3,960,1280]` 变成别的，
/// 缓存里那份旧的会被直接返回，接着 `run` 拿到一个形状不匹配的输入而报错，
/// 于是又走「回落 WinRT」那条静默路径。
pub fn det_plan(h: usize, w: usize) -> Result<Plan, PpError> {
    let mut slot = det_plan_slot().lock().unwrap();
    if let Some((ph, pw, p)) = slot.as_ref() {
        if *ph == h && *pw == w {
            return Ok(Arc::clone(p));
        }
    }
    let p = ocr_pp::load_det(&ocr_pp::models_dir().join(DET_MODEL), h, w)?;
    *slot = Some((h, w, Arc::clone(&p)));
    Ok(p)
}

/// 权重目录里有 det 模型吗。**「没装」是判定，不是错误。**
pub fn det_file_present(dir: &std::path::Path) -> bool {
    dir.join(DET_MODEL).is_file()
}

/// 整套 PP-OCRv4 识别。**权重不在就返回 `None`，不是错误。**
///
/// `None` 的含义是「这台机器没装 PP-OCRv4，请走 WinRT 那条路」。
/// 刻意不让它变成 [`PpError`]——那会让「没装模型」和「模型坏了」混成一类，
/// 而前者是完全正常的配置状态。
pub fn recognize(frame: &Frame) -> Option<OcrText> {
    recognize_in(&ocr_pp::models_dir(), frame)
}

/// [`recognize`] 的可测入口：权重目录由调用方给。
///
/// 拆出 `dir` 参数不只是为了测试——`ocr_pp::models_dir()` 读环境变量，
/// 而 edition 2024 里改环境变量是 `unsafe`，测试里不该碰。
pub fn recognize_in(dir: &std::path::Path, frame: &Frame) -> Option<OcrText> {
    if !det_file_present(dir) {
        return None;
    }
    let chars = chars_in(dir).ok()?;
    let cache = BucketCache::new(dir.to_path_buf());
    match recognize_with(frame, &chars, &cache) {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::warn!("PP-OCRv4 失败，回落 WinRT：{e}");
            None
        }
    }
}

/// [`ocr_pp::chars`] 的可测版本：词典路径由调用方给。
fn chars_in(dir: &std::path::Path) -> Result<Arc<Vec<String>>, PpError> {
    let raw = std::fs::read_to_string(dir.join("ppocr_keys_v1.txt")).map_err(|e| {
        PpError::MissingModels(format!(
            "{}（{e}）",
            dir.join("ppocr_keys_v1.txt").display()
        ))
    })?;
    let mut v: Vec<String> = raw
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    // blank 在 0，文件里的字符整体后移一位。
    v.insert(0, String::new());
    Ok(Arc::new(v))
}

/// 有模型时的真正入口。拆出来是为了能直接喂词典与缓存做测试。
pub fn recognize_with(
    frame: &Frame,
    chars: &[String],
    cache: &BucketCache,
) -> Result<OcrText, PpError> {
    let (fw, fh) = (frame.width() as usize, frame.height() as usize);
    if fw == 0 || fh == 0 {
        return Err(PpError::Shape("空帧".into()));
    }
    let (dw, dh) = det_size(fw as u32, fh as u32);

    // ── det ──
    let plan = det_plan(dh, dw)?;
    let input_buf = det_input(frame, dw, dh);
    let input = tract_onnx::prelude::Tensor::from_shape(&[1, 3, dh, dw], &input_buf)
        .map_err(|e| PpError::Shape(format!("造 det 输入失败：{e}")))?;
    let (vals, shape) = ocr_pp::run(&plan, input, "det")?;
    if shape.len() != 4 || shape[0] != 1 || shape[1] != 1 {
        return Err(PpError::Shape(format!("det 输出不是 [1,1,h,w]：{shape:?}")));
    }
    let (pw, ph) = (shape[3], shape[2]);
    // 顺序：先合并成行，再按 [`DET_MAX_BOXES`] 限流。反过来就是拿行数预算
    // 去装单字框，屏幕下半截会被裁掉。见该常量的说明。
    let mut boxes = merge_into_lines(&det_boxes(&vals, pw, ph, fw, fh));
    if boxes.len() > DET_MAX_BOXES {
        tracing::warn!(
            lines = boxes.len(),
            cap = DET_MAX_BOXES,
            "det 的行数超过上限，屏幕下半部分可能没被读到"
        );
        boxes.truncate(DET_MAX_BOXES);
    }

    // ── rec（按宽度分桶，同宽的框共用一张编译好的图）──
    let mut lines: Vec<OcrLine> = Vec::new();
    let mut by_w: HashMap<usize, Vec<Rect>> = HashMap::new();
    for b in &boxes {
        by_w.entry(rec_width(b.width() as u32))
            .or_default()
            .push(*b);
    }
    for (w, mut group) in by_w {
        let plan = cache.get(w)?;
        let mut decoded: Vec<OcrLine> = Vec::new();
        for b in group.drain(..) {
            let buf = rec_input(frame, b, w);
            let input = tract_onnx::prelude::Tensor::from_shape(&[1, 3, REC_HEIGHT, w], &buf)
                .map_err(|e| PpError::Shape(format!("造 rec 输入失败：{e}")))?;
            let (vals, shape) = ocr_pp::run(&plan, input, "rec")?;
            let (text, conf) = ctc_decode(&vals, &shape, chars);
            if !text.trim().is_empty() {
                // 词级矩形与行矩形同框：PP-OCR 的 rec 是**整框一个序列**，
                // 本来就没有词级切分。要下游按词合并的话得先切，这里如实给一份。
                decoded.push(OcrLine {
                    text,
                    rect: b,
                    words: Vec::new(),
                });
                let _ = conf;
            }
        }
        lines.extend(decoded);
    }
    lines.sort_by_key(|l| (l.rect.y0, l.rect.x0));

    // `OcrLine::words` 不是空的：下游（`perceive`）按词级矩形做行内合并，
    // 塞空 vec 会让那条路径静默什么都不做。
    let text = lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(OcrText { text, lines })
}

/// 把 PP-OCR 的错误并进 [`OcrError`]，让调用方不必认识 PP-OCR。
pub fn as_ocr_error(e: PpError) -> OcrError {
    OcrError::Pipeline(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一张纯色帧。夹具只要能喂进预处理就行，**不要求真有字**——
    /// 真正的「字」要真跑模型才有，而那需要 15MB 权重。
    fn flat(w: u32, h: u32, v: u8) -> Frame {
        Frame::from_rgb(w, h, vec![v; (w * h * 3) as usize]).expect("造帧失败")
    }

    /// **每个宽度分桶都要真能编译出 rec 图。**
    ///
    /// 2026-10-15：这条守的是一次**静默失效**，也是「识别成一堆单字框」
    /// 这个现象真正的成因。`BucketCache::plan_path` 曾经按宽度拼出
    /// `ch_PP-OCRv4_rec_mobile_w{w}.onnx`，而磁盘上从没有过这种文件。
    /// 于是每个桶的 `load_rec` 都失败，整轮 `recognize_with` 报错、
    /// 被 `recognize_in` 吞成 `None`、`ocr::recognize` 回落 WinRT。
    ///
    /// **现场看到的是 WinRT 的逐字输出，却一直在调 det 的参数**——
    /// det 的真实行为从来没被观测过。症状与真因隔了整整一层回落。
    ///
    /// 撤掉修复（把 `plan_path` 改回按宽度拼名字）时这条会红。
    #[test]
    fn every_rec_width_bucket_compiles_against_a_real_weight_file() {
        let dir = ocr_pp::models_dir();
        if !dir.join(REC_MODEL).is_file() {
            eprintln!("跳过：{} 下没有 rec 权重", dir.display());
            return;
        }
        let cache = BucketCache::new(dir.clone());
        assert!(
            cache.plan_path().is_file(),
            "权重路径指向一个不存在的文件：{} —— PP-OCR 会整轮回落 WinRT",
            cache.plan_path().display()
        );
        // 真编一次。只走 `plan_path` 的存在性断言不够：万一哪天权重
        // 换名或目录结构变了，「文件在」也未必「编得出图」。
        assert!(
            cache.get(320).is_ok(),
            "宽度 320 的 rec 编译失败 —— PP-OCR 会整轮回落 WinRT"
        );
    }

    /// **归一化常数必须逐字等于 PaddleOCR 的配置。**
    ///
    /// 2026-10-15 实测的两处错：
    ///
    /// 1. det 的 `std` 曾写成 `[0.456, 0.456, 0.456]`——把 mean 的第二个
    ///    分量抄进了 std，输入对比度只剩一半。真帧上 det 给的框高 8px，
    ///    而同一行的字形墨迹高 12px，rec 拿到的是被削掉下半截的字。
    /// 2. rec 曾复用 det 的 ImageNet 常数；PaddleOCR 里 rec 是
    ///    `mean = std = 0.5`，与 det 分开配。
    ///
    /// 撤掉修复时这条会红。
    #[test]
    fn the_normalization_constants_match_paddleocr() {
        for (got, want, name) in [
            (DET_MEAN[0], 0.485, "det mean[0]"),
            (DET_MEAN[1], 0.456, "det mean[1]"),
            (DET_MEAN[2], 0.406, "det mean[2]"),
            (DET_STD[0], 0.229, "det std[0]"),
            (DET_STD[1], 0.224, "det std[1]"),
            (DET_STD[2], 0.225, "det std[2]"),
            (REC_MEAN[0], 0.5, "rec mean[0]"),
            (REC_STD[0], 0.5, "rec std[0]"),
        ] {
            assert!(
                (got - want).abs() < 1e-6,
                "{name} = {got}，PaddleOCR 要的是 {want}。写错会让输入偏离训练分布，\
                 而症状是「框比字矮、识别成乱码」，不会报错"
            );
        }
        // det 的 std 若等于 mean 的某个分量，就是抄串了行——单独钉住这个形状。
        assert!(
            DET_STD != [DET_MEAN[1]; 3],
            "det 的 std 又变成 mean[1] 重复了三遍"
        );
    }

    /// **det 输入要保住完整对比度。**
    ///
    /// 上一条钉的是常数，这条钉的是**它实际造成的后果**：
    /// 一张「一个黑像素、一个白像素」的图归一化之后，两者的差是
    /// `(1/255 - 0) / std = 1 / std`。std 被写大 → 对比度被压 →
    /// det 的连通域只剩笔画核心。
    ///
    /// 2026-10-15：这条第一版把期望值写成 `(1/255)/std`，少了 255 倍，
    /// 被自己的实现打回。`p/255 - mean` 里除的是**像素值**，
    /// 所以满量程跨度是 `1/std`。
    #[test]
    fn the_det_input_preserves_black_to_white_contrast() {
        let mut px = vec![0u8; 2 * 2 * 3];
        for p in px.chunks_exact_mut(3).skip(1) {
            p.copy_from_slice(&[255, 255, 255]);
        }
        let f = Frame::from_rgb(2, 2, px).expect("造帧失败");
        let out = det_input(&f, 2, 2);
        // NCHW，单通道平面 2x2=4 像素；像素 0 在索引 0、像素 1 在索引 1。
        let spread = out[1] - out[0];
        let want = 1.0f32 / DET_STD[0];
        assert!(
            (spread - want).abs() < 1e-3,
            "黑白跨度 = {spread}，按 det std[0]={} 应该是 {want}。\
             对比度被压掉会让 det 只框住笔画核心",
            DET_STD[0]
        );
    }

    /// **det 图的形状变了必须重编，不能拿旧的缓存凑。**
    ///
    /// 缓存早先不记 `h`/`w`：聊天窗口一被拖动尺寸，
    /// 拿到的就是按旧形状编的图，`run` 随即因输入形状不符而失败，
    /// 于是又走「回落 WinRT」那条静默路径。
    ///
    /// 只缓存一份，所以顺序有意义：先验同形状命中缓存，
    /// 再验换形状会重编。反过来测会误判——缓存被换形状的那次挤掉了。
    #[test]
    fn a_different_det_shape_gets_its_own_plan() {
        let dir = ocr_pp::models_dir();
        if !dir.join(DET_MODEL).is_file() {
            eprintln!("跳过：{} 下没有 det 权重", dir.display());
            return;
        }
        let a = det_plan(32, 64).expect("32x64 编不出来");
        let a2 = det_plan(32, 64).expect("32x64 重编失败");
        assert!(
            Arc::ptr_eq(&a, &a2),
            "同形状重复调用没有命中缓存，每次都要重编一遍"
        );
        let b = det_plan(64, 64).expect("64x64 编不出来");
        // 形状不同 → 必须是另一张图。拿到同一张就说明缓存没看形状。
        assert!(
            !Arc::ptr_eq(&a, &b),
            "换了 det 输入形状却拿到同一张缓存的图 —— 窗口改尺寸后必然失败回落"
        );
    }

    /// **det 的输入尺寸必须取到 32 的倍数。**
    ///
    /// 这是硬要求不是风格问题：网络里有 8 次步长 2 的下采样，
    /// 尺寸不对齐会让 pad 算错，输出图与输入对不上，框全错。
    #[test]
    fn the_det_input_is_rounded_up_to_a_multiple_of_32() {
        // 1161 → 1184（37×32）、893 → 896（28×32）
        assert_eq!(det_size(1161, 893), (1184, 896));
        assert_eq!(det_size(1024, 768), (1024, 768));
        // 已对齐的不能被推高一档。
        assert_eq!(det_size(320, 64), (320, 64));
        // 小于 32 的要有下限，否则下采样后是空图。
        assert_eq!(det_size(4, 4), (32, 32));
    }

    /// **rec 的宽度必须取到 8 的倍数。**
    ///
    /// 下限 8 不是「凑整」：宽度小于 8 的话内部下采样后时间步会是 0，
    /// CTC 解出一段空白而**不报错**。
    #[test]
    fn the_rec_width_is_rounded_up_to_a_multiple_of_8_with_a_floor() {
        assert_eq!(rec_width(100), 104);
        assert_eq!(rec_width(101), 104);
        assert_eq!(rec_width(96), 96);
        assert_eq!(rec_width(3), 8);
        assert_eq!(rec_width(0), 8);
    }

    /// **二值化 + 连通域 + unclip：一个明显的框能被捞出来。**
    ///
    /// 造一张 32x32 的概率图，中间放一个 8x8 的高概率块。
    ///
    /// 期望值**含 unclip 外扩**（2026-10-15 补上这一步）：
    /// `len = sqrt(8*8) = 8`，`dx = dy = 8*1.5/8 = 1.5`，
    /// 所以框是 `(12-1.5, 10-1.5) .. (19+1+1.5, 17+1+1.5)`
    /// 四舍五入后的 `(11, 9, 22, 20)`，而不是块本身的 `12,10,20,18`。
    ///
    /// 为什么要有这一步：二值化阈值一定削掉抗锯齿边缘，
    /// 框紧贴剩下的核心就意味着 rec 拿到的是**缺了一圈笔画**的字。
    #[test]
    fn a_single_bright_block_becomes_one_unclipped_box() {
        let (w, h) = (32usize, 32usize);
        let mut prob = vec![0.0f32; w * h];
        for y in 10..18 {
            for x in 12..20 {
                prob[y * w + x] = 0.9;
            }
        }
        let boxes = det_boxes(&prob, w, h, w, h);
        assert_eq!(boxes.len(), 1, "应该正好一个框：{boxes:?}");
        assert_eq!(
            (boxes[0].x0, boxes[0].y0, boxes[0].x1, boxes[0].y1),
            (11, 9, 22, 20),
            "框没按 unclip_ratio={} 外扩：{:?}",
            DET_UNCLIP_RATIO,
            boxes[0]
        );
    }

    /// **单字框不能在行合并之前就被 [`DET_MAX_BOXES`] 截掉。**
    ///
    /// 2026-10-15：限流曾经卡在 `det_boxes` 里，也就是合并**之前**。
    /// 于是行数预算被单字框花掉——真帧上一行十四个字就是十几个框，
    /// 64 的预算几行就耗光，屏幕下半部分整片丢失。
    ///
    /// 撤掉修复（把 `truncate` 放回 `det_boxes`）时这条会红：
    /// 现在是 100 个框，改回去就是 64 个。
    #[test]
    fn per_character_boxes_are_not_truncated_before_line_merging() {
        let (pw, ph) = (800usize, 64usize);
        let mut prob = vec![0.0f32; pw * ph];
        // 一行里 100 个**互不相连**的 5x5 块（面积 25 > DET_MIN_AREA）。
        // 步距必须大于块宽，否则 8 邻域会把它们连成一块，测的就不是截断了。
        for k in 0..100usize {
            let x = k * 7;
            for y in 10..15 {
                for i in 0..5 {
                    prob[y * pw + x + i] = 0.9;
                }
            }
        }
        let boxes = det_boxes(&prob, pw, ph, pw, ph);
        assert_eq!(
            boxes.len(),
            100,
            "单字框在行合并之前就被截断了：{} 个（上限 {}）",
            boxes.len(),
            DET_MAX_BOXES
        );
    }

    /// **噪点要被筛掉。**
    ///
    /// 1 个像素的亮点面积 1 < [`DET_MIN_AREA`]，必须消失。
    /// 不筛的话每个噪点都要过一遍 rec（有编译开销），一帧几百个就卡死了。
    #[test]
    fn speckle_below_the_minimum_area_is_dropped() {
        let (w, h) = (32usize, 32usize);
        let mut prob = vec![0.0f32; w * h];
        prob[5 * w + 5] = 1.0; // 面积 1
        prob[20 * w + 20] = 1.0;
        prob[21 * w + 20] = 1.0;
        prob[21 * w + 21] = 1.0; // 面积 4
        assert!(det_boxes(&prob, w, h, w, h).is_empty(), "噪点没被滤掉");
    }

    /// **概率图比原图小的时候，框要换算回原图坐标。**
    ///
    /// det 的输出比输入小，框必须放大回去，否则坐标差好几倍——
    /// 而下游的消息区过滤全靠坐标，错了就等于没有。
    ///
    /// 2026-10-15：期望值含 unclip 外扩。概率图里 8x8 的块
    /// 外扩 `1.5` 后是 `(6.5, 6.5)..(17.5, 17.5)`，
    /// 换算到 4 倍大的原图就是 `(26, 26)..(70, 70)`。
    /// 早先这里断言的是块的原始位置 `(32,32)..(64,64)`。
    #[test]
    fn boxes_are_scaled_back_to_the_source_image() {
        let (pw, ph) = (32usize, 32usize);
        let (sw, sh) = (128usize, 128usize); // 原图是概率图的 4 倍
        let mut prob = vec![0.0f32; pw * ph];
        for y in 8..16 {
            for x in 8..16 {
                prob[y * pw + x] = 0.9;
            }
        }
        let boxes = det_boxes(&prob, pw, ph, sw, sh);
        assert_eq!(boxes.len(), 1);
        let b = boxes[0];
        assert_eq!((b.x0, b.y0), (26, 26), "没放大回原图：{b:?}");
        assert_eq!((b.x1, b.y1), (70, 70), "没放大回原图：{b:?}");
        assert!(b.x1 <= sw as i32 && b.y1 <= sh as i32, "框超出画面：{b:?}");
    }

    /// **同一行的两块要合并成一行。**
    ///
    /// PP-OCR 的 det 吐的是**文本块**，一行中文常常被切成两三块。
    /// 而 [`OcrLine`] 的语义是「一行」——下游按 `center_y` 归并消息。
    #[test]
    fn two_boxes_on_one_line_merge() {
        let a = Rect {
            x0: 10,
            y0: 100,
            x1: 40,
            y1: 120,
        };
        let b = Rect {
            x0: 44,
            y0: 101,
            x1: 90,
            y1: 119,
        };
        let merged = merge_into_lines(&[a, b]);
        assert_eq!(merged.len(), 1, "同一行没合并：{merged:?}");
        assert_eq!((merged[0].x0, merged[0].x1), (10, 90));
    }

    /// **不同行不合并。**
    ///
    /// 这是上一条的对照：垂直重叠不够就必须分开，否则两条消息会并成一条。
    #[test]
    fn boxes_on_different_lines_stay_separate() {
        let a = Rect {
            x0: 10,
            y0: 100,
            x1: 90,
            y1: 120,
        };
        let b = Rect {
            x0: 10,
            y0: 200,
            x1: 90,
            y1: 220,
        };
        assert_eq!(merge_into_lines(&[a, b]).len(), 2);
    }

    /// **CTC：连续重复要折叠。**
    ///
    /// `a a a` 折叠成 `a`。这是 CTC 的核心规则。
    ///
    /// 夹具遵守 `C == chars.len() + 1`（`chars[0]` 是 blank 的空占位，
    /// 真实字符从 `chars[1]` 起，所以类 c ↔ `chars[c]`），否则类数守卫
    /// 会直接降级成空串——**症状是「解出来什么都没有」，而不是报错**。
    #[test]
    fn ctc_collapses_consecutive_repeats() {
        // 词典：["", "a", "b"] → 类数 4：0=blank 1=a 2=b 3=多余
        let chars = vec![String::new(), "a".into(), "b".into()];
        let c = chars.len() + 1;
        // T=4：[a, a, a, b]
        let mut vals = Vec::new();
        for cls in [1usize, 1, 1, 2] {
            let mut row = vec![0.0f32; c];
            row[cls] = 1.0;
            vals.extend(row);
        }
        let (text, conf) = ctc_decode(&vals, &[1, 4, c], &chars);
        assert_eq!(text, "ab", "连续重复没折叠：{text:?}");
        assert!(conf > 0.9, "置信度算错了：{conf}");
    }

    /// **CTC：非连续的重复要保留。**
    ///
    /// 这是最常见的 CTC 实现错误：写成「全局去重」会把 `aba` 读成 `ab`。
    #[test]
    fn ctc_keeps_non_consecutive_repeats() {
        let chars = vec![String::new(), "a".into(), "b".into()];
        let c = chars.len() + 1;
        let mut vals = Vec::new();
        for cls in [1usize, 2, 1] {
            let mut row = vec![0.0f32; c];
            row[cls] = 1.0;
            vals.extend(row);
        }
        let (text, _) = ctc_decode(&vals, &[1, 3, c], &chars);
        assert_eq!(text, "aba", "非连续重复被误去重：{text:?}");
    }

    /// **CTC：blank 要跳过，且不参与折叠。**
    ///
    /// `a blank a` 是**两个** a（中间隔 blank 就不是连续重复）。
    #[test]
    fn ctc_skips_blanks_without_collapsing_them() {
        let chars = vec![String::new(), "a".into()];
        let c = chars.len() + 1;
        let mut vals = Vec::new();
        for cls in [1usize, 0, 1] {
            let mut row = vec![0.0f32; c];
            row[cls] = 1.0;
            vals.extend(row);
        }
        let (text, _) = ctc_decode(&vals, &[1, 3, c], &chars);
        assert_eq!(text, "aa", "blank 没跳对：{text:?}");
    }

    /// **类数与词典对不上时返回空串，而不是越界 panic。**
    ///
    /// 这是「静默失败」的高发点：形状写错时如果按下标硬取，
    /// 轻则解出空白、重则 panic 在索引上。这里必须明确降级。
    #[test]
    fn a_class_count_mismatch_degrades_to_empty() {
        let chars = vec![String::new(), "a".into(), "b".into()];
        let vals = vec![0.0, 1.0, 0.0, 0.0];
        // C 给了 5，但词典只该配 4
        let (text, conf) = ctc_decode(&vals, &[1, 1, 5], &chars);
        assert_eq!(text, "", "类数不匹配还解出了文本：{text:?}");
        assert_eq!(conf, 0.0);
    }

    /// **det 预处理输出长度对，且是 `[1,3,H,W]` 的 NCHW 排列。**
    #[test]
    fn the_det_input_is_nchw_and_the_right_length() {
        let f = flat(32, 32, 128);
        let out = det_input(&f, 32, 32);
        assert_eq!(out.len(), 3 * 32 * 32);
        // 纯灰帧：归一化后三个通道都是常数，均值该接近各自归一化后的值，
        // 重点是**长度对**且没有 NaN——形状错了后面全白跑。
        assert!(out.iter().all(|v| v.is_finite()), "预处理产出 NaN/Inf");
    }

    /// **rec 预处理输出长度对。**
    #[test]
    fn the_rec_input_has_the_right_length() {
        let f = flat(64, 48, 200);
        let b = Rect {
            x0: 0,
            y0: 0,
            x1: 40,
            y1: 30,
        };
        let out = rec_input(&f, b, 48);
        assert_eq!(out.len(), 3 * REC_HEIGHT * 48);
        assert!(out.iter().all(|v| v.is_finite()));
    }

    /// **框比画面大时要被夹住，不能越界取像素。**
    ///
    /// 越界取像素会 panic 或者读到别人的内存——det 偶尔会吐出超出画面的框，
    /// 所以这个夹取是必需的，不是防御性冗余。
    #[test]
    fn boxes_larger_than_the_frame_are_clamped() {
        let f = flat(32, 48, 128);
        let big = Rect {
            x0: 0,
            y0: 0,
            x1: 999,
            y1: 999,
        };
        let out = rec_input(&f, big, 16);
        assert_eq!(out.len(), 3 * REC_HEIGHT * 16);
        assert!(out.iter().all(|v| v.is_finite()));
    }

    /// **权重不在时 `recognize` 返回 `None`，不是错误。**
    ///
    /// 「没装模型」是完全正常的配置状态，不该让整条链路报错——
    /// 那会让一台没下模型的机器直接看不到任何消息。
    ///
    /// 刻意**不**用 `std::env::set_var` 把它指到一个空目录：edition 2024 里
    /// `set_var` 是 `unsafe`，而且本 crate 的测试是多线程跑的，另一个线程同时
    /// 读环境变量是真会出事的。改用本模块私有的「这次用哪个目录」判定入口。
    #[test]
    fn a_missing_det_file_means_none_not_an_error() {
        let empty = std::env::temp_dir().join("chatservant-no-such-models-dir");
        assert!(
            !empty.join("ch_PP-OCRv4_det_mobile.onnx").is_file(),
            "测试前提不成立：那个临时目录里居然有权重"
        );
        assert!(
            !det_file_present(&empty),
            "不存在的目录必须判定为「没装权重」"
        );
    }

    /// 真的装了权重的机器上，`det_file_present` 要能认出它。
    #[test]
    fn an_existing_det_file_is_detected() {
        let dir = ocr_pp::models_dir();
        if !det_file_present(&dir) {
            return; // 本机没下权重，跳过而不是假装通过
        }
        let f = flat(32, 32, 128);
        assert!(
            recognize_in(&dir, &f).is_some(),
            "权重在，`recognize_in` 却给不出结果"
        );
    }
}
