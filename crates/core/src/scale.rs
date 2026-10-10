//! 尺度归一与视觉 token 预算。
//!
//! ## 这级解决什么
//!
//! 把图送给模型要花钱，而钱的单位是 **视觉 token**。不同供应商的算法完全不同，
//! 但对调用方来说只有一件事要做：**在预算内把这张图放到尽可能大**。
//!
//! 所以这里的接口只有一个核心函数：[`fit`]——
//! 「原图 w×h，预算 B，最多给我多大？」
//!
//! ## 为什么数字是这个数字
//!
//! 两条公式都抄自供应商自己的文档，不是猜的，也不是从任何参考实现反推的：
//!
//! - **Anthropic**（patch 计费）：一个 patch 是 28×28 像素，叫一个视觉 token，
//!   代价 `⌈w/28⌉ × ⌈h/28⌉`。标准档长边上限 1568px / 1568 token；
//!   高分辨率档（Claude 4.7 及以后）2576px / 4784 token。
//!   <https://platform.claude.com/docs/en/build-with-claude/vision>
//! - **OpenAI**（tile 计费，`gpt-4o-mini` / `gpt-4o` / `gpt-4.1` / `gpt-5.1` 这一族）：
//!   `base + tiles × per_tile`，tile 是 512×512。`gpt-4o-mini` 是
//!   **base 2833 / tile 5667**——**任何一张图，哪怕 `detail:low`，也要 2833 token**。
//!   `gpt-4o`/`gpt-4.1` 是 85 / 170。`gpt-5.1` 是 70 / 140。
//!   <https://platform.openai.com/docs/guides/images-vision>
//!
//! ## 为什么 `gpt-4o-mini` 的 base 高得离谱这件事必须写在这里
//!
//! 2833 是**与图的大小无关**的固定项。一个 64×64 的表情包和一张 1200×1200 的照片，
//! 在 `gpt-4o-mini` 上底账一样。这有两个后果：
//!
//! 1. 预算低于 base 时**无解**——这时要**如实告诉用户预算不够**，
//!    不能悄悄发一张然后超支。
//! 2. 视觉链路的选型**不该由「哪个模型便宜」决定**，该由「这个模型适不适合干视觉」决定。
//!    底账 2833 意味着每轮光图片就 2833 起。
//!
//! ## 这级不做的事
//!
//! - **不上采样。** 小的图原样送，放大只会更贵更糊。合同约束第 3 条
//!   「小图不盲目放大」在这里是硬性的。
//! - **不做模糊/二值化/锐化。** ADR-0002 已明确：对神经 OCR/VLM 是负收益。
//! - **不猜供应商。** 没登记进 [`Billing`] 的模型**没有默认档**，
//!   调用方必须显式声明——宁可报错，也不要拿一个猜出来的常数去算钱。

use std::fmt;

/// 一家供应商的视觉计费规则。
///
/// 两种形态，因为两家的公开文档就是这么定义的：
/// - [`Billing::Patch`]：每 `patch_px` 见方算 1 个 token，**没有底账**。
/// - [`Billing::Tile`]：`base` 是**与图无关的底账**，`per_tile` 是每个
///   `tile_px` 见方块的价钱。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Billing {
    /// 按 patch 计费：每 `patch_px` 见方算1 个 token，**没有底账**。
    Patch {
        /// patch 的边长（像素）。每 `patch_px` 见方算 1 个 token。
        patch_px: u32,
    },
    /// 按 tile 计费：`base` 是与图无关的底账，`per_tile` 是每个 tile 的价钱。
    Tile {
        /// tile 的边长（像素）。
        tile_px: u32,
        /// **与图无关的底账**——哪怕图退化成一个像素也要收这么多。
        base: u32,
        /// 每个 tile 的价钱。
        per_tile: u32,
    },
}

impl Billing {
    /// 28×28 patch。Anthropic 全系。
    pub const ANTHROPIC_PATCH: Self = Self::Patch { patch_px: 28 };

