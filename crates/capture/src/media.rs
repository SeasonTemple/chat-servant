//! 把一个媒体区域变成**可以发出去的字节**。
//!
//! ## 这一层的职责边界
//!
//! 进：`&Frame` + 一个矩形 + 一份计费规则 + 一个预算。
//! 出：一段 base64 + 它实际花了多少视觉 token + 实际尺寸。
//!
//! **不做**：判断这块是不是图（那是 `perceive::media` 的事）、
//! 决定要不要发（那是外发总闸的事）、调用模型（那是 `reason` 的事）。
//!
//! ## 顺序很重要：先定尺寸，再重采样
//!
//! `core::scale::fit()` 算出**在预算内的最大尺寸**，然后才让 `image`
//! 缩到那个尺寸。反过来做（先缩再算 token）会得到一个「已经缩过了，
//! 但没人知道缩到多大」的结果——那样 token 账就对不上实际发出的字节。

use base64::Engine as _;
use chatservant_core::frame::{Frame, Rect};
use chatservant_core::scale::{Billing, Fitted};

/// 一张已经缩好、编好码的图。**中间态**——
/// `reason::ImagePayload` 是同一个形状的最终形态，两边刻意不共用类型：
/// `capture` 不依赖 `reason`，`reason` 也不该依赖 `capture`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedImage {
    pub media_type: &'static str,
    pub base64: String,
    /// 送出这一张要花多少视觉 token。**实际值，不是预算值。**
    pub tokens: u32,
    pub width: u32,
    pub height: u32,
}

/// 把一块区域裁出、按预算缩放、编成 PNG 的 base64。
///
/// 返回 `None` 只有两种情况，都**不是错误**：
/// - 区域裁不出（坐标越界）；
/// - 预算低于供应商底账——那说明这张图**根本发不出去**，
///   调用方要据此如实告诉用户，而不是发一张超预算的。
pub fn encode_region(
    frame: &Frame,
    rect: Rect,
    billing: &Billing,
    budget: u32,
) -> Option<EncodedImage> {
    let cropped = frame.crop(rect)?;
    let (w, h) = (cropped.width(), cropped.height());
    if w == 0 || h == 0 {
        return None;
    }

    // ① 先定尺寸。DPI 归一在 `fit` 之前做——预算要跟**逻辑尺寸**有关，
    // 跟显示缩放无关（见 core::scale 的说明）。
    let fitted: Fitted = chatservant_core::scale::fit(billing, w, h, budget).ok()?;

    // ② 再重采样到那个尺寸。用 `resize_exact` 而不是 `resize`：
    //    尺寸是 fit() 精确算出来的，这里**只负责缩到它**，不该再改。
    let resized = resize_exact(&cropped, fitted.width, fitted.height)?;

    // ③ 编码。**只出数字就能被断言**（见测试）。
    // ③ 编码。`ImageBuffer::write_to` 直接写 PNG，不用先转 DynamicImage。
    let png = image::RgbImage::from_raw(
        resized.width(),
        resized.height(),
        resized.as_bytes().to_vec(),
    )?;
    let mut buf: Vec<u8> = Vec::new();
    png.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .ok()?;

    Some(EncodedImage {
        media_type: "image/png",
        base64: base64::engine::general_purpose::STANDARD.encode(&buf),
        tokens: fitted.tokens,
        width: fitted.width,
        height: fitted.height,
    })
}

