//! PP-OCRv4 本地推理（det + rec），纯 Rust 跑 ONNX。
//!
//! ## 为什么不原样搬 RapidOCR
//!
//! 搬过来的话要带 `opencv`、`libtorch` 那一类东西，而应用是**装进
//! `C:\Program Files` 给普通用户跑的**，塞一个 C++ 运行时进去就多一层
//! 「装不上」的失败面。这里只用 `tract`：纯 Rust，静态链接。
//!
//! ## 当前进度（2026-10-10）
//!
//! **只有「加载层」通了，推理管线还没写。** 见文末「已完成 / 未完成」。
//! 现在能跑通的是：读模型、读输入输出形状、按指定尺寸编译成可执行计划、
//! 跑一次 det 并拿到 `[1,1,H,W]` 的概率图。
//!
//! ## 与 [`crate::ocr`] 的关系
//!
//! **不取代它。** `Windows.Media.Ocr` 系统自带、零模型体积，是默认值；
//! 这条路要 `%APPDATA%\dev.chatservant.app\models\` 里有权重才跑得起来。
//! 谁胜出还没接（[`crate::ocr::recognize`] 仍直接走 WinRT）——见交接文档。
//!
//! ## 词典是 **6624** 类，不是 6623
//!
//! `ppocr_keys_v1.txt` 有 6623 行，**blank 索引 0 不在文件里**——
//! 那是 CTC 约定，不是第 6624 个汉字。见 [`chars`]。

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use tract_onnx::prelude::*;
// ⚠ `Factoid` **不在** `tract_onnx::prelude` 里：prelude 只带了
// `InferenceFact` / `InferenceModel` / `*ModelExt` 加 `tract_core::prelude`，
// 形状维度那一套（`concretize`）藏在 `tract_hir::infer` 下面。
use tract_onnx::tract_hir::infer::Factoid;

/// 编译好的可执行计划。`Arc` 是为了 clone 进多帧复用。
pub type Plan = Arc<RunnableModel<TypedFact, Box<dyn TypedOp>>>;

/// det 的输入事实。`[1,3,H,W]`，H/W 取 32 的倍数。
pub fn det_fact(h: usize, w: usize) -> InferenceFact {
    f32::fact([1usize, 3, h, w]).into()
}

/// rec 的输入事实。`[1,3,48,W]`，W 取 8 的倍数。
pub fn rec_fact(w: usize) -> InferenceFact {
    f32::fact([1usize, 3, 48, w]).into()
}

/// 模型自报的输入输出形状。动态维记 **-1**。
#[derive(Debug, Clone, PartialEq)]
pub struct IoSpec {
    pub inputs: Vec<Vec<i64>>,
    pub outputs: Vec<Vec<i64>>,
}

#[derive(Debug, thiserror::Error)]
pub enum PpError {
    #[error("模型目录不存在或缺文件：{0}\n     下载办法见 docs/HANDOFF.md §4.2")]
    MissingModels(String),

    #[error("加载 {name} 失败：{msg}")]
    Load { name: String, msg: String },

    #[error("{stage} 失败：{msg}")]
    Run { stage: String, msg: String },

    #[error("模型输入输出形状跟代码假定的不一样：{0}")]
    Shape(String),
}

/// 权重目录。默认 `%APPDATA%\dev.chatservant.app\models\`。
///
/// **不进仓库。** 15MB 的二进制权重 + 模型许可不是这个仓库该背的东西。
pub fn models_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("CHATSERVANT_OCR_MODELS") {
        return PathBuf::from(d);
    }
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        // 非 Windows 上没有 APPDATA。测试与 CI 就靠这条落到临时目录去。
        .unwrap_or_else(std::env::temp_dir)
        .join("dev.chatservant.app")
        .join("models")
}