    /// `gpt-4o-mini`：底账 2833，每 512px tile 5667。
    pub const OPENAI_GPT4O_MINI: Self = Self::Tile {
        tile_px: 512,
        base: 2833,
        per_tile: 5667,
    };
    /// `gpt-4o` / `gpt-4.1`：底账 85，每 tile 170。
    pub const OPENAI_GPT4O: Self = Self::Tile {
        tile_px: 512,
        base: 85,
        per_tile: 170,
    };
    /// `gpt-5.1`：底账 70，每 tile 140。
    pub const OPENAI_GPT5_1: Self = Self::Tile {
        tile_px: 512,
        base: 70,
        per_tile: 140,
    };

    /// 无论图多小都收的底账。`Patch` 没有底账，返回 0。
    ///
    /// 预算必须先扣掉它——否则会算出一张「0 tile」的图，
    /// 而真实账单是 `base`。
    pub fn base(&self) -> u32 {
        match self {
            Self::Patch { .. } => 0,
            Self::Tile { base, .. } => *base,
        }
    }

    /// 一张 w×h 的图值多少视觉 token。
    pub fn tokens(&self, w: u32, h: u32) -> u32 {
        let (w, h) = (w.max(1) as u64, h.max(1) as u64);
        match *self {
            Self::Patch { patch_px } => {
                let p = patch_px as u64;
                let cols = w.div_ceil(p);
                let rows = h.div_ceil(p);
                (cols * rows) as u32
            }
            Self::Tile {
                tile_px,
                base,
                per_tile,
            } => {
                let t = tile_px as u64;
                let tiles = w.div_ceil(t) * h.div_ceil(t);
                base + (tiles * per_tile as u64) as u32
            }
        }
    }
}

/// 预算不够时的原因。**分开报，因为修法完全不同。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unfit {
    /// 底账就超了预算——**再小也发不了**，只能换模型或加预算。
    BelowBase {
        /// 该模型的底账。
        base: u32,
        /// 用户给的预算。
        budget: u32,
    },
}

impl fmt::Display for Unfit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BelowBase { base, budget } => write!(
                f,
                "这个模型发任何一张图都要 {base} token，预算只有 {budget}，无解"
            ),
        }
    }
}

impl std::error::Error for Unfit {}

/// 缩放比例的定点精度：1/10000。
///
/// 不用浮点是为了**跨平台可重现**——同一个尺寸在任何机器上
/// 必须 fit 出同一个结果，否则「100% / 150% DPI token 数相同」这条
/// 断言在别的机器上会莫名其妙地飘。
const SCALE_UNITS: u32 = 10_000;

/// 一张图在预算内的**最终尺寸**。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fitted {
    /// 送出去的宽（像素）。
    pub width: u32,
    /// 送去的高（像素）。
    pub height: u32,
    /// 送出去要花多少视觉 token。**实际值，不是预算值。**
    pub tokens: u32,
    /// 相对原图缩小了多少倍（1.0 = 没缩）。
    pub scale: f32,
}

