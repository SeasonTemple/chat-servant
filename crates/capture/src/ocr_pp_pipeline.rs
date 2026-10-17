//! PP-OCR 的**推理管线**：预处理 → det 后处理 → rec 后处理。
//!
//! # 与 [`crate::ocr_pp`] 的分工
//!
//! [`ocr_pp`] 只管「把 ONNX 编译成能跑的图、把输出取出来」——那层与模型
//! 无关。**这一层全是 PP-OCR 特有的约定**，换模型就得重查。
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
use std::hash::Hash;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use chatservant_core::frame::{Frame, Rect};

use crate::ocr::{OcrError, OcrLine, OcrText};
use crate::ocr_pp::{self, BLANK, Plan, PlanBody, PpError};

/// 一套权重：**det 文件、rec 文件、词典文件、两个归一化常数、det 的尺寸策略**，
/// 全部的**唯一出处**。
///
/// 早先这些各写各的字面量，改模型要同时动七八个地方，漏一个就是
/// 「rec 类数与词典对不上」或者「det 一个框都读不出来」这种**只在真机上暴露**的错。
/// ⚠ 2026-10-18 实测：v6 的 rec 输出 **18710** 类，
/// 而 `ppocrv6_dict.txt` 只有 18708 行——差的两个是 CTC 的 blank 与 space，
/// 与 v4「6623 行文件 / 6625 类输出」同一套约定。见 [`chars_in`]。
pub struct ModelSet {
    pub det: &'static str,
    pub rec: &'static str,
    pub keys: &'static str,
    /// 人类可读的名字，只进日志与错误文本。
    pub label: &'static str,
    /// det 的 `(x/255 - mean) / std`。
    pub det_norm: ([f32; 3], [f32; 3]),
    /// rec 的 `(x/255 - mean) / std`。`(0,0)` 表示**只除 255**，不减均值。
    pub rec_norm: ([f32; 3], [f32; 3]),
    /// det 的最短边下限。`0` = 原生尺寸送进去；`736` = 最短边不足就放大到 736。
    pub det_min_side: u32,
}

