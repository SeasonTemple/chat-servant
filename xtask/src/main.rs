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

#[derive(Parser)]
#[command(name = "xtask", about = "chat-servant 开发者探针")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
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
        "printwindow" => chatservant_capture::printwindow::capture(hit.hwnd_raw)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("PrintWindow 抓帧失败")?,
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
