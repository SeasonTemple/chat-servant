//! 感知层的行为测试。
//!
//! 全部用**合成帧**，因为合成帧能精确摆出「两侧气泡 / 灰字 / 时间戳 / 图内字」
//! 这些真实帧里凑不齐的组合。真实帧的回归由 `xtask perceive --file` 负责。

use chatservant_capture::ocr::{OcrLine, OcrWord};
use chatservant_core::frame::{Frame, Rect};

use crate::attribute::{
    HEADER_MIN_CONTRAST, NoiseClass, Palette, SATURATED_MIN_CHROMA, Speaker, classify_all,
};
use crate::merge::group;
use crate::pixels::{COLOR_TOL, PANEL_BG_TOL, Rgb, chroma, near};

const PANEL: [u8; 3] = [30, 30, 31];
/// 实测我方微信气泡色（`cargo xtask measure`）
const MY_BUBBLE: [u8; 3] = [53, 210, 141];
/// 实测对方（小号）微信气泡色 `rgb(47,47,48)`——**纯灰，彩度 0**
const THEIR_BUBBLE: [u8; 3] = [47, 47, 48];
/// 对方气泡里的字是**白字**（与面板上的标题同色），对比 178.4
const THEIR_INK: [u8; 3] = [225, 225, 229];
const DARK_INK: [u8; 3] = [22, 22, 22];
const GRAY_INK: [u8; 3] = [123, 123, 128];
const WHITE_INK: [u8; 3] = [225, 225, 229];

// ── 浅色主题实测色（2026-10-14，target/probe/frame-1791974144.png）──
// ⚠ 对方气泡与面板底色只差 12,12,10 —— 与 COLOR_TOL 同值，这是本组夹具的全部意义。
const LIGHT_PANEL: [u8; 3] = [250, 250, 250];
const LIGHT_MY_BUBBLE: [u8; 3] = [157, 242, 159];
const LIGHT_THEIR_BUBBLE: [u8; 3] = [238, 238, 240];
const LIGHT_INK: [u8; 3] = [25, 25, 26];

const W: u32 = 400;
const H: u32 = 300;

fn line(text: &str, r: Rect, color: [u8; 3]) -> OcrLine {
    let _ = color;
    OcrLine {
        text: text.into(),
        rect: r,
        words: vec![OcrWord {
            text: text.into(),
            rect: r,
        }],
    }
}

fn paint(px: &mut [u8], x: u32, y: u32, c: [u8; 3]) {
    let i = ((y * W + x) * 3) as usize;
    px[i..i + 3].copy_from_slice(&c);
}

fn fill(px: &mut [u8], x0: u32, y0: u32, x1: u32, y1: u32, c: [u8; 3]) {
    for y in y0..y1 {
        for x in x0..x1 {
            paint(px, x, y, c);
        }
    }
}

/// 两侧都有气泡的聊天区。布局照实测：面板底色铺满，
/// 我方绿气泡贴右、对方灰气泡贴左、中间留空。
fn two_sided_frame() -> Frame {
    let mut px = Vec::with_capacity((W * H * 3) as usize);
    for _ in 0..(W * H) {
        px.extend_from_slice(&PANEL);
    }
    fill(&mut px, 240, 40, 380, 300, MY_BUBBLE); // 我方气泡（够装两行字）
    fill(&mut px, 20, 120, 160, 150, THEIR_BUBBLE); // 对方气泡
    fill(&mut px, 180, 180, 230, 192, GRAY_INK); // 面板上的灰字
    fill(&mut px, 20, 15, 120, 27, WHITE_INK); // 面板上的标题

    // **气泡里必须有墨色**，否则 contrast = 0。
    //
    // 画成**稀疏条纹**而不是实心块：墨色若与文字框同高，会在「框外一圈」里
    // 压过气泡底色，探针就把墨色当成底色了。真实文字是稀疏笔画，
    // 合成时必须还原这个稀疏性，否则测出来的数字没有意义。
    stripes(&mut px, 255, 370, 48, DARK_INK); // 我方气泡第一行
    stripes(&mut px, 255, 370, 68, DARK_INK); // 我方气泡第二行
    stripes(&mut px, 35, 145, 128, THEIR_INK); // 对方气泡里的字（实测是白字）

    Frame::from_rgb(W, H, px).expect("frame")
}

/// 在 x 区间里画 3 条 2px 高的墨色条纹，模拟文字笔画的稀疏覆盖。
fn stripes(px: &mut [u8], x0: u32, x1: u32, y_top: u32, c: [u8; 3]) {
    for k in 0..3u32 {
        fill(px, x0, y_top + k * 4, x1, y_top + k * 4 + 2, c);
    }
}