/// 只读形状，不跑推理。给探针和排查用——形状对不上时要能在**不**跑推理的
/// 情况下就知道差在哪。
///
/// ## ⚠ 只有**输入**维可信，输出维读不出来（2026-10-10 实测，三次踩坑）
///
/// - **`model_for_path` 给的模型没推导过。** 输入维来自 ONNX 里写死的
///   `value_info`，读得到；输出维要靠推导。
/// - **`analyse(false)` 之后输出维仍然读不到**（`outlet_fact` 失败，
///   `IoSpec::outputs` 是空的）。
/// - **`into_optimized()` 能读，但必须先 `set_input_fact` 给具体形状**，
///   而「全动态」在 tract 里**不能用 `-1` 表示**——`TDim` 断言每维 `>= 0`
///   （fact.rs:21），`-1` 直接 panic。动态维是**符号**（`s0`/`s1`），
///   得按符号构造，那条路在本轮没走通。
///
/// **所以：输出形状一律靠真跑一次来确认。** det 的
/// [`tests::the_det_plan_runs_on_a_flat_image`] 与 rec 的
/// [`tests::the_rec_plan_emits_6625_classes_for_the_6624_dictionary`]
/// 就是这么写的——那比读 fact 硬，而且顺带证明了图真的跑得动。
///
/// 代价是本函数要带 `Factoid` trait（`tract_onnx::prelude` **不含**它，
/// 藏在 `tract_hir::infer` 下面）。
pub fn model_io(path: &Path) -> Result<IoSpec, PpError> {
    let mut model = tract_onnx::onnx()
        .model_for_path(path)
        .map_err(|e| PpError::Load {
            name: path.display().to_string(),
            msg: e.to_string(),
        })?;
    // ⚠ **推导失败不当作致命错误。** det 里有 `Resize`（带动态 scale），
    // `analyse` 在它上面会失败——而我们要的只是「ONNX 里**声明**了什么」，
    // 那部分本来就在输入的 `value_info` 里，不依赖推导。推导只是为了
    // 顺带看看输出维（实测：**看也看不到**），所以失败了没什么可丢的。
    if let Err(e) = model.analyse(false) {
        tracing::warn!(
            path = %path.display(),
            "det/rec 形状推导失败，只读输入维：{e}"
        );
    }
    let dims = |t: &InferenceFact| -> Vec<i64> {
        t.shape
            .dims()
            // 符号维 `concretize()` 给 `None`，记成 -1。**不能拿 0 顶替**——
            // 0 是个合法的 batch 尺寸，混进去就成了「这模型 batch=0」。
            .map(|d| d.concretize().and_then(|td| td.as_i64()).unwrap_or(-1))
            .collect()
    };
    // ⚠ `input_outlets()` 给的是 `TVec`，**不能按 `Vec<OutletId>` 去声明形参**——
    // 实测那样写 clippy 会报 needless borrow，改成 `.map(outlets)` 才过。
    let outlets = |os: &[OutletId]| -> Vec<Vec<i64>> {
        os.iter()
            .filter_map(|o| model.outlet_fact(*o).ok())
            .map(dims)
            .collect()
    };
    let inputs = model
        .input_outlets()
        .map_err(|e| PpError::Load {
            name: path.display().to_string(),
            msg: e.to_string(),
        })
        .map(outlets)?;
    let outputs = model
        .output_outlets()
        .map_err(|e| PpError::Load {
            name: path.display().to_string(),
            msg: e.to_string(),
        })
        .map(outlets)?;
    Ok(IoSpec { inputs, outputs })
}

/// 编译 det 模型，**输入形状由调用方定**。
///
/// ⚠ 形状不是猜的：PP-OCRv4 的 det 是动态 H/W 的，不给具体形状就会
/// 留下一堆泛型算子。定形状时必须**按 32 的倍数取整**——网络里有 8 次
/// 步长 2 的下采样，总下采样倍数是 32。
pub fn load_det(path: &Path, h: usize, w: usize) -> Result<Plan, PpError> {
    let model = tract_onnx::onnx()
        .model_for_path(path)
        .and_then(|mut m| {
            m.set_input_fact(0, det_fact(h, w))?;
            m.into_optimized()
        })
        .map_err(|e| PpError::Load {
            name: format!("det({})", path.display()),
            msg: e.to_string(),
        })?;
    model.into_runnable().map_err(|e| PpError::Load {
        name: format!("det({})", path.display()),
        msg: e.to_string(),
    })
}

