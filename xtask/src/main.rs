//! 开发者探针与维护命令。
//!
//! 用法：`cargo xtask <子命令>`。这是 `cargo` 的 alias，定义在
//! 根目录 `.cargo/config.toml`。
//!
//! ## 截图落盘的例外
//!
//! 硬约束规定截图不落盘。本 crate 是**唯一**允许写盘的路径，
//! 且每次写盘前都会打印一行明确提示，让用户知道这件事发生了。
//! 产物只写进 `target/probe/`。

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use chatservant_capture::{ChatSurface, WECHAT};
use clap::{Parser, Subcommand};

mod caps;

#[derive(Parser)]
#[command(name = "xtask", about = "chat-servant 开发者探针")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 采集能力体检：这台机器到底支不支持 WGC
    Caps,
    /// 遮挡检测体检：目标窗口现在能不能用第 ③ 级抓到
    Occluded,
    /// 抓一帧（默认 wgc）并立刻本地 OCR，把读到的文字打出来
    Read,
    /// 感知层探针：归类每一行并归并成消息，跑在真实帧上
    Perceive {
        /// 对已有的 PNG 跑；不给就现场抓一帧
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// 标定探针：逐行测出气泡底色、文字墨色与对比度。
    ///
    /// **归类阈值必须从这里的数据来**，不是拍脑袋。参考项目记的那组数值
    /// 是别人机器、别人微信版本的实测值，照搬等于把别人的事实当自己的。
    Measure {
        /// 对已有的 PNG 标定；不给就现场抓一帧
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// 取证一帧的像素结构，为 PaneAnchor 的阈值提供依据
    ///
    /// 不带 `--file` 时现场抓一帧；带 `--file` 时对已存的 PNG 重放。
    /// 取证必须可重放——不然每次改阈值都要重新摆一次窗口状态。
    Layout {
        /// 对已有的 PNG 做取证
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// 抓取与识别的诊断探针
    Probe {
        #[command(subcommand)]
        kind: ProbeCmd,
    },
}

#[derive(Subcommand)]
enum ProbeCmd {
    /// 抓一帧聊天窗口写盘，用来肉眼核对像素
    Frame {
        /// 目标聊天软件
        #[arg(long, default_value = "wechat")]
        app: String,
        /// 直接指定窗口句柄（十六进制，如 0x10AFC）。用来在目标软件
        /// 抓不到时，对照验证"这台机器的 WGC 到底通不通"。
        #[arg(long)]
        hwnd: Option<String>,
        /// 捕获方式：wgc（首选）或 printwindow（WGC 抓不到时的退路）
        #[arg(long, default_value = "wgc")]
        method: String,
        /// 输出路径。默认写到 target/probe/frame-<时间戳>.png
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("xtask=debug,wgc=info")
        .init();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Caps => {
            let c = caps::probe();
            println!("操作系统     = {}", c.os_build);
            match &c.wgc_supported {
                Some(Ok(true)) => println!("WGC 支持     = ✅ 系统层面支持"),
                Some(Ok(false)) => {
                    println!("WGC 支持     = ❌ 系统明确返回不支持");
                    println!("                会话会静默关闭、不报任何错误——与我们的现象完全吻合");
                }
                Some(Err(e)) => println!("WGC 支持     = ⚠ WinRT 调用失败：{e}"),
                None => println!("WGC 支持     = 未知"),
            }
            Ok(())
        }
        Cmd::Occluded => probe_occluded("wechat"),
        Cmd::Read => probe_read(),
        Cmd::Perceive { file } => probe_perceive(file),
        Cmd::Measure { file } => probe_measure(file),
        Cmd::Layout { file } => probe_layout(file),
        Cmd::Probe { kind } => match kind {
            ProbeCmd::Frame {
                app,
                hwnd,
                method,
                out,
            } => probe_frame(&app, hwnd, &method, out),
        },
    }
}

/// 遮挡检测体检。
///
/// 第 ③ 级（区域抓屏）读的是屏幕坐标处的像素，所以「这个窗口现在能不能
/// 抓」这件事必须**如实测出来**，不能靠"抓一次没报错"来判断——之前那次
/// 抓回的其实是被盖住窗口的像素，机制上完全成功，内容完全错误。
fn probe_occluded(app: &str) -> Result<()> {
    set_dpi_aware();
    let surface = surface_for(app)?;
    let hit =
        chatservant_capture::find_chat_window(&surface).map_err(|e| anyhow::anyhow!("{e}"))?;

    let (x0, y0, x1, y1) = window_rect(hit.hwnd_raw)?;
    println!(
        "窗口 = {} ({})  0x{:X}",
        surface.label, hit.exe, hit.hwnd_raw
    );
    println!(
        "屏幕矩形 = ({x0}, {y0}) - ({x1}, {y1})   {}x{}",
        x1 - x0,
        y1 - y0
    );

    // 采样点：四角内缩 + 中心 + 三等分。够覆盖"只被盖住一角"的情况。
    let inset = 24i32;
    let w = x1 - x0;
    let h = y1 - y0;
    let samples: Vec<(&str, i32, i32)> = vec![
        ("左上", x0 + inset, y0 + inset),
        ("右上", x1 - inset, y0 + inset),
        ("左下", x0 + inset, y1 - inset),
        ("右下", x1 - inset, y1 - inset),
        ("中心", x0 + w / 2, y0 + h / 2),
        ("上中", x0 + w / 2, y0 + inset),
        ("下中", x0 + w / 2, y1 - inset),
        ("左中", x0 + inset, y0 + h / 2),
        ("右中", x1 - inset, y0 + h / 2),
    ];

    println!();
    println!("采样点命中检查（9 个点）：");
    let mut covered = 0;
    for (name, x, y) in &samples {
        let hit_hwnd = top_window_at(*x, *y);
        let is_target = hit_hwnd == hit.hwnd_raw;
        if !is_target {
            covered += 1;
        }
        println!(
            "  {:<4} ({:>5},{:>5})  命中 0x{:X}  {}",
            name,
            x,
            y,
            hit_hwnd,
            if is_target {
                "目标窗口"
            } else {
                "← 被别的窗口盖住"
            }
        );
    }

    println!();
    let verdict = if covered == 0 {
        "✅ 全部采样点都命中目标窗口，第 ③ 级可以抓"
    } else {
        "❌ 有采样点被遮挡，第 ③ 级抓到的会是盖在上面的东西"
    };
    println!("{verdict}   （{covered}/{} 被遮挡）", samples.len());
    println!(
        "is_occluded() 判定 = {}",
        chatservant_capture::region::is_occluded(
            hit.hwnd_raw,
            &samples.iter().map(|&(_, x, y)| (x, y)).collect::<Vec<_>>()
        )
    );
    Ok(())
}

/// 某一点命中的顶层窗口句柄。取不到时返回 0。
fn top_window_at(x: i32, y: i32) -> isize {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::WindowFromPoint;
    let h = unsafe { WindowFromPoint(POINT { x, y }) };
    if h.is_invalid() { 0 } else { h.0 as isize }
}

fn window_rect(hwnd_raw: isize) -> Result<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &raw mut r) }.context("GetWindowRect 失败")?;
    Ok((r.left, r.top, r.right, r.bottom))
}