/// 浅色主题的同一布局。版面照 `two_sided_frame`，颜色换成 2026-10-14 实测值。
fn light_frame() -> Frame {
    let mut px = Vec::with_capacity((W * H * 3) as usize);
    for _ in 0..(W * H) {
        px.extend_from_slice(&LIGHT_PANEL);
    }
    fill(&mut px, 240, 40, 380, 300, LIGHT_MY_BUBBLE);
    fill(&mut px, 20, 120, 160, 150, LIGHT_THEIR_BUBBLE);
    fill(&mut px, 180, 180, 230, 192, GRAY_INK); // 面板上的灰字
    fill(&mut px, 20, 15, 120, 27, LIGHT_INK); // 面板上的会话名

    stripes(&mut px, 255, 370, 48, LIGHT_INK); // 我方气泡第一行
    stripes(&mut px, 255, 370, 68, LIGHT_INK); // 我方气泡第二行
    stripes(&mut px, 35, 145, 128, LIGHT_INK); // 对方气泡里的字（实测也是深字）

    Frame::from_rgb(W, H, px).expect("frame")
}

fn chat_of(f: &Frame) -> Rect {
    Rect::new(0, 0, f.width() as i32, f.height() as i32)
}

#[test]
fn measured_colors_have_huge_chroma_gap() {
    // 从合成帧里读像素，而不是断言常量——既是运行时求值，
    // 也顺带证明 chroma() 对帧里的真实像素有效。
    let f = two_sided_frame();
    let mine = f.pixel(300, 42).expect("我方气泡内应有像素");
    let panel = f.pixel(200, 100).expect("面板上应有像素");
    assert_eq!(chroma(mine), 157, "实测我方气泡 rgb(53,210,141) 的彩度");
    assert_eq!(chroma(panel), 1, "实测面板底色 rgb(30,30,31) 的彩度");
    assert!(
        SATURATED_MIN_CHROMA > chroma(panel) && SATURATED_MIN_CHROMA < chroma(mine),
        "饱和阈值 {} 必须落在面板({})与我方气泡({})之间",
        SATURATED_MIN_CHROMA,
        chroma(panel),
        chroma(mine)
    );
}

/// 真机三档彩度钉死，且**自证会响**。
///
/// 数据来源：2026-10-15 `scripts/shotwin.ps1 0x1F080A` 抓真机微信帧，
/// 在消息区 `x 316..1256`（`anchor.rs` 实测的 `chat.x0`）做 1:1 调色板普查。
///
/// 为什么要单独钉：饱和阈值 `SATURATED_MIN_CHROMA` 的立足点是
/// 「我方气泡够饱和、其余都不够」。这个余量在代码里看不出来，
/// 只能靠把真机量到的三档数字钉死，**换主题/换微信版本时才会响**。
///
/// ⚠ 头像那档彩度 26 离阈值只有 4，是**已知的窄边**——它撞不上是因为
/// `probe.bg` 采的是文字框外一圈底色、采不到头像，真撞上还有 `RING_PURITY_MIN` 挡。
/// 把 26 显式写进来，是为了让「差 4」在数据漂移时能报警，而不是等它悄悄翻车。
#[test]
fn the_dark_theme_chroma_separation_the_threshold_rests_on_is_pinned() {
    /// 2026-10-15 真机实测。
    const MEASURED_MY_BUBBLE: [u8; 3] = [53, 210, 141]; // 我方气泡底色
    const MEASURED_PEER_BUBBLE: [u8; 3] = [47, 47, 48]; // 对方气泡底色
    const MEASURED_PANEL: [u8; 3] = [30, 30, 31]; // 面板底色
    const MEASURED_AVATAR: [u8; 3] = [247, 238, 221]; // 头像里的暖白，窄边

    assert_eq!(chroma(MEASURED_MY_BUBBLE), 157);
    assert_eq!(chroma(MEASURED_PEER_BUBBLE), 1);
    assert_eq!(chroma(MEASURED_PANEL), 1);
    assert_eq!(chroma(MEASURED_AVATAR), 26);

    // 阈值必须高于「非我方那一档里最高的」，低于我方。
    // 头像的 26 是当前最窄的一档——阈值只要不升到 27 以上就撞不上它。
    let highest_non_mine = chroma(MEASURED_PEER_BUBBLE)
        .max(chroma(MEASURED_PANEL))
        .max(chroma(MEASURED_AVATAR));
    assert!(
        SATURATED_MIN_CHROMA > highest_non_mine,
        "饱和阈值 {} 已经高过非我方那一档的最高彩度 {highest_non_mine}，\
         头像/对方气泡会被误判成我方",
        SATURATED_MIN_CHROMA
    );
    assert!(
        SATURATED_MIN_CHROMA < chroma(MEASURED_MY_BUBBLE),
        "饱和阈值 {} 已经高过我方气泡的真实彩度 {}，我方消息会被当成对方",
        SATURATED_MIN_CHROMA,
        chroma(MEASURED_MY_BUBBLE)
    );
}

