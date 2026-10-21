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

/// 放大用的重采样方式。
///
/// **为什么单列一个类型**：最近邻与双线性在同一个 `--scale` 下**不可比**——
/// 一个只复制像素、一个造出源图里没有的颜色。混在一个整数开关里，
/// 「只换重采样方式」的对照实验就做不出来。
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Resample {
    /// 只复制像素，字形一字不差（诊断口径）。
    Nearest,
    /// 边缘按邻域插值（生产候选）。
    Bilinear,
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
    /// 会话闸对照：给两张帧各建一次头部指纹，打印相似度与闸的结论
    ///
    /// 回答的是「**闸**认不认得出换人了」，不是「会话名 OCR 读不读得出」。
    /// 这两条路不同——闸认像素、名字认 OCR——本票的根因正是把两者混成一件。
    GateCmp {
        /// 前一帧（PNG）
        a: PathBuf,
        /// 换会话后的后一帧（PNG）
        b: PathBuf,
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
        /// 放大用哪种重采样：`nearest`（诊断口径）或 `bilinear`（生产候选）
        ///
        /// **为什么要有这个开关**：最近邻是**诊断**口径（字形一字不差，
        /// 结论才站得住），而生产要的是**读对**。两者不是同一个开关的两档，
        /// 必须能并排跑同一帧、同一倍数、只换重采样方式。
        #[arg(long, default_value = "nearest")]
        resample: Resample,
        /// 只量 det 的框粒度与行组装，不打印识别文本
        ///
        /// **这是「识别成一堆单字框」的定位工具。** 最终文本分不出
        /// 是 det 的锅、rec 的锅、还是压根没走到 PP-OCR（回落 WinRT）——
        /// 2026-10-15 那次就是在错的层上调参数。框数/框高/框宽是唯一
        /// 能把层分开的量。
        #[arg(long)]
        stats: bool,
        /// 打印 det 概率图的一带，字符画。格式 `y0:y1` 或 `y0:y1:x0:x1`
        ///
        /// 坐标是**原图像素**。例：`--prob 280:320:120:230`
        ///
        /// 框统计只说得出「框比字矮」，答不出「概率图整体弱、
        /// 还是只削掉了边缘」——两者修法不同。看一眼图就分得清。
        #[arg(long)]
        prob: Option<String>,
        /// 跑几遍，把每一遍的耗时都打出来（默认 1）
        ///
        /// **为什么需要它**：单跑一次量的是**冷启动**——第一次调用要建推理
        /// 会话、按那个形状编译图（默认后端是 `ort`，不是 `tract`）。
        /// 而生产里 OCR 跑在**长驻进程**里，每一帧都是热调用：
        /// det 图按 `(h, w)` 缓存、rec 图按宽度分桶复用，
        /// 两者都**跨帧存活**（2026-10-18 起；此前 rec 缓存是每帧新建的，
        /// 所以「热调用」那一档其实每遍都在重编 rec 图——那条已修）。
        /// ⇒ 「一次跑要多久」和「稳态一帧要多久」是两个数，
        /// 而能不能踩进 `TICK = 1000ms` 看的是**后者**。
        /// 报告里必须写清报的是哪一段（冷/热），不然这个数字没法比较。
        #[arg(long, default_value_t = 1)]
        repeat: u32,
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
    /// 连抓 N 帧并**逐次打印进程线程数**——用来定位线程泄漏到底出在
    /// 「抓帧」这一步，还是出在别的地方。
    ///
    /// 起因：实机观察到 Tauri 应用 1300+ 线程、增速 9 个/秒。
    /// 循环每秒只抓 1 帧，对不上，说明泄漏源不在抓帧线程本身。
    /// 但这需要**没有 UI、没有模型、没有 OCR** 的隔离环境才能确认。
    ThreadProbe {
        /// 抓多少帧
        #[arg(long, default_value_t = 10)]
        n: u32,
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
        /// **声明「当前显示的是测试场景」，否则拒绝写盘。**
        ///
        /// ⚠ 2026-10-18 加。抓帧命令只认「窗口标题/类名匹配」，
        /// **它不知道你屏幕上开的是哪个会话**——实测踩过：
        /// 微信窗口在、没最小化、抓帧成功，而当时打开的**不是**测试会话，
        /// 帧里是真人聊天内容、联系人名和一条银行提醒。
        ///
        /// 所以这一步只能由人声明，机器负责**强制它存在**：
        /// 不传这个开关就报错退出、一个字节都不写。
        /// 这比「记得别抓错窗口」可靠——后者靠自觉，前者靠退出码。
        #[arg(long)]
        scene_confirmed: bool,
    },
}

/// 探针的日志过滤器。**必须有全局档位**——见 [`the_log_filter_has_a_global_level`]。
const LOG_FILTER: &str = "info,xtask=debug,wgc=trace";

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        // `wgc` 的 "Item closed, stopping capture" 是 debug!，info 级别看不到它。
        // 排查抓帧失败要靠这一行区分「item 被系统关掉」和「帧没来」，所以开 trace。
        //
        // ⚠ `info` 那段**不能省**（2026-10-18 补）。`EnvFilter` 没有全局档位时，
        // 未列出的 target 一律被丢掉，于是 `chatservant_capture` 的记录整条看不见——
        // 包括「PP-OCR 不可用、回落 Windows OCR」那一句。
        // 后果实测过一次：PP-OCR 每帧都失败回落，而 `xtask ocr` 打出来的是
        // WinRT 的逐字输出，看上去像「PP-OCR 把整张图切成了单字框」，
        // 排查方向整段跑偏——调了半天 det 参数，det 一次都没真跑起来。
        .with_env_filter(LOG_FILTER)
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
        Cmd::GateCmp { a, b } => gatecmp(&a, &b),
        Cmd::Ocr {
            file,
            scale,
            resample,
            stats,
            prob,
            repeat,
        } => {
            if let Some(spec) = prob {
                probe_ocr_prob(&file, scale, resample, &spec)
            } else if stats {
                probe_ocr_stats(&file, scale, resample)
            } else {
                probe_ocr(&file, scale, resample, repeat)
            }
        }
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
        Cmd::ThreadProbe { n } => probe_threads(n),
        Cmd::Probe { kind } => match kind {
            ProbeCmd::Frame {
                app,
                hwnd,
                method,
                out,
                scene_confirmed,
            } => probe_frame(&app, hwnd, &method, out, scene_confirmed),
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

    // **判据是「遮挡有没有解除」，不是「有没有变成前台」。**
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
/// `--scale` 在 OCR **之前**放大，用来检验「字太小识别器不收」这类假设。
/// 放大用哪种重采样由 `--resample` 选：默认最近邻（**诊断口径**：字形一字不差，
/// 所以「放大 N 倍还是乱码」能推导出关于**字号**的结论），
/// `bilinear` 是**生产候选**（边缘插值，字更像它本来的样子，
/// 但也造出了源图里没有的颜色 ⇒ 只能喂给 OCR，不能喂给按像素判定的东西）。
///
/// ⚠ 这两个口径**不是同一个开关的两档**：混成一个整数开关，
/// 「只换重采样方式」的对照实验就做不出来。
///
/// `--repeat N` 跑 N 遍并把**每一遍**的耗时都打出来。生产里 OCR 在长驻
/// 进程里跑，第一遍要建会话、按形状编译图，那是**冷启动**；
/// 能不能踩进 `TICK`（`watch.rs`，1000ms）看的是**后面那几遍**。
fn probe_ocr(file: &Path, scale: u32, resample: Resample, repeat: u32) -> Result<()> {
    set_dpi_aware();
    let (w, h) = file_png_dims(file)?;
    let frame = load_frame(file, scale, resample)?;

    println!("来源   = {}", file.display());
    println!(
        "尺寸   = {}x{}  放大 {}x ({}) -> {}x{}",
        w,
        h,
        scale,
        match resample {
            Resample::Nearest => "nearest",
            Resample::Bilinear => "bilinear",
        },
        frame.width(),
        frame.height()
    );

    // 跑 repeat 遍，全部结果按顺序留着，最后一遍的文本拿去打印。
    // 少于一遍就当一遍——参数写 0 不该让探针什么都不干。
    let rounds = repeat.max(1);
    let mut ocr = None;
    let mut ms: Vec<u128> = Vec::with_capacity(rounds as usize);
    for i in 1..=rounds {
        let t0 = std::time::Instant::now();
        let got =
            chatservant_capture::ocr::recognize(&frame).map_err(|e| anyhow::anyhow!("{e}"))?;
        let took = t0.elapsed().as_millis();
        ms.push(took);
        println!(
            "第 {i}/{rounds} 遍：OCR {took} ms  逐行 {} 行",
            got.lines.len()
        );
        ocr = Some(got);
    }
    let ocr = ocr.expect("rounds 至少 1，上面必然跑过一遍");

    if rounds > 1 {
        // 冷/热分开报。混在一起报一个平均数，看的人会以为那是热态。
        let cold = ms[0];
        let warm = &ms[1..];
        let mut sorted = warm.to_vec();
        sorted.sort_unstable();
        let mid = sorted[sorted.len() / 2];
        println!(
            "冷启动 {cold} ms   稳态 min/中位/max = {}/{mid}/{} ms   （{} 遍热调用）",
            sorted[0],
            sorted[sorted.len() - 1],
            warm.len()
        );
    }

    print_readability(&ocr);

    for (i, l) in ocr.lines.iter().enumerate() {
        // ⚠ **x 与宽度必须打出来**：只有 y 的话，
        // 分不清这一行落在**侧边栏**（会话列表）还是**消息区**——
        // 2026-10-21 那次量票 41 的 ③ 就是因为缺 x，
        // 把「联系人名」当成了「对方的消息」，白跑一轮对照。
        println!(
            "{i:>3} | x={:>4} w={:>4} y={:>4} h={:>2} | {}",
            l.rect.x0,
            l.rect.width(),
            l.rect.y0,
            l.height(),
            l.text
        );
        // **逐词间隙**。行文本里那个空格到底该不该有，只有间隙能回答：
        // 中文被逐字切开时「古 文 山」是假空格，而真正的空格间隙是字高的
        // 几倍。凭印象调阈值会来回摇摆——实测就是「B0」被插成「B 0」。
        // 打印出来直接看数。
        if l.words.len() > 1 {
            for pair in l.words.windows(2) {
                let gap = pair[1].rect.x0 - pair[0].rect.x1;
                let h = pair[0].rect.height().max(pair[1].rect.height());
                println!(
                    "      ·{:>12}│{:>12}│ 间隙 {:>3}px  字高 {:>3}  比值 {:.2}{}",
                    pair[0].text,
                    pair[1].text,
                    gap,
                    h,
                    gap as f32 / h.max(1) as f32,
                    if gap * 3 >= h {
                        "  ← 现在会插空格"
                    } else {
                        ""
                    }
                );
            }
        }
    }
    println!("\n---- 整图 ----\n{}", ocr.text);
    Ok(())
}

/// 「整图拼接出来的中文成不成句子」的判据。
///
/// ## 为什么要在意
///
/// 用户看到的症状是「识别结果像乱码」，但**乱码有两种完全不同的成因**，
/// 修法不一样：
///
/// - **字认错了** → rec 的问题（输入裁错、归一化错、形状错）
/// - **字都对，但每个字被空格拆开** → 行组装的问题。
///   中文逐字被插空格以后，一整句看着就是乱的，**而一个字都没错**
///
/// 所以只报「准确率」不够，得报**每行几个 token** 与**空格占比**。
/// 一句正常中文一行 8~15 个字；一行 1 个 token、或者空格占到字符的两成，
/// 就说明行组装把句子切碎了。
///
/// 两种引擎的 token 口径不同：**WinRT** 逐词给 `words`（中文会逐字切开），
/// **PP-OCR** 一个框就是一行、只给 `text`。所以有 `words` 就按它数，
/// 没有就按字符数。
fn print_readability(ocr: &chatservant_capture::ocr::OcrText) {
    let mut per_line: Vec<usize> = Vec::with_capacity(ocr.lines.len());
    let mut chars = 0usize;
    let mut spaces = 0usize;
    for l in &ocr.lines {
        per_line.push(if l.words.is_empty() {
            l.text.chars().filter(|c| !c.is_whitespace()).count()
        } else {
            l.words.len()
        });
        chars += l.text.chars().filter(|c| !c.is_whitespace()).count();
        spaces += l.text.chars().filter(|c| c.is_whitespace()).count();
    }
    let mut sorted: Vec<i32> = per_line.iter().map(|n| *n as i32).collect();
    sorted.sort_unstable();
    let single = per_line.iter().filter(|n| **n == 1).count();
    let space_ratio = if chars == 0 {
        0.0
    } else {
        spaces as f64 / chars as f64
    };
    println!(
        "\n---- 成句性 ----\n行 {} 行  非空白字符 {}  空白 {}  空格/字符 = {:.3}\n\
         每行 token 数 min/中位/max = {}/{}/{}  只有 1 个 token 的行 = {}/{}",
        ocr.lines.len(),
        chars,
        spaces,
        space_ratio,
        pct(&sorted, 0.0),
        pct(&sorted, 0.5),
        pct(&sorted, 1.0),
        single,
        ocr.lines.len()
    );
}

/// 只量 det 的框粒度与行组装，**不打印识别文本**。
///
/// ## 为什么要有它
///
/// 「识别结果是一堆单字框」这个现象至少有三种成因，分属三层：
///
/// - det 把一行切成了一个个单字框（框高 == 单字高，框数上百）
/// - det 框是对的，后面的**行组装**按间隙把它们又切开了
/// - **压根没走到 PP-OCR**，[`probe_ocr`] 打的是回落后的 WinRT 的输出
///
/// 只看最终文本这三者长得一模一样。2026-10-15 那次就是第三种，
/// 而当时在调 det 的后处理参数——白调。判据只能是**中间量**：
/// det 输入尺寸、概率图尺寸、二值化前景像素、连通域框数与框的宽高分布。
///
/// 印的是几何量，**不印识别文本**——`target/probe/` 下的帧可能含真实
/// 聊天内容，而这份输出是要给人看、也可能被贴进票据的。
fn probe_ocr_stats(file: &Path, scale: u32, resample: Resample) -> Result<()> {
    set_dpi_aware();
    let frame = load_frame(file, scale, resample)?;

    println!("来源   = {}", file.display());
    let (w, h) = file_png_dims(file)?;
    println!(
        "尺寸   = {}x{}  放大 {}x -> {}x{}",
        w,
        h,
        scale,
        frame.width(),
        frame.height()
    );

    let t0 = std::time::Instant::now();
    let s = chatservant_capture::ocr_pp_pipeline::det_stats(&frame)
        .map_err(|e| anyhow::anyhow!("det 诊断失败：{e}"))?;
    println!("det 推理 {} ms", t0.elapsed().as_millis());
    println!(
        "det 输入 = {}x{}（32 的倍数）  概率图 = {}x{}",
        s.det_input_w, s.det_input_h, s.prob_w, s.prob_h
    );
    let fg_ratio = s.fg_pixels as f64 / (s.prob_w * s.prob_h).max(1) as f64;
    println!(
        "前景像素 = {}（占概率图 {:.2}%，阈值 {}）",
        s.fg_pixels,
        fg_ratio * 100.0,
        chatservant_capture::ocr_pp_pipeline::DET_THRESH
    );

    println!("\n---- 合并前（连通域原始框）----");
    print_box_summary(s.raw_count, &s.raw_widths, &s.raw_heights);
    println!("\n---- 合并后（行）----");
    print_box_summary(s.merged_count, &s.line_widths, &s.line_heights);
    println!(
        "限流上限 {}  保留 {}  **丢弃 {} 行**（丢了 = 屏幕下半截没进 rec）",
        chatservant_capture::ocr_pp_pipeline::DET_MAX_BOXES,
        s.line_count,
        s.dropped_lines
    );

    println!("\n---- 逐框（合并后，按 y 排序，最多 60 个）----");
    for (i, r) in s.line_rects.iter().take(60).enumerate() {
        println!(
            "{i:>3} | x={:>4} y={:>4}  w={:>4} h={:>3}",
            r.x0,
            r.y0,
            r.width(),
            r.height()
        );
    }
    if s.line_rects.len() > 60 {
        println!("（还有 {} 个没印）", s.line_rects.len() - 60);
    }
    Ok(())
}

/// 把 det 概率图的一带打成字符画。
///
/// 纵轴是原图的 y（每一行都缩到 1 个字符行），横轴每 `step` 个像素取一个样。
/// 字符按概率分档，`#` 最亮。看得见「概率图到底盖住了多少字形高度」，
/// 而不只是「框有多高」。
fn probe_ocr_prob(file: &Path, scale: u32, resample: Resample, spec: &str) -> Result<()> {
    set_dpi_aware();
    let frame = load_frame(file, scale, resample)?;
    let parts: Vec<i32> = spec
        .split(':')
        .map(|p| p.parse::<i32>().context("--prob 的坐标必须是整数"))
        .collect::<Result<Vec<_>>>()?;
    let (y0, y1) = (parts[0], parts[1]);
    let (x0, x1) = if parts.len() >= 4 {
        (parts[2], parts[3])
    } else {
        (0, frame.width() as i32)
    };
    let (vals, pw, ph) = chatservant_capture::ocr_pp_pipeline::det_prob_map(&frame)
        .map_err(|e| anyhow::anyhow!("det 概率图跑不出来：{e}"))?;
    // 概率图与原图不同尺寸时要换算；本仓 det 是等比 1:1 附近，
    // 但换算写在这里，别让调用方默认「坐标通用」。
    let sy = ph as f64 / frame.height() as f64;
    let sx = pw as f64 / frame.width() as f64;
    println!(
        "概率图 {pw}x{ph}   原图 {}x{}",
        frame.width(),
        frame.height()
    );
    println!("打印 y={y0}..{y1}  x={x0}..{x1}（原图坐标）");
    println!("图例  ' ' <0.1  '.' <0.3(阈值以下)  ':' <0.5  '+' <0.7  '*' <0.9  '#' >=0.9\n");
    for y in y0..y1 {
        let py = ((y as f64) * sy) as usize;
        if py >= ph {
            continue;
        }
        let mut line = String::new();
        let mut x = x0;
        while x < x1 {
            let px = (((x as f64) * sx) as usize).min(pw - 1);
            let v = vals[py * pw + px];
            line.push(match v {
                v if v < 0.1 => ' ',
                v if v < 0.3 => '.',
                v if v < 0.5 => ':',
                v if v < 0.7 => '+',
                v if v < 0.9 => '*',
                _ => '#',
            });
            x += 2;
        }
        println!("y={y:>4} |{line}|");
    }
    Ok(())
}

/// 一组框的数量与宽高分位。
///
/// 分位数而不是均值：均值会被一两个巨大的框拉走，而这里要回答的是
/// 「典型的一行长什么样」。
fn print_box_summary(count: usize, widths: &[i32], heights: &[i32]) {
    let mut w: Vec<i32> = widths.to_vec();
    let mut h: Vec<i32> = heights.to_vec();
    w.sort_unstable();
    h.sort_unstable();
    println!(
        "框数 = {count}   宽 min/中位/max = {}/{}/{}   高 min/中位/max = {}/{}/{}",
        pct(&w, 0.0),
        pct(&w, 0.5),
        pct(&w, 1.0),
        pct(&h, 0.0),
        pct(&h, 0.5),
        pct(&h, 1.0),
    );
}

fn pct(sorted: &[i32], q: f64) -> i32 {
    if sorted.is_empty() {
        return 0;
    }
    let i = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

/// 读一张 PNG 成帧，`--scale` 先做放大。
///
/// **放大方式由 `--resample` 选**：默认最近邻（诊断口径，字形一字不差），
/// `bilinear` 是生产候选。做「只换重采样方式」的对照实验时，
/// 两个口径必须能并排跑同一帧、同一倍数。
fn load_frame(file: &Path, scale: u32, resample: Resample) -> Result<chatservant_capture::Frame> {
    let img = image::open(file)
        .with_context(|| format!("打开 PNG 失败：{}", file.display()))?
        .to_rgb8();
    let (w, h) = img.dimensions();
    let base = chatservant_capture::Frame::from_rgb(w, h, img.into_raw())
        .context("源图尺寸与缓冲区不匹配")?;
    if scale <= 1 {
        return Ok(base);
    }
    match resample {
        Resample::Nearest => base.upscaled_nearest(scale).context("放大后的尺寸溢出"),
        Resample::Bilinear => base.upscaled_bilinear(scale).context("放大后的尺寸溢出"),
    }
}

fn file_png_dims(file: &Path) -> Result<(u32, u32)> {
    let img = image::open(file)
        .with_context(|| format!("打开 PNG 失败：{}", file.display()))?
        .to_rgb8();
    Ok(img.dimensions())
}

/// 按产品的规则组装真实传输。**变量名规则来自 reason crate**，
/// 不在这里另抄一份——两处各抄一份就会漂移，而漂移出来的症状是
/// 「应用读不到 key 却没有任何提示」。
///
/// 指定了 `only` 就只看那一家；没指定就按产品同样的顺序探测。
fn real_reasoner(
    only: Option<&str>,
) -> Result<Option<Box<dyn chatservant_reason::Reasoner + Send>>> {
    use chatservant_reason::{HttpReasoner, Provider};

    let order: Vec<Provider> = match only {
        Some(k) => vec![Provider::from_key(k).ok_or_else(|| {
            anyhow::anyhow!("未知的供应商 {k}（可选 openai / anthropic / minimax）")
        })?],
        None => vec![Provider::OpenAi, Provider::Anthropic, Provider::MiniMax],
    };

    for p in order {
        let Some(key) = key_from_env(p) else { continue };
        println!(
            "传输 = {}（协议层 genai，模型：{}）",
            p.key(),
            p.default_model()
        );
        return Ok(Some(Box::new(HttpReasoner::new(p, key).map_err(|e| {
            anyhow::anyhow!("装不起 {} 的传输：{e}", p.key())
        })?)));
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

/// 连抓 N 帧，逐次打印**本进程线程数**。
///
/// ## 它回答什么
///
/// 实机观察到 Tauri 应用 1300+ 线程、增速 9 个/秒，而观察循环每秒只抓
/// 一帧——对不上。两种可能：抓帧本身泄漏，或者泄漏在 UI/模型那条路上。
/// 这个命令把其它全部拿掉（没有 webview、没有模型、没有 OCR），
/// **线程数还涨就是抓帧的锅**，不涨就别再往抓帧上找。
///
/// 线程数用 ToolHelp 快照数——Rust 稳定版没有「进程线程总数」的 API。
fn probe_threads(n: u32) -> Result<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };

    /// 数**本进程**当前的线程。
    ///
    /// `CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)` 的 pid 传 0 是
    /// **全系统**所有线程——本机轻松七千多。必须再按
    /// `th32OwnerProcessID` 过滤，否则会把整台机器的增长算到自己头上。
    fn threads() -> u32 {
        use windows::Win32::System::Threading::GetCurrentProcessId;
        unsafe {
            let me = GetCurrentProcessId();
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) else {
                return 0;
            };
            let mut e = THREADENTRY32 {
                dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                ..Default::default()
            };
            let mut n = 0u32;
            if Thread32First(snap, &mut e).is_ok() {
                loop {
                    if e.th32OwnerProcessID == me {
                        n += 1;
                    }
                    if Thread32Next(snap, &mut e).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
            n
        }
    }

    set_dpi_aware();
    let hit = chatservant_capture::find_chat_window(&WECHAT)
        .map_err(|e| anyhow::anyhow!("找窗口失败：{e}"))?;
    println!("窗口 = {}  0x{:X}", hit.title, hit.hwnd_raw);
    // 最小化的窗口 WGC 抓不到（Windows 根本不渲染它），先无激活还原。
    // 与 `probe frame` 同一套动作，别让「最小化」污染这个探针的结论。
    if chatservant_capture::restore_without_activating(hit.hwnd_raw) {
        println!("窗口原本是最小化状态：已无激活还原");
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
    println!("抓帧前线程数 = {}", threads());

    for i in 1..=n {
        let t0 = std::time::Instant::now();
        match chatservant_capture::wgc_alt::capture_one_frame(hit.hwnd_raw) {
            Ok(f) => println!(
                "第 {i} 帧 ok {}x{}  {}ms  线程数 {}",
                f.width(),
                f.height(),
                t0.elapsed().as_millis(),
                threads()
            ),
            Err(e) => println!("第 {i} 帧 失败：{e}  线程数 {}", threads()),
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    println!("抓完线程数 = {}", threads());
    Ok(())
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
    // `y` 这一列是 2026-10-14 加的。之前这张表只有「依据」没有「位置」，
    //   于是「为什么 `conversation_name` 返回 None」根本没法从输出里判断——
    //   `header_h` 的过滤是**按 y 做的**，不看位置就只能猜。
    //   坐标是**面板局部**（`r.anchor.chat` 裁出来的那块），不是窗口坐标。
    println!(
        "{:<3} {:<11} {:<7} {:<9} {:<30} 文字",
        "#", "y 范围", "说话人", "归类", "依据"
    );
    println!("{}", "-".repeat(122));
    for (i, a) in r.attributions.iter().enumerate() {
        println!(
            "{:<3} {:<11} {:<7} {:<9} {:<30} {}",
            i,
            format!("{}-{}", a.rect.y0, a.rect.y1),
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
    // 2026-10-14 票 31：这里原先打的是
    //   「会话名判据上界 header_h = 80」与「指纹带子 header_top..+40」，
    //   两个**写死的常量**，而且注释自己都承认「两者不是同一个带子」。
    //   现在头部位置由 `discover` 推出来，两者统一吃同一个推导结果。
    let discovered =
        chatservant_perceive::identity::HeaderPrint::discover(&frame, r.anchor.chat, 400);
    let head = discovered.as_ref().map(|(_, rect)| *rect);
    match head {
        Some(rect) => println!(
            "  推导出的头部行        y {} .. {}（高 {}，相对 chat.y0 +{}）",
            rect.y0,
            rect.y1,
            rect.height(),
            rect.y0 - r.anchor.chat.y0
        ),
        None => println!("  推导出的头部行        **推不出来**"),
    }
    println!();
    // 会话身份：闸认的是**像素指纹**，不是这段文字。
    // 头部的**归类**（NoiseClass::Header）与头部的**文字读得对不对**
    // 是两件事：真实帧上分类是对的，OCR 却可能把「小号」读成别的。
    // 两边都打出来，才能看出闸到底认的是什么。
    // 只认**推导出的头部行**里的 Header。判据见 `conversation_name` 的说明：
    // 实机上它曾把一个时间戳认成会话名。
    //
    // 上界从写死的 `HEADER_NAME_H_PX = 80` 改成 `discover` 推出来的行底。
    //   那个 80 是第三次修复留下的（票 30），同样属于「猜坐标」。
    let name_upper = head.map_or(r.anchor.chat.y0, |rect| rect.y1);
    match chatservant_perceive::conversation_name(&r.attributions, name_upper) {
        Some(name) => println!("  整帧归类读到的会话名：{name}"),
        None => println!("  整帧归类读到的会话名：（没读出标题行）"),
    }
    // ⚠⚠ **生产那条路**（窄带 OCR）——与产品**共用一份实现**
    // （`perceive::read_peer_name`）⇒ **这行才证明得了产品**。
    //
    // 2026-10-21 之前这里只有上一行（整帧归类），而产品走的是窄带 ⇒
    // 「探针读到名字」与「产品读不到名字」可以**同时为真**，
    // 而没有任何报错（票 77）。⇒ 两行都打，且**标明来源**。
    match head.and_then(|rect| chatservant_perceive::read_peer_name(&frame, rect, r.anchor.chat)) {
        Some(name) => println!("  生产路径读到的会话名：{name}"),
        None => println!("  生产路径读到的会话名：（这条路上没读出来）"),
    }

    // ── 会话名到底住在哪（2026-10-14 诊断，票 30）────────────────────
    //
    // ⚠⚠ **这段是「诊断」，不是产品那条路**（2026-10-21，票 77）：
    // 产品的读名字走 `perceive::read_peer_name`（上面那行「生产路径读到的会话名」）。
    // ⚠ 而且这段的**反色那一遍其实不是「反色」**：它把 2× 的视图**又放大一次**
    // ⇒ 那是 **4×**。所以「反色读到」的真实原因是**倍率**，不是颜色
    // （实测：原色在 3×/4× 也读得到，颜色根本不是变量）。
    // ⇒ 别把「反色有用」当结论。
    //
    // 2026-10-14 票 31：这段原先是票 30 的探索脚手架——「会话名住哪」，
    //   里面裁了**三个写死的矩形**（窗口标题栏 `0..header_top`、
    //   「消息区顶部 40px」、以及一份手抄的 `HEAD_BAND_H = 40`），
    //   外加一次反色对照。它们回答的是「猜的三个位置哪个对」，
    //   而那个问题**已经被 `discover` 回答掉了**：位置是推出来的，不是猜的。
    //
    //   现在改成一件还有意义的事：把**推导出的那一行**拿去 OCR，
    //   看深色主题下文字读不读得出来。位置交给 `discover`，
    //   这里只回答「这一行的字认不认得出」。
    if let Some(rect) = head {
        println!("  —— 对推导出的头部行做 OCR（原色 / 反色）——");
        println!(
            "     采样框：({},{})-({},{}) 高 {}",
            rect.x0,
            rect.y0,
            rect.x1,
            rect.y1,
            rect.height()
        );
        // OCR 对很窄的行常常返回 0 行，所以上下各留一点余量再送进去。
        let pad = 12;
        let view_rect = chatservant_core::frame::Rect::new(
            rect.x0,
            (rect.y0 - pad).max(0),
            rect.x1,
            rect.y1 + pad,
        );
        match frame
            .crop(view_rect)
            .and_then(|f| f.upscaled_nearest(chatservant_perceive::OCR_UPSCALE))
        {
            None => println!("     （裁不到）"),
            Some(view) => match chatservant_capture::ocr::recognize(&view) {
                Ok(ocr) => {
                    let texts: Vec<_> = ocr
                        .lines
                        .iter()
                        .map(|l| l.text.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect();
                    if texts.is_empty() {
                        println!(
                            "     原色读到：（空）——OCR 返回了 {} 行，全是空白",
                            ocr.lines.len()
                        );
                    } else {
                        for t in &texts {
                            println!("     原色读到：{t}");
                        }
                    }

                    // 深色主题下要不要反色：Windows OCR 在「浅底深字」上
                    //   表现好、在「深底浅字」上常常整段读不出来（票 30 实测）。
                    //   同一块反色后再 OCR 一次，直接对照。
                    let w = view.width();
                    let h = view.height();
                    let mut inv = Vec::with_capacity(view.as_bytes().len());
                    for px in view.as_bytes().chunks_exact(3) {
                        inv.push(255 - px[0]);
                        inv.push(255 - px[1]);
                        inv.push(255 - px[2]);
                    }
                    match chatservant_core::frame::Frame::from_rgb(w, h, inv)
                        .and_then(|f| f.upscaled_nearest(chatservant_perceive::OCR_UPSCALE))
                    {
                        None => println!("     反色帧构造失败"),
                        Some(view2) => match chatservant_capture::ocr::recognize(&view2) {
                            Ok(ocr2) => {
                                let t2: Vec<_> = ocr2
                                    .lines
                                    .iter()
                                    .map(|l| l.text.trim().to_string())
                                    .filter(|t| !t.is_empty())
                                    .collect();
                                if t2.is_empty() {
                                    println!("     反色读到：（空）——反色也没用");
                                } else {
                                    for t in &t2 {
                                        println!("     反色读到：{t}");
                                    }
                                }
                            }
                            Err(e) => println!("     反色 OCR 失败：{e}"),
                        },
                    }
                }
                Err(e) => println!("     OCR 失败：{e}"),
            },
        }
    }
    println!();
    // 指纹来自**像素推导出来的**头部行，与观察循环走同一条路
    // （`HeaderPrint::discover`），不依赖 OCR 认得准——
    // 所以就算上面那串字是错的，闸照样认得出人。
    //
    // 2026-10-14 票 31：原先这里**手拼**了
    //   `header_top..header_top+40` 的矩形，还在**已裁过**的 `pane` 上取，
    //   两处都错（坐标写死 + 坐标系不匹配）。现在复用上面已经算好的
    //   `discovered`（同一帧只推一次），与 `watch.rs` 的接缝完全一致。
    match discovered {
        Some((p, rect)) => {
            println!(
                "  闸用的像素指纹：{} px（推导出的头部行 {}x{}，位置 y={}，阈值 {:.2}）",
                p.pixels(),
                rect.width(),
                rect.height(),
                rect.y0,
                chatservant_perceive::identity::SAME_THRESHOLD
            );
            println!("  ⚠ 认的是这块**像素**，不是上面那串文字。");
        }
        None => println!(
            "  闸的像素指纹：**推不出来** ⇒ 按「认不出」处理（关闸、不起草）。\
             顶部 {} px 内没有成行的字。",
            72
        ),
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

/// 会话闸对照：两张帧 → 两个头部指纹 → `is_same` 的结论。
///
/// ## 为什么要有这个命令
///
/// 票 30 最后一条待验是「闸能否分开两个会话」。肉眼能看见 `peer` 变了，
/// 但那证明的是**会话名 OCR 通了**，不是**闸**分开了——闸认的是像素指纹，
/// 名字认的是 OCR，是两条不同的路。本票的根因正是把这两件事混成一件，
/// 所以这里必须有一个只问闸的命令。
///
/// ## 失败方向
///
/// 任何一张帧推不出指纹就整体报错退出，**不打印任何「相同/不同」的结论**。
/// 理由与生产一致：认不出必须按「换人了」处理（关闸），而探针若在认不出的
/// 情况下打印一个结论，那结论就是猜的——本项目踩过好几次这种坑。
fn gatecmp(a: &Path, b: &Path) -> Result<()> {
    use chatservant_perceive::identity::{HeaderPrint, SAME_THRESHOLD};

    // 与 `probe_perceive` 走同一条接缝：`anchor::derive` + `HeaderPrint::discover`，
    // 不自己拼矩形。探针和生产走同一条路，否则「探针说分开了」说明不了产品会分开。
    let print_of = |p: &Path| -> Result<(HeaderPrint, chatservant_core::frame::Rect)> {
        let img = image::open(p)
            .with_context(|| format!("打开 PNG 失败：{}", p.display()))?
            .to_rgb8();
        let (w, h) = img.dimensions();
        let frame = chatservant_capture::Frame::from_rgb(w, h, img.into_raw())
            .context("PNG 尺寸与缓冲区不匹配")?;
        let anchor = chatservant_capture::anchor::derive(&frame)
            .map_err(|e| anyhow::anyhow!("推导锚点失败：{e}"))?;
        let (print, rect) = HeaderPrint::discover(&frame, anchor.chat, 400).with_context(|| {
            format!(
                "{}：推不出头部指纹（锚点 chat.y0 = {}，顶部 72 px 内没有成行的字）\
                 ⇒ 闸按「认不出」处理，不能拿它比另一张帧",
                p.display(),
                anchor.chat.y0
            )
        })?;
        println!(
            "  {}  锚点 ({},{})-({},{})  头部行 y {}..{}（{}x{}）  指纹 {} px",
            p.display(),
            anchor.chat.x0,
            anchor.chat.y0,
            anchor.chat.x1,
            anchor.chat.y1,
            rect.y0,
            rect.y1,
            rect.width(),
            rect.height(),
            print.pixels()
        );
        Ok((print, rect))
    };

    println!("== 会话闸对照 ==");
    let (pa, ra) = print_of(a)?;
    println!();
    let (pb, rb) = print_of(b)?;

    let sim = pa.similarity(&pb);
    println!();
    println!("相似度 = {sim:.4}   阈值 = {SAME_THRESHOLD:.2}");
    if pa.is_same(&pb) {
        println!(
            "闸的结论：**同一个会话**（判据：{}x{} 与 {}x{} 的墨迹像到 {:.0}%）",
            ra.width(),
            ra.height(),
            rb.width(),
            rb.height(),
            sim * 100.0
        );
    } else {
        println!(
            "闸的结论：**换人了**（相似度 {:.4} 低于阈值 {SAME_THRESHOLD:.2} ⇒ 闸会关）",
            sim
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
    scene_confirmed: bool,
) -> Result<()> {
    // ⚠ 场景闸：**在枚举窗口之前**就拦下来，一个字节都不写。
    // 放最前面是故意的——这条命令唯一「不可逆」的动作就是把真人画面
    // 落到磁盘，而窗口枚举本身不产生副作用，没有理由排在它后面。
    //
    // 判据是「有没有显式声明」，不是「窗口像不像微信」：
    // 匹配标题/类名只能证明**程序**对，证明不了**内容**可以看。
    // 实测踩过——窗口在、没最小化、抓帧成功，而当时开着的不是测试会话。
    if !scene_confirmed {
        bail!(
            "拒绝抓帧：没带 `--scene-confirmed`。\n\
             \n\
             这条命令只认「窗口标题/类名匹配」，它**不知道你屏幕上开的是哪个会话**。\n\
             帧里若是真人聊天内容，它会把联系人名、聊天正文、银行提醒一类的东西\n\
             落到磁盘上——2026-10-18 实测踩过，帧删得掉，对话里读过的内容收不回。\n\
             \n\
             请先自己看一眼屏幕上是什么，确认是测试场景，再重跑并加这个开关。\n\
             确认完记得删掉帧：target\\probe\\ 下面的是可再生成的，别留。"
        );
    }
    // 混合 DPI 下窗口矩形要用物理像素，必须在枚举之前就声明 per-monitor v2，
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
        // **默认走 `wgc` crate 那条，不是 windows-capture。**
        //
        // `windows-capture` 2.0.1 在本机对所有窗口一律失败（会话送来
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

#[cfg(test)]
mod filter_tests {
    use super::LOG_FILTER;

    /// **过滤器必须带全局档位，否则 `chatservant_capture` 的记录全被丢掉。**
    ///
    /// `EnvFilter` 的规则是「没列出的 target 一律丢弃」，而不是「没列出的按默认档」。
    /// 写成 `"xtask=debug,wgc=trace"` 时，capture 里那句
    /// 「PP-OCR 不可用、回落 Windows OCR」**根本不会出现在输出里**——
    /// 于是探针打出来的是 WinRT 的逐字结果，看上去像 PP-OCR 把整张图切成了单字框。
    /// 这个误判实测过一次，调了半天 det 参数，det 一次都没真跑起来。
    ///
    /// 判据取「逗号前的第一段有没有 `=`」：没有 `=` 就是全局档位。
    /// 用结构判断而不是 `starts_with("info")`，这样换档位也不会失效。
    #[test]
    fn the_log_filter_has_a_global_level() {
        let first = LOG_FILTER.split(',').next().unwrap_or_default();
        assert!(
            !first.contains('='),
            "LOG_FILTER 的第一段 `{first}` 带 target 名 ⇒ 没有全局档位，\
             未列出的 target（含 chatservant_capture）会被整条丢掉"
        );
    }

    /// `scripts/run-app.ps1` 里那条 `RUST_LOG` **真的能解析**。
    ///
    /// ⚠ 为什么必须验「能不能解析」而不是只验字符串长相：
    /// `EnvFilter` 对一条写错的指令是**整条丢弃**，而那正好等于
    /// 「你什么日志都收不到」——它不报错，只是安静。
    /// 而这正是改之前那样子的情况：`info,chat_servant=debug`
    /// **一档都没命中**（实际 target 是 `chat_servant_app` 与两个显式 target）。
    #[test]
    fn the_run_app_log_filter_parses() {
        let filter = run_app_rust_log();
        tracing_subscriber::EnvFilter::try_new(&filter).unwrap_or_else(|e| {
            panic!("run-app.ps1 的 RUST_LOG 解析失败（整条被丢弃 ⇒ 什么日志都收不到）：{e}\n  内容：{filter}")
        });
    }

    /// **源码里每一个显式 `target:` 都必须在 `run-app.ps1` 的过滤器里被点名。**
    ///
    /// 跨文件的不变量：`watch.rs` 用 `chat-servant::watch`（连字符）、
    /// `commands.rs` 用 `chat_servant::pump`（下划线），两者不一致却都是
    /// 跑验收时**唯一可信的证据行**（tick/peer/candidates、emit readout phase）。
    /// 谁新增一个显式 target 而忘了同步诊断脚本 ⇒ 这条红。
    ///
    /// ⚠ 判据取**产物**（脚本里那行字），不是某份抄写出来的常量——
    /// 抄一份就等于把「守卫守的是原件」降级成「守的是副本」。
    #[test]
    fn every_explicit_tracing_target_is_named_in_the_run_app_filter() {
        let filter = run_app_rust_log();
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/src-tauri/src");
        let mut checked = 0usize;
        for entry in std::fs::read_dir(&src).expect("读 ui/src-tauri/src 失败") {
            let path = entry.expect("目录项读不出来").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("源码读不出来");
            for line in text.lines() {
                let Some(rest) = line.split_once("target: \"").map(|(_, r)| r) else {
                    continue;
                };
                let target = rest.split('"').next().unwrap_or_default();
                if target.is_empty() {
                    continue;
                }
                checked += 1;
                let named = filter.split(',').any(|d| {
                    let (t, _) = d.split_once('=').unwrap_or((d, ""));
                    t.trim() == target
                });
                assert!(
                    named,
                    "源码里有显式 tracing target `{target}`（{}），\
                     但 scripts/run-app.ps1 的 RUST_LOG 没点名它 ⇒ \
                     跑真机验收时这一路的证据行收不到。\n  过滤器：{filter}",
                    path.file_name().and_then(|f| f.to_str()).unwrap_or("?")
                );
            }
        }
        assert!(
            checked > 0,
            "一个显式 target 都没扫到 —— 扫描方式坏了，不是源码变了"
        );
    }

    /// 从 `scripts/run-app.ps1` 里取出 `RUST_LOG` 的值。
    fn run_app_rust_log() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/run-app.ps1");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("读 {} 失败：{e}", path.display()));
        text.lines()
            .filter_map(|l| l.split_once("$env:RUST_LOG"))
            .find_map(|(_, rest)| {
                rest.trim()
                    .trim_start_matches('=')
                    .trim()
                    .strip_prefix('\'')
                    .and_then(|s| s.split_once('\'').map(|(v, _)| v))
            })
            .unwrap_or_else(|| panic!("run-app.ps1 里找不到 `$env:RUST_LOG = '...'` 那一行"))
            .to_string()
    }
}