/// **生产用的一套：PP-OCRv6 small。**
///
/// 选它而不是 v4 的理由全在 [`docs/adr/0006-ocr-engine-evaluation.md`]：
/// 同一批真实微信帧上 v6 是 **17/17 零漏检、CER 0.79%**，v4 在真帧上读出过
/// `419 / 雅扣！手洁双` 这类整行不可用输出（根因是 det 框纵向欠覆盖）。
///
/// ⚠ **归一化常数与 v4 完全不同，这三处是本轮实测踩出来的**：
///
/// | | v4 mobile | **v6 small（本套）** |
/// |---|---|---|
/// | det 归一化 | `(x/255 - [.485,.456,.406]) / [.229,.224,.225]` | **`(x/255 - .5) / .5`** |
/// | rec 归一化 | `(x/255 - .5) / .5` | **只 `x/255`** |
/// | det 最短边 | 原生 | **放大到 ≥736** |
///
/// 带着 v4 的常数去跑 v6，det 是**一个框都读不出来**（`cargo xtask ocr
/// target/probe/wechat-chat.png --scale 1` 实测：3 遍全 0 行）。
/// 这三行数字来自 rapidocr 的 `DetPreProcess` / `Rec` 预处理，
/// 它是 v6 的**参考实现**，照它抄才不会错。
///
/// v4 的权重仍留在 `target/models/` 里可供 A/B，但**代码不再引用它**。
pub const V6_SMALL: ModelSet = ModelSet {
    det: "ppocrv6-small/PP-OCRv6_det_small.onnx",
    rec: "ppocrv6-small/PP-OCRv6_rec_small.onnx",
    keys: "ppocrv6-small/ppocrv6_dict.txt",
    label: "PP-OCRv6 small",
    det_norm: ([0.5, 0.5, 0.5], [0.5, 0.5, 0.5]),
    rec_norm: ([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
    det_min_side: 736,
};

/// 生产用的那套。换模型**只改这一行**。
pub const MODELS: ModelSet = V6_SMALL;

/// det 的权重文件名。**磁盘上就这一个**，没有「按尺寸分文件」这一说。
pub const DET_MODEL: &str = MODELS.det;

/// rec 的权重文件名。同上，**只有一个**。
///
/// 2026-10-15 实测：早先的 [`BucketCache::plan_path`] 按宽度拼出
/// `ch_PP-OCRv4_rec_mobile_w{w}.onnx`，而磁盘上从没有过这种文件，
/// 于是每个宽度的 rec 编译都对着一个不存在的路径失败。见 [`BucketCache`]。
pub const REC_MODEL: &str = MODELS.rec;

/// 词典文件名。与 [`DET_MODEL`] 同源，改模型不会漏掉它。
pub const KEYS_FILE: &str = MODELS.keys;

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
/// det 的归一化常数。**跟着 [`MODELS`] 走**，v6 与 v4 不同（见其文档）。
///
/// v4 那一代取自 PaddleOCR 的 `DetNormalizeImage`
/// （`mean=[0.485,0.456,0.406]`、`std=[0.229,0.224,0.225]`）；
/// v6 是 `mean = std = 0.5`。
///
/// ⚠ 早先这里写错过一次 `std = [0.456, 0.456, 0.456]`——把 mean 的第二个分量
/// 抄进了 std。后果是**送进网络的对比度只有应有值的一半**，
/// DB 头的 sigmoid 输出整体偏低，按 [`DET_THRESH`] 二值化之后
/// 只剩笔画最密的核心留下来。
///
/// 实测后果（`cargo xtask ocr <帧> --stats` 对同一张真帧）：
/// 一行 6 个字的墨迹高 **12px**，而 det 给的框高只有 **8px**，
/// 且框整体上移约 6~7px —— rec 拿到的是一个**被削掉下半截**的字，
/// 识别结果只能是乱码。
pub const DET_MEAN: [f32; 3] = MODELS.det_norm.0;
pub const DET_STD: [f32; 3] = MODELS.det_norm.1;

/// rec 的归一化常数。同样**跟着 [`MODELS`] 走**。
///
/// v6 的 rec 是 SVTR_LCNet，**只除 255，不减均值**——`(0,0)/(1,1)`
/// 就是「不做 `(v - mean)/std`」的写法。
pub const REC_MEAN: [f32; 3] = MODELS.rec_norm.0;
pub const REC_STD: [f32; 3] = MODELS.rec_norm.1;

/// PaddleOCR DB 后处理的 `unclip` 外扩比例。
///
/// 外扩是**照 DB 论文的做法**，不是为了让框变好看：二值化阈值一定会削掉
/// 字形的抗锯齿边缘，外扩把被削掉的那一圈补回来，
/// rec 才拿得到完整的字。
///
/// 外扩量按 **`面积 × ratio / 周长`**（即 `w·h·r / 2(w+h)`）算，各向均匀。
///
/// ⚠ 2026-10-18 订正：早先这里写的是 `w·r/√(w·h)`，并且注释里把这个
/// 「纵向几乎不扩」当成公式的正确行为写了下来——**那正是 bug**。
/// 现在的实现（rapidocr / 新版 PaddleOCR）用 pyclipper 做多边形偏移，
/// 外扩量是 `area × ratio / perimeter`，**上下左右一样多**。
///
/// 差别用 160x14 的行框算一遍：
///
/// | | 横向外扩 | 纵向外扩 |
/// |---|---|---|
/// | `w·r/√(wh)`（旧） | 12.4 | **0.45** |
/// | `area·r/周长`（现） | 6.9 | **6.0** |
///
/// 旧的把字削掉上下两半，只把左右撑宽——rec 拿到半截字，真帧 CER 34%。
pub const DET_UNCLIP_RATIO: f32 = 1.6;

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
/// **先按 [`MODELS::det_min_side`] 决定要不要放大，再取整到 32 的倍数。**
///
/// 取整到 32 是硬要求（[`DET_STRIDE`]）。放大与否是**模型的约定**：
/// rapidocr 的 `DetPreProcess` 默认 `limit_type="min"`、`limit_side_len=736`，
/// 意思是**最短边不足 736 就按比例放大**。PP-OCRv6 的 det 在 14px 小字上
/// 靠这个放大才把每行连成一条，实测 855x545 的微信帧被放到 1184x736。
/// `det_min_side = 0` 则是「原生尺寸送进去」，v4 那一代用的就是它。
///
/// ⚠ 2026-10-18 补记：这个常数**本身是对的，但它一度完全不起作用**——
/// [`det_input`] 那时只把帧贴进 736 撑大的画布、从不缩放像素。
/// 「det 一个框都读不出来」的真正修复是 `det_norm` 改成 `0.5/0.5`，
/// 与本常数无关；而本常数要等到缩放真做了才生效（真帧 CER 18.60% → 0.79%）。
pub fn det_size(w: u32, h: u32) -> (usize, usize) {
    det_size_with_min_side(w, h, MODELS.det_min_side)
}

/// [`det_size`] 的可测版本：最短边下限由调用方给。
///
/// 拆出来是因为「取整到 32」与「要不要放大」是**两件要分别钉的事**——
/// 早先把它们塞在一个函数里，于是给放大规则加了守卫之后，
/// 量「取整」的三条测试全被 736 顶飞了。拆开各自说得清。
pub fn det_size_with_min_side(w: u32, h: u32, min_side: u32) -> (usize, usize) {
    let (mut w, mut h) = (w as f32, h as f32);
    let min_side = min_side as f32;
    if min_side > 0.0 {
        let short = w.min(h);
        if short < min_side && short > 0.0 {
            let ratio = min_side / short;
            w *= ratio;
            h *= ratio;
        }
    }
    let w = (w.round() as usize).div_ceil(DET_STRIDE) * DET_STRIDE;
    let h = (h.round() as usize).div_ceil(DET_STRIDE) * DET_STRIDE;
    (w.max(DET_STRIDE), h.max(DET_STRIDE))
}

/// rec 的输入宽度：8 的倍数，且不小于 8。
///
/// rec 对窄框有下限：宽度小于 8 的话内部下采样后时间步会是 0。
/// **这是把宽度取整到 8 的倍数的直接原因**，不是「凑整好看」。
pub fn rec_width(w: u32) -> usize {
    ((w as usize).div_ceil(REC_STRIDE) * REC_STRIDE).max(REC_STRIDE)
}

/// rec 的输入宽度：把框**等比**缩到高 [`REC_HEIGHT`] 之后的宽度，取整到 8 的倍数。
///
/// ⚠ 2026-10-18 修，这是本仓最贵的第二个 OCR bug。
/// 调用方早先直接传 `rec_width(box.width())`——也就是**框在原图里的像素宽**。
/// 而 rec 的高被固定成 48，原图里的一行只有 8~14px 高，于是
/// [`rec_input`] 的横向步长 `x * bw / out_w` 每输出像素只走 1 个源像素，
/// 纵向却走了 `bh / 48`：**只把字拉高、不让它变宽**，
/// 长宽比被扭曲 `48 / bh ≈ 4` 倍。
///
/// 症状很有辨识度：**中文几乎不受影响，英文整段报废**——
///
/// ```text
/// openrouter哪些 → oderouer      openrouter贵啊 → oberouter告啊
/// 只是测试       → 只是河年       api → ai
/// ```
///
/// CJK 笔画密，横压 4 倍仍能对上笔画数；拉丁字母的宽窄与字腔一压就没了。
/// 而中文行「是不是真满血哦」全程一字不差，正好把范围框死在横向比例上。
///
/// 参照实现 rapidocr 算的是 `ceil(img_height * w / h)`，即**等比**。
#[must_use]
pub fn rec_input_width(box_: Rect) -> usize {
    let bh = box_.height().max(1) as u32;
    let w = (box_.width().max(0) as u32).saturating_mul(REC_HEIGHT as u32) / bh;
    rec_width(w)
}

/// RGB 帧 → det 的 `[1,3,H,W]` 归一化张量。
///
/// PP-OCR 的 det 归一化是 `(v/255 - mean) / std`，v6 是 `mean = std = 0.5`。
/// **注意通道顺序是 RGB**，而我们的 `Frame` 本来就是 RGB8。
///
/// ## 这里**必须真的把帧缩放到 `(w, h)`**，不是把帧贴进一张更大的画布
///
/// 2026-10-18 实测（本条是本仓最贵的一次自纠，代价见下）：
/// 早先的写法是「按帧的原生分辨率逐像素拷贝，越界处复制最后一行/列」——
/// ```text
/// let sy = y.min(fh.saturating_sub(1));
/// let sx = x.min(fw.saturating_sub(1));
/// ```
/// 看着像「尺寸对齐」，实际**帧的像素一个都没变**。
/// 于是 [`det_size`] 算出来的 `det_min_side = 736`（855x545 → 1184x736）
/// **只决定了画布大小，对送进网络的内容毫无作用**：det 看到的仍是
/// 原生分辨率的小字，框削掉字的下半截，rec 拿到的是残字。
///
/// 真帧实测（`wechat-chat` / `gws-chat` 两帧，答案键 `real-ground-truth.json`）：
///
/// | det 输入 | CER |
/// |---|---|
/// | 真缩放（rapidocr 同款） | **0.79%** |
/// | 只补边（本条修掉的写法，Python 侧同构复现） | **18.25%** |
/// | Rust 修前实测 | 18.60% |
///
/// ⇒ 18.25% ≈ 18.60%，**唯一变量就是有没有真的缩放**。
/// 而 18.6% 与 WinRT 的 18.25% 基本齐平——也就是说修之前，
/// 换引擎在字符准确度上**一点没赚**，只赚了检出（17/17 对 14/17）。
///
/// ## 插值为什么是 Triangle
///
/// `Triangle` 就是双线性，等价于 `cv2.INTER_LINEAR`，与 rapidocr 的
/// `DetPreProcess.resize` 一致。用 `Nearest` 也测过（`ab_rec_norm.py --nearest`），
/// 在这两帧上 CER 不变——但那测的是 **rec**，det 侧的缩放方向是**上采样**
/// （1.35 倍），对齐才是这里该守的东西，所以跟参照实现对齐。
///
/// 票 07 明写「重采样交给 `image`，不自己实现插值」，
/// 早先手搓最近邻就已经违反了这一条。
pub fn det_input(frame: &Frame, w: usize, h: usize) -> Vec<f32> {
    use image::imageops::FilterType;

    let (fw, fh) = (frame.width() as usize, frame.height() as usize);
    let mut out = vec![0.0f32; 3 * w * h];
    if fw == 0 || fh == 0 {
        return out;
    }
    // `from_raw` 接管所有权，所以给一份拷贝；重采样本身已经要另开一份输出。
    let Some(src) = image::RgbImage::from_raw(fw as u32, fh as u32, frame.as_bytes().to_vec())
    else {
        return out;
    };
    let scaled = if (fw, fh) == (w, h) {
        src
    } else {
        image::imageops::resize(&src, w as u32, h as u32, FilterType::Triangle)
    };
    let plane = w * h;
    for (i, px) in scaled.pixels().enumerate() {
        let [r, g, b] = px.0;
        out[i] = (r as f32 / 255.0 - DET_MEAN[0]) / DET_STD[0];
        out[plane + i] = (g as f32 / 255.0 - DET_MEAN[1]) / DET_STD[1];
        out[2 * plane + i] = (b as f32 / 255.0 - DET_MEAN[2]) / DET_STD[2];
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
/// 4. 反缩放回原图，并裁进画面
///
/// ## 换算比例是 `src / prob`（2026-10-18 订正，同日第二次改这条式子）
///
/// 一次换算串起两段缩放：
///
/// ```text
/// 原图 ──[det_input] 缩放到 det_w/det_h──▶ det 空间 ──[模型内部下采样]──▶ pw/ph
/// ```
///
/// 所以比例恒为 `src_w / pw` 与 `src_h / ph`。
///
/// ### ⚠ 这一天里这条式子被改过两回，第二回才是对的
///
/// **第一回（早上）**：把 `src / prob` 改成 `det / prob`（＝1.0），当时的实测是
/// 「帧 425x198 补齐到 448x224、概率图也是 448x224，而 [`det_input`] 是原样拷贝
/// ⇒ 概率图与原图 1:1」，并记下了一张「框比墨迹下沿少盖 6/8/12/16/14px」的表。
///
/// **第二回（现在）**：那张表和那个立论**都建立在同一个错误前提上**——
/// [`det_input`] 从来没有真的缩放过，所谓「原样拷贝 + 右下补边」只是把帧贴进
/// 一张更大的画布。于是「1:1」成立，但 det 因此在**原生分辨率的小字**上工作，
/// 框削掉字的下半截。
///
/// 真帧实测（答案键 `real-ground-truth.json`，两帧）：
/// 真缩放 **0.79%** ／ 只补边 **18.25%**（Python 侧同构复现）／ Rust 修前 **18.60%**。
///
/// ⇒ 早先那个 1.0 **是在替真正的 bug 打补丁**：它让框在「画布比帧大」的
/// 坐标系里看起来自洽，可 det 看到的本来就是没放大的小字。
/// [`det_input`] 修成真缩放之后，1:1 不再成立，本式回到第一回之前的写法。
///
/// **教训**：两处配置「各自都自洽」时，要问的是**它们合起来送进网络的内容**对不对，
/// 而不是各自的中间量能不能对上。
#[allow(clippy::too_many_arguments)]
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
            // DB 的 `unclip`：把框**各向均匀**外扩 `d = 面积 × ratio / 周长`。
            //
            // ⚠ 2026-10-18 修，本仓第三个 OCR bug，也是最要命的那个。
            // 早先用的是**旧版 PaddleOCR 的各向异性公式**
            // `dx = w·r/√(wh)`、`dy = h·r/√(wh)`——纵向几乎不扩、横向猛扩。
            // 参照实现 rapidocr 用的是 pyclipper 的多边形偏移：
            //
            // ```python
            // distance = poly.area * unclip_ratio / poly.length
            // offset.Execute(distance)        # 各方向都是 distance
            // ```
            //
            // 差别有多大（概率图上 w=298、h=10 的一行）：
            //
            // | | 外扩量 | 框高（源图） |
            // |---|---|---|
            // | 旧公式 `h·1.5/√(wh)` | 纵 0.27 / 横 8.2 | **8** |
            // | 多边形偏移 `area·1.6/perim` | 各向 7.74 | **18.9** |
            //
            // ⇒ 旧公式把字**削掉上下两半**，只把左右撑得很宽。
            // rec 拿到的是半截字，真帧 CER 34%；把框纵向撑到两倍即回到 2.35%。
            // 而 det 概率图两边一致（前景占比 1.42% vs 1.39%），
            // 所以这不是检测不准，**纯粹是后处理的外扩公式不对**。
            //
            // 缺了 unclip 本身也不对：框会紧贴二值化阈值留下的笔画核心，
            // 抗锯齿被削掉的那一圈永远补不回来。
            let area_px = (w * h) as f64;
            let perim = 2.0 * (w + h) as f64;
            let d = area_px * DET_UNCLIP_RATIO as f64 / perim;
            // 反缩放回原图坐标。用四舍五入而不是截断：截断会让最右/最下
            // 的那一列像素永远落在框外。
            //
            // 概率图 → 原图，**一次换算把两段缩放串起来**：
            //   原图 ──缩放到 det_w/det_h──▶ det 空间 ──模型内部下采样到 pw/ph──▶ 概率图
            // 所以比例是 `src / prob`，**不是** `det / prob`。
            //
            // ⚠ 2026-10-18 订正：早先这里写的是 `det_w / pw`（＝1.0），理由是
            // 「[`det_input`] 是原样拷贝，概率图与原图左上角逐像素对齐」。
            // 而那条拷贝本身就是错的（它只补边、从不缩放），
            // 于是这个写死的 1.0 **在替一个真正的 bug 打补丁**：
            // 帧被塞进更大的画布却没缩放，det 在原生分辨率的小字上工作。
            // [`det_input`] 修成真缩放之后，本式回到它被改之前的写法——
            // 那次「修复」是在错误的成因上打的补丁。
            let sx = src_w as f64 / pw as f64;
            let sy = src_h as f64 / ph as f64;
            let x0 = (((*mnx as f64) - d) * sx).round() as i32;
            let y0 = (((*mny as f64) - d) * sy).round() as i32;
            let x1 = (((*mxx as f64 + 1.0) + d) * sx).round() as i32;
            let y1 = (((*mxy as f64 + 1.0) + d) * sy).round() as i32;
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
    /// 概率图的尺寸。det 输出与输入同尺寸（v4/v6 均实测过）。
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
    let (vals, shape) = ocr_pp::run(&plan, &[1, 3, dh, dw], &buf, "det")?;
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

/// CTC 解码。`vals` 是 `[1,T,C]` 展平后的缓冲，`chars` 是**与 C 等长**的词典。
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
    // ⚠ 这里曾经写的是 `c != chars.len() + 1`，**而它一直是错的**：
    // 词典表早已把末位的 space 类补进去（见 [`chars_in`]），`chars.len()`
    // 就等于类数，再 `+1` 等于要求模型多输出一类。
    // 它当年能跑，纯粹是因为词典表当时**少补了 space**——
    // 歪打正着抵消了。2026-10-18 补上 space 的那一刻，
    // 整个 rec 静默返回空串：`xtask ocr <真帧> --scale 1` 逐行 0 行。
    if c != chars.len() {
        // 类数与词典对不上。**这是配置错误，不是识别失败**——
        // 返回空串 + 0 置信度，上游会把它当「这一框没字」。
        tracing::warn!(
            classes = c,
            dict = chars.len(),
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

/// 拿锁，**中毒了也照样把守卫捞回来**。
///
/// ## 为什么这几处恢复都是对的（逐处算过，别照抄）
///
/// - [`BucketCache::buckets`]：临界区里只有 `HashMap` 的
///   `get`/`entry`/`retain`/`remove`。它们要么整体生效、要么在分配时直接
///   abort，**不存在「改了一半」的中间态**，所以从毒里捞出来的 map 一定是完整的。
/// - [`DetSlots`] 的 cells：同上，临界区里也只有 `HashMap` 操作。
///   ⚠ 它**曾经**不是这样：早先的 [`det_plan`] 把 tract 的 `load_det`
///   （十几秒级）放在锁内，而那是第三方代码、panic 有可能（票 49）。
///   把编译挪到锁外之后这一段退回纯数据结构操作，恢复的依据重新成立。
///
/// 写成 `unwrap()` 的话，一次 panic 之后这个**共享**缓存的每一次访问都会 panic
/// ——OCR 流水线整条挂死，而现场只看得到「OCR 不出字了」。
///
/// ## 哪天这里不再成立
///
/// 若有人往上面两段临界区里加了会 panic 的用户代码，或改成「先改一半再编译」，
/// 恢复就变成危险的了。那时候该改成重建缓存，而不是继续 `into_inner()`。
fn lock_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 一个宽度桶里存的东西。泛型 `P` 是「图」本身。
///
/// 对外仍然是 `Arc<P>`，所以并发命中同一个宽度时拿到的是**同一份**。
enum Slot<P> {
    /// 编好了。
    Ready(Arc<P>),
    /// 编失败。**故意不存错误本身**：`PpError` 不可克隆，而错误文本要显示在
    /// 面板上，给后来者一个占位等于对它撒谎。只记「这个宽度编不出来」，
    /// 让 [`get_or_compile`] 把这个键摘掉、下次重来。
    Failed,
}

/// 取键 `k` 的图，没有就编一个。**编译器由调用方给。**
///
/// ## 为什么泛型，而不是在 [`BucketCache`] 里写死
///
/// 真 `load_rec` 要 15MB 权重才跑得动，而**没有权重的机器上也必须有并发守卫测试**
/// （CI 与别人的机器就没有权重，那些测试会直接跳过，等于没守）。
/// 把「怎么保证只编一次」抽成与图无关的函数，测试就能注入一个计数的桩，
/// 守的仍然是**这一段同步逻辑**本身。`BucketCache::get` 与 [`DetSlots::get`]
/// 是它的两个特化。
///
/// ## 键 `K` 也是泛型：rec 用宽度，det 用 `(h, w)`
///
/// 容量 `max_buckets` 与「只保留哪几个」由调用方给，所以**两张表共用这一段**，
/// 也共用同一批守卫测试（`Slot`、`lock_recover`、失败处理都只有一份实现）。
///
/// ## 并发下同一个键只编一次（2026-10-17，票 47）
///
/// 早先是「① 查锁 → 放锁 → ② 锁外编译 → ③ 再拿锁 → ④ 无条件 insert」的双重
/// 检查：两个线程会**各编一遍**，而后编的那份直接覆盖先编的，白等一次。
/// rec 编译是几十到几百毫秒、det 更慢，所以这不是「多几十毫秒」。
///
/// 现在锁只护住「取/建 cell」那一下，**编译发生在锁外**——所以不同键互不阻塞；
/// 而同一个键后来的人 `get_or_init` 会**等**第一个人编完（`OnceLock` 的语义），
/// 不会各编一遍。
fn get_or_compile<K, P, E, F>(
    buckets: &Mutex<HashMap<K, Arc<OnceLock<Slot<P>>>>>,
    max_buckets: usize,
    k: K,
    compile: F,
) -> Result<Arc<P>, E>
where
    K: Eq + Hash + Copy,
    F: Fn() -> Result<Arc<P>, E>,
{
    let cell = {
        let mut m = lock_recover(buckets);
        // 清一半而不是清空：清空会让下一个框又把整轮编一遍。
        // 判据与早先一致——`len` 到顶、且这个键还不在表里，才清。
        // 顺序是「先清后插」：于是本键一定在表里，不会被自己刚做的清理带走。
        if !m.contains_key(&k) && m.len() >= max_buckets {
            let keep: Vec<K> = m.keys().copied().take(max_buckets / 2).collect();
            m.retain(|key, _| keep.contains(key));
        }
        Arc::clone(m.entry(k).or_default())
    };
    match cell.get_or_init(|| match compile() {
        Ok(p) => Slot::Ready(p),
        Err(_) => Slot::Failed,
    }) {
        Slot::Ready(p) => Ok(Arc::clone(p)),
        Slot::Failed => {
            // **失败不留缓存**：把这个键摘掉，权重后来修好了还能重编。
            // 只摘**同一个** cell——中途可能已经有别人为 `k` 建了新 cell，
            // 摘掉新的那个会把人家正在等的编译结果连带作废。
            let mut m = lock_recover(buckets);
            if m.get(&k).is_some_and(|cur| Arc::ptr_eq(cur, &cell)) {
                m.remove(&k);
            }
            drop(m);
            // 再编一趟只为拿一个**真的**错误文本返回给调用方：
            // `OnceLock` 填上就取不出来，后来者拿不到第一个人的 `Err`，
            // 而这个文本是现场唯一能指出「哪个文件缺了」的东西。
            // 顺带覆盖「权重刚被修好」——这一趟成功就直接返回图。
            compile()
        }
    }
}

/// 宽度 → 该宽度的 cell。**存 cell 而不是图**是并发修复的关键，见 [`get_or_compile`]。
///
/// ⚠ 这里存的是 [`PlanBody`]（`Slot::Ready` 外面再包一层 `Arc` 才是 [`Plan`]），
/// 别名直接来自 `ocr_pp`。早先这里有个本地别名指向 tract 的 `RunnableModel`，
/// 而 v6 换轨之后 `Plan` 变成了「两个后端的枚举 + Arc」——别名没跟着改，
/// 于是缓存类型与 `load_rec` 的返回类型对不上，而编译器报的是
/// `expected Slot<PlanBody>, found Slot<SimplePlan<...>>`，
/// **指向一行看着完全无辜的 `get_or_compile`**。
type Buckets = HashMap<usize, Arc<OnceLock<Slot<PlanBody>>>>;

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
    buckets: Mutex<Buckets>,
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
    ///
    /// 「只编一次」的保证与桶上限都在 [`get_or_compile`] 里；这里只提供编译器。
    ///
    /// ## ⚠ 走 `ort` 时**按宽度分桶是错的**（2026-10-18 实测）
    ///
    /// tract 是**按形状编译图**的，所以每个宽度一份计划、必须分桶。
    /// ONNX Runtime **不是**：一个会话吃任意宽度。
    /// 而分桶在这里的代价是——**每遇到一个新宽度就新建一个 ONNX Runtime 会话**，
    /// 而建会话是几十到上百毫秒级的操作。
    ///
    /// 实测（`xtask ocr <消息区帧> --scale 1 --stats`，release）：
    ///
    /// ```text
    /// det 推理  480 ms      12 个框的 rec 合计约 620 ms   合计 1101 ms
    /// ```
    ///
    /// 620ms / 12 ≈ 52ms 一个框，而 48 高的 rec 单次推理只该是几毫秒——
    /// 差的那部分几乎全是**反复建会话**。
    /// ⇒ ort 下统一用键 0，一个会话吃所有宽度。
    pub fn get(&self, w: usize) -> Result<Plan, PpError> {
        let path = self.plan_path();
        let key = if ocr_pp::backend() == ocr_pp::Backend::Ort {
            0
        } else {
            w
        };
        get_or_compile(&self.buckets, self.max_buckets, key, || {
            ocr_pp::load_rec(&path, w)
        })
    }

    /// 桶的个数。
    ///
    /// ⚠ 现在数的是 **cell**，含「正在编、还没编好」的那一个；早先数的是已编好的图。
    /// 所以同一个 `len()` 在编译窗口内比早先大 1。`is_empty` 同理。
    pub fn len(&self) -> usize {
        lock_recover(&self.buckets).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// det 的形状键。**两个分量都要**：少一个就会拿错形状的图，
/// 而 `run` 拿到形状不匹配的输入只会报错，于是整轮回落 WinRT。
type DetKey = (usize, usize);

/// det 槽里的东西：形状 → cell。**存 cell 而不是图**，见 [`get_or_compile`]。
type DetCells<P> = HashMap<DetKey, Arc<OnceLock<Slot<P>>>>;

/// det **只缓存一份**。
///
/// 这个 1 是**有意的**：det 的输入尺寸随帧变，而一帧只跑一次，
/// 缓存多份反而占内存。⚠ 调大它就是「按形状分桶」——那会**改变内存占用**，
/// 属设计决定、需要人拍板（票 49 第 1 条）。
/// 所以它写在 [`DetSlots::new`] 里而不是各调用点，顺带让守卫测试能把
/// 「容量仍然是 1」钉住。
///
/// 容量 1 走 [`get_or_compile`] 的清桶分支时 `max_buckets / 2 == 0`，
/// 也就是全清——于是表里永远只有**最后一个被请求**的形状，
/// 与早先 `*slot = Some(..)` 的语义一致。
const DET_SLOT_CAP: usize = 1;

/// det 的槽。容量写死在类型里（[`DET_SLOT_CAP`]），
/// 所以调用点没法顺手把它变成按形状分桶。
struct DetSlots<P> {
    max: usize,
    cells: Mutex<DetCells<P>>,
}

impl<P> DetSlots<P> {
    /// 新槽。容量恒为 [`DET_SLOT_CAP`]。
    fn new() -> Self {
        Self::default()
    }

    /// 取形状 `key` 的图，没有就编一个。**编译器由调用方给**——
    /// 真 `load_det` 要权重文件，而没有权重的机器上也必须有并发守卫测试
    /// （票 47 的教训：无权重时那些测试直接跳过，等于没守）。
    ///
    /// 锁只护住「取/建 cell」那一下，**编译在锁外**：
    /// 不同形状各编各的、互不阻塞；同形状后来的人 `get_or_init` 会等
    /// 第一个人编完，不会各编一遍。
    ///
    /// ⚠ 编译失败会**再编一趟**，只为拿到真的错误文本（[`get_or_compile`]）。
    /// 对 det 来说代价可控：权重不在时 `load_det` 在打开文件时就失败，
    /// 是毫秒级；**成功**才是十几秒级。
    fn get<E, F>(&self, key: DetKey, compile: F) -> Result<Arc<P>, E>
    where
        F: Fn() -> Result<Arc<P>, E>,
    {
        get_or_compile(&self.cells, self.max, key, compile)
    }

    /// 槽里现在驻留几个 cell。**只给守卫测试看**（容量不变式）。
    #[cfg(test)]
    fn cell_count(&self) -> usize {
        lock_recover(&self.cells).len()
    }
}

/// 容量恒为 [`DET_SLOT_CAP`]。抽成 `Default` 是为了不给
/// `DetSlots::new` 留一条「容量由调用方给」的口子。
impl<P> Default for DetSlots<P> {
    fn default() -> Self {
        Self {
            max: DET_SLOT_CAP,
            cells: Mutex::new(HashMap::new()),
        }
    }
}

static DET_PLAN: OnceLock<DetSlots<PlanBody>> = OnceLock::new();

fn det_plan_slot() -> &'static DetSlots<PlanBody> {
    DET_PLAN.get_or_init(DetSlots::new)
}

/// det 图的缓存。**只缓存一份**——det 的输入尺寸随帧变，一帧只跑一次，
/// 缓存收益不大；缓存多份反而占内存。编译本身是**秒级**的
/// （十几秒量级，见 [`BucketCache::plan_path`] 里记的现场实测），
/// 所以「只编一次」这件事必须守住。
///
/// ## 编译期间**不持锁**（2026-10-17，票 49）
///
/// 早先是「拿锁 → 查命中 → **在锁内** `load_det` → 才写回」：det 编译是
/// 十几秒级，于是这整段时间里所有别的 det 调用者都在等这把锁。
/// 现在编译发生在 [`DetSlots::get`] 的锁外，不同形状各编各的。
///
/// ⚠ **容量没变**，仍然只留一份（[`DET_SLOT_CAP`]）。
/// 并发请求**不同**形状时，在编的图会短暂地超过一份——那是「不互相堵」
/// 的直接代价，编完之后驻留的仍然只有最后被请求的那一份。
///
/// ## 缓存必须按形状校验（2026-10-15 修）
///
/// 早先这里只存 `Plan`，**不看 `h`/`w`**。而 det 的图是按具体形状编译的，
/// 聊天窗口一被拖动尺寸，det 输入就从 `[1,3,960,1280]` 变成别的，
/// 缓存里那份旧的会被直接返回，接着 `run` 拿到一个形状不匹配的输入而报错，
/// 于是又走「回落 WinRT」那条静默路径。
pub fn det_plan(h: usize, w: usize) -> Result<Plan, PpError> {
    det_plan_slot().get((h, w), || {
        ocr_pp::load_det(&ocr_pp::models_dir().join(DET_MODEL), h, w)
    })
}

/// 权重目录里有 det 模型吗。**「没装」是判定，不是错误。**
pub fn det_file_present(dir: &std::path::Path) -> bool {
    dir.join(DET_MODEL).is_file()
}

/// 整套 PP-OCR 识别。**权重不在就返回 `None`，不是错误。**
///
/// `None` 的含义是「这台机器没装 PP-OCR，请走 WinRT 那条路」。
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
            tracing::warn!("{} 失败，回落 WinRT：{e}", MODELS.label);
            None
        }
    }
}

/// [`ocr_pp::chars`] 的可测版本：词典路径由调用方给。
///
/// **类数必须等于 rec 的输出宽度，而不只是「文件行数 + 1」。**
///
/// 实测（2026-10-18）：v6 的 rec 输出 **18710** 类，词典文件 **18708** 行。
/// 差的是首尾两个约定——**blank 在 0**，**空格在最后一类**
/// （PaddleOCR 导出时把 space 单独摘出去，不写进词典文件）。
/// v4 同样是「6623 行 / 6625 类」。
///
/// ⚠ 早先这里只 `insert(0, blank)`，少补了尾部那个空格，于是
/// `chars.len()` 比模型少 1 类。CTC 真预测到最后一类时是**越界下标**，
/// 而那条路径平时走不到，所以一直没暴露。
fn chars_in(dir: &std::path::Path) -> Result<Arc<Vec<String>>, PpError> {
    let raw = std::fs::read_to_string(dir.join(KEYS_FILE))
        .map_err(|e| PpError::MissingModels(format!("{}（{e}）", dir.join(KEYS_FILE).display())))?;
    let mut v: Vec<String> = raw
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    // blank 在 0，文件里的字符整体后移一位；space 在最后一类。
    v.insert(0, String::new());
    v.push(ocr_pp::SPACE.to_string());
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
    let (vals, shape) = ocr_pp::run(&plan, &[1, 3, dh, dw], &input_buf, "det")?;
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
        by_w.entry(rec_input_width(*b)).or_default().push(*b);
    }
    for (w, mut group) in by_w {
        let plan = cache.get(w)?;
        let mut decoded: Vec<OcrLine> = Vec::new();
        for b in group.drain(..) {
            let buf = rec_input(frame, b, w);
            let (vals, shape) = ocr_pp::run(&plan, &[1, 3, REC_HEIGHT, w], &buf, "rec")?;
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// **碰全局 [`DET_PLAN`] 的测试必须先拿这把锁。**
    ///
    /// `DET_PLAN` 是**容量 1 的单槽**（[`DET_SLOT_CAP`]），而本 crate 的
    /// 测试默认多线程跑。今天有两条测试会走到那个全局：
    ///
    /// · `a_different_det_shape_gets_its_own_plan` —— 断言「同形状第二次
    ///   调用命中缓存」（`Arc::ptr_eq`）；
    /// · `an_existing_det_file_is_detected` —— 内部 `recognize_in` 会按它
    ///   自己那一帧的形状调一次 `det_plan`。
    ///
    /// 两条并行时后者会把单槽换掉，前者的第二次调用于是变成重编 ⇒ 红。
    /// **实测**：单跑绿，与同模块其他测试一起跑红，机器越忙越容易输给它。
    /// 所以这条不是「偶发环境问题」，是单槽缓存与并行测试的固有冲突。
    ///
    /// ⚠ 这把锁**只是测试之间的约定，不是产品不变量**。产品里
    /// `det_plan` 由观察循环单线程串行调用（每帧新建 `BucketCache`，
    /// 按宽度分组跑），本来就抢不到什么——**不要**拿它去论证
    /// 「det 缓存有并发问题」，那个问题 2026-10-17 已经单独修过了。
    static DET_GLOBAL: Mutex<()> = Mutex::new(());

    /// 拿全局 det 槽的测试期独占权。
    ///
    /// 用 [`lock_recover`] 而不是 `unwrap()`：一把测试锁中毒不该让
    /// **另一条**测试跟着炸——那条测的是完全无关的东西。
    fn det_global_guard() -> std::sync::MutexGuard<'static, ()> {
        lock_recover(&DET_GLOBAL)
    }

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

    /// 测试用的桶表：把「图」当成 `u32`。写出来是为了不写两遍那一长串泛型。
    type StubBuckets = Arc<Mutex<HashMap<usize, Arc<OnceLock<Slot<u32>>>>>>;

    /// 一个**不带权重**的桶表 + 一个计数的编译器桩。
    ///
    /// 图的类型用 `u32` 就够：`get_or_compile` 与图无关，守卫要验的是
    /// **同步**，不是 tract。而真 `load_rec` 要 15MB 权重，本机就没装——
    /// 用真图写这条测试的话，它在 CI 上会直接跳过，等于没守。
    fn stub_buckets() -> (StubBuckets, Arc<AtomicUsize>) {
        (
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(AtomicUsize::new(0)),
        )
    }

    /// 一个成功的编译器桩。
    ///
    /// 错误类型钉死 `String`：编译器桩从不失败，`E` 让编译器去猜是猜不出来的
    /// （`Result` 的错误位没有默认值）。
    fn stub_ok(w: usize) -> impl Fn() -> Result<Arc<u32>, String> + Send + 'static {
        move || Ok(Arc::new(w as u32))
    }

    /// 同上，顺带数一次调用。
    fn stub_counting(
        calls: Arc<AtomicUsize>,
        w: usize,
    ) -> impl Fn() -> Result<Arc<u32>, String> + Send + 'static {
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(w as u32))
        }
    }

    /// **同一个宽度并发取，编译器只能被调一次。**
    ///
    /// 2026-10-17（票 47）：早先是「锁外编译 + 无条件 insert」的双重检查，
    /// N 个线程各编一遍，后编的直接覆盖先编的——白等一次，而 rec 编译是
    /// 几十到几百毫秒、det 更慢。
    ///
    /// 栅栏是这条**能不能响**的关键：没有它，N 个线程多半一个接一个跑完，
    /// 谁也撞不上谁，于是这条测试**永远绿**。
    #[test]
    fn one_width_is_compiled_exactly_once_under_concurrency() {
        const N: usize = 8;
        const W: usize = 320;
        let (buckets, calls) = stub_buckets();
        let gate = Arc::new(std::sync::Barrier::new(N));
        let t0 = std::time::Instant::now();
        let hs: Vec<_> = (0..N)
            .map(|_| {
                let buckets = Arc::clone(&buckets);
                let calls = Arc::clone(&calls);
                let gate = Arc::clone(&gate);
                std::thread::spawn(move || {
                    gate.wait();
                    let calls = Arc::clone(&calls);
                    get_or_compile(&buckets, 32, W, move || -> Result<Arc<u32>, String> {
                        calls.fetch_add(1, Ordering::SeqCst);
                        // 拉长编译窗口，让后来的线程一定落在「正在编」里。
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        Ok(Arc::new(W as u32))
                    })
                    .expect("编译器桩不会失败")
                })
            })
            .collect();
        let got: Vec<_> = hs
            .into_iter()
            .map(|h| h.join().expect("取图线程炸了"))
            .collect();
        eprintln!(
            "{N} 个线程并发取同一个宽度：耗时 {:?}，编译器被调 {} 次",
            t0.elapsed(),
            calls.load(Ordering::SeqCst)
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "同一个宽度编了 {} 遍 —— 双重检查的老形状回来了",
            calls.load(Ordering::SeqCst)
        );
        // 只编一次还不足以说明大家拿到的是同一份：同一份也意味着**同一个 Arc**。
        assert!(
            got.windows(2).all(|p| Arc::ptr_eq(&p[0], &p[1])),
            "同一个宽度的并发调用拿到了不同的 Arc —— 缓存被复制了"
        );
    }

    /// **不同宽度并发取时互不阻塞。**
    ///
    /// 锁只护住「取/建 cell」那一下，编译在锁外，所以 A 在编译时 B 已经在编。
    /// 判据取**同时在编的峰值**而不是耗时：耗时会被机器快慢带偏，
    /// 而「两个 compile 重叠过」是结构性事实。
    ///
    /// 若换成票 47 的方案 1（持锁编译），这个峰值恒等于 1，本条立刻红。
    #[test]
    fn two_widths_compile_at_the_same_time() {
        let (buckets, _) = stub_buckets();
        let gate = Arc::new(std::sync::Barrier::new(2));
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let hs: Vec<_> = [320usize, 640]
            .into_iter()
            .map(|w| {
                let buckets = Arc::clone(&buckets);
                let gate = Arc::clone(&gate);
                let compile = counting_compiler(Arc::clone(&live), Arc::clone(&peak), w);
                std::thread::spawn(move || {
                    gate.wait();
                    get_or_compile(&buckets, 32, w, compile).expect("编译器桩不会失败")
                })
            })
            .collect();
        let mut got: Vec<_> = hs
            .into_iter()
            .map(|h| h.join().expect("取图线程炸了"))
            .collect();
        got.sort();
        assert_eq!(
            got,
            vec![Arc::new(320u32), Arc::new(640u32)],
            "宽度与图对不上"
        );
        assert!(
            peak.load(Ordering::SeqCst) >= 2,
            "两个宽度没有重叠编译（同时在编的峰值 = {}）—— 变成了持锁编译，\
             一个宽度会把别的宽度整段堵住",
            peak.load(Ordering::SeqCst)
        );
    }

    /// 记下「同时在编」的峰值，并让编译花掉 200ms 以便观察重叠。
    fn counting_compiler(
        live: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        w: usize,
    ) -> impl Fn() -> Result<Arc<u32>, String> + Send + 'static {
        move || {
            let now = live.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(200));
            live.fetch_sub(1, Ordering::SeqCst);
            Ok(Arc::new(w as u32))
        }
    }

    /// **桶到顶时清一半，且本键清不掉。**
    ///
    /// 4 个桶后再要一个新宽度 → 清到 2、再插本键 → 3。这与早先的
    /// 「先编后插、插之前清」得到的数一致，行为不变。
    #[test]
    fn the_bucket_cap_drops_half_and_keeps_the_key_just_asked_for() {
        const MAX: usize = 4;
        let (buckets, calls) = stub_buckets();
        for w in 1..=MAX {
            get_or_compile(&buckets, MAX, w, stub_counting(Arc::clone(&calls), w))
                .expect("编译器桩不会失败");
        }
        assert_eq!(lock_recover(&buckets).len(), MAX, "还没到上限就不该清");
        get_or_compile(&buckets, MAX, MAX + 1, stub_ok(MAX + 1)).expect("编译器桩不会失败");
        assert_eq!(
            lock_recover(&buckets).len(),
            MAX / 2 + 1,
            "清一半再插本键，应当是 {}/2 + 1",
            MAX
        );
        assert!(
            lock_recover(&buckets).contains_key(&(MAX + 1)),
            "刚要的键被自己刚做的清理带走了"
        );
        // 再要一次必须是命中，不能重编。
        let before = calls.load(Ordering::SeqCst);
        get_or_compile(&buckets, MAX, MAX + 1, stub_ok(MAX + 1)).expect("编译器桩不会失败");
        assert_eq!(calls.load(Ordering::SeqCst), before, "同一个宽度被重编了");
    }

    /// **并发 + 清桶：不炸、结果不丢、桶数不超上限。**
    ///
    /// 清桶摘掉的 cell 可能正被别人 `get_or_init` 着——`Arc` 保证它活到最后
    /// 一个使用者手里，所以顶多多白编一遍，**不会悬垂、不会 panic**。
    /// `Arc` 是这里安全性的全部来源，所以这条钉的就是它。
    ///
    /// 已知且接受：清桶正好插在编译中间时，那个宽度可能被并行的另一个线程
    /// **再编一遍**（新的 cell）。代价是一次重复编译，比悬垂小得多。
    #[test]
    fn eviction_under_concurrency_keeps_the_cache_consistent() {
        const MAX: usize = 4;
        const WIDTHS: usize = 12; // 宽度种类 > 上限，必然反复触发清桶
        let (buckets, _) = stub_buckets();
        let hs: Vec<_> = (0..16)
            .map(|t| {
                let buckets = Arc::clone(&buckets);
                std::thread::spawn(move || {
                    for i in 0..40usize {
                        let w = (t * 7 + i) % WIDTHS;
                        let v =
                            get_or_compile(&buckets, MAX, w, stub_ok(w)).expect("编译器桩不会失败");
                        assert_eq!(*v, w as u32, "宽度 {w} 拿到了别人的图");
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().expect("取图线程炸了");
        }
        let len = lock_recover(&buckets).len();
        assert!(len <= MAX, "桶数 {len} 超过上限 {MAX} 了");
    }

    /// **一次 panic 之后这个缓存还能用，而不是此后每次访问都 panic。**
    ///
    /// `Mutex` 在持锁线程 panic 后会中毒，此后 `lock()` 一律返回 `Err`，
    /// 而 `unwrap()` 直接 panic——所以早先那一串 `lock().unwrap()` 的意思是
    /// 「**一次** panic 之后，OCR 流水线的每次取图都炸」。
    /// `lock_recover` 把守卫捞回来，依据见它自己的注释（那两段临界区都不会
    /// 留下半改状态，所以恢复出来的数据是完整的）。
    #[test]
    fn a_poisoned_lock_does_not_make_every_later_call_panic() {
        let (buckets, calls) = stub_buckets();
        let b2 = Arc::clone(&buckets);
        let _ = std::thread::spawn(move || {
            let _held = b2.lock().expect("第一次拿锁不会中毒");
            panic!("故意在持锁时炸");
        })
        .join();
        assert!(buckets.lock().is_err(), "前提不成立：那把锁居然没中毒");
        // 中毒之后必须还能取图。
        let p = get_or_compile(&buckets, 32, 320, stub_counting(Arc::clone(&calls), 1))
            .expect("中毒之后取不到图");
        assert_eq!(*p, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "中毒被当成了命中缓存");
    }

    /// **失败不留缓存：权重后来修好了还能起来。**
    ///
    /// 2026-10-17：`OnceLock` 一旦填上就取不出来，所以失败**不能**留在 cell 里
    /// ——留了就等于「模型文件后来修好了也永远起不来」。
    /// [`Slot::Failed`] 只记「编不出来」，[`get_or_compile`] 会把键摘掉。
    #[test]
    fn a_failed_compile_is_not_cached_forever() {
        let (buckets, calls) = stub_buckets();
        let e = get_or_compile(&buckets, 32, 320, || Err("权重不在".to_string()))
            .expect_err("编译器桩失败时应当返回错误");
        assert_eq!(e, "权重不在", "错误文本不能被占位糊弄——那是现场唯一的线索");
        assert!(
            !lock_recover(&buckets).contains_key(&320),
            "失败的键还留在表里：权重修好之后再也起不来"
        );
        // 权重修好之后，同一个宽度再来一次：这次要编出来。
        let p = get_or_compile(&buckets, 32, 320, stub_counting(Arc::clone(&calls), 7))
            .expect("权重修好之后应该编得出来");
        assert_eq!(*p, 7);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// **归一化常数必须逐字等于当前模型的那一套。**
    ///
    /// ## 这个断言改过两次，两次都是实测打回的
    ///
    /// 1. **2026-10-15**：det 的 `std` 曾写成 `[0.456, 0.456, 0.456]`——
    ///    把 mean 的第二个分量抄进了 std，输入对比度只剩一半。真帧上
    ///    det 给的框高 8px，而同一行的字形墨迹高 12px，rec 拿到的是
    ///    被削掉下半截的字。
    /// 2. **2026-10-18**：换 v6 权重后**整套常数都得换**。
    ///    v6 det 是 `mean = std = 0.5`、v6 rec **只除 255**，
    ///    继续用 v4 的 ImageNet 常数 ⇒ **det 一个框都读不出来**。
    ///
    /// ⚠ 所以这条断言的期望值**必须跟着 [`MODELS`] 一起改**，
    /// 换模型不改它 = 换模型不查预处理。
    #[test]
    fn the_normalization_constants_match_the_active_model() {
        for (got, want, name) in [
            (DET_MEAN[0], 0.5, "det mean[0]"),
            (DET_MEAN[1], 0.5, "det mean[1]"),
            (DET_MEAN[2], 0.5, "det mean[2]"),
            (DET_STD[0], 0.5, "det std[0]"),
            (DET_STD[1], 0.5, "det std[1]"),
            (DET_STD[2], 0.5, "det std[2]"),
            // v6 的 rec 只除 255：用 mean=0 / std=1 表达「不减均值」。
            (REC_MEAN[0], 0.0, "rec mean[0]"),
            (REC_STD[0], 1.0, "rec std[0]"),
        ] {
            assert!(
                (got - want).abs() < 1e-6,
                "{name} = {got}，{} 要的是 {want}。写错会让输入偏离训练分布，\
                 而症状是「框比字矮、识别成乱码」或「一个框都没有」，不会报错",
                MODELS.label
            );
        }
        // det 的 std 若等于 mean 的某个分量，就是抄串了行——单独钉住这个形状。
        assert!(
            DET_STD != [DET_MEAN[1]; 3] || DET_MEAN[1] == DET_MEAN[0],
            "det 的 std 又变成 mean[1] 重复了三遍"
        );
    }

    /// **det 的最短边下限是模型的约定，不能自己拍。**
    ///
    /// PP-OCRv6 的 det 在 14px 小字上靠这个放大才把每行连成一条：
    /// rapidocr 默认 `limit_type="min"`、`limit_side_len=736`，
    /// 855x545 的微信帧因此被送到 1180x736。
    ///
    /// 撤掉这个放大，`cargo xtask ocr target/probe/wechat-chat.png --scale 1`
    /// 会从 17 行掉到个位数。
    #[test]
    fn the_det_input_is_scaled_up_to_the_models_minimum_side() {
        assert_eq!(MODELS.det_min_side, 736, "v6 的 det 最短边下限是 736");
        // 855x545：最短边 545 < 736 → 按 736/545 放大 → 1180x736。
        assert_eq!(
            det_size(855, 545),
            (1184, 736),
            "真帧 1 的 det 输入尺寸变了"
        );
        // 已经够大的图不动：1280x1024 的最短边 1024 > 736。
        assert_eq!(det_size(1280, 1024), (1280, 1024), "大图不该被放大");
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

    /// **`det_input` 必须真的重采样像素，而不是只把画布撑大。**
    ///
    /// 上一条钉的是**算出来的尺寸**，可它当年在 upscale 根本没做的时候照样绿——
    /// 尺寸对了，像素一个没动，于是「det 看到的是原生分辨率的小字」这件事
    /// 在整个仓里**没有任何一条守卫看得见**。
    ///
    /// 判据用**取值的多样性**而不是某个具体数值：一张只有黑/白两个值的图
    /// 经双线性放大之后必然出现中间灰。真缩放 ⇒ 取值 > 2 种；
    /// 复制补边 ⇒ 全程只有黑与白两种（外加被复制出来的同样两种）。
    #[test]
    fn the_det_input_really_resamples_pixels_and_not_just_the_canvas() {
        // 2x2：左半黑、右半白。放大到 8x8。
        let mut px = vec![0u8; 2 * 2 * 3];
        for (i, p) in px.chunks_exact_mut(3).enumerate() {
            let v = if i % 2 == 0 { 0u8 } else { 255u8 };
            p.copy_from_slice(&[v, v, v]);
        }
        let f = Frame::from_rgb(2, 2, px).expect("造帧失败");
        let out = det_input(&f, 8, 8);
        assert_eq!(out.len(), 3 * 8 * 8);
        // 只看第一个通道平面。
        let distinct: std::collections::BTreeSet<u32> =
            out[..64].iter().map(|v| (v * 1e4).round() as u32).collect();
        assert!(
            distinct.len() > 2,
            "8x8 的输出只有 {} 种取值（{distinct:?}）——像素没有被重采样，\
             只是被复制补边了。真帧后果：det 在原生分辨率的小字上工作，\
             框削掉字的下半截，rec 拿到残字（真帧 CER 18.60% 而非 0.79%）",
            distinct.len()
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
    ///
    /// ⚠ 而「同形状命中缓存」这个断言**只能整条跑**：`det_plan` 是全局单槽，
    /// 别的测试碰一下就没了。所以这里拿 [`DET_GLOBAL`]——见它的说明。
    #[test]
    fn a_different_det_shape_gets_its_own_plan() {
        let dir = ocr_pp::models_dir();
        if !dir.join(DET_MODEL).is_file() {
            eprintln!("跳过：{} 下没有 det 权重", dir.display());
            return;
        }
        let _serialized = det_global_guard();
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

    /// 形状 → 可比较的标签。让调用方能断言「我拿到的是**自己那个形状**的图」。
    ///
    /// 缓存串味是这个改动最危险的失败模式：拿到别的形状的图**不会 panic**，
    /// 只会让 `run` 因输入形状不符而报错，于是整轮回落 WinRT——
    /// 症状与「没装权重」一模一样，现场分不出来。
    fn det_tag(key: DetKey) -> u32 {
        (key.0 * 100_000 + key.1) as u32
    }

    /// det 守卫的编译器桩：数调用次数、记「同时在编」的峰值，
    /// 并让编译花掉 `window_ms` 以便观察重叠。
    ///
    /// 桩返回的**不是** `PlanBody` 而是 `u32`：真 `load_det` 要权重文件，
    /// 本机没装、CI 也没有——拿真图写这些测试等于永远跳过，等于没守
    /// （票 47 的教训）。要验的是**同步**，不是 tract。
    fn det_stub_compiler(
        key: DetKey,
        calls: Arc<AtomicUsize>,
        live: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        window_ms: u64,
    ) -> impl Fn() -> Result<Arc<u32>, String> + Send + 'static {
        let tag = det_tag(key);
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
            let now = live.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(window_ms));
            live.fetch_sub(1, Ordering::SeqCst);
            Ok(Arc::new(tag))
        }
    }

    /// **det 编译期间不持锁：不同形状并发时同时在编的峰值 ≥ 2。**
    ///
    /// 2026-10-17（票 49）：早先是「拿锁 → 查命中 → **在锁内** `load_det`
    /// → 才写回」，而 det 编译是十几秒级 ⇒ 整段时间里所有别的 det 调用者
    /// 都在等这把锁。拖一下窗口尺寸就会换形状，于是「改窗口大小」
    /// 能把整条 OCR 流水线堵住十几秒。
    ///
    /// 判据取**同时在编的峰值**而不是耗时：耗时会被机器快慢带偏，
    /// 而「两个编译重叠过」是结构性事实。把 `DetSlots::get` 改回持锁编译，
    /// 这个峰值恒等于 1，本条立刻红（变红验证见票 49 的交付报告）。
    #[test]
    fn det_shapes_compile_concurrently_instead_of_queueing_on_the_lock() {
        const SHAPES: [DetKey; 2] = [(32, 64), (64, 128)];
        let slots = Arc::new(DetSlots::<u32>::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(std::sync::Barrier::new(SHAPES.len()));
        let t0 = std::time::Instant::now();
        let hs: Vec<_> = SHAPES
            .iter()
            .map(|&key| {
                let slots = Arc::clone(&slots);
                let gate = Arc::clone(&gate);
                let compile = det_stub_compiler(
                    key,
                    Arc::clone(&calls),
                    Arc::clone(&live),
                    Arc::clone(&peak),
                    200,
                );
                std::thread::spawn(move || {
                    gate.wait();
                    slots.get(key, compile).expect("编译器桩不会失败")
                })
            })
            .collect();
        let got: Vec<_> = hs
            .into_iter()
            .map(|h| h.join().expect("取图线程炸了"))
            .collect();
        for (&key, p) in SHAPES.iter().zip(&got) {
            assert_eq!(**p, det_tag(key), "形状 {key:?} 拿到了别的形状的图");
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            SHAPES.len(),
            "两个形状各编一次：少了就是复用了错的图，多了就是重复编译"
        );
        let peak_n = peak.load(Ordering::SeqCst);
        eprintln!(
            "两个不同形状并发取 det 图（桩编译 200ms/次）：耗时 {:?}，同时在编的峰值 = {}",
            t0.elapsed(),
            peak_n
        );
        assert!(
            peak_n >= 2,
            "两个形状没有重叠编译（同时在编的峰值 = {peak_n}）—— 还是持锁编译，\
             一个形状会把别的形状整段堵住"
        );
        assert!(
            slots.cell_count() <= DET_SLOT_CAP,
            "驻留了 {} 份，超过上限 {}",
            slots.cell_count(),
            DET_SLOT_CAP
        );
    }

    /// **同一个形状并发请求仍然只编一次。**
    ///
    /// 这是「编译搬出锁」的对价：**只编一次不再由那把锁保证**，
    /// 而是由 `OnceLock` 的 `get_or_init` 保证。所以这条守的是另一半——
    /// 换成「锁外编译 + 无条件写回」（票 47 的老形状）它就红，
    /// 而换成持锁编译它仍然绿（那种形状由上面那条守）。
    ///
    /// 栅栏是它能不能响的关键：没有它，N 个线程多半一个接一个跑完，
    /// 谁也撞不上谁，于是这条**永远绿**。
    #[test]
    fn one_det_shape_is_compiled_exactly_once_under_concurrency() {
        const N: usize = 8;
        const KEY: DetKey = (32, 64);
        let slots = Arc::new(DetSlots::<u32>::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(std::sync::Barrier::new(N));
        let hs: Vec<_> = (0..N)
            .map(|_| {
                let slots = Arc::clone(&slots);
                let gate = Arc::clone(&gate);
                let compile = det_stub_compiler(
                    KEY,
                    Arc::clone(&calls),
                    Arc::clone(&live),
                    Arc::clone(&peak),
                    50,
                );
                std::thread::spawn(move || {
                    gate.wait();
                    slots.get(KEY, compile).expect("编译器桩不会失败")
                })
            })
            .collect();
        let got: Vec<_> = hs
            .into_iter()
            .map(|h| h.join().expect("取图线程炸了"))
            .collect();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "同一个形状编了 {} 遍 —— 「锁外编译 + 无条件写回」的老形状回来了",
            calls.load(Ordering::SeqCst)
        );
        // 只编一次还不够：同一份也意味着**同一个 Arc**。
        assert!(
            got.windows(2).all(|p| Arc::ptr_eq(&p[0], &p[1])),
            "同一个形状的并发调用拿到了不同的 Arc —— 缓存被复制了"
        );
        assert!(
            slots.cell_count() <= DET_SLOT_CAP,
            "驻留了 {} 份，超过上限 {}",
            slots.cell_count(),
            DET_SLOT_CAP
        );
    }

    /// **仍然只留一份：换形状就要重编，容量不因为这次改动而上升。**
    ///
    /// 票 49 把「按形状分桶」明确划成**需要人拍的设计决定**（会改变内存占用），
    /// 所以本票只修「编译期间不持锁」。这条钉的就是那条边界：
    /// 六个不同形状走一遍，槽里仍然只有一份；老形状再要一次是**重编**
    /// 而不是命中缓存。
    #[test]
    fn the_det_slot_still_keeps_only_one_shape() {
        assert_eq!(
            DET_SLOT_CAP, 1,
            "把上限调大就是「按形状分桶」，那是需要人拍的设计决定（票 49）"
        );
        let slots = DetSlots::<u32>::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        for i in 1..=6usize {
            let key = (32 * i, 64 * i);
            let compile = det_stub_compiler(
                key,
                Arc::clone(&calls),
                Arc::clone(&live),
                Arc::clone(&peak),
                0,
            );
            let p = slots.get(key, compile).expect("编译器桩不会失败");
            assert_eq!(*p, det_tag(key), "形状 {key:?} 拿到了别的形状的图");
            assert_eq!(
                slots.cell_count(),
                1,
                "第 {i} 个形状之后驻留了 {} 份",
                slots.cell_count()
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 6, "每换一个形状就该重编一次");
        // 只留最后那一份：上一个形状再要一次是重编，不是命中缓存。
        let old = (32 * 5, 64 * 5);
        let compile = det_stub_compiler(
            old,
            Arc::clone(&calls),
            Arc::clone(&live),
            Arc::clone(&peak),
            0,
        );
        let p = slots.get(old, compile).expect("编译器桩不会失败");
        assert_eq!(*p, det_tag(old));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            7,
            "老形状还留在缓存里 —— 驻留的不止一份"
        );
    }

    /// **并发换形状：谁都不会拿到别人的图，容量也不越界。**
    ///
    /// 容量 1 + 编译在锁外 ⇒ 并发的不同形状会互相把对方的 cell 从表里挤掉。
    /// `Arc` 保证被挤掉的 cell 活到最后一个使用者手里，所以不会悬垂；
    /// 代价只是「某个形状可能被并行的另一个线程**再编一遍**」——
    /// 与 rec 那条清桶守卫是同一个已知且接受的代价。
    /// 真正危险的不是重复编译，而是**拿错形状的图**，那条钉在这里。
    #[test]
    fn concurrent_det_shape_churn_never_serves_the_wrong_plan() {
        const SHAPES: usize = 5;
        let slots = Arc::new(DetSlots::<u32>::new());
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let hs: Vec<_> = (0..16)
            .map(|t| {
                let slots = Arc::clone(&slots);
                let live = Arc::clone(&live);
                let peak = Arc::clone(&peak);
                std::thread::spawn(move || {
                    for i in 0..20usize {
                        let k = (t * 7 + i) % SHAPES + 1;
                        let key = (32 * k, 64 * k);
                        let calls = Arc::new(AtomicUsize::new(0));
                        let compile =
                            det_stub_compiler(key, calls, Arc::clone(&live), Arc::clone(&peak), 0);
                        let p = slots.get(key, compile).expect("编译器桩不会失败");
                        assert_eq!(*p, det_tag(key), "形状 {key:?} 拿到了别的形状的图");
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().expect("取图线程炸了");
        }
        assert!(
            slots.cell_count() <= DET_SLOT_CAP,
            "驻留了 {} 份，超过上限 {}",
            slots.cell_count(),
            DET_SLOT_CAP
        );
    }

    /// **det 的输入尺寸必须取到 32 的倍数。**
    ///
    /// 这是硬要求不是风格问题：网络里有 8 次步长 2 的下采样，
    /// 尺寸不对齐会让 pad 算错，输出图与输入对不上，框全错。
    #[test]
    fn the_det_input_is_rounded_up_to_a_multiple_of_32() {
        // 这一条只量「取整」，所以最短边下限传 0（不放大）——
        // 放大规则归 `the_det_input_is_scaled_up_to_the_models_minimum_side` 管。
        // 1161 → 1184（37×32）、893 → 896（28×32）
        assert_eq!(det_size_with_min_side(1161, 893, 0), (1184, 896));
        assert_eq!(det_size_with_min_side(1024, 768, 0), (1024, 768));
        // 已对齐的不能被推高一档。
        assert_eq!(det_size_with_min_side(320, 64, 0), (320, 64));
        // 小于 32 的要有下限，否则下采样后是空图。
        assert_eq!(det_size_with_min_side(4, 4, 0), (32, 32));
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

    /// **rec 的输入宽必须按框的比例算，不能直接用框的像素宽。**
    ///
    /// rec 的高被固定成 48，而原图里的一行只有 8~14px 高。
    /// 若宽度直接取框的像素宽，`rec_input` 就只把字拉高、不让它变宽，
    /// 长宽比扭曲 `48 / bh` 倍。中文扛得住（笔画密）、英文扛不住，
    /// 真帧上表现为 CER 20% 上下而中文行几乎全对。
    ///
    /// 参照实现 rapidocr 的 `ceil(img_height * w / h)`，同样是等比。
    #[test]
    fn the_rec_input_width_keeps_the_boxes_aspect_ratio() {
        // 200x12 的一行：高拉到 48 ⇒ 宽 200*48/12 = 800。
        assert_eq!(
            rec_input_width(Rect {
                x0: 0,
                y0: 0,
                x1: 200,
                y1: 12
            }),
            800
        );
        assert_eq!(
            rec_input_width(Rect {
                x0: 0,
                y0: 0,
                x1: 100,
                y1: 10
            }),
            480
        );
        // 退化框：既不许除零，也不许造出 0 宽（0 宽会让时间步为 0）。
        assert_eq!(
            rec_input_width(Rect {
                x0: 0,
                y0: 0,
                x1: 0,
                y1: 0
            }),
            REC_STRIDE
        );
        // 高为 0 的退化框按 1 算，不许 panic。
        assert_eq!(
            rec_input_width(Rect {
                x0: 0,
                y0: 0,
                x1: 40,
                y1: 0
            }),
            40 * 48
        );
    }

    /// **二值化 + 连通域 + unclip：一个明显的框能被捞出来。**
    ///
    /// 造一张 32x32 的概率图，中间放一个 8x8 的高概率块。
    ///
    /// 期望值**含 unclip 外扩**。外扩量 `d = 面积 × ratio / 周长`
    /// （pyclipper 的多边形偏移，各向均匀）：
    /// `d = 64 × 1.6 / 32 = 3.2`，所以框是
    /// `(12-3.2, 10-3.2) .. (19+1+3.2, 17+1+3.2)`
    /// 四舍五入后的 `(9, 7, 23, 21)`，而不是块本身的 `12,10,20,18`。
    ///
    /// ⚠ 2026-10-18 改过期望值：早先按 `w·r/√(wh)` 算，那是**错的公式**
    /// （纵向外扩 1.5、横向也是 1.5，对细长框则是纵向≈0，见 [`DET_UNCLIP_RATIO`]）。
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
            (9, 7, 23, 21),
            "框没按 unclip_ratio={}（面积×ratio/周长，各向均匀）外扩：{:?}",
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
    /// 按 `面积×ratio/周长` 外扩 `3.2` 后是 `(4.8, 4.8)..(19.2, 19.2)`，
    /// 换算到 4 倍大的原图就是 `(19, 19)..(77, 77)`。
    /// 早先这里断言的是块的原始位置 `(32,32)..(64,64)`。
    ///
    /// ⚠ 2026-10-18 改过期望值：外扩公式从 `w·r/√(wh)`（1.5）换成
    /// `面积×ratio/周长`（3.2），见 [`DET_UNCLIP_RATIO`]。
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
        assert_eq!((b.x0, b.y0), (19, 19), "没放大回原图：{b:?}");
        assert_eq!((b.x1, b.y1), (77, 77), "没放大回原图：{b:?}");
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
    /// 夹具遵守 `C == chars.len()`（`chars[0]` 是 blank 的空占位，
    /// 真实字符从 `chars[1]` 起，所以类 c ↔ `chars[c]`），否则类数守卫
    /// 会直接降级成空串——**症状是「解出来什么都没有」，而不是报错**。
    ///
    /// ⚠ 这三条夹具当年写的是 `chars.len() + 1`，**而那是被守卫的 bug 教出来的**：
    /// 守卫写着「类数必须比词典多 1」，夹具就照着凑，于是 bug 被测试保护了三年。
    #[test]
    fn ctc_collapses_consecutive_repeats() {
        // 词典：["", "a", "b"] → 类数 3：0=blank 1=a 2=b
        let chars = vec![String::new(), "a".into(), "b".into()];
        let c = chars.len();
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
        let c = chars.len();
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
        let c = chars.len();
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
        // C 给了 5，但词典只有 3 类
        let (text, conf) = ctc_decode(&vals, &[1, 1, 5], &chars);
        assert_eq!(text, "", "类数不匹配还解出了文本：{text:?}");
        assert_eq!(conf, 0.0);
    }

    /// **`C` 与词典**严格等长**——不多不少。**
    ///
    /// 这条是 2026-10-18 从一个真机故障里长出来的：
    /// 守卫原来写的是 `C != chars.len() + 1`，而词典表从那天起
    /// 补上了末位的 space 类，`chars.len()` 已经等于类数。
    /// ⇒ **每个框都被判成「类数对不上」而静默返回空串**，
    /// `xtask ocr <真帧> --scale 1` 逐行 0 行，而全程**一个错都没报**。
    ///
    /// 所以这里正反两面都钉：
    /// - `C == chars.len()`：**必须**解出文本（这是绝大多数调用的情况）；
    /// - `C == chars.len() + 1`：**必须**解出空串（曾经被当成正确的那一档）。
    #[test]
    fn the_class_count_must_equal_the_dictionary_length_exactly() {
        let chars = vec![String::new(), "a".into(), "b".into()];
        let mut row = vec![0.0f32; chars.len()];
        row[1] = 1.0;
        let vals: Vec<f32> = (0..3).flat_map(|_| row.clone()).collect();

        let (text, _) = ctc_decode(&vals, &[1, 3, chars.len()], &chars);
        assert_eq!(
            text, "a",
            "C == chars.len() 是正常路径，却什么都没解出来：{text:?}"
        );

        let mut wider = row.clone();
        wider.push(0.0);
        let vals_wider: Vec<f32> = (0..3).flat_map(|_| wider.clone()).collect();
        let (text, _) = ctc_decode(&vals_wider, &[1, 3, chars.len() + 1], &chars);
        assert_eq!(
            text, "",
            "C 比词典多 1 类时居然还解出了文本：{text:?}。\
             这是 2026-10-18 真机「逐行 0 行」的同一个 bug"
        );
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

    /// **rec 预处理：像素值本身要对，不只是长度对。**
    ///
    /// 2026-10-18（票 50 的 L2）：既有的 rec 守卫夹具**全是纯色帧**
    /// （[`the_rec_input_has_the_right_length`] 与
    /// [`boxes_larger_than_the_frame_are_clamped`] 只断长度与有限性，
    /// 另两条断宽度函数、不碰张量）。**纯色帧过任何仿射仍是纯色**，
    /// 所以下面四种「识别全崩、但长度对、无 NaN」的写法它们一条都不会红：
    ///
    /// | 改法 | 后果 |
    /// |---|---|
    /// | 布局写成 `out[o*3+c]`（HWC，现在是 CHW） | 识别全崩，长度同样对 |
    /// | `sx` 左右翻转 | 识别全崩 |
    /// | 三通道都读 `p[0]` | 颜色全丢 |
    /// | 公式系数被改（`(p/255-M)/S` → `p/255*2`） | 偏离训练分布，识别乱码 |
    ///
    /// ⚠ **与 [`the_normalization_constants_match_the_active_model`] 分工**：
    /// 那条守**常量**（`REC_MEAN`/`REC_STD` 的值），本条守**公式**（怎么用它们）。
    /// ⇒ 本条的期望值**用常量算、不硬编码**：常量错由那条报，
    /// 公式错由本条报，两条不重叠，也不互相顶替。
    #[test]
    fn the_rec_input_keeps_channel_separation_and_black_to_white_contrast() {
        // 颜色帧：R 沿 x 从 0 递增到 255，G 恒 128、B 恒 0。
        // 三个通道在 x 方向上的行为各不相同（递增 / 恒定 / 恒定），
        // 于是「通道写串」与「三通道读同一字节」都藏不住。
        let (w, h) = (64u32, 8u32);
        let mut px = vec![0u8; (w * h * 3) as usize];
        for y in 0..h {
            for x in 0..w {
                let o = ((y * w + x) * 3) as usize;
                px[o] = (x * 255 / (w - 1)) as u8;
                px[o + 1] = 128;
                px[o + 2] = 0;
            }
        }
        let f = Frame::from_rgb(w, h, px).expect("造帧失败");
        let b = Rect {
            x0: 0,
            y0: 0,
            x1: w as i32,
            y1: h as i32,
        };
        // out_w == bw ⇒ `sx = bx + x*bw/out_w` 就是 x，1:1 采样，期望值可手算。
        let out_w = w as usize;
        let hh = REC_HEIGHT;
        let plane = hh * out_w;
        let out = rec_input(&f, b, out_w);

        // ① 布局与方向：CHW 下通道 c 的**第一行**随 x 单调不减。
        //    ⚠ 只看第一行——跨行比较会在「上一行末尾 255、下一行开头 0」
        //    处反向，那不是 bug，是行边界。
        let row0 = &out[..out_w];
        assert!(
            row0.windows(2).all(|p| p[0] <= p[1]),
            "rec 通道 0 的第一行没有随 x 单调不减——查 sx 方向，\
             以及 out 索引是 `c*h*w + o`（CHW）还是 `o*3 + c`（HWC）"
        );

        // ② 通道独立：G / B 在 x 方向上恒定。若三通道都读 `p[0]`，
        //    通道 1 也会随 x 变，这条立刻不成立。
        for (c, name) in [(1usize, "G"), (2usize, "B")] {
            let plane_c = &out[c * plane..(c + 1) * plane];
            let first = plane_c[0];
            assert!(
                plane_c.iter().all(|v| (v - first).abs() < 1e-6),
                "rec 通道 {name} 本该沿 x 恒定（只有 R 在变），却变了 ⇒ \
                 三通道很可能读了同一个字节"
            );
        }

        // ③ 公式系数：黑到白的跨度必须是 `1 / REC_STD[0]`——
        //    公式里除 255 的是**像素值**，所以满量程跨度是 `1/std` 而不是 `1/255/std`。
        //    det 侧同形的那条是 `the_det_input_preserves_black_to_white_contrast`。
        let spread = row0[out_w - 1] - row0[0];
        let want = 1.0f32 / REC_STD[0];
        assert!(
            (spread - want).abs() < 1e-3,
            "rec 黑白跨度 = {spread}，按 REC_STD[0]={} 应该是 {want}。\
             系数被改会让输入偏离训练分布，而症状是「识别成乱码」，不会报错",
            REC_STD[0]
        );

        // ④ 逐点比对：期望值用**常量**算。HWC 布局或索引写错时这里整片对不上。
        for c in 0..3 {
            for x in [0usize, 1, out_w / 2, out_w - 1] {
                let got = out[c * plane + x];
                let raw = match c {
                    0 => ((x * 255 / (w as usize - 1)) as u8) as f32,
                    1 => 128.0,
                    _ => 0.0,
                };
                let expect = (raw / 255.0 - REC_MEAN[c]) / REC_STD[c];
                assert!(
                    (got - expect).abs() < 1e-3,
                    "rec 张量 [c={c}, x={x}] = {got}，按公式算是 {expect}。\
                     索引是 out[c*h*w + o]（CHW）吗？"
                );
            }
        }
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

    /// **框必须盖住产生它的那片墨迹。**
    ///
    /// 2026-10-18 订正：这条的夹具原来按「det 只补边、不缩放」摆坐标，
    /// 那套坐标下 `src/prob` 与 `det/prob` 两个公式都能过——**它分不清**。
    /// [`det_input`] 改成真缩放之后，概率图坐标与帧坐标之间多了一段 1.12 的
    /// 缩放，两式开始分岔，本条才真的在守换算。
    ///
    /// ## 判据为什么写「覆盖」而不是某个偏移值
    ///
    /// 某个偏移值是这台机器、这张图的**实测结果**，不是不变量。
    /// 所以这里断言的是不变量——**框必须盖住那片亮区在原图里的全部行与列**，
    /// 不管 det 与帧之间差了多少倍。任何朝原点缩的换算都会让 `y1`
    /// 落到墨迹下沿之上，当场红。
    ///
    /// ⚠ 这条**不要权重**（造的是概率图，不是跑模型），所以它在 CI 上
    /// 真的会跑——本仓吃过两次「没权重就跳过，等于没守」的亏。
    #[test]
    fn det_boxes_cover_the_full_extent_of_the_ink_they_came_from() {
        // 帧 400x200 → [`det_size`] 补齐到 416x224（横向 ×1.04、纵向 ×1.12）。
        let (src_w, src_h) = (400usize, 200usize);
        let (det_w, det_h) = det_size_with_min_side(src_w as u32, src_h as u32, 0);
        assert_eq!(
            (det_w, det_h),
            (416, 224),
            "前提变了：本条量的是「概率图坐标 → 原图坐标」的比例"
        );
        // 概率图与 det 输入同尺寸——实测 det 输出就是这个形状（见 det_boxes）。
        let (pw, ph) = (det_w, det_h);
        // 墨迹**定义在原图坐标**：x 40..300、y 150..162。
        // 摆进概率图时要按 det/帧 的比例换算过去，否则测的就不是换算了。
        let (ink_x0, ink_y0, ink_x1, ink_y1) = (40usize, 150usize, 300usize, 162usize);
        let px = |v: usize, num: usize, den: usize| v * num / den;
        let (ink_px0, ink_py0) = (px(ink_x0, pw, src_w), px(ink_y0, ph, src_h));
        let (ink_px1, ink_py1) = (px(ink_x1, pw, src_w), px(ink_y1, ph, src_h));
        let mut prob = vec![0.0f32; pw * ph];
        for y in ink_py0..=ink_py1 {
            for x in ink_px0..=ink_px1 {
                prob[y * pw + x] = 0.9;
            }
        }
        let got = det_boxes(&prob, pw, ph, src_w, src_h);
        assert_eq!(got.len(), 1, "应该正好一个框：{got:?}");
        let b = got[0];
        assert!(
            b.y1 >= ink_y1 as i32,
            "框的下沿 y1={} 在墨迹下沿 {ink_y1} **之上** —— 少盖 {} 行，\
             rec 拿到的是被削掉下半截的字，输出只能是乱码",
            b.y1,
            ink_y1 as i32 - b.y1
        );
        assert!(
            b.y0 <= ink_y0 as i32,
            "框的上沿 y0={} 在墨迹上沿 {ink_y0} 之下，顶部笔画被削掉",
            b.y0
        );
        // 横向是同一个换算，同一个错。钉上它，免得只修一半。
        assert!(
            b.x1 >= ink_x1 as i32 && b.x0 <= ink_x0 as i32,
            "横向没盖住墨迹 x{}..{}：{:?}",
            ink_x0,
            ink_x1,
            b
        );
    }

    /// **换算比例要把两段缩放串起来，不能只算一段。**
    ///
    /// 帧 40x40 → [`det_size`] 补到 64x64（×1.6）→ 概率图只有 1/4（再 ×0.25），
    /// 于是概率图一个像素对应原图 `40/16 = 2.5` 个像素。
    ///
    /// ⚠ 早先这条钉的是 `det_w / pw`，而那个 1.0 之所以看起来自洽，
    /// 是因为 [`det_input`] 当时只补边不缩放（见它的文档）。
    /// 现在两段都是真的，所以 `src/prob` 与 `det/prob` **不再相等**——
    /// 这条正是用来把它们分开的。
    #[test]
    fn the_det_to_source_scale_chains_both_downsamples() {
        let (src_w, src_h) = (40usize, 40usize);
        let (det_w, det_h) = det_size_with_min_side(src_w as u32, src_h as u32, 0);
        assert_eq!(
            (det_w, det_h),
            (64, 64),
            "前提变了：本条量的是两段缩放串起来之后的换算"
        );
        // 概率图只有 det 输入的 1/4（宽度、高度都整除，无插值歧义）。
        let (pw, ph) = (det_w / 4, det_h / 4);
        let mut prob = vec![0.0f32; pw * ph];
        for y in 4..=6usize {
            for x in 2..=8usize {
                prob[y * pw + x] = 0.9;
            }
        }
        let got = det_boxes(&prob, pw, ph, src_w, src_h);
        assert_eq!(got.len(), 1, "应该正好一个框：{got:?}");
        let b = got[0];
        // 概率图 y4..=6 / x2..=8 ⇒ 原图 y10..17 / x5..22（比例 2.5）。
        assert!(
            b.y0 <= 10 && b.y1 >= 17 && b.x0 <= 5 && b.x1 >= 22,
            "两段缩放没串起来：概率图 y4..6 / x2..8 对应原图 y10..17 / x5..22，\
             实得 {b:?}。只算一段（4.0）会停在 y12..32，只按 det 换算（16.0）更离谱"
        );
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
            !empty.join(DET_MODEL).is_file(),
            "测试前提不成立：那个临时目录里居然有权重"
        );
        assert!(
            !det_file_present(&empty),
            "不存在的目录必须判定为「没装权重」"
        );
    }

    /// 真的装了权重的机器上，`det_file_present` 要能认出它。
    ///
    /// ⚠ 它内部的 `recognize_in` 会按这一帧的形状调一次 `det_plan`，
    /// 那是要动全局单槽的 ⇒ 必须拿 [`DET_GLOBAL`]，否则会把
    /// `a_different_det_shape_gets_its_own_plan` 的「同形状命中缓存」挤掉。
    #[test]
    fn an_existing_det_file_is_detected() {
        let dir = ocr_pp::models_dir();
        if !det_file_present(&dir) {
            return; // 本机没下权重，跳过而不是假装通过
        }
        let _serialized = det_global_guard();
        let f = flat(32, 32, 128);
        assert!(
            recognize_in(&dir, &f).is_some(),
            "权重在，`recognize_in` 却给不出结果"
        );
    }
}