/// 在 `budget` 内把 w×h 放到尽可能大。**不放大**。
///
/// ## 三条不能破的性质
///
/// 1. **不上采样**：已经装得下就原样返回。
/// 2. **保长宽比**：只缩不放，两边同比。
/// 3. **单调**：预算越大结果不小、token 不多。滑块往右拖不会让图变小——
///    「越大越差」是那种只有跑出来看才发现的 bug。
///
/// 底账就超预算时返回 [`Unfit`]，**不返回一个假的尺寸**。
pub fn fit(billing: &Billing, w: u32, h: u32, budget: u32) -> Result<Fitted, Unfit> {
    let (w, h) = (w.max(1), h.max(1));

    // 底账先扣。不扣的话 tile 型会算出「0 个 tile」这种不存在的图。
    if billing.base() > budget {
        return Err(Unfit::BelowBase {
            base: billing.base(),
            budget,
        });
    }

    if billing.tokens(w, h) <= budget {
        return Ok(Fitted {
            width: w,
            height: h,
            tokens: billing.tokens(w, h),
            scale: 1.0,
        });
    }

    // 定点二分。tokens 随 scale 单调不增，所以「最后一个装得下的刻度」就是答案。
    let mut lo = 0u32; // 已知装不下（除极端情况外 0 也不装得下，但保险起见从 0 扫）
    let mut hi = SCALE_UNITS; // 已知装不下
    let mut best: Option<(u32, u32)> = None;
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let (tw, th) = scaled(w, h, mid);
        if billing.tokens(tw, th) <= budget {
            best = Some((tw, th));
            lo = mid + 1;
        } else {
            // mid == 0 一定装得下（Patch 最小 1 token；Tile 底账已保证）
            if mid == 0 {
                break;
            }
            hi = mid - 1;
        }
    }

    let (tw, th) = best.ok_or(Unfit::BelowBase {
        base: billing.base(),
        budget,
    })?;
    // 二分的浮点保护：最终尺寸必须真的装得下。装不下就退一格。
    let (tw, th) = if billing.tokens(tw, th) <= budget {
        (tw, th)
    } else {
        let (tw2, th2) = scaled(w, h, SCALE_UNITS.min(1));
        (tw2.max(1), th2.max(1))
    };

    Ok(Fitted {
        width: tw,
        height: th,
        tokens: billing.tokens(tw, th),
        scale: tw as f32 / w as f32,
    })
}

/// 按 1/10000 的刻度等比缩。**至少 1×1**。
fn scaled(w: u32, h: u32, units: u32) -> (u32, u32) {
    let nw = ((w as u64 * units as u64) / SCALE_UNITS as u64).max(1) as u32;
    let nh = ((h as u64 * units as u64) / SCALE_UNITS as u64).max(1) as u32;
    (nw, nh)
}

/// Windows 的「无缩放」基准 DPI。
pub const DPI_BASE: u32 = 96;

/// 物理像素 → 逻辑像素。
///
/// ## 为什么预算要按**逻辑**尺寸算
///
/// 同一个布局在 150% DPI 下物理像素是 1.5 倍。按物理尺寸算预算，
/// 同样的对话在两台机器上花的钱差 2.25 倍——用户什么都没做，账单变了。
/// 按逻辑尺寸算，token 数就只跟**布局**有关，跟显示缩放无关。
///
/// DPI 读不出来时**按 1.0 处理**（返回原尺寸）：宁可低估 DPI 也不要
/// 凭空把图缩小，缩小会丢可读性，而丢可读性比多花一点钱糟得多。
pub fn logical_size(w: u32, h: u32, dpi: u32) -> (u32, u32) {
    if dpi <= DPI_BASE {
        return (w, h);
    }
    (
        ((w as u64 * DPI_BASE as u64) / dpi as u64).max(1) as u32,
        ((h as u64 * DPI_BASE as u64) / dpi as u64).max(1) as u32,
    )
}

