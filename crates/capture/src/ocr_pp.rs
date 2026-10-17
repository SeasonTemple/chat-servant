//! PP-OCR 本地推理（det + rec），纯 Rust 跑 ONNX。
//!
//! ## 为什么不原样搬 RapidOCR
//!
//! 搬过来的话要带 `opencv`、`libtorch` 那一类东西，而应用是**装进
//! `C:\Program Files` 给普通用户跑的**，塞一个 C++ 运行时进去就多一层
//! 「装不上」的失败面。这里只用 `tract`：纯 Rust，静态链接。
//!
//! ## 当前进度（2026-10-18 订正）
//!
//! **加载层与推理管线都通了。** det 后处理 / rec CTC 解码 / 宽度分桶见
//! [`crate::ocr_pp_pipeline`]，已接进 [`crate::ocr::recognize`]。
//! 能跑通的是：读模型、按指定尺寸编译成可执行计划、跑 det 拿到 `[1,1,H,W]`
//! 概率图、CTC 解码出文本。
//!
//! ⚠ **权重已经装上了**（在 `<仓库>/target/models/`，2026-10-17），
//!   所以「跑不跑得起来」不再是问题。**真正的问题是跑出来的东西对不对**：
//!   同一帧真图上它输出 `419 / 雅扣！手洁双 / 28于证液卖成906正`，
//!   根因是 **det 框纵向只盖住字形上半**（票 39，正在修）。
//!   ⇒ **在那个 bug 修好之前，PP-OCR 的输出不能用来论证任何事**——
//!   那是拿一个有缺陷的实现去代表整条技术路线。
//!
//! ## 与 [`crate::ocr`] 的关系
//!
//! **不取代它。** `Windows.Media.Ocr` 系统自带、零模型体积，是默认值；
//! 这条路要 `<仓库>/target/models/`（或 `CHATSERVANT_OCR_MODELS` 指定的目录）
//! 里有权重才跑得起来。
//!
//! **2026-10-18：默认路改了。** [`crate::ocr::recognize`] 按 `CHATSERVANT_OCR`
//! （`ppocr` / `winrt`）选路，而**默认值恒为 WinRT**——
//! 早先是「auto = 检测到权重就走 PP-OCR」，那条规则让「装一次权重」
//! 悄悄换掉了生产引擎，而没有人做过那个决定。
//! **PP-OCR 是增强不是必需**：没权重或它跑挂都必须继续用 WinRT，
//! 而不是让界面报错。
//!
//! 推理管线（det 后处理 / rec CTC 解码 / 宽度分桶）在
//! [`crate::ocr_pp_pipeline`]，**那个模块才是干活的**，本模块只管把 ONNX
//! 编译成能跑的图。
//!
//! ## 词典类数 = 文件行数 **+2**
//!
//! PP-OCRv6 small 的 `ppocrv6_dict.txt` 有 18708 行，而 rec 输出 **18710** 类。
//! 差的两个是 CTC 约定，**都不在词典文件里**：
//! **blank 落在索引 0**，**space 落在最后一类**。见 [`chars`] 与 [`SPACE`]。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use tract_onnx::prelude::*;
// `Factoid` **不在** `tract_onnx::prelude` 里：prelude 只带了
// `InferenceFact` / `InferenceModel` / `*ModelExt` 加 `tract_core::prelude`，
// 形状维度那一套（`concretize`）藏在 `tract_hir::infer` 下面。
use tract_onnx::tract_hir::infer::Factoid;

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
    // 这条消息是**用户看得见**的（会经 `reason` 显示在面板上），
    // 所以它必须自足：给出目录与环境变量，而不是指向某篇文档的某一节——
    // 文档会被重写，而引用会静默失效。
    #[error(
        "模型目录不存在或缺文件：{0}\n     \
         把权重放到该目录，或用 CHATSERVANT_OCR_MODELS 指定它；\
         没有权重时 OCR 会自动退回 Windows.Media.Ocr，功能不受影响"
    )]
    MissingModels(String),

    #[error("加载 {name} 失败：{msg}")]
    Load { name: String, msg: String },

    #[error("{stage} 失败：{msg}")]
    Run { stage: String, msg: String },

    #[error("模型输入输出形状跟代码假定的不一样：{0}")]
    Shape(String),
}

