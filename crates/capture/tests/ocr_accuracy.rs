//! **L3：一条真的跑完整 OCR 推理、并断言字准的测试。**
//!
//! # 它守什么
//!
//! L1（det 像素级后处理）与 L2（rec 预处理张量）都是**不需要权重**的纯函数，
//! 真正的 CI 价值在那两条。但它们都停在**送进模型之前**：票 30 的原始症状
//! 是「结构全绿而字是乱码」——框数对、行数对、张量长度对，字却全是错的。
//! 这条测试补的就是那一段：**从像素一路跑到认出来的字**，再拿逐字已知的
//! 答案键算 CER。
//!
//! 夹具是 `scripts/ocrbench/gen_corpus.py` 造的**合成 IM 截图**，每条气泡的
//! ground truth 就是渲染上去的那个字符串，**逐字已知**（`truth.json`）。
//! 合成而非真帧是纪律要求（硬约束第 1 条：真人会话内容不入库），代价见下。
//!
//! # ⚠ 它**不**守什么（这一段不许被删，也别因为「CI 有这条测试」就改口）
//!
//! · **不守真机微信帧的字准。** 合成字的渲染、抗锯齿、字距、气泡布局
//!   都不是微信的；这里测不到 det 在真实气泡尾巴上的漏检，也测不到小字号
//!   在真实缩放下的表现。**真帧字准仍然只由
//!   `scripts/ocrbench/cer_real_frame.py` 那个人工台架负责**（见
//!   `CONTRIBUTING.md` §维护陷阱 第 6 条）。
//! · **在 CI 上必然跳过。** 权重（det 9.5 MB + rec 20.3 MB）在
//!   `target/models/`、是 gitignored 的，CI 上没有 ⇒ 走「缺件即跳过」，
//!   与 `crates/perceive/tests/golden.rs` 同一模式。
//!   ⚠ 所以**「CI 绿」不等于「字准验过了」**——票 30 末尾踩过这个坑，
//!   票 50 把它写成了验收项。定位是「**本地每次改 OCR 都该跑**」。
//!
//! # 跑法
//!
//! ```text
//! cargo test -p chatservant-capture --test ocr_accuracy -- --nocapture
//! ```
//!
//! 默认只跑 [`SAMPLE`] 里的固定子集（门禁跑全工作区时不要拖慢它）；
//! 要跑全部 60 张设 `CS_OCR_CER_FULL=1`。
//!
//! # 自证记录（2026-10-18，判据一律用**退出码**，不用输出文本匹配）
//!
//! 「四条断言都写了」与「四条断言都验过会响」在输出上完全同形——
//! 早先这四条是连着的 `assert!`，注入坏词典后**只有第一条执行到**，
//! 另外三条从未被运行过。改成收集全部违规后才看得见。
//!
//! | 方向 | 注入什么 | 结果 | 退出码 |
//! |---|---|---|---|
//! | S3 字认错 | 词典换成「每个字都是 X」（长度不变 ⇒ 类数不变、推理**不报错**、L1/L2 全绿） | CER **117.80%**、召回 0%、全对 0%，**3 条违规** | **101** |
//! | S4 框检不到 | 识别行的 y 整体 +100000（推出答案带） | 几何漏检 **100%**，**4 条违规** | **101** |
//! | S5 回落 | 权重目录里只放 det + 词典、**不放 rec**，用 `CHATSERVANT_OCR_MODELS` 指过去 | `recognize_in` 返回 `None` ⇒ **panic 而非跳过** | **101** |
//! | 恢复 | 三处注入全部撤掉 | CER **0.23%**、召回 100%、全对 97%、几何漏检 0% | **0** |
//!
//! ⚠ **S3 是本条测试独有、L1/L2 覆盖不到的破坏面**：词典的**类号↔字**
//! 对应关系错了，推理照跑、张量长度照对、**没有任何一层会报错**，
//! 只有认出来的字是乱的。票 30 的原始症状正是这一类。
//!
//! ⚠ **S5 的做法记在这里是为了能被复现**：造一个只含
//! `ppocrv6-small/PP-OCRv6_det_small.onnx` 与 `ppocrv6_dict.txt` 的目录
//! （**刻意不放 rec**），设 `CHATSERVANT_OCR_MODELS` 指过去即可。
//! `det_file_present` 会过（所以不会被当成「没装模型」跳过），
//! 而 `load_rec` 失败 ⇒ `recognize_with` 报错 ⇒ `recognize_in` 回落。
//!
//! ⚠ **没自证的两条**，别当成验过了：
//! · 「缺件即跳过」这一支（CI 上走的就是它）——它是刻意的行为，
//!   验它等于验「测试能跳过」，没有守卫价值。
//! · `SAMPLE` 里的 id 与 `truth.json` 对不上时的红——那是一条纯
//!   `assert!`，逻辑只有一句「挑中的图不为空」，没有跑过注入。