/// 抓一帧 + 本地 OCR 读文字，一条命令走完「像素 → 文字」。
///
/// 这是目标判据的验证入口：帧从**被遮挡的窗口**抓来，文字能读出来，
/// 就说明「遮挡下抓到的帧内容正确」这条链是通的。
fn probe_read() -> Result<()> {
    set_dpi_aware();
    let hit = chatservant_capture::find_chat_window(&WECHAT).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("窗口 = {}  0x{:X}", hit.exe, hit.hwnd_raw);

    let t0 = std::time::Instant::now();
    let frame = chatservant_capture::wgc_alt::capture_one_frame(hit.hwnd_raw)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("wgc 抓帧失败")?;
    let grab_ms = t0.elapsed().as_millis();

    let t1 = std::time::Instant::now();
    let ocr = chatservant_capture::ocr::recognize(&frame).map_err(|e| anyhow::anyhow!("{e}"))?;
    let ocr_ms = t1.elapsed().as_millis();

    println!("抓帧 {}ms  {}x{}", grab_ms, frame.width(), frame.height());
    println!("OCR  {}ms  逐行 {} 行", ocr_ms, ocr.lines.len());
    println!();
    println!("---- 识别出的文字 ----");
    for (i, l) in ocr.lines.iter().enumerate() {
        println!(
            "{:>3} | y={:>4} h={:>2} | {}",
            i,
            l.rect.y0,
            l.rect.height(),
            l.text
        );
    }
    println!();
    println!("---- 整图 ----");
    println!("{}", ocr.text);
    Ok(())
}

