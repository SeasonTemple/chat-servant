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
//! # ⚠ 形状顺序：类在**最后一维**（2026-10-10 实测）
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

/// det 的总下采样倍数。**8 次步长 2 的下采样 = 32。**
///
/// ⚠ 所以 det 的输入 H/W 必须是 32 的倍数，否则图里的 pad 算错。
pub const DET_STRIDE: usize = 32;

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

/// 一张图上最多取多少个框。
///
/// 保护性的上限：det 在一块纯色背景上也可能吐出成百上千个噪点框，
/// 每一个都要过一遍 rec（有编译开销）。取 64 足够覆盖一屏聊天消息。
pub const DET_MAX_BOXES: usize = 64;

// ────────────────────────────────────────────────────────────────────────
// 预处理
// ────────────────────────────────────────────────────────────────────────

/// det 的输入尺寸：32 的倍数。
///
/// ⚠ **不缩放比例**——PP-OCRv4 的 det 内部有 `Resize` 按比例缩到 1/4，
/// 输入给多大就按多大算。取整到 32 的倍数是硬要求（[`DET_STRIDE`]）。
pub fn det_size(w: u32, h: u32) -> (usize, usize) {
    let w = (w as usize).div_ceil(DET_STRIDE) * DET_STRIDE;
    let h = (h as usize).div_ceil(DET_STRIDE) * DET_STRIDE;
    (w.max(DET_STRIDE), h.max(DET_STRIDE))
}

/// rec 的输入宽度：8 的倍数，且不小于 8。
///
/// ⚠ rec 对窄框有下限：宽度小于 8 的话内部下采样后时间步会是 0。
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
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.456, 0.456, 0.456];
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
                out[c * w * h + o] = (p[c] as f32 / 255.0 - MEAN[c]) / STD[c];
            }
        }
    }
    out
}

/// 裁一个框 → rec 的 `[1,3,48,W']` 归一化张量。
///
/// 高**固定** 48（[`REC_HEIGHT`]），宽按框的比例算再取整到 8 的倍数。
/// ⚠ 只缩不裁：宽高比要保持，否则字会被拉变形，识别率掉得很厉害。
pub fn rec_input(frame: &Frame, box_: Rect, out_w: usize) -> Vec<f32> {
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.456, 0.456, 0.456];
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
                out[c * h * out_w + o] = (p[c] as f32 / 255.0 - MEAN[c]) / STD[c];
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
    // ⚬ `prob` 长度不保证正好是 `n`（det 输出形状对不上时会有）。逐个
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
            let w = mxx - mnx + 1;
            let h = mxy - mny + 1;
            if w < DET_MIN_W || h < DET_MIN_H {
                return None;
            }
            // 反缩放回原图坐标。用四舍五入而不是截断：截断会让最右/最下
            // 的那一列像素永远落在框外。
            let sx = src_w as f64 / pw as f64;
            let sy = src_h as f64 / ph as f64;
            let x0 = ((*mnx as f64) * sx).round() as i32;
            let y0 = ((*mny as f64) * sy).round() as i32;
            let x1 = ((*mxx as f64 + 1.0) * sx).round() as i32;
            let y1 = ((*mxy as f64 + 1.0) * sy).round() as i32;
            Some(Rect {
                x0,
                y0,
                x1: x1.clamp(0, src_w as i32),
                y1: y1.clamp(0, src_h as i32),
            })
        })
        .filter(|r| r.width() > 0 && r.height() > 0)
        .collect();

    // 按面积从大到小（文字块比噪点大），取前 N 个。
    boxes.sort_by_key(|b| -(b.width() * b.height()));
    boxes.truncate(DET_MAX_BOXES);
    // 读出来是自上而下、自左而右。
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
// rec 后处理：CTC 解码
// ────────────────────────────────────────────────────────────────────────