/// 权重目录。**默认在仓库的 `target/models/`，不是 `%APPDATA%`。**
///
/// **不进仓库，但进本项目的 `target/`。** 15MB 的二进制权重 + 模型许可
/// 不是这个仓库该背的；而 `%APPDATA%` 落在系统盘，是这台机器上不该被占的地方。
/// 本项目所有可再生产物（`target/probe/` 的帧、门禁日志、构建产物）
/// 一律进 `target/`，权重跟它们是同一类东西——**可再下载，所以就该可再删**。
///
/// 查找顺序（先命中先赢）：
///
/// 1. `CHATSERVANT_OCR_MODELS` —— 显式指定，永远最优先；
/// 2. **`<仓库>/target/models/`** —— 开发、探针、`cargo test` 都走这条
///    （由编译期 `CARGO_MANIFEST_DIR` 反推，`crates/capture` 上两级）；
/// 3. `<可执行文件所在目录>/models` —— 发布形态：权重装在应用旁边；
/// 4. 系统临时目录 —— 兜底（CI 里拿不到仓库路径时的最后一条）。
///
/// ⚠ 2026-10-18 之前默认是 `%APPDATA%\dev.chatservant.app\models\`。
/// 那条被删掉不是因为它不能用，是因为**它在 C 盘**：用户明确要求
/// 产物与权重都留在项目所在盘，不去占系统盘。
/// ⇒ 装机场景要走第 3 条或第 1 条，别再指望 `%APPDATA%`。
pub fn models_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("CHATSERVANT_OCR_MODELS") {
        return PathBuf::from(d);
    }
    if let Some(d) = repo_target_models() {
        return d;
    }
    if let Some(d) = exe_sibling_models() {
        return d;
    }
    std::env::temp_dir()
        .join("dev.chatservant.app")
        .join("models")
}

/// `<仓库>/target/models`。编译期路径；拿不到（或它已经不在仓库里了）就 `None`。
fn repo_target_models() -> Option<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo = manifest.parent()?.parent()?; // crates/capture -> 仓库根
    Some(repo.join("target").join("models"))
}

/// `<可执行文件所在目录>/models`。发布形态用；开发时会被上一条抢先。
fn exe_sibling_models() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("models"))
}

/// 只读形状，不跑推理。给探针和排查用——形状对不上时要能在**不**跑推理的
/// 情况下就知道差在哪。
///
/// ## 只有**输入**维可信，输出维读不出来（2026-10-10 实测，三次踩坑）
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
/// [`tests::the_rec_plan_emits_exactly_one_class_per_dictionary_entry`]
/// 就是这么写的——那比读 fact 硬，而且顺带证明了图真的跑得动。
///
/// 代价是本函数要带 `Factoid` trait（`tract_onnx::prelude` **不含**它，
/// 藏在 `tract_hir::infer` 下面）。
pub fn model_io(path: &Path) -> Result<IoSpec, PpError> {
    let mut model = tract_onnx::onnx()
        .model_for_path(path)
        .map_err(|e| PpError::Load {
            name: path.display().to_string(),
            msg: format!("{e:#}"),
        })?;
    // **推导失败不当作致命错误。** det 里有 `Resize`（带动态 scale），
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
    // `input_outlets()` 给的是 `TVec`，**不能按 `Vec<OutletId>` 去声明形参**——
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
            msg: format!("{e:#}"),
        })
        .map(outlets)?;
    let outputs = model
        .output_outlets()
        .map_err(|e| PpError::Load {
            name: path.display().to_string(),
            msg: format!("{e:#}"),
        })
        .map(outlets)?;
    Ok(IoSpec { inputs, outputs })
}

/// 编译 det 模型，**输入形状由调用方定**。
///
/// 形状不是猜的：det 是动态 H/W 的，不给具体形状就会
/// 留下一堆泛型算子。定形状时必须**按 32 的倍数取整**——网络里有 8 次
/// 步长 2 的下采样，总下采样倍数是 32。
pub fn load_det(path: &Path, h: usize, w: usize) -> Result<Plan, PpError> {
    load_det_with(backend(), path, h, w)
}