/// 取证一帧的像素结构。
///
/// 这是 PaneAnchor 的**前置**：阈值必须从真实帧里推，不能照搬参考项目的
/// 数字（那是别人机器、别人微信版本的实测值）。所以这个命令只打印
/// 像素分布，不推导任何结论——结论由人看着数据定，写进 PaneAnchor 的测试里。
/// 感知层探针：从聊天区学配色 → 归类每一行 → 归并成消息。
fn probe_perceive(file: Option<PathBuf>) -> Result<()> {
    use chatservant_perceive::{NoiseClass, Palette, Speaker, classify_all, group};

    let frame = match file {
        Some(p) => {
            let img = image::open(&p).context("打开 PNG 失败")?.to_rgb8();
            let (w, h) = img.dimensions();
            println!("来源 = {}", p.display());
            chatservant_capture::Frame::from_rgb(w, h, img.into_raw())
                .context("PNG 尺寸与缓冲区不匹配")?
        }
        None => {
            set_dpi_aware();
            let hit = chatservant_capture::find_chat_window(&WECHAT)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("来源 = 现场抓帧 0x{:X}", hit.hwnd_raw);
            chatservant_capture::wgc_alt::capture_one_frame(hit.hwnd_raw)
                .map_err(|e| anyhow::anyhow!("{e}"))
                .context("wgc 抓帧失败")?
        }
    };

    let anchor = chatservant_capture::anchor::derive(&frame)
        .map_err(|e| anyhow::anyhow!("锚点推导失败：{e}"))?;
    let pane = frame.crop(anchor.chat).context("裁剪消息区失败")?;

    // 配色只学面板底色。「在不在气泡里」看底色是否等于它，
    // 「我方还是对方」看彩度——两个判断都不需要先知道气泡色。
    let palette = Palette::learn(&frame, anchor.chat).ok_or_else(|| {
        anyhow::anyhow!("学不到配色（聊天区太平，可能是空会话）——上层应降级，不许退回硬编码颜色")
    })?;
    println!(
        "配色 = 面板 rgb({},{},{})",
        palette.panel_bg[0], palette.panel_bg[1], palette.panel_bg[2]
    );

    let ocr = chatservant_capture::ocr::recognize(&pane).map_err(|e| anyhow::anyhow!("{e}"))?;
    let attrs = classify_all(&ocr.lines, &pane, &palette);

    println!();
    println!(
        "{:<3} {:<7} {:<10} {:<30} 文字",
        "#", "说话人", "归类", "依据"
    );
    println!("{}", "-".repeat(110));
    for (i, a) in attrs.iter().enumerate() {
        println!(
            "{:<3} {:<7} {:<10} {:<30} {}",
            i,
            match a.speaker {
                Speaker::Me => "我",
                Speaker::Them => "对方",
                Speaker::System => "-",
            },
            match a.noise {
                NoiseClass::Text => "正文",
                NoiseClass::Header => "标题",
                NoiseClass::Gray => "灰字",
                NoiseClass::ImageText => "图内字",
                NoiseClass::Fragment => "碎片",
            },
            a.evidence.to_string(),
            a.text
        );
    }

    println!();
    println!("== 归并后的消息 ==");
    for m in group(&attrs) {
        println!(
            "  [{}] ({},{})-({},{})  {}",
            match m.speaker {
                Speaker::Me => "我",
                Speaker::Them => "对方",
                Speaker::System => "系统",
            },
            m.rect.x0,
            m.rect.y0,
            m.rect.x1,
            m.rect.y1,
            m.text
        );
    }
    Ok(())
}

