//! 真实微信帧的**脱敏夹具**黄金测试。
//!
//! ## 这些夹具是什么
//!
//! `crates/capture/fixtures/*.png` 是从**真实抓帧**脱敏而来的。
//! 原始帧里有真实联系人名和消息正文，**绝不入库**；脱敏后只保留布局几何
//! 与配色（面板底色、两侧气泡、选中行、边框线），文字/头像/图片全部刷掉。
//!
//! ## 锁什么、不锁什么
//!
//! - **锁几何**：`PaneAnchor` 推出来的 `chat` 矩形逐像素一致。
//! - **锁配色**：`Palette` 学到的面板底色一致——它决定「在不在气泡里」。
//! - **锁气泡几何**：两侧气泡各自的像素数与包围盒。这是三条归类判据
//!   （底色相等性 / 对比度 / 彩度）的**输入**，阈值一动这里就红。
//! - **不锁文字内容**：夹具里本来就没有文字，OCR 读不出任何东西。
//!
//! ## 为什么必须用夹具而不是原始帧
//!
//! 原始帧不入库，测试就只能在合成帧上跑——而合成帧的**假设**正是最容易被
//! 悄悄改错的东西（比如把气泡底色当成另一种灰）。夹具把真值固化下来。
//!
//! 更新基线：删掉对应 png 重新 `cargo xtask fixture`，然后把这里的期望值
//! 改成新值并**在提交信息里说清为什么**。
//!
//! ## 公开仓库里没有这些夹具
//!
//! 夹具是从**实机抓帧**脱敏而来的，而实机抓帧对应的是**用户自己的真实聊天**。
//! 即使脱敏后只剩布局几何与配色，来源仍然指向真实会话，因此**不入公开仓库**
//! （`.gitignore` 里 `crates/capture/fixtures/`）。代价是这些黄金测试在
//! 没有夹具的机器上**不跑**——所以它们一律「缺件即跳过」而不是失败：
//! 跳过要说清缺的是哪几个，别让 CI 的绿灯被读成「黄金基线验过了」。
//!
//! 本机要恢复它们：重跑
//! `cargo xtask fixture --name wechat-dark-2side --source target\probe\<你的帧>.png`
//! （脱敏与拒收校验都在那一步里，见 `.toml` 头部的说明）。

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use chatservant_capture::Frame;
use chatservant_capture::anchor;
use chatservant_core::frame::Rect;
use chatservant_perceive::Palette;
use image::GenericImageView;

type Rgb = [u8; 3];

/// 夹具的黄金基线。数值来自实机抓帧 + 脱敏后的产物。
struct Golden {
    name: &'static str,
    width: u32,
    height: u32,
    chat: Rect,
    panel_bg: Rgb,
    /// 消息区里三个**语义色**的精确像素数。
    ///
    /// 直接锁这三个而不是「出现最多的前 N 个」：前 N 会被图片马赛克之类的
    /// 布局色挤进来，测不到真正要锁的东西。
    pane_panel: usize,
    pane_me: usize,
    pane_them: usize,
}

/// 消息区里允许出现的**全部**布局色。
///
/// 就是脱敏时挑出来的那一套：面板底色 + 两侧气泡底色 + 边框线 +
/// 侧边栏的次级底色（聊天区左缘的抗锯齿边界会带进来几个像素）。
/// 任何不在这里、也不在 `COLOR_TOL` 内的颜色，都意味着有原始像素没被刷掉。
const LAYOUT_PALETTE: &[Rgb] = &[
    [30, 30, 31],    // 面板底色
    [114, 205, 144], // 我方气泡
    [47, 47, 48],    // 对方气泡（与侧边栏底色同色）
    [37, 37, 38],    // 侧边栏次级底色
    [59, 59, 61],    // 侧边栏三级底色
    [39, 39, 40],    // 输入框上边框
    [36, 36, 37],    // 消息区顶边
];