#[test]
fn contrast_split_separates_header_from_gray_text() {
    // 实测面板上只有两档：灰字 93.5、标题 195.4。阈值必须在中间。
    let gray = measure_contrast_of(&two_sided_frame(), GRAY_INK);
    let header = measure_contrast_of(&two_sided_frame(), WHITE_INK);
    assert!(gray < HEADER_MIN_CONTRAST, "灰字({gray:.1})不该被判成标题");
    assert!(
        header > HEADER_MIN_CONTRAST,
        "标题({header:.1})不该被判成灰字"
    );
}

/// 在面板底色上量一段纯色块与面板底色的感知距离，模拟实测时的对比度读数。
fn measure_contrast_of(f: &Frame, ink: Rgb) -> f32 {
    // ink 越小 → 对比越大，与实测里「灰字 93.5 / 标题 195.4」的次序一致
    let px = f.pixel(200, 205).unwrap_or(PANEL);
    crate::pixels::dist(ink, px).max(crate::pixels::dist(ink, PANEL))
}

#[test]
fn palette_only_needs_panel_bg() {
    let f = two_sided_frame();
    let palette = Palette::learn(&f, chat_of(&f)).expect("应学得到配色");
    assert!(near(palette.panel_bg, PANEL), "面板底色应学成实测值");
}

#[test]
fn two_sided_frame_classifies_into_speakers() {
    let f = two_sided_frame();
    let palette = Palette::learn(&f, chat_of(&f)).expect("配色");

    let lines = vec![
        line("我说的", Rect::new(250, 50, 370, 64), DARK_INK),
        line("他说的", Rect::new(30, 130, 150, 144), THEIR_INK),
        line("昨天 20:00", Rect::new(190, 182, 222, 190), GRAY_INK),
        line("群公告", Rect::new(30, 17, 110, 25), WHITE_INK),
    ];
    let attrs = classify_all(&lines, &f, &palette);
    assert_eq!(attrs.len(), 4, "不该有行被丢掉");

    assert_eq!(attrs[0].speaker, Speaker::Me, "饱和气泡 = 我");
    assert_eq!(attrs[0].noise, NoiseClass::Text);

    assert_eq!(attrs[1].speaker, Speaker::Them, "近灰气泡 = 对方");
    assert_eq!(attrs[1].noise, NoiseClass::Text);

    assert_eq!(attrs[2].noise, NoiseClass::Gray, "时间戳是灰字");
    assert_eq!(attrs[3].noise, NoiseClass::Header, "白字大标题");
}

/// 浅色主题下对方气泡与面板底色逐通道只差 12,12,10 —— **恰好等于旧的
/// `COLOR_TOL = 12`**。那一步之差让对方的话被判成「标题」，于是它不会变成
/// 消息，系统等于认为**对方没说过话**。
///
/// **自证会响**：把 `classify` 里的 `is_panel_bg` 退回 `near`（容差 12）
/// ⇒ `12 <= 12` 成立 ⇒ 本条立刻红。
#[test]
fn a_peer_bubble_stays_a_bubble_even_when_it_is_nearly_the_panel_color() {
    let f = light_frame();
    let palette = Palette::learn(&f, chat_of(&f)).expect("配色");
    assert!(
        near(palette.panel_bg, LIGHT_PANEL),
        "先确认学到的确实是浅色面板底色，否则下面测的不是这条路径"
    );

    // 把「差多少」钉成数字：这组夹具的全部意义就是这两个数挨得极近。
    // ⚠ 断言必须在**运行时**求值，所以色值从帧里读，不直接比常量。
    let bubble_px = f.pixel(40, 125).expect("对方气泡内应有像素");
    let panel_px = f.pixel(200, 100).expect("面板上应有像素");
    let deltas = [
        (i32::from(bubble_px[0]) - i32::from(panel_px[0])).abs(),
        (i32::from(bubble_px[1]) - i32::from(panel_px[1])).abs(),
        (i32::from(bubble_px[2]) - i32::from(panel_px[2])).abs(),
    ];
    assert_eq!(
        deltas,
        [12, 12, 10],
        "浅色实测值变了就要回来改这条，别让夹具悄悄失去意义"
    );
    assert!(
        deltas.iter().all(|v| *v <= COLOR_TOL),
        "旧的宽松容差 {COLOR_TOL} 会把这组差值整个吃掉——本判据正是为此而存在"
    );
    assert!(
        deltas.iter().all(|v| *v > PANEL_BG_TOL),
        "收紧到 {PANEL_BG_TOL} 才分得开；这条红了说明容差被调回去了"
    );

    let lines = vec![
        line("我说的", Rect::new(250, 50, 370, 64), LIGHT_INK),
        line("他说的", Rect::new(30, 130, 150, 144), LIGHT_INK),
        line("昨天 20:00", Rect::new(190, 182, 222, 190), GRAY_INK),
        line("G8W", Rect::new(30, 17, 110, 25), LIGHT_INK),
    ];
    let attrs = classify_all(&lines, &f, &palette);
    assert_eq!(attrs.len(), 4, "不该有行被丢掉");

    assert_eq!(attrs[0].speaker, Speaker::Me, "饱和气泡 = 我");
    assert_eq!(attrs[0].noise, NoiseClass::Text);

    assert_eq!(
        attrs[1].speaker,
        Speaker::Them,
        "对方气泡底色与面板只差 12,12,10，仍必须判成对方说的话"
    );
    assert_eq!(
        attrs[1].noise,
        NoiseClass::Text,
        "判成 Header 就意味着它不会变成消息"
    );

    assert_eq!(
        attrs[2].noise,
        NoiseClass::Gray,
        "面板上的灰字仍然该是灰字——收紧容差不许把它带进气泡分支"
    );
    assert_eq!(
        attrs[3].noise,
        NoiseClass::Header,
        "会话名底色就是面板底色，仍该判成标题"
    );
}