/// 标定探针：把每一行 OCR 的几何与颜色测出来。
///
/// 输出四样东西，就是归类阈值需要的全部输入：
/// - `bg`     文字框**外圈**的主色 = 气泡底色（框内会被文字污染）
/// - `ink`    框内与 bg 差别最大的像素 = 文字墨色
/// - `对比`   bg 与 ink 的感知距离，用来切「正文 / 灰字」
/// - `pos`    相对面板中心的左右位置，用来切「我 / 对方」
fn probe_measure(file: Option<PathBuf>) -> Result<()> {
    let frame = match file {
        Some(p) => {
            let img = image::open(&p).context("打开 PNG 失败")?.to_rgb8();
            let (w, h) = img.dimensions();
            println!("来源 = {}", p.display());
            chatservant_capture::Frame::from_rgb(w, h, img.into_raw())
                .context("PNG 尺寸与缓冲区不匹配")?
        }
        None => {
            set_dpi_aware();
            let hit = chatservant_capture::find_chat_window(&WECHAT)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("来源 = 现场抓帧 0x{:X}", hit.hwnd_raw);
            chatservant_capture::wgc_alt::capture_one_frame(hit.hwnd_raw)
                .map_err(|e| anyhow::anyhow!("{e}"))
                .context("wgc 抓帧失败")?
        }
    };

    let anchor = chatservant_capture::anchor::derive(&frame)
        .map_err(|e| anyhow::anyhow!("锚点推导失败：{e}"))?;
    println!(
        "消息区 = ({}, {}) - ({}, {})",
        anchor.chat.x0, anchor.chat.y0, anchor.chat.x1, anchor.chat.y1
    );
    println!(
        "面板底色 = rgb({}, {}, {})",
        anchor.panel_bg[0], anchor.panel_bg[1], anchor.panel_bg[2]
    );
    println!();

    let pane = frame.crop(anchor.chat).context("裁剪消息区失败")?;
    let ocr = chatservant_capture::ocr::recognize(&pane).map_err(|e| anyhow::anyhow!("{e}"))?;

    let cx = (anchor.chat.x0 + anchor.chat.x1) as f32 / 2.0;
    println!(
        "{:<3} {:<28} {:>4} {:>18} {:>18} {:>7}  位置/文字",
        "#", "rect", "h", "气泡底色", "文字墨色", "对比"
    );
    println!("{}", "-".repeat(120));
    for (i, l) in ocr.lines.iter().enumerate() {
        let Some((bg, ink, contrast)) = measure_line(&pane, l.rect) else {
            continue;
        };
        let mid = l.rect.x0 as f32 + l.rect.width() as f32 / 2.0;
        let pos = if mid > cx + anchor.chat.width() as f32 * 0.15 {
            "右"
        } else if mid < cx - anchor.chat.width() as f32 * 0.15 {
            "左"
        } else {
            "中"
        };
        println!(
            "{:<3} ({:>5},{:>4})-({:>5},{:>4}) {:>4}  rgb({:>3},{:>3},{:>3})  rgb({:>3},{:>3},{:>3})  {:>6.1}  {}  {}",
            i,
            l.rect.x0,
            l.rect.y0,
            l.rect.x1,
            l.rect.y1,
            l.rect.height(),
            bg[0],
            bg[1],
            bg[2],
            ink[0],
            ink[1],
            ink[2],
            contrast,
            pos,
            l.text
        );
    }
    Ok(())
}

/// 测一行：外圈主色（气泡底色）、框内最异样的像素（文字墨色）、两者感知距离。
fn measure_line(
    frame: &chatservant_capture::Frame,
    r: chatservant_core::frame::Rect,
) -> Option<([u8; 3], [u8; 3], f32)> {
    use std::collections::HashMap;

    // 外圈：文字框向外扩 6px 的一圈，避开字形本身
    let pad = 6i32;
    let ox0 = r.x0 - pad;
    let oy0 = r.y0 - pad;
    let ox1 = r.x1 + pad;
    let oy1 = r.y1 + pad;
    let mut hist: HashMap<[u8; 3], usize> = HashMap::new();
    for y in oy0.max(0)..oy1.min(frame.height() as i32) {
        for x in ox0.max(0)..ox1.min(frame.width() as i32) {
            if x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1 {
                continue; // 框内不算
            }
            if let Some(p) = frame.pixel(x as u32, y as u32) {
                *hist.entry(p).or_insert(0) += 1;
            }
        }
    }
    let bg = hist.into_iter().max_by_key(|(_, n)| *n).map(|(c, _)| c)?;

    // 框内：与 bg 距离最大的像素 = 墨色
    let mut ink = bg;
    let mut best = 0f32;
    for y in r.y0.max(0)..r.y1.min(frame.height() as i32) {
        for x in r.x0.max(0)..r.x1.min(frame.width() as i32) {
            if let Some(p) = frame.pixel(x as u32, y as u32) {
                let d = dist(p, bg);
                if d > best {
                    best = d;
                    ink = p;
                }
            }
        }
    }
    Some((bg, ink, best))
}