use std::path::{Path, PathBuf};

use chatservant_capture::Frame;
use chatservant_capture::ocr_pp;
use chatservant_capture::ocr_pp_pipeline::{self, recognize_in};
use chatservant_core::frame::Rect;
use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;

/// 台架语料与答案键的落点。**都在 `target/` 下、被 gitignore**，
/// 所以这条测试在干净 clone 上是「跳过」而不是「绿着骗人」。
fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/capture 的上上级是仓库根")
        .join("target")
        .join("probe")
        .join("ocrbench")
}

/// **默认子集**：3 种语言 × 明暗各一 × **最小的 12px 与中间的 16px**，共 12 张。
///
/// 挑法是**按实测定的**，不是「少拿几张省时间」：
///
/// · **语言 3 种全要**（zh / en / mixed）——语言决定字形集，
///   `mixed` 那档里已经暴露过只有它才出的错。
/// · **明暗各一**——明暗决定 det 阈值那条曲线的两端。
/// · **12px 必须在**（2026-10-18 实测）：全量 60 张的 8 个编辑里，
///   「完→亡」和「I→l」**只在 12px 出现**，其他字号不出。
///   ⚠ 也就是说 12px 是一个**独立的弱档**，不测它就等于默认子集
///   恰好绕开了最容易坏的那一档。
/// · **16px 代表中段，21px 不进默认子集**——大字号是三档里最容易的，
///   漏测风险最低，留给全量。
///
/// ⚠ 子集与全量的代价：默认子集约 17 秒，全量 60 张约 88 秒
/// （都是 debug 构建、ort 后端、2026-10-18 实测）。
const SAMPLE: &[&str] = &[
    "zh_12_light_s0",
    "zh_12_dark_s0",
    "zh_16_light_s0",
    "zh_16_dark_s0",
    "en_12_light_s0",
    "en_12_dark_s0",
    "en_16_light_s0",
    "en_16_dark_s0",
    "mixed_12_light_s0",
    "mixed_12_dark_s0",
    "mixed_16_light_s0",
    "mixed_16_dark_s0",
];

// ── truth.json 的形状（字段名必须与 `gen_corpus.py` 写出的逐字一致）──

#[derive(Deserialize)]
struct Truth {
    images: Vec<TruthImage>,
}

#[derive(Deserialize)]
struct TruthImage {
    id: String,
    path: String,
    lines: Vec<TruthLine>,
}

#[derive(Deserialize)]
struct TruthLine {
    text: String,
    /// 文字自身的行带 `[x0, y0, x1, y1]`，**不含**气泡内边距。
    band: [i32; 4],
}

// ── 尺子：与 `scripts/ocrbench/bench_lib.py` 逐字同口径 ──

/// 行带相交至少要覆盖较矮一方的一半，才算「对应上了同一条消息」。
/// 与 `bench_lib.OVERLAP_RATIO` 同一个 0.5。
const OVERLAP_RATIO: f64 = 0.5;

/// [`normalize`] 之后相似度低于此值就算漏检。与 `bench_lib.MISS_SIM` 同值。
const MISS_SIM: f64 = 0.5;

