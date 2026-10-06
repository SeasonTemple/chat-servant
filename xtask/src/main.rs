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
        println!("{:>3} | {}", i, l);
    }
    println!();
    println!("---- 整图 ----");
    println!("{}", ocr.text);
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