/// 感知距离：亮度项 + 色度项。单纯的欧氏距离会把「深灰 vs 纯黑」判得过近。
fn dist(a: [u8; 3], b: [u8; 3]) -> f32 {
    let luma = |c: [u8; 3]| 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
    let dl = luma(a) - luma(b);
    let chroma = [
        a[0] as f32 - b[0] as f32,
        a[1] as f32 - b[1] as f32,
        a[2] as f32 - b[2] as f32,
    ];
    // 色度去掉亮度分量后的模长 = 「有多不一样」，与亮度分开算
    let mean = (chroma[0] + chroma[1] + chroma[2]) / 3.0;
    let sat = (chroma[0] - mean).powi(2) + (chroma[1] - mean).powi(2) + (chroma[2] - mean).powi(2);
    (dl * dl + sat).sqrt()
}

fn probe_layout(file: Option<PathBuf>) -> Result<()> {
    let frame = match file {
        Some(p) => {
            println!("来源 = {}（重放，不抓新帧）\n", p.display());
            let img = image::open(&p).context("打开 PNG 失败")?.to_rgb8();
            let (w, h) = img.dimensions();
            chatservant_capture::Frame::from_rgb(w, h, img.into_raw())
                .context("PNG 尺寸与缓冲区不匹配")?
        }
        None => {
            set_dpi_aware();
            let hit = chatservant_capture::find_chat_window(&WECHAT)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let f = chatservant_capture::wgc_alt::capture_one_frame(hit.hwnd_raw)
                .map_err(|e| anyhow::anyhow!("{e}"))
                .context("wgc 抓帧失败")?;
            println!("来源 = 现场抓帧 0x{:X}\n", hit.hwnd_raw);
            f
        }
    };
    let diag = chatservant_capture::anchor_diag::analyze(&frame);
    match chatservant_capture::anchor::derive(&frame) {
        Ok(a) => println!(
            "== 推导出的 PaneAnchor ==\n  消息区     = ({}, {}) - ({}, {})   {}x{}\n  面板左右   = ({}, {})\n  面板底色   = rgb({}, {}, {})\n  标题栏下沿 = y={}\n  输入框上沿 = y={}",
            a.chat.x0,
            a.chat.y0,
            a.chat.x1,
            a.chat.y1,
            a.chat.width(),
            a.chat.height(),
            a.panel.x0,
            a.panel.x1,
            a.panel_bg[0],
            a.panel_bg[1],
            a.panel_bg[2],
            a.header_top,
            a.input_top
        ),
        Err(e) => println!("== 推导失败 ==\n  {e}\n  -> 上层应降级重试，不猜默认坐标"),
    }
    println!();
    println!("{}", chatservant_capture::anchor_diag::render(&diag));
    Ok(())
}

fn surface_for(key: &str) -> Result<ChatSurface> {
    match key {
        "wechat" => Ok(WECHAT),
        other => bail!("未知的聊天软件：{other}（当前只支持 wechat）"),
    }
}