/// 编译 rec 模型。
///
/// ⚠ **宽度每次不同都要重编。** rec 是全动态的，而 tract 的形状是编译期
/// 决定的——所以「每个文本框的宽度都不一样」意味着「每个框编译一次」。
/// 那是几十到几百毫秒的量级，**不能对每个框都这么做**。要么按长度分桶
/// 缓存几个宽度，要么固定宽度（真帧上的长文本会缩窄）。**这一条没做，
/// 见交接文档的「未完成」。**
pub fn load_rec(path: &Path, w: usize) -> Result<Plan, PpError> {
    let model = tract_onnx::onnx()
        .model_for_path(path)
        .and_then(|mut m| {
            m.set_input_fact(0, rec_fact(w))?;
            m.into_optimized()
        })
        .map_err(|e| PpError::Load {
            name: format!("rec({})", path.display()),
            msg: e.to_string(),
        })?;
    model.into_runnable().map_err(|e| PpError::Load {
        name: format!("rec({})", path.display()),
        msg: e.to_string(),
    })
}

/// 跑一次推理，返回**第 0 个输出**展平成的 `f32` 缓冲与它的形状。
///
/// 展平成一维是故意的：det/rec 后处理都要按下标走，二维以上各自
/// reshape 的方式不一样，留在调用点写更清楚。
///
/// ⚠ `TValue` 是 `Arc<Tensor>` 的 newtype，靠 `Deref` 才拿得到 Tensor 的方法。
/// `Tensor::as_slice` 在 0.23.8 里**不存在**——取数要走 `view().as_slice()`，
/// 它自带 dtype 校验：输出万一不是 f32 会**报错**，而不是默默按字节重解释
/// 出一堆垃圾数值。前提是存储是连续 plain RAM，而计划跑出来的输出一定是。
pub fn run(plan: &Plan, input: Tensor, stage: &str) -> Result<(Vec<f32>, Vec<usize>), PpError> {
    let out = plan.run(tvec!(input.into())).map_err(|e| PpError::Run {
        stage: stage.to_string(),
        msg: e.to_string(),
    })?;
    let t: &Tensor = &out[0];
    let shape: Vec<usize> = t.shape().to_vec();
    let data = t
        .view()
        .as_slice::<f32>()
        .map_err(|e| PpError::Run {
            stage: stage.to_string(),
            msg: format!("输出不是 f32：{e}"),
        })?
        .to_vec();
    Ok((data, shape))
}

/// 词典。**索引 0 是 CTC blank，不在文件里。**
///
/// ⚠ 文件每行是一个字符/符号，但 Windows 上下载的权重每行带尾随 `\r`。
/// 不剥掉的话最后一个类会变成 `"字\r"`，识别出来末尾挂一个回车符——
/// **症状是每个词末尾多一个不可见字符**，而且在任何编辑器里都看不见。
pub fn chars() -> Result<Arc<Vec<String>>, PpError> {
    static CACHE: OnceLock<Arc<Vec<String>>> = OnceLock::new();
    if let Some(c) = CACHE.get() {
        return Ok(Arc::clone(c));
    }
    let path = models_dir().join("ppocr_keys_v1.txt");
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| PpError::MissingModels(format!("{}（{e}）", path.display())))?;
    let mut v: Vec<String> = raw
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    // blank 在 0，文件里的字符整体后移一位。
    v.insert(0, String::new());
    let arc = Arc::new(v);
    let _ = CACHE.set(Arc::clone(&arc));
    Ok(arc)
}

/// CTC blank 的类索引。**恒为 0**——这是 CTC 的约定，不是这批权重的选择。
pub const BLANK: usize = 0;

#[cfg(test)]
mod tests {
    use super::*;

    /// 模型文件不在本机就**跳过**，不假装通过。
    ///
    /// 这批测试要真下载 15MB 权重才能跑，而 CI 与别人的机器上没有它们。
    /// 硬跑只会变成「永远红」，红久了就没人看了。
    fn require_models() -> Option<PathBuf> {
        let dir = models_dir();
        if dir.join("ppocr_keys_v1.txt").is_file() {
            Some(dir)
        } else {
            eprintln!("跳过：{} 下没有权重", dir.display());
            None
        }
    }