/// **归一化：唯一口径的三步，顺序与 `bench_lib.normalize` 一致。**
///
/// 1. NFKC（全角转半角）
/// 2. 删掉**全部** Unicode 空白
/// 3. 小写
///
/// ⚠ **为什么这里要自己算，而不是读 `truth.json` 里预存的答案**：
/// ground truth 那一侧确实可以抄现成的，但**识别输出那一侧必须同样处理**
/// ——PP-OCR 的输出同样可能带全角与空格。所以两边必须过同一个函数；
/// 只归一化一边会把「词间空格丢了」和「字认错了」混进同一个编辑距离，
/// 而这两件**修法完全不同**。
///
/// ⚠ 一处已知差异，不要当成 bug：Python 的 `str.isspace()` 额外把
/// U+001C..U+001F（文件/组/记录/单元分隔符）算作空白，Rust 的
/// `char::is_whitespace()` 不算。这四个码位既不在合成语料里、
/// 也不会从 OCR 词典里出来。
fn normalize(s: &str) -> String {
    let nfkc: String = s.nfkc().collect();
    nfkc.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

/// 标准编辑距离（插入/删除/替换各 1）。滚动数组，与 `bench_lib.levenshtein`
/// 同算法；**按 Unicode 标度计数**（`char` vs Python 的 code point，一致）。
fn levenshtein(a: &[char], b: &[char]) -> usize {
    if a == b {
        return 0;
    }
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j + 1] + 1)
                .min(cur[j] + 1)
                .min(prev[j] + (ca != cb) as usize);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// 一条识别行在评测里需要的全部（与 `bench_lib.Hyp` 同三个字段）。
struct Hyp<'a> {
    text: &'a str,
    y0: i32,
    y1: i32,
}

/// 一条 ground truth 行的判定结果。**几何漏检与「字全错」必须分开**，
/// 因为修法完全不同：一个要调 det 阈值，一个要调 rec / 后处理。
struct Judged {
    edits: usize,
    /// 引擎**根本没提出**这一行（一个候选框都没有）。
    geom_missed: bool,
    /// 与选中候选的相似度，口径照 `bench_lib.align`：
    /// `1 - d / max(1, max(len(gt), len(hyp)))`。
    sim: f64,
    /// 选中候选的归一化文本。留着是为了**错例报告能说出错在哪**，
    /// 而不是只报一句「编辑 N」——那种报告在真出事时没有排查价值。
    hyp: String,
}

/// 把 ground truth 行与识别行配对。**口径照 `bench_lib.align`**：
/// 纵向带相交覆盖较矮一方 ≥ 50% 算「可能是同一条」，取编辑距离最小的候选；
/// 一个候选都没有就是**几何漏检**，编辑次数按「整行都错」计。
fn judge(gt_norm: &str, gt_band: Rect, hyps: &[Hyp]) -> Judged {
    let span = (gt_band.y1 - gt_band.y0).max(1);
    let gt: Vec<char> = gt_norm.chars().collect();

    let mut best: Option<(usize, Vec<char>)> = None; // (编辑距离, 候选字符)
    for h in hyps {
        let hs = (h.y1 - h.y0).max(1);
        let ov = gt_band.y1.min(h.y1) - gt_band.y0.max(h.y0);
        if ov <= 0 || (ov as f64) < OVERLAP_RATIO * span.min(hs) as f64 {
            continue;
        }
        let hyp: Vec<char> = normalize(h.text).chars().collect();
        let d = levenshtein(&gt, &hyp);
        best = Some(match best {
            Some((bd, _)) if bd <= d => (bd, hyp),
            _ => (d, hyp),
        });
    }
    match best {
        Some((d, hyp)) => Judged {
            edits: d,
            geom_missed: false,
            sim: 1.0 - d as f64 / gt.len().max(hyp.len()).max(1) as f64,
            hyp: hyp.into_iter().collect(),
        },
        // 无候选 = 引擎没提这一行，编辑次数按整行字符数计（与 bench_lib.align 一致）。
        None => Judged {
            edits: gt.len(),
            geom_missed: true,
            sim: 0.0,
            hyp: String::new(),
        },
    }
}

/// 报出「第一个不一样的位置」。给人看的，**不参与判定**。
///
/// ⚠ 只看第 i 字是否相等**不是**编辑对齐（那会漏掉单字符的插入/删除），
/// 所以它只用来**指路**，不替代 `levenshtein` 的计数。
fn first_diff(gt: &str, hyp: &str) -> String {
    let (a, b): (Vec<char>, Vec<char>) = (gt.chars().collect(), hyp.chars().collect());
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or('\u{2205}');
        let y = b.get(i).copied().unwrap_or('\u{2205}');
        if x != y {
            return format!("第 {i} 字：真值 {x} / 识别 {y}");
        }
    }
    "逐字一致".into()
}