/// 抓到一帧写盘。回答的是「捕获链路在**这台机器**上到底通不通」。
fn probe_frame(
    app: &str,
    hwnd_arg: Option<String>,
    method: &str,
    out: Option<PathBuf>,
) -> Result<()> {
    // 混�� DPI 下窗口矩形要用物理像素，必须在枚举之前就声明 per-monitor v2，
    // 之后 GetWindowRect 才是对的。
    set_dpi_aware();

    let hit = match &hwnd_arg {
        Some(hex) => {
            let raw = usize::from_str_radix(hex.trim_start_matches("0x"), 16)
                .with_context(|| format!("hwnd 不是合法十六进制：{hex}"))?;
            if !chatservant_capture::is_alive(raw as isize) {
                bail!("hwnd 0x{raw:X} 已经不是有效窗口了");
            }
            let size = window_size(raw as isize)?;
            println!("按 --hwnd 直接抓取：0x{raw:X}  尺寸 {}x{}", size.0, size.1);
            chatservant_capture::WindowHit {
                hwnd_raw: raw as isize,
                pid: 0,
                exe: "<--hwnd>".into(),
                title: "<--hwnd>".into(),
                area: i64::from(size.0) * i64::from(size.1),
            }
        }
        None => {
            let surface = surface_for(app)?;
            let hit = chatservant_capture::find_chat_window(&surface)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("找到窗口：");
            println!("  软件   = {}", surface.label);
            println!("  进程   = {}", hit.exe);
            println!("  pid    = {}", hit.pid);
            println!("  标题   = {}", hit.title);
            println!("  hwnd   = 0x{:X}", hit.hwnd_raw);

            // 最小化的窗口不渲染，直接抓是黑帧。先用不抢焦点的方式还原。
            if chatservant_capture::restore_without_activating(hit.hwnd_raw) {
                println!("窗口原本是最小化状态：已用不激活方式还原（不抢焦点）。");
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            let size = window_size(hit.hwnd_raw)?;
            println!("窗口尺寸 = {}x{}", size.0, size.1);
            hit
        }
    };

    let size = window_size(hit.hwnd_raw)?;
    let path = out.unwrap_or_else(|| default_probe_path("frame"));

    // 写盘提示——硬约束的显式例外，必须让人看见。
    println!();
    println!("⚠  即将把截图写到：{}", path.display());
    println!("   （截图不落盘是本程序的硬约束，probe 是唯一例外）");

    let t0 = std::time::Instant::now();
    let frame = match method {
        "wgc2" => {
            chatservant_capture::wgc_alt::capture_one_frame(hit.hwnd_raw).context("wgc 抓帧失败")?
        }
        "printwindow" => chatservant_capture::printwindow::capture(hit.hwnd_raw)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("PrintWindow 抓帧失败")?,
        "region" => chatservant_capture::region::capture(hit.hwnd_raw)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("区域抓屏失败")?,
        // 诊断用：抓整个主显示器，用来区分「窗口不可捕获」和「整条 WGC 链路坏了」
        "monitor" => {
            chatservant_capture::wgc::capture_primary_monitor().context("显示器抓帧失败")?
        }
        _ => chatservant_capture::capture_one_frame(hit.hwnd_raw, size).context("WGC 抓帧失败")?,
    };
    let elapsed = t0.elapsed();

    println!();
    println!("抓到一帧：");
    println!("  尺寸   = {}x{}", frame.width(), frame.height());
    println!("  通道   = RGB8（已从 RGBA8 转换）");
    println!("  耗时   = {} ms", elapsed.as_millis());
    println!(
        "  全黑   = {}",
        if frame.is_black() {
            "是 ← 这不该发生"
        } else {
            "否"
        }
    );

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    write_png(&frame, &path)?;
    println!();
    println!(
        "✅ 已写入 {}（{} 字节）",
        path.display(),
        std::fs::metadata(&path)?.len()
    );
    println!("   用看图工具打开确认像素对不对。");
    Ok(())
}

fn window_size(hwnd_raw: isize) -> Result<(i32, i32)> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &raw mut rect) }
        .context("GetWindowRect 失败（窗口可能刚被关掉）")?;
    Ok((rect.right - rect.left, rect.bottom - rect.top))
}

fn set_dpi_aware() {
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    // 拿不到就退回系统默认设置——这是优化项，不该让探针直接失败。
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

fn default_probe_path(stem: &str) -> PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Path::new("target")
        .join("probe")
        .join(format!("{stem}-{ts}.png"))
}

/// 编码用 `image` crate，不自己实现 PNG 编码（ADR-0003）。
fn write_png(frame: &chatservant_capture::Frame, path: &Path) -> Result<()> {
    let img = image::RgbImage::from_raw(frame.width(), frame.height(), frame.as_bytes().to_vec())
        .context("构造图像失败")?;
    img.save(path).context("写 PNG 失败")?;
    Ok(())
}