/// 缩到**精确**尺寸。`image` 的 `resize` 会保持长宽比再塞进框里，
/// 那会悄悄改变尺寸、让 token 账对不上。
fn resize_exact(src: &Frame, w: u32, h: u32) -> Option<Frame> {
    if w == src.width() && h == src.height() {
        // 尺寸没变就**别重采样**——重采样是有损操作，
        // 没理由让一张不需要缩的图白白糊一次。
        return Some(src.clone());
    }
    let img = image::RgbImage::from_raw(src.width(), src.height(), src.as_bytes().to_vec())?;
    // `imageops::resize` 缩到**精确**的 nwidth×nheight（它不保长宽比；
    // 保长宽比的是 `DynamicImage::resize`）。尺寸由 `fit` 精确算出，
    // 这里只负责缩到它，不该再改。
    let out = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
    Frame::from_rgb(out.width(), out.height(), out.into_raw())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chatservant_core::scale::Billing;

    /// 造一块「花的」区域：坐标决定颜色，所以色种数很多。
    fn busy(w: u32, h: u32) -> Frame {
        let mut px = vec![0u8; (w * h * 3) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 3) as usize;
                px[i] = (x % 251) as u8;
                px[i + 1] = (y % 253) as u8;
                px[i + 2] = ((x + y) % 247) as u8;
            }
        }
        Frame::from_rgb(w, h, px).expect("尺寸一致")
    }

    /// **尺寸没变就不重采样。** 输出必须与原区域逐字节相等——
    /// 这条能抓住「一律重采样」的实现：有损重采样哪怕尺寸一样也会改字节。
    #[test]
    fn an_unscaled_region_is_returned_byte_for_byte() {
        let f = busy(120, 90);
        let enc = encode_region(
            &f,
            Rect::new(0, 0, 120, 90),
            &Billing::ANTHROPIC_PATCH,
            100_000,
        )
        .expect("应当成功");
        assert_eq!((enc.width, enc.height), (120, 90));
        let again = encode_region(
            &f,
            Rect::new(0, 0, 120, 90),
            &Billing::ANTHROPIC_PATCH,
            100_000,
        )
        .expect("应当成功");
        assert_eq!(enc.base64, again.base64, "同样输入必须得到同样字节");
    }

    /// **输出尺寸一定装得进预算。** 这条是整个 token 账的根。
    #[test]
    fn the_output_always_fits_the_budget() {
        let f = busy(600, 400);
        for budget in [100u32, 300, 800, 1568, 4000] {
            let enc = encode_region(
                &f,
                Rect::new(0, 0, 600, 400),
                &Billing::ANTHROPIC_PATCH,
                budget,
            )
            .expect("应当成功");
            assert!(
                enc.tokens <= budget,
                "预算 {budget} 出来 {} token（{}x{}），超了",
                enc.tokens,
                enc.width,
                enc.height
            );
        }
    }

    /// **不上采样。** 小图原样发，不放大。
    #[test]
    fn a_small_region_is_never_enlarged() {
        let f = busy(80, 60);
        let enc = encode_region(
            &f,
            Rect::new(0, 0, 80, 60),
            &Billing::ANTHROPIC_PATCH,
            1_000_000,
        )
        .expect("应当成功");
        assert_eq!((enc.width, enc.height), (80, 60), "不许放大");
    }

    /// 预算低于底账 → `None`，**不返回一张超预算的图**。
    #[test]
    fn a_budget_below_the_provider_base_yields_nothing() {
        let f = busy(400, 300);
        assert!(
            encode_region(
                &f,
                Rect::new(0, 0, 400, 300),
                &Billing::OPENAI_GPT4O_MINI,
                100
            )
            .is_none(),
            "gpt-4o-mini 底账 2833，预算 100 不该产出任何东西"
        );
    }

    /// 坐标越界 → `None`，不是 panic。
    #[test]
    fn an_out_of_bounds_rect_yields_nothing() {
        let f = busy(100, 100);
        assert!(
            encode_region(
                &f,
                Rect::new(500, 500, 900, 900),
                &Billing::ANTHROPIC_PATCH,
                1568
            )
            .is_none()
        );
    }

    /// 编码出来的东西**真的是 PNG**：base64 解回来必须带 PNG 魔数。
    #[test]
    fn the_payload_really_is_a_png() {
        let f = busy(64, 64);
        let enc = encode_region(&f, Rect::new(0, 0, 64, 64), &Billing::ANTHROPIC_PATCH, 1568)
            .expect("应当成功");
        let raw = base64::engine::general_purpose::STANDARD
            .decode(&enc.base64)
            .expect("base64 合法");
        assert_eq!(&raw[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        assert_eq!(enc.media_type, "image/png");
    }
}