/// [`load_det`] 的可测版本：后端由调用方给。
///
/// 拆出来的理由很具体：`load_det` 内部读**全局** `backend()`（环境变量），
/// 于是「按后端逐个测一遍」根本做不到——测试想测 ort，实际加载的可能还是 tract。
/// 症状是 `Ort 推理失败：det 失败：Clashing resolution 32=32 != 64`，
/// 报错里那个 `32` 正是测试传给 `load_det` 的**另一个**后端编出来的固定形状。
/// 也就是说：**报错指向 ort，错的却是 tract 的那次加载**。
pub fn load_det_with(backend: Backend, path: &Path, h: usize, w: usize) -> Result<Plan, PpError> {
    if backend == Backend::Ort {
        return Ok(Arc::new(PlanBody::Ort(Mutex::new(ort_session(path)?))));
    }
    let model = onnx_reader()
        .model_for_path(path)
        .and_then(|mut m| {
            m.set_input_fact(0, det_fact(h, w))?;
            m.into_optimized()
        })
        .map_err(|e| PpError::Load {
            name: format!("det({})", path.display()),
            msg: format!("{e:#}"),
        })?;
    model
        .into_runnable()
        .map(|m| Arc::new(PlanBody::Tract(m)) as Plan)
        .map_err(|e| PpError::Load {
            name: format!("det({})", path.display()),
            msg: format!("{e:#}"),
        })
}

/// ONNX Runtime 会话。**4 线程**——本机实测的最优档（ADR-0007）。
///
/// ⚠ v6 的 rec 输入高被模型钉死在 48、宽度是动态的，而 ONNX Runtime
/// **不像 tract 那样按形状编译图**：一个会话吃任意宽度。
/// ⇒ 走 ort 这条路时，[`crate::ocr_pp_pipeline::BucketCache`] 的分桶
/// 缓存**不再有存在意义**（只是多花点时间）。
fn ort_session(path: &Path) -> Result<ort::session::Session, PpError> {
    ort::session::Session::builder()
        .map_err(|e| PpError::Load {
            name: format!("ort({})", path.display()),
            msg: format!("{e:#}"),
        })?
        .with_intra_threads(4)
        .map_err(|e| PpError::Load {
            name: format!("ort({})", path.display()),
            msg: format!("{e:#}"),
        })?
        .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
        .map_err(|e| PpError::Load {
            name: format!("ort({})", path.display()),
            msg: format!("{e:#}"),
        })?
        .commit_from_file(path)
        .map_err(|e| PpError::Load {
            name: format!("ort({})", path.display()),
            msg: format!("{e:#}"),
        })
}

/// 编译 rec 模型。
///
/// **宽度每次不同都要重编。** rec 是全动态的，而 tract 的形状是编译期
/// 决定的——所以「每个文本框的宽度都不一样」意味着「每个框编译一次」。
/// 那是几十到几百毫秒的量级，**不能对每个框都这么做**。
///
/// **解法已实现**：[`crate::ocr_pp_pipeline::BucketCache`] 按取整后的宽度
/// 分桶缓存编译好的图，聊天里高度重复的气泡宽度命中率高。
pub fn load_rec(path: &Path, w: usize) -> Result<Plan, PpError> {
    load_rec_with(backend(), path, w)
}

/// [`load_rec`] 的可测版本，理由同 [`load_det_with`]。
pub fn load_rec_with(backend: Backend, path: &Path, w: usize) -> Result<Plan, PpError> {
    if backend == Backend::Ort {
        // ⚠ 宽度在这条路上**不起作用**：ONNX Runtime 的会话接受任意宽度，
        // 不像 tract 那样按形状编译图。签名保留 `w` 是为了让两个后端同形。
        return Ok(Arc::new(PlanBody::Ort(Mutex::new(ort_session(path)?))));
    }
    let model = onnx_reader()
        .model_for_path(path)
        .and_then(|mut m| {
            m.set_input_fact(0, rec_fact(w))?;
            m.into_optimized()
        })
        .map_err(|e| PpError::Load {
            name: format!("rec({})", path.display()),
            msg: format!("{e:#}"),
        })?;
    model
        .into_runnable()
        .map(|m| Arc::new(PlanBody::Tract(m)) as Plan)
        .map_err(|e| PpError::Load {
            name: format!("rec({})", path.display()),
            msg: format!("{e:#}"),
        })
}

