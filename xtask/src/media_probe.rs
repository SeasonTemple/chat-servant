//! 媒体区域探针：对一张 PNG 跑 `chatservant_perceive::media::find`。
//!
//! ## 为什么要有这个命令
//!
//! `media.rs` 里的阈值大多标着「未实测」——手头没有含媒体的真实帧可标定。
//! 这个命令就是**标定的入口**：拿到一张含图的帧跑一遍，屏幕上的数字就是
//! 更新 `MIN_MEDIA_*` / `MEDIA_MIN_COLORS` 的依据。凭印象调阈值等于
//! 拿别人的数字当自己的事实。
//!
//! **不写盘**：只读 PNG，只打印。不产出任何图像文件。
//!
//! 用法：`cargo xtask media <png路径>`

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use chatservant_core::frame::Rect;
use chatservant_perceive::Palette;

pub fn run(path: PathBuf) -> Result<()> {
    println!("== 媒体区域探针 ==");

    let img = image::open(&path)
        .with_context(|| format!("打开 PNG 失败：{}", path.display()))?
        .to_rgb8();
    let (w, h) = img.dimensions();
    println!("来源 = {}  {}x{}", path.display(), w, h);

    let frame = chatservant_capture::Frame::from_rgb(w, h, img.into_raw())
        .context("PNG 尺寸与缓冲区不匹配")?;

    // 消息区优先用锚点推出来的那块；推不出来就退回整张图——
    // 探针的职责是把像素摆出来，不是替锚点做判断。
    let pane = match chatservant_capture::anchor::derive(&frame) {
        Ok(a) => {
            println!(
                "   锚点消息区 ({},{})-({},{})",
                a.chat.x0, a.chat.y0, a.chat.x1, a.chat.y1
            );
            a.chat
        }
        Err(e) => {
            println!("   锚点推不出来（{e}），退回整张图");
            Rect::new(0, 0, w as i32, h as i32)
        }
    };

    let palette =
        Palette::learn(&frame, pane).with_context(|| "学不到面板底色，这张图可能不是聊天区")?;
    println!(
        "   面板底色 rgb({},{},{})",
        palette.panel_bg[0], palette.panel_bg[1], palette.panel_bg[2]
    );

    let found = chatservant_perceive::find(&frame, pane);
    println!();
    if found.regions.is_empty() {
        println!("没有检出媒体区域");
        println!("（空结果是合法的。纯文字帧就该是空的——这条是防误判的基线）");
        return Ok(());
    }

    println!("检出 {} 个媒体区域：", found.regions.len());
    for (i, r) in found.regions.iter().enumerate() {
        println!(
            "  {:>2}. {:<4} ({},{})-({},{})  {}x{}  {} 种色  众数 {:.2}",
            i + 1,
            r.kind.label(),
            r.rect.x0,
            r.rect.y0,
            r.rect.x1,
            r.rect.y1,
            r.rect.width(),
            r.rect.height(),
            r.colors,
            r.purity
        );
    }
    println!();
    println!("阈值（BAND_H / BAND_GAP / MIN_MEDIA_* / MEDIA_MIN_COLORS / AVATAR_*）");
    println!("除 FLAT_MAX_COLORS 外均标「未实测」——把上面的数字记下来再回去改。");

    encode_all(&frame, &found);
    Ok(())
}

/// 把每个检出区域**真的走一遍编码链路**，打印送出去会是什么样。
///
/// ## 为什么必须走到这一步
///
/// 只报「检出 4 个区域」只证明**检测**通了，证明不了**发送**那一段：
/// 尺寸算得对不对、token 数得准不准、上采样了没有、base64 有多大。
/// 这些都只有真跑一遍编码才知道，而且**不需要任何 API key**——
/// 编码是纯本地计算。
///
/// 最后那一跳（HTTP 发送）才是需要 key 的部分，而这个探针到那儿为止。
fn encode_all(frame: &chatservant_capture::Frame, found: &chatservant_perceive::media::Found) {
    use chatservant_core::scale::Billing;

    println!();
    println!("== 编码链路（纯本地，不需要 key） ==");
    for (name, billing, base) in [
        ("Anthropic patch", Billing::ANTHROPIC_PATCH, 0u32),
        ("gpt-4o-mini tile", Billing::OPENAI_GPT4O_MINI, 2833),
    ] {
        // 预算取默认的 1200，与设置页默认值一致。
        const BUDGET: u32 = 1200;
        println!("-- {name}（预算 {BUDGET}） --");
        if base > BUDGET {
            println!("   底账 {base} > 预算 {BUDGET}：**无解**，这张模型发不出任何图。");
            println!("   界面必须如实这么说，而不是悄悄发一张超预算的。");
            continue;
        }
        let mut total = 0u32;
        for r in &found.regions {
            match chatservant_capture::media::encode_region(frame, r.rect, &billing, BUDGET) {
                Some(e) => {
                    total += e.tokens;
                    println!(
                        "   ({},{})-({},{}) 原 {}x{}  ->  发 {}x{}  {} token  base64 {:.1} KB",
                        r.rect.x0,
                        r.rect.y0,
                        r.rect.x1,
                        r.rect.y1,
                        r.rect.width(),
                        r.rect.height(),
                        e.width,
                        e.height,
                        e.tokens,
                        e.base64.len() as f64 / 1024.0,
                    );
                }
                None => println!("   ({},{}) 编码失败（区域越界？）", r.rect.x0, r.rect.y0),
            }
        }
        println!("   合计 {total} token。");
        assert!(
            found.regions.iter().all(|r| {
                chatservant_capture::media::encode_region(frame, r.rect, &billing, BUDGET)
                    .is_none_or(|e| e.tokens <= BUDGET)
            }),
            "有图超出了预算——token 账和实际发出的字节对不上"
        );
    }
}