    /// **词典必须有 6624 类，且 0 号是 blank。**
    ///
    /// `ppocr_keys_v1.txt` 实测 6623 行。CTC 的 blank 落在索引 0，
    /// 所以类总数是 **6624**。只按文件行数去查表会让最后一个类越界。
    #[test]
    fn the_dictionary_is_6624_classes_with_a_blank_at_zero() {
        if require_models().is_none() {
            return;
        }
        let cs = chars().expect("词典读不出来");
        assert_eq!(cs.len(), 6624, "类数不对，rec 的输出宽度与词典对不上");
        assert!(cs[BLANK].is_empty(), "0 号必须是 blank，不能是字符");
        assert!(!cs[1].is_empty(), "1 号应该是第一个真实字符");
        assert!(
            cs.iter()
                .skip(1)
                .all(|c| !c.contains('\r') && !c.contains('\n')),
            "词典里有行没剥掉 \\r，识别结果会挂一个看不见的回车"
        );
    }

    /// **det 的输入是 `[1,3,H,W]`——模型里全是动态维。**
    ///
    /// ⚠ 2026-10-10 **实测**：`[-1, 3, -1, -1]`，**batch 也是动态的**。
    /// 写断言时我按惯例先假定 batch 固定 1，被实测打回。
    /// `[1,3,H,W]` 那个具体形状是**我们给的**（[`det_fact`]），不是模型的。
    #[test]
    fn the_det_model_takes_nchw_rgb_and_the_batch_is_dynamic_too() {
        let Some(dir) = require_models() else { return };
        let spec = model_io(&dir.join("ch_PP-OCRv4_det_mobile.onnx")).expect("det 形状读不出来");
        assert_eq!(spec.inputs.len(), 1, "det 输入应该只有一个");
        assert_eq!(
            spec.inputs[0],
            vec![-1, 3, -1, -1],
            "det 输入形状变了：{:?}。3 是 RGB 通道数（唯一被钉死的）；\
             batch/H/W 都是动态的，由 `det_fact` 按图算出来",
            spec.inputs[0]
        );
        // ⚠ `analyse` 在 det 上会失败（里面有带动态 scale 的 `Resize`），
        // 所以输出维拿不到——见 `model_io` 的说明。det 的输出形状由
        // `the_det_plan_runs_on_a_flat_image` **真跑一次**证明为 `[1,1,H,W]`。
    }

    /// **rec 是全动态 `[1,3,H,W]`——**不是**固定高 48。
    ///
    /// ⚠ 这条断言 2026-10-10 **实测推翻过原来的假设**。写它的时候我以为
    /// `ch_PP-OCRv4_rec_mobile.onnx` 的输入高固定 48（那是 PaddleOCR
    /// `RecImageGenerator` 的行为，属于**数据增强侧**），实测 `model_io`
    /// 报的是 `[-1, 3, -1, -1]`——**四个维全是动态的**，模型自己不管尺寸。
    ///
    /// 所以 **48 这个数是我们要自己给的**（[`rec_fact`] 里写死），不是模型
    /// 给的。这反而好：宽高都能自由选，不用迁就某份导出时写死的宽度。
    /// 代价是**宽度一变就得重编一次图**——见 [`load_rec`] 的说明。
    #[test]
    fn the_rec_model_is_fully_dynamic_and_we_supply_the_size() {
        let Some(dir) = require_models() else { return };
        let spec = model_io(&dir.join("ch_PP-OCRv4_rec_mobile.onnx")).expect("rec 形状读不出来");
        assert_eq!(spec.inputs.len(), 1, "rec 输入应该只有一个");
        assert_eq!(
            spec.inputs[0],
            vec![-1, 3, -1, -1],
            "rec 输入形状变了：{:?}。48 高是**我们自己给的**，不是模型的；\
             若这里变了，rec 的预处理与宽度分桶策略都要重查",
            spec.inputs[0]
        );
    }