/// tract 的 ONNX 读取器，**关掉 `value_info` 提示**。
///
/// ## 为什么必须关（2026-10-18 实测，PP-OCRv6）
///
/// Paddle2ONNX 导出的图会在 `graph.value_info` 里给每个中间张量写死形状。
/// tract 把这些当**硬约束**参与 `analyse` 的形状 unification，于是 v6 的
/// det 与 rec **都编译不出来**，报的是一句看不出所以然的话：
///
/// ```text
/// Failed analyse for node #170 "Conv.0" ConvHir:
///   Unifying shapes DynamicDimension.0,48,24,(DynamicDimension.1+1)/2
///                    and 1,48,24,160:
///   Impossible to unify Sym(DynamicDimension.0) with Val(1)
/// ```
///
/// 那个 `1,48,24,160` 是导出时**某一个样本输入**的回写形状，batch 被钉成 1，
/// 而我们给的 batch 也是 1 —— 冲突出在 tract 把两边的符号名当不同变量。
/// 形状本身 tract 推得出来（`into_optimized()` 就是在推），
/// 提示反而成了枷锁。`with_ignore_value_info(true)` 关掉它。
///
/// ⚠ 图的**输出**形状本来就默认不采信（`use_output_shapes` 默认 `false`），
/// 所以这条只影响中间张量的提示，不影响我们的输入约定。
fn onnx_reader() -> tract_onnx::Onnx {
    tract_onnx::onnx().with_ignore_value_info(true)
}

/// ONNX 推理后端。**默认 tract**，见 [`backend`] 的说明。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// `tract-onnx`：纯 Rust，无额外二进制依赖。
    Tract,
    /// `ort`：ONNX Runtime，需要一份 C++ 动态库。
    Ort,
}

/// 这次用哪个后端。**由 `CHATSERVANT_OCR_BACKEND` 决定，默认 tract。**
///
/// ## 为什么默认还是 tract（2026-10-18）
///
/// 同一张真实消息区、同一份 v6 small 权重、release 稳态中位：
///
/// ```text
/// tract-onnx 0.23.8     2501~2552 ms
/// onnxruntime 4 线程       443~ 455 ms
/// ```
///
/// 差 **5.6 倍**，而 `TICK = 1000ms` ⇒ tract 这条**进不了预算**。
/// 三条排查都指向「是引擎本身」：
/// tract 的运行时开关逐个试过没有一个带来可测变化；
/// `RAYON_NUM_THREADS` 从 12 扫到 2 几乎不变（**2 线程 ≈ 12 线程**）；
/// 并发两个进程只贵 1.35×，所以它并非单线程。
///
/// 但「要不要为了 5.6 倍接受一份 C++ 动态库」是**装机形态的决定**，
/// 不是代码能替你做的——所以默认值不变，改用环境变量试。
/// 上线与否取决于本机与目标机的实测，不取决于这个默认。
pub fn backend() -> Backend {
    match std::env::var("CHATSERVANT_OCR_BACKEND").as_deref() {
        Ok("ort") | Ok("onnxruntime") => Backend::Ort,
        _ => Backend::Tract,
    }
}

/// 编译好的可执行计划。`Arc` 是为了 clone 进多帧复用。
pub type Plan = Arc<PlanBody>;

/// 计划本体。两个后端各占一支，**上层只认 [`Plan`]**。`n///`n/// ⚠ 名字**不能**叫 `PlanKind`：`tract_onnx::prelude::*` 里已经有一个`n/// `PlanKind<F, O>`（tract 的计划种类），同名会被它顶掉——`n/// 而症状是 `PlanKind::Tract(Arc::new(m))` 报「expected SimplePlan」，`n/// 指向的却是 `std::sync::Arc::new`，看起来和这行毫无关系。
///
/// `ort` 那支套了 [`Mutex`]：它的 `run` 要 `&mut self`，而我们的
/// [`crate::ocr_pp_pipeline::BucketCache`] 要把同一张图 clone 进多处。
/// 加锁不影响产品——观察循环本来就一次只跑一帧。
pub enum PlanBody {
    Tract(Arc<RunnableModel<TypedFact, Box<dyn TypedOp>>>),
    Ort(Mutex<ort::session::Session>),
}