// ── 阈值 ──
//
// ⚠ **这些数来自 2026-10-18 的实测**，而且**必须同时看两个规模**——
// 因为默认子集与全量走的是**同一组阈值**，两个样本量的分母不同、
// 统计噪声也不同，只按一个定线，另一个就可能假阳性。
//
// | 指标 | 子集实测（60 行 / 865 字符） | 全量实测（300 行 / 4400 字符） | 线 | 子集余量 |
// |---|---|---|---|---|
// | 消息召回率 | 100% | 100% | `RECALL_FLOOR` 95% | 允许 3 行掉到 sim<0.5（现在 0 行） |
// | 整表 CER | 0.23%（2 编辑） | 0.18%（8 编辑） | `CER_CEILING` 1% | 允许 8.6 编辑（现在 2 个，**4.3 倍**） |
// | 逐行全对率 | 97%（2 行非全对） | 97%（9 行非全对） | `EXACT_FLOOR` 90% | 允许 6 行非全对（现在 2 行，**3 倍**） |
// | 几何漏检率 | 0% | 0% | `GEOM_MISS_CEILING` 2% | 允许 1 行没检出（现在 0 行） |
//
// ⚠ **为什么不是照抄实测值**：夹具在 `target/` 下、**不是入库产物**。
// 换台机器重跑 `gen_corpus.py`，Pillow 版本与系统字体都可能不同 ⇒
// PNG 未必逐字节相同 ⇒ 数字会漂。所以线必须留**能解释的余量**——
// 余量倍数就写在上面那一列里，不靠「感觉够了」。
// ⚠ 反过来也不能松到「几乎退化也绿」：那正是票 50 立项前的样子，
// 三个数只写在票据正文里，门禁一声不吭。
//
// ⚠ 对照票 30 给**真帧**钉的三条下限（召回 ≥80%、全对 ≥50%、
// 字符准确 ≥75%，即 CER ≤25%）：本线的严度是真帧下限的数倍，**这是有意的**。
// 真帧那次读数 CER 0.79%、合成语料 0.23%，把线放在 25% 会让
// 「几乎全错」也绿，等于没写。
//
// ⚠ **已知的系统性混淆（不要当噪声，它稳定复现）**：全量 8 个编辑里
// **6 个是同一处**——「retry 一**次」→「retry **—**次」，
// 在 12/16/21 三个字号、light/dark 两个主题上**全部**出现。
// ⇒ 它是**模型层面的字形混淆**（一横 vs 一字），与字号、主题无关，
// 不是渲染或阈值问题。**阈值必须容忍它**：语料换成别的句子后这 6 个编辑
// 未必还在，但线上要留得出它们消失、或在别的字体下加倍的空间。
// ⚠ 若哪天它被修好，CER 会降到 ~0.05%，线仍然是绿的——**不冲突**。
// 另外两处（「完→亡」「I→l」）**只在 12px 出现**，所以 12px 在默认子集里；
// ⚠ 但要注意它们落在 `_s1` 上，而子集只取 `_s0` ——
// **默认子集覆盖的是「12px 这个字号」，不是「12px 的每一张图」**，
// 那两张图仍要全量跑才覆盖。

/// 消息召回率下限：`sim >= MISS_SIM` 的行占比。票 30 管这个叫「消息召回率」。
const RECALL_FLOOR: f64 = 0.95;

/// 整表字符错误率上限。
const CER_CEILING: f64 = 0.01;

/// 几何漏检率上限：**引擎根本没提出这一行**的比例。
///
/// ⚠ 票 30 的三条里**没有**它：纯漏检是最硬的退化，且修法与字准完全不同
/// （一个调 det 阈值、一个调 rec / 后处理），混在一个数里会让人改错地方。
const GEOM_MISS_CEILING: f64 = 0.02;

/// 逐行全对率下限：`edit == 0` 的行占比。
///
/// ⚠ 它挡住的是**总 CER 掩盖单行质量**这件事：一条 20 字的消息错 1 字，
/// 对 865 字的分母只贡献 0.1%，而这种「每行都差一点」在 CER 上几乎看不见。
const EXACT_FLOOR: f64 = 0.90;
/// 整表 CER 真超线时的报告里最多列几行错例。
const MAX_EXAMPLES: usize = 8;