const TWO_SIDE: Golden = Golden {
    name: "wechat-dark-2side",
    width: 1262,
    height: 958,
    chat: Rect::new(304, 31, 1260, 683),
    panel_bg: [30, 30, 31],
    // 小号会话，两侧都有气泡。对方 77487 > 我方 28956——
    // **两侧气泡像素数会打平甚至反过来**，所以判「谁说的」只能用彩度，
    // 绝不能用「哪边气泡多」。这条数字就是防止有人又想起用面积判据的。
    pane_panel: 511_783,
    pane_me: 28_956,
    pane_them: 77_487,
};

const MY_SIDE: Golden = Golden {
    name: "wechat-dark-myside",
    width: 1262,
    height: 958,
    chat: Rect::new(304, 31, 1260, 683),
    panel_bg: [30, 30, 31],
    // G8W 会话只有我方气泡。`pane_them` 那 5509 px 来自聊天里的**图片**
    // （降采样后的马赛克），不是对方气泡——数量上完全不能混淆两者。
    pane_panel: 579_357,
    pane_me: 19_736,
    pane_them: 5_509,
};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../capture/fixtures")
}

/// 夹具在**公开仓库里不存在**（见模块文档），所以「没有」是一种正常状态，
/// 不是错误。缺件时返回 `None` 让调用方跳过，而不是 panic。
///
/// 先判 png 在不在，**再**去读 `.toml` manifest —— 两者同属被 gitignore 的目录，
/// 顺序反了会卡在「读 manifest 失败」上，丢掉跳过这条路径。
fn load(name: &str) -> Option<Frame> {
    let dir = fixture_dir();
    let path = dir.join(format!("{name}.png"));
    if !path.exists() {
        return None;
    }
    verify_digest(&dir, name, &path);
    let img = image::open(&path).unwrap_or_else(|e| panic!("读夹具 {} 失败：{e}", path.display()));
    let (w, h) = img.dimensions();
    Some(Frame::from_rgb(w, h, img.to_rgb8().into_raw()).expect("尺寸与缓冲区一致"))
}

/// 夹具的字节必须与 manifest 里记的 sha256 相符。
///
/// ## 为什么要有这道
///
/// 夹具是**像素级**的黄金基线。文件被改坏（截断、换工具重存、被 git
/// 的换行设置悄悄改写）时，黄金测试不会报「夹具坏了」，而是报
/// 「色种数 843 变成 812」——那是一条**看不出根因**的失败。
/// 这里直接报出**是哪一个夹具**、期望多少、实际多少。
fn verify_digest(dir: &Path, name: &str, png: &Path) {
    let manifest = std::fs::read_to_string(dir.join(format!("{name}.toml")))
        .unwrap_or_else(|e| panic!("读夹具 manifest {name}.toml 失败：{e}"));
    let want = manifest
        .lines()
        .find_map(|l| l.strip_prefix("sha256 = \"")?.strip_suffix('"'))
        .unwrap_or_else(|| {
            panic!("{name}.toml 里没有 sha256 字段——夹具无法自证没有被改坏");
        });
    let got = sha256_of(png);
    assert_eq!(
        got, want,
        "夹具 {name}.png 的 sha256 不符：\n  期望 {want}\n  实际 {got}\n\
         夹具是像素级黄金基线，被改坏会让黄金测试报出看不出根因的差异。"
    );
}