#[test]
fn non_flat_region_is_image_text_not_a_bubble() {
    // 图片/头像区域颜色杂乱，外圈纯度低——不该当成气泡。
    let (w, h) = (200u32, 200u32);
    let mut px = Vec::with_capacity((w * h * 3) as usize);
    for i in 0..(w * h) as usize {
        px.extend_from_slice(&[(i % 251) as u8, (i % 253) as u8, (i % 247) as u8]);
    }
    let f = Frame::from_rgb(w, h, px).unwrap();
    let palette = Palette { panel_bg: PANEL };
    let lines = vec![line("图里的字", Rect::new(40, 90, 160, 110), DARK_INK)];
    let attrs = classify_all(&lines, &f, &palette);
    assert_eq!(
        attrs[0].noise,
        NoiseClass::ImageText,
        "杂乱区域应判为图内字"
    );
}

#[test]
fn every_attribution_carries_its_numbers() {
    let f = two_sided_frame();
    let palette = Palette::learn(&f, chat_of(&f)).expect("配色");
    let lines = vec![line("x", Rect::new(250, 45, 370, 65), DARK_INK)];
    let attrs = classify_all(&lines, &f, &palette);
    let ev = attrs[0].evidence;
    assert!(ev.contrast > 0.0);
    assert!(!ev.on_panel);
    // Display 不能因缺字段 panic——调试视图每帧都要渲染它
    let s = ev.to_string();
    assert!(s.contains("在气泡里"), "依据要能一眼看出在哪判的：{s}");
}

#[test]
fn consecutive_lines_from_same_speaker_merge() {
    let f = two_sided_frame();
    let palette = Palette::learn(&f, chat_of(&f)).expect("配色");
    let lines = vec![
        line("第一行", Rect::new(250, 50, 370, 64), DARK_INK),
        line("第二行", Rect::new(250, 70, 370, 84), DARK_INK),
    ];
    let msgs = group(&classify_all(&lines, &f, &palette));
    assert_eq!(msgs.len(), 1, "同一气泡的相邻两行应并成一条");
    assert!(msgs[0].text.contains("第一行") && msgs[0].text.contains("第二行"));
}

#[test]
fn merge_adds_space_between_latin_words_only() {
    // 实测踩到的：日文句子按行断在 "...Bloomer de" + "KOjin Lesson..."
    // 直接拼会得到 "deKOjin"。
    assert!(crate::merge::needs_space("Bloomer de", "KOjin"));
    // 中文按字断行，补空格反而错
    assert!(!crate::merge::needs_space("他是我的", "朋友"));
    // 只有一侧是拉丁字符时也不补
    assert!(!crate::merge::needs_space("中文结尾", "English"));
    assert!(!crate::merge::needs_space("English", "中文开头"));
}

#[test]
fn far_apart_lines_do_not_merge() {
    let f = two_sided_frame();
    let palette = Palette::learn(&f, chat_of(&f)).expect("配色");
    // 两条都是我的，但隔得远——必须是两条消息
    let lines = vec![
        line("上面那条", Rect::new(250, 50, 370, 64), DARK_INK),
        line("下面那条", Rect::new(250, 265, 370, 279), DARK_INK),
    ];
    let msgs = group(&classify_all(&lines, &f, &palette));
    assert_eq!(msgs.len(), 2, "间距够大就不该并");
}