/// CTC 解码。`vals` 是 `[1,T,C]` 展平后的缓冲，`chars` 是 6624 长的词典。
///
/// 三步，**顺序是 CTC 的定义，不能换**：
/// 1. 每个时间步取最大类（argmax）
/// 2. 折叠**连续重复**的类
/// 3. 去掉 blank
///
/// ⚠ 第 2 步只折叠**连续**的。`aab` 要变成 `ab`（去重），
/// 而 `aba` 要保持 `aba`——因为中间那个 `a` 不是连续重复。
/// 写成「全局去重」会把 `aba` 读成 `ab`，是最常见的 CTC 实现错误。
///
/// ⚠ 返回里还带上每步的置信度（max prob 的均值），供上游筛掉垃圾行。
pub fn ctc_decode(vals: &[f32], shape: &[usize], chars: &[String]) -> (String, f32) {
    // ⚠ 形状容错：万一某版权重给的是 [1,C,T]，这里明确报错而不是猜。
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
            // ⚠ 直接用 `chars[best]`，**不是 `chars[best - 1]`**。
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

    fn plan_path(&self, w: usize) -> PathBuf {
        self.dir.join(format!("ch_PP-OCRv4_rec_mobile_w{w}.onnx"))
    }

    /// 拿宽度 `w` 的可执行计划，没有就编一个。
    pub fn get(&self, w: usize) -> Result<Plan, PpError> {
        if let Some(p) = self.buckets.lock().unwrap().get(&w) {
            return Ok(Arc::clone(p));
        }
        let plan = ocr_pp::load_rec(&self.plan_path(w), w)?;
        let mut m = self.buckets.lock().unwrap();
        if m.len() >= self.max_buckets && !m.contains_key(&w) {
            // ⚠ 清一半而不是清空：清空会让下一个框又把整轮编一遍。
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

static DET_PLAN: OnceLock<Mutex<Option<Plan>>> = OnceLock::new();

fn det_plan_slot() -> &'static Mutex<Option<Plan>> {
    DET_PLAN.get_or_init(|| Mutex::new(None))
}

/// det 图的缓存。**只缓存一份**——det 的输入尺寸随帧变，编一个几十毫秒，
/// 而一帧只跑一次，缓存收益不大；缓存多份反而占内存。
pub fn det_plan(h: usize, w: usize) -> Result<Plan, PpError> {
    let mut slot = det_plan_slot().lock().unwrap();
    if let Some(p) = slot.as_ref() {
        return Ok(Arc::clone(p));
    }
    let p = ocr_pp::load_det(
        &ocr_pp::models_dir().join("ch_PP-OCRv4_det_mobile.onnx"),
        h,
        w,
    )?;
    *slot = Some(Arc::clone(&p));
    Ok(p)
}

/// 权重目录里有 det 模型吗。**「没装」是判定，不是错误。**
pub fn det_file_present(dir: &std::path::Path) -> bool {
    dir.join("ch_PP-OCRv4_det_mobile.onnx").is_file()
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
/// ⚠ 拆出 `dir` 参数不只是为了测试——`ocr_pp::models_dir()` 读环境变量，
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
    let boxes = merge_into_lines(&det_boxes(&vals, pw, ph, fw, fh));

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
                // ⚬ 词级矩形与行矩形同框：PP-OCR 的 rec 是**整框一个序列**，
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

    // ⚠ `OcrLine::words` 不是空的：下游（`perceive`）按词级矩形做行内合并，
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
    /// ⚠ 下限 8 不是「凑整」：宽度小于 8 的话内部下采样后时间步会是 0，
    /// CTC 解出一段空白而**不报错**。
    #[test]
    fn the_rec_width_is_rounded_up_to_a_multiple_of_8_with_a_floor() {
        assert_eq!(rec_width(100), 104);
        assert_eq!(rec_width(101), 104);
        assert_eq!(rec_width(96), 96);
        assert_eq!(rec_width(3), 8);
        assert_eq!(rec_width(0), 8);
    }

    /// **二值化 + 连通域：一个明显的框能被捞出来。**
    ///
    /// 造一张 32x32 的概率图，中间放一个 8x8 的高概率块。
    #[test]
    fn a_single_bright_block_becomes_one_box() {
        let (w, h) = (32usize, 32usize);
        let mut prob = vec![0.0f32; w * h];
        for y in 10..18 {
            for x in 12..20 {
                prob[y * w + x] = 0.9;
            }
        }
        let boxes = det_boxes(&prob, w, h, w, h);
        assert_eq!(boxes.len(), 1, "应该正好一个框：{boxes:?}");
        assert_eq!(boxes[0].x0, 12);
        assert_eq!(boxes[0].y0, 10);
        assert_eq!(boxes[0].x1, 20);
        assert_eq!(boxes[0].y1, 18);
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
    /// det 的输出是 1/4 尺寸的，框必须放大回去，否则坐标差 4 倍——
    /// 而下游的消息区过滤全靠坐标，错了就等于没有。
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
        assert_eq!((b.x0, b.y0), (32, 32), "没放大回原图：{b:?}");
        assert_eq!((b.x1, b.y1), (64, 64), "没放大回原图：{b:?}");
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
    /// ⚬ 夹具遵守 `C == chars.len() + 1`（`chars[0]` 是 blank 的空占位，
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
    /// ⚠ 这是最常见的 CTC 实现错误：写成「全局去重」会把 `aba` 读成 `ab`。
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
    /// ⚠ 这是「静默失败」的高发点：形状写错时如果按下标硬取，
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
    /// ⚠ 刻意**不**用 `std::env::set_var` 把它指到一个空目录：edition 2024 里
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
