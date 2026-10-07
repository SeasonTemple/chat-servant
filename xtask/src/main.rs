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
mod e2e;
mod inject_probe;
mod media_probe;
mod secret_probe;

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
    /// 把目标窗口提到前台。**这会抢焦点**——抓帧失败时的救急手段
    Focus,
    /// 抓一帧（默认 wgc）并立刻本地 OCR，把读到的文字打出来
    Read,
    /// 生成脱敏后的回归夹具
    Fixture {
        /// 夹具名，同时作为文件名
        #[arg(long)]
        name: String,
        /// 源 PNG；不给就现场抓一帧
        #[arg(long)]
        source: Option<PathBuf>,
    },
    /// 感知层探针：归类每一行并归并成消息，跑在真实帧上
    Perceive {
        /// 对已有的 PNG 跑；不给就现场抓一帧
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// 只跑 OCR，把读到的行原样打出来——**不做锚定、不做归类**
    ///
    /// 为什么要有这个：`perceive` 里 OCR 和归类是串在一起的，
    /// 一旦「没读到对方消息」，分不清是**识别器没读出来**，
    /// 还是**读出来了但被判成了别的类**。这个命令把那一层剥掉。
    /// `--scale` 会在 OCR 前把图放大，用来验证「字太小识别器不收」
    /// 这类假设。
    Ocr {
        /// 要识别的 PNG 路径
        file: PathBuf,
        /// OCR 前的放大倍数。1 = 原尺寸
        #[arg(long, default_value_t = 1)]
        scale: u32,
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
    /// 媒体区域探针：在一张 PNG 上找「这里有一张图」的区域
    ///
    /// `media.rs` 里的阈值多数标着「未实测」——**这个命令就是标定入口**。
    /// 拿到含媒体的真实帧跑一遍，屏幕上的数字就是更新阈值的依据。
    /// 纯文字帧预期输出「没有检出媒体区域」，那是防误判的基线。
    Media {
        /// 要检测的 PNG 路径
        file: PathBuf,
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
    /// 扫描仓库里是否残留 API key
    ///
    /// key 从环境变量 `CHATSERVANT_PROBE_KEY` 读，**不进命令行**
    /// （进命令行就会留在 shell 历史和进程列表里）。
    SecretScan,
    /// 实机验证注入：剪贴板逐字节往返 + 证明鼠标指针没动过
    ///
    /// **不碰微信**——只验剪贴板通路与「不动鼠标」这两件事。
    /// 真往输入框里打字需要目标窗口在前台，那是人工验证。
    InjectProbe,
    /// 端到端：抓帧 → 锚定 → OCR → 归类 → 推理 → 填入 → **回读验证**
    ///
    /// 需要**微信窗口开着**。失败时会指名道姓地说哪一段挂了。
    /// 默认 `--dry-run`：只跑到起草，不碰用户的输入框。
    E2e {
        /// 真的往输入框里填（会抢焦点）。不给就是只跑到起草。
        #[arg(long)]
        inject: bool,
        /// 只用这一家传输（openai / anthropic / minimax）。
        /// 不给就按产品同样的顺序，谁有 key 用谁。
        #[arg(long)]
        provider: Option<String>,
        /// 强制用**预置假传输**，即使机器上有 key。
        #[arg(long)]
        preset: bool,
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
        /// 捕获方式：wgc（默认，`wgc` crate）/ wgc1（windows-capture，本机已知失败，
        /// 仅供复核 ADR-0004）/ printwindow / region / monitor（抓整个主显示器）
        #[arg(long, default_value = "wgc")]
        method: String,
        /// 输出路径。默认写到 target/probe/frame-<时间戳>.png
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        // `wgc` 的 "Item closed, stopping capture" 是 debug!，info 级别看不到它。
        // 排查抓帧失败要靠这一行区分「item 被系统关掉」和「帧没来」，所以开 trace。
        .with_env_filter("xtask=debug,wgc=trace")
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
        Cmd::Focus => probe_focus("wechat"),
        Cmd::Read => probe_read(),
        Cmd::Fixture { name, source } => make_fixture(&name, source),
        Cmd::Perceive { file } => probe_perceive(file),
        Cmd::Ocr { file, scale } => probe_ocr(&file, scale),
        Cmd::Measure { file } => probe_measure(file),
        Cmd::Layout { file } => probe_layout(file),
        Cmd::Media { file } => media_probe::run(file),
        Cmd::SecretScan => secret_probe::run(),
        Cmd::InjectProbe => inject_probe::run().map_err(|e| anyhow::anyhow!("{e}")),
        Cmd::E2e {
            inject,
            provider,
            preset,
        } => e2e_cmd(inject, provider, preset),
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

/// 把目标窗口提到前台。**会抢焦点**——命令名就叫 focus，语义明确。
///
/// ## 为什么需要它
///
/// WGC 名义上能抓被遮挡的窗口，**但这台机器上实测不行**：
/// 窗口被别的东西盖住时，抓帧报「捕获会话在送来第一帧之前就关闭」。
/// 所以「能不能抓到」这件事和「窗口露没露出来」直接相关，
/// 抓不到时的第一条救急动作就是把它露出来。
///
/// 走的是 `inject::focus`——注入路径在用的同一套
/// `AttachThreadInput` + `SetForegroundWindow`。不在 PowerShell 里重写一遍：
/// 前台锁的细节（挂哪个线程、要不要先按 Alt）试过三种写法都失败，
/// 而这段 Rust 是**在生产里跑过**的。
fn probe_focus(app: &str) -> Result<()> {
    set_dpi_aware();
    let surface = surface_for(app)?;
    let hit = chatservant_capture::find_chat_window(&surface)
        .map_err(|e| anyhow::anyhow!("找窗口失败：{e}"))?;
    println!("目标 0x{:X}  {}", hit.hwnd_raw, hit.title);

    let hwnd = windows::Win32::Foundation::HWND(hit.hwnd_raw as *mut core::ffi::c_void);
    let focused = chatservant_capture::inject::focus(hwnd).is_ok();

    // ⚠ **判据是「遮挡有没有解除」，不是「有没有变成前台」。**
    //
    // 早先只用 SetForegroundWindow，然后拿「前台窗口是不是它」当结果，
    // 于是报了「未成功」。那是个**验错了指标的错误**：
    // `SWP_NOACTIVATE` 的 z-order 改动**本来就不激活窗口**，
    // 但它已经把窗口移到最上面、不再被遮挡——而 WGC 要的正是这个。
    // 前台只是顺带的副作用，判它会把「其实已经奏效」误报成失败。
    if !focused {
        // 前台锁拒绝时仍然能改 z-order：临时置顶再撤下，窗口留在最前。
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
        };
        const TOPMOST: windows::Win32::Foundation::HWND = HWND_TOPMOST;
        const NOTOPMOST: windows::Win32::Foundation::HWND = HWND_NOTOPMOST;
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        unsafe {
            let _ = SetWindowPos(hwnd, Some(TOPMOST), 0, 0, 0, 0, flags);
            std::thread::sleep(std::time::Duration::from_millis(200));
            let _ = SetWindowPos(hwnd, Some(NOTOPMOST), 0, 0, 0, 0, flags);
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(400));
    println!(
        "{} 已尝试提前台（激活={}）。复查遮挡：",
        hit.title,
        if focused {
            "成功"
        } else {
            "被前台锁拒绝"
        }
    );
    probe_occluded(app)
}

/// 遮挡检测体检。
///
/// 第 ③ 级（区域抓屏）读的是屏幕坐标处的像素，所以「这个窗口现在能不能
/// 抓」这件事必须**如实测出来**，不能靠"抓一次没报错"来判断——之前那次
/// 抓回的其实是被盖住窗口的像素，机制上完全成功，内容完全错误。
/// 端到端命令的实现。**默认 dry-run**：不碰用户的输入框。
///
/// 真实传输还没接（ticket 10 的 `Reasoner` 实现），所以这里用一个
/// **明确标注的预置传输**——它会返回可辨识的假候选。
/// 返回假候选时命令**必须失败**，不能让用户以为链路通了。
fn e2e_cmd(inject: bool, provider: Option<String>, preset: bool) -> Result<()> {
    use e2e::{Stage, run};

    println!(
        "== 端到端（{}）==",
        if inject {
            "会填入输入框"
        } else {
            "dry-run：只跑到起草"
        }
    );

    // 传输选择与产品**保持一致**：谁有 key 用谁，`--preset` 才强制用假的。
    //
    // 早先这里无条件用 `PresetReasoner`，注释还写着「真实传输还没接」——
    // 那条注释过期很久了。于是端到端命令**从来没有真的调过一次模型**，
    // 却看起来像验过了整条链。诊断命令用占位实现，等于没验。
    let mut reasoner: Box<dyn chatservant_reason::Reasoner + Send> = if preset {
        eprintln!("传输 = 预置（显式指定。候选是假的，不代表模型通了）");
        Box::new(e2e::PresetReasoner::new())
    } else {
        match real_reasoner(provider.as_deref())? {
            Some(r) => r,
            None => {
                eprintln!(
                    "传输 = 预置（没找到可用的 key）。候选是假的，不代表模型通了。\n\
                     \x20     要验真实模型：设好环境变量后重跑，或用 --provider 指定。"
                );
                Box::new(e2e::PresetReasoner::new())
            }
        }
    };

    let mut injector = ClipboardInjector;
    let mut back = ClipboardReadback;

    let out = run(&mut *reasoner, &mut injector, &mut back, inject)?;

    println!("✅ 全链通过，用时 {:?}", out.elapsed);
    println!(
        "  锚点     ({},{})-({},{})",
        out.anchor.chat.x0, out.anchor.chat.y0, out.anchor.chat.x1, out.anchor.chat.y1
    );
    println!(
        "  判定     {} 条消息 / {} 条归类",
        out.messages.len(),
        out.attributions.len()
    );
    println!(
        "  判断     {}{}",
        out.intent.summary(),
        if out.judged {
            ""
        } else {
            "  ← 盲拟，没读懂"
        }
    );
    println!("  候选     {} 条", out.candidates.len());
    for (i, c) in out.draft_view() {
        println!("    {}. {:>3.0}%  {}", i + 1, c.probability * 100.0, c.text);
    }

    // 进度清单：**只点亮真正走过的段**。
    // 全亮的清单在 dry-run 下是骗人的——第 7、8 段根本没跑。
    println!("  段位");
    for s in Stage::ALL {
        let done = out.passed.contains(&s);
        println!(
            "    [{}] {} {}",
            if done { "✓" } else { " " },
            s as u32,
            s.name()
        );
    }

    if out.injected_and_read_back {
        println!(
            "  剪贴板   与注入内容**字节级相等**（{} 字节，没被别的程序换走）",
            out.injected.len()
        );
        println!(
            "  输入框   像素回读：「{}」",
            if out.input_box_text.is_empty() {
                "（空）"
            } else {
                &out.input_box_text
            }
        );
    } else {
        println!("  回读     **没跑**（dry-run：没有注入，也就没有回读证据）");
    }
    Ok(())
}

/// 真的往目标窗口输入框里填。
struct ClipboardInjector;

impl e2e::Injector for ClipboardInjector {
    fn fill(&mut self, hwnd: isize, text: &str) -> Result<(), String> {
        let h = windows::Win32::Foundation::HWND(hwnd as *mut core::ffi::c_void);
        // inject::fill_input_box 内部会做剪贴板回读与焦点处理
        match chatservant_capture::inject::fill_input_box(text, h) {
            chatservant_core::dispatch::DeliveryReceipt::PossiblySent
            | chatservant_core::dispatch::DeliveryReceipt::Sent => Ok(()),
            chatservant_core::dispatch::DeliveryReceipt::Failed(e) => Err(e),
            other => Err(format!("投递状态 {other:?}——不能当作成功")),
        }
    }
}

/// 从系统剪贴板读回。
struct ClipboardReadback;

impl e2e::Readback for ClipboardReadback {
    fn read(&mut self) -> Result<String, String> {
        chatservant_capture::inject::read_clipboard_public()
    }
}

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
    // 第三个维度：cloaked。可见 + 未遮挡 + 被藏起来 —— 前两个都过
    // 而抓帧仍然失败时，就是它。
    let cloaked = chatservant_capture::region::is_cloaked(hit.hwnd_raw);
    println!("is_cloaked() 判定 = {cloaked}   （cloaked 的窗口不参与 DWM 合成，WGC 拿不到帧）");
    if cloaked {
        println!("   → 窗口很可能在**另一个虚拟桌面**上。切回那个桌面。");
    }
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

/// 只跑 OCR：不做锚定、不做归类，原样打印识别到的行。
///
/// 存在的理由是**分层定位**。`perceive` 把锚定、OCR、归类串在一起，
/// 「没读到对方消息」这个现象至少有两个完全不同的成因：
///
/// - 识别器根本没读出来（像素/字号/对比度问题）
/// - 读出来了但被判成了别的类（阈值问题）
///
/// 两者对应的修法完全不同，混在一起报「归类阈值失效」会把人引向错误方向。
/// 这个命令把归类那层剥掉，只回答第一个问题。
///
/// `--scale` 在 OCR **之前**最近邻放大，用来检验「字太小识别器不收」
/// 这类假设——放大必须是最近邻，插值会把字形边缘抹掉，掩盖真实问题。
fn probe_ocr(file: &Path, scale: u32) -> Result<()> {
    set_dpi_aware();
    let img = image::open(file)
        .with_context(|| format!("打开 PNG 失败：{}", file.display()))?
        .to_rgb8();
    let (w, h) = img.dimensions();

    let frame = if scale <= 1 {
        chatservant_capture::Frame::from_rgb(w, h, img.into_raw())
            .context("源图尺寸与缓冲区不匹配")?
    } else {
        let base = chatservant_capture::Frame::from_rgb(w, h, img.into_raw())
            .context("源图尺寸与缓冲区不匹配")?;
        base.upscaled_nearest(scale).context("放大后的尺寸溢出")?
    };

    println!("来源   = {}", file.display());
    println!(
        "尺寸   = {}x{}  放大 {}x -> {}x{}",
        w,
        h,
        scale,
        frame.width(),
        frame.height()
    );

    let t0 = std::time::Instant::now();
    let ocr = chatservant_capture::ocr::recognize(&frame).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!(
        "OCR {} ms  逐行 {} 行",
        t0.elapsed().as_millis(),
        ocr.lines.len()
    );

    for (i, l) in ocr.lines.iter().enumerate() {
        println!(
            "{i:>3} | y={:>4} h={:>2} | {}",
            l.rect.y0,
            l.height(),
            l.text
        );
    }
    println!("\n---- 整图 ----\n{}", ocr.text);
    Ok(())
}

/// 按产品的规则组装真实传输。**变量名规则来自 reason crate**，
/// 不在这里另抄一份——两处各抄一份就会漂移，而漂移出来的症状是
/// 「应用读不到 key 却没有任何提示」。
///
/// 指定了 `only` 就只看那一家；没指定就按产品同样的顺序探测。
fn real_reasoner(
    only: Option<&str>,
) -> Result<Option<Box<dyn chatservant_reason::Reasoner + Send>>> {
    use chatservant_reason::{HttpReasoner, Provider, UreqHttp};

    let order: Vec<Provider> = match only {
        Some(k) => vec![Provider::from_key(k).ok_or_else(|| {
            anyhow::anyhow!("未知的供应商 {k}（可选 openai / anthropic / minimax）")
        })?],
        None => vec![Provider::OpenAi, Provider::Anthropic, Provider::MiniMax],
    };

    for p in order {
        let Some(key) = key_from_env(p) else { continue };
        let (judge, draft) = p.models();
        println!(
            "传输 = {}（模型：判断 {} / 起草 {}）",
            p.key(),
            judge,
            draft
        );
        let http = UreqHttp::new(std::time::Duration::from_secs(60));
        return Ok(Some(Box::new(HttpReasoner::new(
            std::sync::Arc::new(http),
            p,
            key,
        ))));
    }
    Ok(None)
}

/// 从环境变量读 key。**只读变量名与长度，绝不打印值。**
fn key_from_env(p: chatservant_reason::Provider) -> Option<String> {
    for var in p.key_env_names() {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                // 登记给 redact：读取那一刻就是它进入本进程内存的那一刻。
                chatservant_reason::redact(&v);
                println!("  key 来源 = {}（长度 {}，值不显示）", var, v.len());
                return Some(v);
            }
        }
    }
    None
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
/// 生成脱敏后的回归夹具。
///
/// **真实帧绝不能直接入库**——里面有真实联系人名和消息正文。
/// 脱敏按**面积**保留布局色（底色/气泡/边框），把小面积高频色（文字/头像）
/// 吸附掉。不能用亮度阈值：我方气泡底色亮度 205，比任何白字都高。
///
/// 关键：**脱敏前后锚点与配色必须完全一致**，否则这张夹具锁不住几何，
/// 命令直接拒收。
fn make_fixture(name: &str, source: Option<PathBuf>) -> Result<()> {
    use chatservant_perceive::Palette;

    let dir = std::path::Path::new("crates/capture/fixtures");
    std::fs::create_dir_all(dir).ok();
    let out_png = dir.join(format!("{name}.png"));
    if out_png.exists() {
        bail!(
            "夹具 {} 已存在。覆盖会让回归基线悄悄变掉——确认要更新就手动删掉再跑。",
            out_png.display()
        );
    }

    let frame = match source {
        Some(p) => {
            let img = image::open(&p).context("打开源 PNG 失败")?.to_rgb8();
            let (w, h) = img.dimensions();
            println!("来源 = {}", p.display());
            chatservant_capture::Frame::from_rgb(w, h, img.into_raw())
                .context("源 PNG 尺寸与缓冲区不匹配")?
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

    let before_anchor = chatservant_capture::anchor::derive(&frame);
    let before_palette = before_anchor
        .as_ref()
        .ok()
        .and_then(|a| Palette::learn(&frame, a.chat));

    let (redacted, keep) = {
        let a = before_anchor.as_ref().ok();
        let p = before_palette.ok_or_else(|| anyhow::anyhow!("学不到配色，无法脱敏"))?;
        let a = a.ok_or_else(|| anyhow::anyhow!("锚点推不出来，无法确定要保留哪些颜色"))?;
        let keep = chatservant_perceive::keep_colors(
            &frame,
            a.chat,
            p.panel_bg,
            a.header_top,
            a.input_top,
        );
        println!("保留色（{} 个）：{:?}", keep.len(), keep);
        (chatservant_perceive::redact_with(&frame, &keep), keep)
    };

    // 产物调色板必须**封闭在保留色里**。这条守卫是血换来的：
    // 夹具脱敏连续出过五次泄漏，全都能被它一眼抓住——
    //   ① 文字变绿（退回全局最近保留色）
    //   ② 绿气泡上的正文是深色麻点（普查容差太松，深字被算成面板色）
    //   ③ 头像整块变绿（空块退回全局最近）
    //   ④ 与头像宫格同形的深色剪影（照片暗部恰好等于某个保留色）
    //   ⑤ 能看出字形的麻点（抗锯齿边缘落在表面容差内）
    // 只要有一个像素既不是保留色、也不在它的容差内，就是泄漏。
    let mut off_palette = 0usize;
    let mut first_off: Option<(u32, u32, [u8; 3])> = None;
    for y in 0..redacted.height() {
        for x in 0..redacted.width() {
            let px = redacted.pixel(x, y).expect("在范围内");
            if !keep
                .iter()
                .any(|k| chatservant_perceive::fixture::near_color(px, *k))
            {
                off_palette += 1;
                first_off.get_or_insert((x, y, px));
            }
        }
    }
    if let Some((x, y, px)) = first_off {
        bail!(
            "产物有 {off_palette} 个像素不在保留色内（首个 ({x},{y}) = rgb({},{},{})）——\
             泄漏了原始像素。保留色：{keep:?}",
            px[0],
            px[1],
            px[2]
        );
    }

    let after_anchor = chatservant_capture::anchor::derive(&redacted);
    let after_palette = after_anchor
        .as_ref()
        .ok()
        .and_then(|a| Palette::learn(&redacted, a.chat));

    match (&before_anchor, &after_anchor) {
        (Ok(a), Ok(b)) if a.chat == b.chat => {}
        (Ok(a), Ok(b)) => bail!(
            "脱敏改变了锚点：chat ({},{})-({},{}) → ({},{})-({},{})；面板左右 ({},{}) → ({},{})；header {} → {}；input {} → {}",
            a.chat.x0,
            a.chat.y0,
            a.chat.x1,
            a.chat.y1,
            b.chat.x0,
            b.chat.y0,
            b.chat.x1,
            b.chat.y1,
            a.panel.x0,
            a.panel.x1,
            b.panel.x0,
            b.panel.x1,
            a.header_top,
            b.header_top,
            a.input_top,
            b.input_top
        ),
        _ => bail!("脱敏前后有锚点推导失败（{before_anchor:?} / {after_anchor:?}）"),
    }
    if before_palette != after_palette {
        bail!("脱敏改变了配色（{before_palette:?} → {after_palette:?}），气泡判定会跟着漂");
    }
    let a = after_anchor.expect("上面已校验");
    let p = after_palette.expect("上面已校验");

    image::RgbImage::from_raw(
        redacted.width(),
        redacted.height(),
        redacted.as_bytes().to_vec(),
    )
    .context("构造脱敏图失败")?
    .save(&out_png)
    .context("写夹具 PNG 失败")?;

    // PNG 落盘**之后**才算 sha256：摘要必须覆盖最终写出的那些字节，
    // 覆盖编码前的像素是没意义的——换个编码器结果就变，那样夹具会
    // 因为无关的原因报「被改坏」。
    let digest = sha256_of(&out_png);

    let manifest = format!(
        "# 夹具 {name}\n\
         #\n\
         # 由 `cargo xtask fixture --name {name} --source <png>` 生成。\n\
         # 原始帧含真实联系人名与消息正文，**不入库**。\n\
         #\n\
         # 脱敏分两步：①按**面积**挑出布局色（面板/侧边栏/两侧气泡/选中行/边框线），\n\
         # 整帧切 8x8 的块，每块记下块内最多的布局色作为它的**底色**；\n\
         # ②渲染时，凡是与所在块底色不是同一片表面的像素，一律刷成块底色。\n\
         # 空块（头像/图片）向邻近块**传播**底色，不往全局找。\n\
         #\n\
         # 生成时会当场校验三件事，任何一条不成立就拒收、不写盘：\n\
         #   · 产物的每个像素都在保留色内（调色板封闭）\n\
         #   · 锚点 chat 矩形与脱敏前逐像素相同\n\
         #   · 学到的面板底色与脱敏前相同\n\
         # 更新基线需手动删掉 png 再重跑——不允许静默覆盖。\n\
         #\n\
         # 锁的是几何与配色，不锁 OCR 读出的文字内容（夹具里本来就没有文字）。\n         \n\
         width = {}\n\
         height = {}\n\
         chat_x0 = {}\n\
         chat_y0 = {}\n\
         chat_x1 = {}\n\
         chat_y1 = {}\n\
         panel_bg = [{}, {}, {}]\n\
         sha256 = \"{}\"\n",
        redacted.width(),
        redacted.height(),
        a.chat.x0,
        a.chat.y0,
        a.chat.x1,
        a.chat.y1,
        p.panel_bg[0],
        p.panel_bg[1],
        p.panel_bg[2],
        digest,
    );
    std::fs::write(dir.join(format!("{name}.toml")), manifest)?;

    println!();
    println!("✅ 夹具 {}", name);
    println!(
        "   锚点 ({}, {}) - ({}, {})",
        a.chat.x0, a.chat.y0, a.chat.x1, a.chat.y1
    );
    println!(
        "   配色 rgb({},{},{})",
        p.panel_bg[0], p.panel_bg[1], p.panel_bg[2]
    );
    println!("   sha256 {digest}");
    Ok(())
}

/// 文件字节的 sha256，小写十六进制。
fn sha256_of(path: &Path) -> String {
    use sha2::{Digest as _, Sha256};
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("读 {} 失败：{e}", path.display()));
    let mut h = Sha256::new();
    h.update(&bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 感知层探针：走**真正的生产接缝** `chatservant_perceive::read`，
/// 从一张帧一路跑到归并后的消息流。
///
/// 这里刻意不自己拼装各步骤——探针和生产走同一条路，
/// 否则「探针能跑」根本不能说明「产品能跑」。
fn probe_perceive(file: Option<PathBuf>) -> Result<()> {
    use chatservant_perceive::{NoiseClass, Speaker, SystemOcr, read};

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

    let r = read(&frame, &SystemOcr).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!(
        "   锚点 ({},{})-({},{})  配色 rgb({},{},{})",
        r.anchor.chat.x0,
        r.anchor.chat.y0,
        r.anchor.chat.x1,
        r.anchor.chat.y1,
        r.palette.panel_bg[0],
        r.palette.panel_bg[1],
        r.palette.panel_bg[2],
    );

    println!();
    println!(
        "{:<3} {:<7} {:<10} {:<30} 文字",
        "#", "说话人", "归类", "依据"
    );
    println!("{}", "-".repeat(110));
    for (i, a) in r.attributions.iter().enumerate() {
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
    // 会话身份：闸认的是**像素指纹**，不是这段文字。
    // 头部的**归类**（NoiseClass::Header）与头部的**文字读得对不对**
    // 是两件事：真实帧上分类是对的，OCR 却可能把「小号」读成别的。
    // 两边都打出来，才能看出闸到底认的是什么。
    match chatservant_perceive::conversation_name(&r.attributions) {
        Some(name) => println!("  OCR 读到的会话名：{name}"),
        None => println!("  OCR 读到的会话名：（没读出标题行）"),
    }
    // 指纹来自**锚点给出的固定头带**，与观察循环用的是同一块——
    // 不依赖 OCR 认得准，所以就算上面那串字是错的，闸照样认得出人。
    let band = chatservant_core::frame::Rect::new(
        r.anchor.chat.x0,
        r.anchor.header_top,
        r.anchor.chat.x0 + (r.anchor.chat.x1 - r.anchor.chat.x0).min(400),
        r.anchor.header_top + 40,
    );
    if let Some(pane) = frame.crop(r.anchor.chat)
        && let Some(p) = chatservant_perceive::identity::HeaderPrint::of(&pane, band)
    {
        println!(
            "  闸用的像素指纹：{} px（区域 {}x{}，阈值 {:.2}）",
            p.pixels(),
            band.width(),
            band.height(),
            chatservant_perceive::identity::SAME_THRESHOLD
        );
        println!("  ⚠ 认的是这块**像素**，不是上面那串文字。");
    }

    println!();
    println!("== 归并后的消息 ==");
    for m in &r.messages {
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
        // ⚠ **默认走 `wgc` crate 那条，不是 windows-capture。**
        //
        // `windows-capture` 2.0.1 在本机对所有窗口一��失败（会话送来
        // 第一帧之前就关闭），是 ADR-0004 里记着的已知问题。产品默认路径
        // 用的也是 `wgc` crate。诊断命令的默认值必须和产品一致——
        // 否则每次排查都在测一条永远走不通的路。
        // windows-capture 保留为显式的 `wgc1`，需要复核那个已知问题时再用。
        "wgc1" => {
            chatservant_capture::capture_one_frame(hit.hwnd_raw, size).context("WGC 抓帧失败")?
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
        _ => chatservant_capture::wgc_alt::capture_one_frame(hit.hwnd_raw)
            .context("WGC(wgc crate) 抓帧失败")?,
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