fn sha256_of(path: &Path) -> String {
    use sha2::{Digest as _, Sha256};
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("读 {} 失败：{e}", path.display()));
    let mut h = Sha256::new();
    h.update(&bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 区域里的精确色直方图，降序。
fn histogram(frame: &Frame, r: Rect) -> Vec<(Rgb, usize)> {
    let mut hist: HashMap<Rgb, usize> = HashMap::new();
    for y in (r.y0.max(0) as u32)..(r.y1.max(0) as u32).min(frame.height()) {
        for x in (r.x0.max(0) as u32)..(r.x1.max(0) as u32).min(frame.width()) {
            if let Some(p) = frame.pixel(x, y) {
                *hist.entry(p).or_insert(0) += 1;
            }
        }
    }
    let mut v: Vec<(Rgb, usize)> = hist.into_iter().collect();
    v.sort_by_key(|a| std::cmp::Reverse(a.1));
    v
}

fn check(g: &Golden) {
    // 夹具不在公开仓库里。缺件要说清是缺哪个，别让「跳过」在 CI 日志里
    // 只留下一行 test result: ok，被读成「黄金基线验过了」。
    let Some(frame) = load(g.name) else {
        eprintln!(
            "跳过 {}：夹具不在本机（公开仓库不含真实会话的脱敏帧）",
            g.name
        );
        return;
    };
    assert_eq!(
        (frame.width(), frame.height()),
        (g.width, g.height),
        "{}：夹具尺寸变了。重新生成夹具并更新基线",
        g.name
    );

    let a = anchor::derive(&frame).unwrap_or_else(|e| panic!("{}：锚点推导失败：{e}", g.name));
    assert_eq!(
        a.chat, g.chat,
        "{}：锚点漂了。这是最脆弱的阈值所在，改之前先想清楚",
        g.name
    );
    assert_eq!(a.panel_bg, g.panel_bg, "{}：面板底色变了", g.name);

    let palette =
        Palette::learn(&frame, a.chat).unwrap_or_else(|| panic!("{}：学不到配色", g.name));
    assert_eq!(
        palette.panel_bg, g.panel_bg,
        "{}：学到的面板底色变了——「在不在气泡里」这条判据会跟着漂",
        g.name
    );

    // 气泡几何：三条判据的**输入**。面板底色决定「在不在气泡里」，
    // 气泡底色决定「我方还是对方」，两者一动归类就会漂。
    let hist: HashMap<Rgb, usize> = histogram(&frame, g.chat).into_iter().collect();
    let count = |c: Rgb| hist.get(&c).copied().unwrap_or(0);
    let got = [
        ("面板底色", g.panel_bg, count(g.panel_bg), g.pane_panel),
        (
            "我方气泡",
            [114, 205, 144],
            count([114, 205, 144]),
            g.pane_me,
        ),
        ("对方气泡", [47, 47, 48], count([47, 47, 48]), g.pane_them),
    ];
    for (what, rgb, a, b) in got {
        assert_eq!(
            a, b,
            "{}：消息区里{what} rgb({:?}) 的像素数从 {b} 变成了 {a}",
            g.name, rgb
        );
    }
}

#[test]
fn two_sided_conversation_matches_golden() {
    check(&TWO_SIDE);
}

#[test]
fn my_sided_conversation_matches_golden() {
    check(&MY_SIDE);
}

/// 消息区里**不能出现任何布局色之外的像素**。
///
/// 「布局色」= 面板底色 + 两侧气泡底色。夹具里其余的几十种深浅灰都是
/// 这三者的抗锯齿过渡，按 `COLOR_TOL` 归拢。真实帧里光这一项就占消息区
/// 的 0.6%（978 px 的 `rgb(31,31,32)`），所以判据必须是容差而不是「精确相等」。
///
/// 这条与生成时的守卫是同一件事，但要在测试里再锁一次：生成是一次性动作，
/// 测试是每次提交都会跑的。
#[test]
fn fixture_pane_contains_no_pixel_outside_the_layout_palette() {
    for g in [&TWO_SIDE, &MY_SIDE] {
        let Some(frame) = load(g.name) else {
            eprintln!("跳过 {}：夹具不在本机", g.name);
            continue;
        };
        for (r, n) in histogram(&frame, g.chat) {
            assert!(
                LAYOUT_PALETTE
                    .iter()
                    .any(|k| chatservant_perceive::pixels::near(r, *k)),
                "{}：消息区里出现了布局色之外的 rgb({:?})（{n} px）——\
                 有原始像素没被刷掉",
                g.name,
                r
            );
        }
    }
}