/// **合成语料上的字准没有塌。**
///
/// ⚠ **判据是退出码。** 回落（[`recognize_in`] 返回 `None`）在这里是
/// **失败**而不是「跳过」——票 50 与 ADR-0006 都记了这一类：
/// PP-OCR 不可用时只打 `WARN`、**退出码仍是 0**，任何以退出码判成败的
/// 脚本都会全绿。所以这条测试显式把回落变成 panic。
///
/// 同理，语料或权重缺失**只在这条测试开始时**判一次；跑起来之后
/// 任何一步拿不到东西都必须炸，不许「当作跳过」。
#[test]
fn the_synthetic_corpus_keeps_its_characters() {
    let root = corpus_root();
    let truth_path = root.join("truth.json");
    let dir = ocr_pp::models_dir();

    // ── 缺件即跳过：两样都必须在**开跑之前**确认 ──
    if !truth_path.is_file() {
        eprintln!(
            "跳过：没有 {} —— 先跑 scripts\\ocrbench\\gen_corpus.py 造合成语料。\
             ⚠ 这条测试在 CI 上永远走这一支；它绿**不代表**字准验过了。",
            truth_path.display()
        );
        return;
    }
    if !ocr_pp_pipeline::det_file_present(&dir) {
        eprintln!(
            "跳过：{} 下没有 det 权重 —— ⚠ 同样不代表字准验过了。",
            dir.display()
        );
        return;
    }

    let truth: Truth =
        serde_json::from_str(&std::fs::read_to_string(&truth_path).expect("读 truth.json 失败"))
            .expect("解析 truth.json 失败——字段名与 gen_corpus.py 对不上了？");

    let full = std::env::var_os("CS_OCR_CER_FULL").is_some();
    let picked: Vec<&TruthImage> = truth
        .images
        .iter()
        .filter(|im| full || SAMPLE.contains(&im.id.as_str()))
        .collect();

    assert!(
        !picked.is_empty(),
        "默认子集一张都没挑中：SAMPLE 里的 id 与 truth.json 对不上。\
         语料重新生成过就会这样——**这会让这条测试静默变成永不运行**，\
         所以按「量具坏了」处理，直接红。truth.json 里的 id 有：{:?}",
        truth
            .images
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>()
    );

    let mut total_edits = 0usize;
    let mut total_chars = 0usize;
    let mut exact_lines = 0usize;
    let mut recalled_lines = 0usize;
    let mut counted_lines = 0usize;
    let mut geom_miss = 0usize;
    let mut examples: Vec<String> = Vec::new();

    // ⚠ 自证注入（S4 已验完，此处已恢复）：Y_SHIFT = 0。
    // 原注入：把识别框的 y 整体推出答案带 ⇒ 几何漏检 100%。

    for im in &picked {
        let png = repo_root().join(&im.path);
        let frame = load_frame(&png);

        // ⚠ 回落在这里必须炸，见本测试的 doc comment。
        let ocr = recognize_in(&dir, &frame).unwrap_or_else(|| {
            panic!(
                "{}：PP-OCR 回落了（recognize_in 返回 None）。\
                 ⚠ 回落只打 WARN、退出码仍是 0，不主动炸就等于全绿。",
                im.id
            )
        });

        let hyps: Vec<Hyp> = ocr
            .lines
            .iter()
            .map(|l| Hyp {
                text: l.text.as_str(),
                y0: l.rect.y0,
                y1: l.rect.y1,
            })
            .collect();

        for gl in &im.lines {
            let g = normalize(&gl.text);
            if g.is_empty() {
                // 归一化后为空的标准答案不该进分母——留着会让分母虚高、CER 假性变好。
                // 与 `bench_lib.score` 同一处理。
                continue;
            }
            let band = Rect {
                x0: gl.band[0],
                y0: gl.band[1],
                x1: gl.band[2],
                y1: gl.band[3],
            };
            let j = judge(&g, band, &hyps);
            total_edits += j.edits;
            total_chars += g.chars().count();
            counted_lines += 1;
            if j.edits == 0 {
                exact_lines += 1;
            }
            if j.sim >= MISS_SIM {
                recalled_lines += 1;
            }
            if j.geom_missed {
                geom_miss += 1;
            }
            if j.edits > 0 && examples.len() < MAX_EXAMPLES {
                let note = if j.geom_missed {
                    "几何漏检：引擎没提出这一行".to_string()
                } else {
                    first_diff(&g, &j.hyp)
                };
                examples.push(format!(
                    "  [{}] 真值 {:?}\n      识别 {:?}（编辑 {}）—— {}",
                    im.id, gl.text, j.hyp, j.edits, note
                ));
            }
        }
    }

    let cer = total_edits as f64 / total_chars as f64;
    let exact_rate = exact_lines as f64 / counted_lines as f64;
    let recall = recalled_lines as f64 / counted_lines as f64;
    let geom_rate = geom_miss as f64 / counted_lines as f64;

    eprintln!(
        "合成语料字准（{}）：行 {counted_lines}，字符 {total_chars}，\
         编辑 {total_edits}，**CER {:.2}%**，召回 {:.0}%，逐行全对 {:.0}%，几何漏检 {:.0}%",
        if full {
            "全量 60 张"
        } else {
            "默认子集"
        },
        cer * 100.0,
        recall * 100.0,
        exact_rate * 100.0,
        geom_rate * 100.0,
    );
    for e in &examples {
        eprintln!("{e}");
    }

    // ⚠ **四条线一次全报，而不是第一个违规就 panic。**
    //
    // 早先写成四个连着的 `assert!`，于是**只有第一条会被执行到**：
    // 2026-10-18 实测，注入坏词典后第一条 CER 断言炸掉，
    // 召回 / 全对 / 几何那三条**根本没跑**——「有断言」与「断言验过」
    // 在输出上完全同形。⇒ 自证时必须能看到**全部**违规项，
    // 否则「三条都活着」只是我以为的。
    let mut violations: Vec<String> = Vec::new();
    if cer > CER_CEILING {
        violations.push(format!(
            "· CER {:.2}% 超过线 {:.2}%（行 {counted_lines} / 字符 {total_chars}）",
            cer * 100.0,
            CER_CEILING * 100.0,
        ));
    }
    if exact_rate < EXACT_FLOOR {
        violations.push(format!(
            "· 逐行全对率 {:.0}% 低于线 {:.0}% —— 总 CER 可能还看得过去，\
             但那是「每行都差一点」把平均稀释掉了，不是真准",
            exact_rate * 100.0,
            EXACT_FLOOR * 100.0,
        ));
    }
    if recall < RECALL_FLOOR {
        violations.push(format!(
            "· 消息召回率 {:.0}% 低于线 {:.0}%（口径 sim >= {MISS_SIM}，\
             与 `bench_lib.MISS_SIM` 同一个数）。票 30 给真帧钉的下限是 80%",
            recall * 100.0,
            RECALL_FLOOR * 100.0,
        ));
    }
    if geom_rate > GEOM_MISS_CEILING {
        violations.push(format!(
            "· 几何漏检 {:.0}% 超过线 {:.0}% —— 引擎有 {geom_miss} 行根本没提出框。\
             ⚠ 这与字准是两回事：字准要调 rec/后处理，漏检要调 det 阈值",
            geom_rate * 100.0,
            GEOM_MISS_CEILING * 100.0,
        ));
    }

    assert!(
        violations.is_empty(),
        "合成语料字准不达标（{}），共 {} 条违规：\n{}\n\
         ⚠ 读数见上。**先确认不是回落**——回落只打 WARN、退出码仍是 0；\
         真回落时 `recognize_in` 是 None、跑字之前就 panic 了。\n\
         错例（最多 {MAX_EXAMPLES} 行）：\n{}",
        if full {
            "全量 60 张"
        } else {
            "默认子集"
        },
        violations.len(),
        violations.join("\n"),
        examples.join("\n"),
    );
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/capture 的上上级是仓库根")
        .to_path_buf()
}

/// 读一张合成语料 PNG 成 [`Frame`]。
///
/// ⚠ 失败必须 panic：**语料文件损了不算「跳过」**，那会让这条测试在
/// 有人改了语料之后变成永不运行。
fn load_frame(png: &Path) -> Frame {
    let bytes =
        std::fs::read(png).unwrap_or_else(|e| panic!("读不到语料图 {}：{e}", png.display()));
    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .unwrap_or_else(|e| panic!("解不开语料图 {}：{e}", png.display()));
    let rgb = img.to_rgb8();
    Frame::from_rgb(rgb.width(), rgb.height(), rgb.into_raw()).expect("Frame::from_rgb 拒了这张图")
}