    /// **rec 输出的类数必须是 6625 = 6624 字符 + 1 blank。**
    ///
    /// 这条是**真跑一次**得到的，不是读 fact 读来的（读不到，见 [`model_io`]）。
    /// 它是 CTC 解码表的下标上界——类数一旦对不上，解出来的不是错字，
    /// 而是**越界或者整段空白**。
    #[test]
    fn the_rec_plan_emits_6625_classes_for_the_6624_dictionary() {
        let Some(dir) = require_models() else { return };
        let (h, w) = (48usize, 64usize);
        let plan = load_rec(&dir.join("ch_PP-OCRv4_rec_mobile.onnx"), w).expect("rec 编译失败");
        let buf = vec![0.0f32; 3 * h * w];
        let input = Tensor::from_shape(&[1, 3, h, w], &buf).expect("造 rec 输入失败");
        let (vals, shape) = run(&plan, input, "rec").expect("rec 推理失败");
        // ⚠ ⚠ **实测出来是 `[1, T, 6625]`，不是 `[1, 6625, T]`。**
        // 2026-10-10 本机真跑 64px 宽的空白图得到 `[1, 8, 6625]`：
        // **类在最后一维**，时间步在中间维。写这条断言时我先入为主按
        // PaddleOCR 训练侧的布局想成 `[1, 6625, T]`，被实测打回。
        // CTC 解码的下标要按 `[t][c]` 走，写反了会得到一整段空白——
        // 而且它**不报错**，所以只能靠真跑一次钉死。
        assert_eq!(shape.len(), 3, "rec 输出应该是 [1,T,6625]，实际 {shape:?}");
        assert_eq!(shape[0], 1, "rec batch 是 1：{shape:?}");
        assert_eq!(
            shape[2], 6625,
            "rec 类数与 6624 词典对不上：{shape:?}（类在**最后一维**）"
        );
        assert!(shape[1] > 0, "rec 时间步不能是 0：{shape:?}");
        assert_eq!(
            vals.len(),
            shape[1] * 6625,
            "展平后的元素数与 [1,T,6625] 对不上：{} vs {}",
            vals.len(),
            shape[1] * 6625
        );
        // softmax 之后每一格都该是概率。
        assert!(
            vals.iter().all(|v| (-0.01..=1.01).contains(v)),
            "rec 输出不在 [0,1]，说明没过 softmax 或预处理错了"
        );
    }

    /// **det 编译得出来，且 32 倍数的形状能跑。**
    ///
    /// 微信聊天窗口的典型尺寸，不是随便取的整数。
    #[test]
    fn the_det_plan_compiles_for_a_realistic_chat_window() {
        let Some(dir) = require_models() else { return };
        let p = load_det(&dir.join("ch_PP-OCRv4_det_mobile.onnx"), 768, 512).expect("det 编译失败");
        assert!(Arc::strong_count(&p) >= 1);
    }

    /// **rec 编译得出来。**
    #[test]
    fn the_rec_plan_compiles() {
        let Some(dir) = require_models() else { return };
        load_rec(&dir.join("ch_PP-OCRv4_rec_mobile.onnx"), 320).expect("rec 编译失败");
    }

    /// **一次 det 推理真的产得出合法概率图。**
    ///
    /// 喂一张全灰的 32x32 图。det 对纯色图**本来就不该有框**，
    /// 这里要的只是「图跑通了、输出形状是 `[1,1,h,w]`、概率都落在 [0,1]」。
    #[test]
    fn the_det_plan_runs_on_a_flat_image() {
        let Some(dir) = require_models() else { return };
        let (h, w) = (32usize, 32usize);
        let plan = load_det(&dir.join("ch_PP-OCRv4_det_mobile.onnx"), h, w).expect("det 编译失败");
        let buf = vec![0.0f32; 3 * h * w];
        let input = Tensor::from_shape(&[1, 3, h, w], &buf).expect("造 det 输入失败");
        let (vals, shape) = run(&plan, input, "det").expect("det 推理失败");
        assert_eq!(shape, vec![1, 1, h, w], "det 输出形状不对：{shape:?}");
        assert_eq!(vals.len(), h * w);
        assert!(
            vals.iter().all(|v| (-0.01..=1.01).contains(v)),
            "det 输出不在 [0,1]，说明没经过 sigmoid 或预处理错了"
        );
    }
}