/// 换算成 DPI 相对基准的比例，保留 3 位小数，**给界面显示用**。
///
/// 界面要显示「这张图在 150% 缩放下按 100% 的成本算」，
/// 光给整数会让这句话说不清楚。
pub fn dpi_ratio(dpi: u32) -> f32 {
    (dpi.max(1) as f32 / DPI_BASE as f32 * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Anthropic 自己的表格**：200×200 → 64 token。
    #[test]
    fn reproduces_anthropics_published_200x200() {
        assert_eq!(Billing::ANTHROPIC_PATCH.tokens(200, 200), 64);
    }

    /// 1000×1000 → 36×36 = 1296。
    #[test]
    fn reproduces_anthropics_published_1000x1000() {
        assert_eq!(Billing::ANTHROPIC_PATCH.tokens(1000, 1000), 1296);
    }

    /// 1092×1092 → 39×39 = 1521。1092 正好是 28 的整数倍，
    /// 所以这条卡的是 `div_ceil` 而不是舍入误差。
    #[test]
    fn reproduces_anthropics_published_1092x1092() {
        assert_eq!(Billing::ANTHROPIC_PATCH.tokens(1092, 1092), 1521);
    }

    /// 公式就是 `⌈w/28⌉ × ⌈h/28⌉`——边界上 1 像素要多一个 patch。
    #[test]
    fn a_patch_is_28_pixels_and_ceil_lands_right() {
        let b = Billing::ANTHROPIC_PATCH;
        assert_eq!(b.tokens(28, 28), 1, "正好一块 = 1 token");
        assert_eq!(b.tokens(29, 28), 2, "多一列要多一格");
        assert_eq!(b.tokens(1, 1), 1, "再小也是 1");
    }

    /// **`gpt-4o-mini` 的底账 2833 与图无关。**
    ///
    /// 这条锁的是「小图不便宜」这个反直觉的事实。谁要是哪天把 base 写成 0
    /// 让预算算起来好看，测试会立刻红。
    #[test]
    fn gpt4o_mini_has_an_enormous_size_independent_base() {
        let b = Billing::OPENAI_GPT4O_MINI;
        assert_eq!(b.tokens(64, 64), 2833 + 5667, "表情包也按一整个 tile 收");
        assert_eq!(b.base(), 2833);
        // 放大 100 倍：169 个 tile vs 1 个，差额是 168 × 5667。
        // 底账（2833）在两项里都出现，差额里必须**不含**它。
        assert_eq!(b.tokens(6400, 6400) - b.tokens(64, 64), 168 * 5667);
        assert_eq!(b.tokens(6400, 6400), 2833 + 169 * 5667);
    }

    /// 预算低于底账 → **如实报错**，不返回假尺寸。
    #[test]
    fn a_budget_below_the_base_is_reported_not_faked() {
        let r = fit(&Billing::OPENAI_GPT4O_MINI, 800, 600, 1000);
        assert_eq!(
            r.unwrap_err(),
            Unfit::BelowBase {
                base: 2833,
                budget: 1000
            }
        );
    }

    /// **不上采样。** 小图原样送。
    #[test]
    fn a_small_image_is_never_enlarged() {
        let f = fit(&Billing::ANTHROPIC_PATCH, 120, 90, 1_000_000).unwrap();
        assert_eq!((f.width, f.height), (120, 90), "尺寸不能变");
        assert_eq!(f.scale, 1.0);
    }

    /// **保长宽比。** 两边同比缩，误差在 1 像素内。
    #[test]
    fn fitting_preserves_aspect_ratio() {
        let f = fit(&Billing::ANTHROPIC_PATCH, 1200, 600, 400).unwrap();
        let got = f.width as f64 / f.height as f64;
        let want = 1200.0 / 600.0;
        assert!(
            (got - want).abs() < 0.02,
            "长宽比被改了：{got} vs {want}（{}×{}）",
            f.width,
            f.height
        );
    }

    /// **结果一定真的装进预算。** 不是「差不多」。
    #[test]
    fn the_fitted_size_really_is_within_budget() {
        for budget in [64u32, 100, 256, 500, 800, 1200, 1568, 3000] {
            for (w, h) in [(100u32, 100u32), (300, 400), (1920, 1080), (37, 913)] {
                let f = fit(&Billing::ANTHROPIC_PATCH, w, h, budget).unwrap();
                assert!(
                    f.tokens <= budget,
                    "{w}×{h} 预算 {budget} 出来 {}×{} = {} token，超了",
                    f.width,
                    f.height,
                    f.tokens
                );
                assert!(f.width <= w && f.height <= h, "不许放大");
                assert!(f.width >= 1 && f.height >= 1);
            }
        }
    }

    /// **预算越大，图不小、token 不多。**「越大越差」在这里被钉死。
    #[test]
    fn more_budget_never_makes_the_image_smaller_or_costlier() {
        let (w, h) = (1920u32, 1080u32);
        let mut prev: Option<Fitted> = None;
        for budget in [64u32, 200, 400, 800, 1200, 1568, 2000, 4000] {
            let f = fit(&Billing::ANTHROPIC_PATCH, w, h, budget).unwrap();
            if let Some(p) = prev {
                assert!(
                    f.tokens >= p.tokens,
                    "预算变大 token 反而少了：{p:?} → {f:?}"
                );
                assert!(
                    f.width * f.height >= p.width * p.height,
                    "预算变大图反而小了：{p:?} → {f:?}"
                );
            }
            prev = Some(f);
        }
    }

    /// **100% 与 150% DPI 下同样的逻辑布局，token 数相同。**
    ///
    /// 不需要真的改系统 DPI：同一个逻辑矩形在 150% 下物理尺寸是 1.5 倍，
    /// 把它除回逻辑尺寸就应当还原出完全一样的结果。
    #[test]
    fn dpi_scale_does_not_change_the_token_count() {
        let logical = (400u32, 300u32);
        let at_100 = logical_size(logical.0, logical.1, 96);
        let at_150 = logical_size(logical.0 * 150 / 100, logical.1 * 150 / 100, 144);
        assert_eq!(at_100, at_150, "150% 下除回逻辑尺寸应完全相等");

        for budget in [300u32, 900, 1568] {
            let a = fit(&Billing::ANTHROPIC_PATCH, at_100.0, at_100.1, budget).unwrap();
            let b = fit(&Billing::ANTHROPIC_PATCH, at_150.0, at_150.1, budget).unwrap();
            assert_eq!(a.tokens, b.tokens, "预算 {budget} 下两档 DPI 的 token 不同");
            assert_eq!((a.width, a.height), (b.width, b.height));
        }
    }

    /// 150% 下**确实缩了图**（不是空转），且缩的正是 DPI 那一倍。
    #[test]
    fn a_150_percent_window_really_is_downscaled() {
        let (lw, lh) = logical_size(600, 400, 144);
        assert_eq!((lw, lh), (400, 266), "600×400@150% 应等效 400×266");
        assert_eq!(dpi_ratio(144), 1.5);
        assert_eq!(dpi_ratio(96), 1.0);
    }

    /// DPI 读不出来（0）或低于基准时**按不缩放处理**——不凭空丢可读性。
    #[test]
    fn an_unknown_dpi_does_not_shrink_the_image() {
        assert_eq!(logical_size(800, 600, 0), (800, 600));
        assert_eq!(logical_size(800, 600, 96), (800, 600));
    }

    /// tile 型在预算内的实际取值。`gpt-4o-mini` 一个 tile 5667，
    /// 预算 15000 只能给 2 个 tile。
    #[test]
    fn tile_billing_fits_a_whole_number_of_tiles() {
        let b = Billing::OPENAI_GPT4O_MINI;
        let f = fit(&b, 2000, 2000, 2833 + 2 * 5667).unwrap();
        assert!(f.tokens <= 2833 + 2 * 5667, "{}", f.tokens);
        assert!(f.width <= 2000);
        // 预算再多也超不过 3 个 tile
        let g = fit(&b, 2000, 2000, 2833 + 3 * 5667).unwrap();
        assert!(g.width >= f.width);
    }

    /// 极端细长图不能被 fit 弄成 0 宽。
    #[test]
    fn a_degenerate_aspect_ratio_stays_positive() {
        for (w, h) in [(1u32, 5000u32), (5000, 1), (2, 2)] {
            let f = fit(&Billing::ANTHROPIC_PATCH, w, h, 10).unwrap();
            assert!(f.width >= 1 && f.height >= 1, "{w}×{h} 出来 {f:?}");
        }
    }
}