/// 跑一次推理，返回**第 0 个输出**展平成的 `f32` 缓冲与它的形状。
///
/// 展平成一维是故意的：det/rec 后处理都要按下标走，二维以上各自
/// reshape 的方式不一样，留在调用点写更清楚。
///
/// `TValue` 是 `Arc<Tensor>` 的 newtype，靠 `Deref` 才拿得到 Tensor 的方法。
/// `Tensor::as_slice` 在 0.23.8 里**不存在**——取数要走 `view().as_slice()`，
/// 它自带 dtype 校验：输出万一不是 f32 会**报错**，而不是默默按字节重解释
/// 出一堆垃圾数值。前提是存储是连续 plain RAM，而计划跑出来的输出一定是。
///
/// `shape` 与 `data` 分开传而不是传 `Tensor`，是因为两个后端的张量类型不同
/// （tract 的 `Tensor` vs `ort::value::Tensor`），**在 `Plan` 之上再统一一次**
/// 比在每个调用点做两套转换便宜。
pub fn run(
    plan: &Plan,
    shape: &[usize],
    data: &[f32],
    stage: &str,
) -> Result<(Vec<f32>, Vec<usize>), PpError> {
    match plan.as_ref() {
        PlanBody::Tract(m) => {
            let input = tract_onnx::prelude::Tensor::from_shape(shape, data)
                .map_err(|e| PpError::Shape(format!("造 {stage} 输入失败：{e}")))?;
            let out = m.run(tvec!(input.into())).map_err(|e| PpError::Run {
                stage: stage.to_string(),
                msg: format!("{e:#}"),
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
        PlanBody::Ort(sess) => {
            let mut s = sess.lock().map_err(|_| PpError::Run {
                stage: stage.to_string(),
                msg: "ONNX Runtime 会话被毒化了（上一条推理 panic 过）".into(),
            })?;
            let dims: Vec<i64> = shape.iter().map(|d| *d as i64).collect();
            let t = ort::value::Tensor::from_array((dims, data.to_vec()))
                .map_err(|e| PpError::Shape(format!("造 {stage} 输入失败：{e}")))?;
            let outs = s.run(ort::inputs!["x" => t]).map_err(|e| PpError::Run {
                stage: stage.to_string(),
                msg: format!("{e:#}"),
            })?;
            let v = outs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| PpError::Run {
                    stage: stage.to_string(),
                    msg: format!("输出不是 f32：{e}"),
                })?;
            let shape: Vec<usize> = v.0.iter().map(|d| *d as usize).collect();
            Ok((v.1.to_vec(), shape))
        }
    }
}

/// **ONNX Runtime 后端**能不能真的跑起 v6 的两个模型。
///
/// 这是一条**可行性探针**，不是产品路径：它只回答
/// 「`ort` 这条路在本机通不通、通了有多快」。
///
/// 存在的理由很具体：tract 在同一份输入上要 **2501~2552 ms**，
/// 超 `TICK = 1000ms` 两倍半；而 ONNX Runtime（Python 侧实测）只要
/// **443~455 ms**。那 5.6 倍的差距决定 PP-OCR 能不能上线，
/// 而**在把它接进管线之前必须先证明它在本机能跑**——
/// 「Python 那边能跑」不等于「Rust 这边能跑」：
/// `ort` 要一份 ONNX Runtime 动态库，构建时还要下载二进制。
///
/// 因此这条测试在**没有权重时也跑**（只测环境），有权重时多跑一层真推理。
#[test]
fn the_onnxruntime_backend_can_load_and_run_the_v6_models() {
    use std::time::Instant;

    let dir = models_dir();
    let det_path = dir.join(crate::ocr_pp_pipeline::DET_MODEL);
    let rec_path = dir.join(crate::ocr_pp_pipeline::REC_MODEL);

    let build = |p: &std::path::Path| {
        ort::session::Session::builder()
            .expect("建 SessionBuilder")
            .with_intra_threads(4)
            .expect("设 intra_op_num_threads=4")
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
            .expect("设图优化等级")
            .commit_from_file(p)
    };

    let Ok(mut sess) = build(&det_path) else {
        eprintln!(
            "跳过：{} 下没有 v6 det 权重（PP-OCR 的其它测试也一起跳过）",
            dir.display()
        );
        return;
    };

    // 喂一张 32x32 的灰图：det 对纯色图本来就不该有框，
    // 这里要的只是「图跑通了、输出是 [1,1,h,w]、值落在 [0,1]」。
    let (h, w) = (32usize, 32usize);
    let buf: Vec<f32> = vec![0.0; 3 * h * w];
    let t0 = Instant::now();
    // ⚠ `SessionOutputs` 借用着 session，所以必须**在这个作用域里把它用完**，
    // 出了作用域才能再 `run` 一次（`&mut sess` 不能重叠借两次）。
    {
        let outs = sess
            .run(ort::inputs!["x" => ort::value::Tensor::from_array((
                    vec![1i64, 3, h as i64, w as i64],
                    buf
                ))
                .expect("造输入")])
            .expect("det 推理失败");

        let v = outs["fetch_name_0"]
            .try_extract_tensor::<f32>()
            .expect("取 det 输出");
        let shape: Vec<i64> = v.0.to_vec();
        assert_eq!(
            shape,
            vec![1, 1, h as i64, w as i64],
            "det 输出形状不对：{shape:?}"
        );
        assert!(
            v.1.iter().all(|v| (-0.01..=1.01).contains(v)),
            "det 输出不在 [0,1]"
        );
    }
    let cold = t0.elapsed();

    // 热调用才是产品关心的那个数：观察循环每帧都调。
    let mut hot = Vec::new();
    for _ in 0..5 {
        let buf: Vec<f32> = vec![0.0; 3 * h * w];
        let t = ort::value::Tensor::from_array((vec![1i64, 3, h as i64, w as i64], buf))
            .expect("造输入");
        let t0 = Instant::now();
        sess.run(ort::inputs!["x" => t]).expect("det 推理失败");
        hot.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    hot.sort_by(|a, b| a.partial_cmp(b).unwrap());
    eprintln!(
        "ort det 32x32：首遍 {cold:?}  热调用中位 {:.1} ms（{:?}）",
        hot[hot.len() / 2],
        hot.iter().map(|v| format!("{v:.1}")).collect::<Vec<_>>()
    );

    let mut rec = build(&rec_path).expect("rec 也得能编译");
    let (rh, rw) = (48usize, 64usize);
    let buf: Vec<f32> = vec![0.0; 3 * rh * rw];
    let o = rec
        .run(ort::inputs!["x" => ort::value::Tensor::from_array((vec![1i64, 3, rh as i64, rw as i64], buf)).expect("造输入")])
        .expect("rec 推理失败");
    let v = o["fetch_name_0"]
        .try_extract_tensor::<f32>()
        .expect("取 rec 输出");
    let classes = chars().expect("词典读不出来").len();
    assert_eq!(
        v.0[v.0.len() - 1],
        classes as i64,
        "rec 输出类数与词典对不上：{:?}",
        v.0
    );
}

/// **两个后端都要能真跑一遍 det**，不是只验默认那个。
///
/// 存在的理由：`CHATSERVANT_OCR_BACKEND` 是**运行时环境变量**，
/// 而门禁只跑默认那一条路径。⇒ 「把 `ort` 那条写坏了，本机 CI 不会响」，
/// 直到有人真的去设那个变量、然后在真机上读到一片空白。
///
/// 这条测试**显式切换后端**跑两遍，所以两个都被覆盖。
#[test]
fn both_backends_run_the_det_model() {
    let dir = models_dir();
    let det = dir.join(crate::ocr_pp_pipeline::DET_MODEL);
    if !det.is_file() {
        eprintln!("跳过：{} 下没有 v6 det 权重", dir.display());
        return;
    }
    let (h, w) = (64usize, 64usize);
    let buf: Vec<f32> = vec![0.0; 3 * h * w];
    for backend in [Backend::Tract, Backend::Ort] {
        let plan = load_det_with(backend, &det, h, w)
            .unwrap_or_else(|e| panic!("{backend:?} 加载 det 失败：{e}"));
        let (vals, shape) = run(&plan, &[1, 3, h, w], &buf, "det")
            .unwrap_or_else(|e| panic!("{backend:?} 推理失败：{e}"));
        assert_eq!(
            shape,
            vec![1, 1, h, w],
            "{backend:?} 的 det 输出形状不对：{shape:?}"
        );
        assert!(
            vals.iter().all(|v| (-0.01..=1.01).contains(v)),
            "{backend:?} 的 det 输出不在 [0,1]，说明没过 sigmoid 或预处理错了"
        );
    }
}

/// 词典。**索引 0 是 CTC blank，不在文件里。**
///
/// 文件每行是一个字符/符号，但 Windows 上下载的权重每行带尾随 `\r`。
/// 不剥掉的话最后一个类会变成 `"字\r"`，识别出来末尾挂一个回车符——
/// **症状是每个词末尾多一个不可见字符**，而且在任何编辑器里都看不见。
pub fn chars() -> Result<Arc<Vec<String>>, PpError> {
    static CACHE: OnceLock<Arc<Vec<String>>> = OnceLock::new();
    if let Some(c) = CACHE.get() {
        return Ok(Arc::clone(c));
    }
    let path = models_dir().join(crate::ocr_pp_pipeline::KEYS_FILE);
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| PpError::MissingModels(format!("{}（{e}）", path.display())))?;
    let mut v: Vec<String> = raw
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    // blank 在 0，文件里的字符整体后移一位；space 在最后一类。
    v.insert(0, String::new());
    v.push(SPACE.to_string());
    let arc = Arc::new(v);
    let _ = CACHE.set(Arc::clone(&arc));
    Ok(arc)
}

/// CTC blank 的类索引。**恒为 0**——这是 CTC 的约定，不是这批权重的选择。
pub const BLANK: usize = 0;

/// 空格类。**恒为最后一类**——PaddleOCR 导出时把它从词典文件里摘出去了。
///
/// ⚠ 2026-10-18 之前词典只补了首头的 blank、没补这个尾巴，于是
/// `chars.len()` 比 rec 的输出宽度**少 1**（v4：6624 vs 6625；v6：18709 vs 18710）。
/// 平时预测不到最后一类，所以一直没炸；它是一颗只在特定字符上响的地雷。
pub const SPACE: &str = " ";

#[cfg(test)]
mod tests {
    use super::*;

    /// 模型文件不在本机就**跳过**，不假装通过。
    ///
    /// 这批测试要真下载权重才能跑，而 CI 与别人的机器上没有它们。
    /// 硬跑只会变成「永远红」，红久了就没人看了。
    fn require_models() -> Option<PathBuf> {
        let dir = models_dir();
        if dir.join(crate::ocr_pp_pipeline::KEYS_FILE).is_file() {
            Some(dir)
        } else {
            eprintln!("跳过：{} 下没有权重", dir.display());
            None
        }
    }

    /// 词典里真实字符的行数。**2026-10-18 实测 PP-OCRv6 small 是 18708 行。**
    const DICT_LINES: usize = 18_708;

    /// **词典类数 = 文件行数 + blank + space，且 0 号是 blank、末号是空格。**
    ///
    /// 早先的断言只钉了「文件行数 + 1」，漏掉了末位那个空格类，
    /// 而 **rec 真的输出多 1 类**（v4 6625、v6 18710）——
    /// 见 [`SPACE`] 的说明。这条断言改成两端都钉死。
    #[test]
    fn the_dictionary_covers_every_class_the_rec_model_emits() {
        if require_models().is_none() {
            return;
        }
        let cs = chars().expect("词典读不出来");
        let classes = DICT_LINES + 2;
        assert_eq!(
            cs.len(),
            classes,
            "类数不对：词典 {} 行 + blank + space 应为 {classes} 类，实得 {}",
            DICT_LINES,
            cs.len()
        );
        assert_eq!(cs[BLANK], "", "0 号必须是 blank，不能是字符");
        assert_eq!(
            cs[classes - 1],
            SPACE,
            "末号必须是 space：CTC 真预测到它时是越界下标"
        );
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
    /// 2026-10-10 **实测**：`[-1, 3, -1, -1]`，**batch 也是动态的**。
    /// 写断言时我按惯例先假定 batch 固定 1，被实测打回。
    /// `[1,3,H,W]` 那个具体形状是**我们给的**（[`det_fact`]），不是模型的。
    #[test]
    fn the_det_model_takes_nchw_rgb_and_the_batch_is_dynamic_too() {
        let Some(dir) = require_models() else { return };
        let spec =
            model_io(&dir.join(crate::ocr_pp_pipeline::DET_MODEL)).expect("det 形状读不出来");
        assert_eq!(spec.inputs.len(), 1, "det 输入应该只有一个");
        assert_eq!(
            spec.inputs[0],
            vec![-1, 3, -1, -1],
            "det 输入形状变了：{:?}。3 是 RGB 通道数（唯一被钉死的）；\
             batch/H/W 都是动态的，由 `det_fact` 按图算出来",
            spec.inputs[0]
        );
        // `analyse` 在 det 上会失败（里面有带动态 scale 的 `Resize`），
        // 所以输出维拿不到——见 `model_io` 的说明。det 的输出形状由
        // `the_det_plan_runs_on_a_flat_image` **真跑一次**证明为 `[1,1,H,W]`。
    }

    /// **rec 的输入是 `[1,3,48,W]`——高被模型钉死在 48，宽才是动态的。**
    ///
    /// 这条断言**在两个模型上给过相反的答案**，两次都是实测打回的：
    ///
    /// - **v4**（2026-10-10）：`[-1, 3, -1, -1]`，四个维全是动态的，
    ///   「48 是我们自己给的」。当年的注释按 PaddleOCR 训练侧想当然，
    ///   断言写反了。
    /// - **v6**（2026-10-18 实测 `PP-OCRv6_rec_small.onnx`）：`[-1, 3, 48, -1]`
    ///   ——**模型自己就要求 48**。所以 [`rec_fact`] 里的 48 不再是
    ///   「我们的选择」，而成了**必须遵守的契约**：换个别的 rec 权重，
    ///   这个数要跟着模型走。
    ///
    /// 宽度仍然是动态的，代价不变：**宽度一变就得重编一次图**——见 [`load_rec`]。
    #[test]
    fn the_rec_model_pins_its_input_height_to_48() {
        let Some(dir) = require_models() else { return };
        let spec =
            model_io(&dir.join(crate::ocr_pp_pipeline::REC_MODEL)).expect("rec 形状读不出来");
        assert_eq!(spec.inputs.len(), 1, "rec 输入应该只有一个");
        assert_eq!(
            spec.inputs[0],
            vec![-1, 3, 48, -1],
            "rec 输入形状变了：{:?}。48 高是**模型要求**的，不是我们的选择；\
             若这里变了，rec 的预处理与宽度分桶策略都要重查",
            spec.inputs[0]
        );
    }

    /// **rec 输出的类数必须等于词典类数（v6 small = 18710）。**
    ///
    /// 这条是**真跑一次**得到的，不是读 fact 读来的（读不到，见 [`model_io`]）。
    /// 它是 CTC 解码表的下标上界——类数一旦对不上，解出来的不是错字，
    /// 而是**越界或者整段空白**。
    #[test]
    fn the_rec_plan_emits_exactly_one_class_per_dictionary_entry() {
        let Some(dir) = require_models() else { return };
        let (h, w) = (48usize, 64usize);
        let plan = load_rec(&dir.join(crate::ocr_pp_pipeline::REC_MODEL), w).expect("rec 编译失败");
        let buf = vec![0.0f32; 3 * h * w];
        let (vals, shape) = run(&plan, &[1, 3, h, w], &buf, "rec").expect("rec 推理失败");
        // **实测出来是 `[1, T, C]`，不是 `[1, C, T]`。**
        // 2026-10-10 本机真跑 64px 宽的空白图得到 `[1, 8, 6625]`（v4）：
        // **类在最后一维**，时间步在中间维。写这条断言时我先入为主按
        // PaddleOCR 训练侧的布局想成 `[1, 6625, T]`，被实测打回。
        // CTC 解码的下标要按 `[t][c]` 走，写反了会得到一整段空白——
        // 而且它**不报错**，所以只能靠真跑一次钉死。
        let classes = chars().expect("词典读不出来").len();
        assert_eq!(shape.len(), 3, "rec 输出应该是 [1,T,C]，实际 {shape:?}");
        assert_eq!(shape[0], 1, "rec batch 是 1：{shape:?}");
        assert_eq!(
            shape[2], classes,
            "rec 输出 {0} 类而词典 {1} 类，对不上：{shape:?}（类在**最后一维**）",
            shape[2], classes
        );
        assert!(shape[1] > 0, "rec 时间步不能是 0：{shape:?}");
        assert_eq!(
            vals.len(),
            shape[1] * classes,
            "展平后的元素数与 [1,T,C] 对不上：{} vs {}",
            vals.len(),
            shape[1] * classes
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
        let p =
            load_det(&dir.join(crate::ocr_pp_pipeline::DET_MODEL), 768, 512).expect("det 编译失败");
        assert!(Arc::strong_count(&p) >= 1);
    }

    /// **rec 编译得出来。**
    #[test]
    fn the_rec_plan_compiles() {
        let Some(dir) = require_models() else { return };
        load_rec(&dir.join(crate::ocr_pp_pipeline::REC_MODEL), 320).expect("rec 编译失败");
    }

    /// **一次 det 推理真的产得出合法概率图。**
    ///
    /// 喂一张全灰的 32x32 图。det 对纯色图**本来就不该有框**，
    /// 这里要的只是「图跑通了、输出形状是 `[1,1,h,w]`、概率都落在 [0,1]」。
    #[test]
    fn the_det_plan_runs_on_a_flat_image() {
        let Some(dir) = require_models() else { return };
        let (h, w) = (32usize, 32usize);
        let plan =
            load_det(&dir.join(crate::ocr_pp_pipeline::DET_MODEL), h, w).expect("det 编译失败");
        let buf = vec![0.0f32; 3 * h * w];
        let (vals, shape) = run(&plan, &[1, 3, h, w], &buf, "det").expect("det 推理失败");
        assert_eq!(shape, vec![1, 1, h, w], "det 输出形状不对：{shape:?}");
        assert_eq!(vals.len(), h * w);
        assert!(
            vals.iter().all(|v| (-0.01..=1.01).contains(v)),
            "det 输出不在 [0,1]，说明没经过 sigmoid 或预处理错了"
        );
    }
}
