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
    Ok(())
}
