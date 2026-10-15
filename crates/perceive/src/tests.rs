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

// ── 票 41 的 ①：头像/图标被读成文字 ─────────────────────────────────
//
// 真机 2× 实测：30 行里有 7 行是被检测成文本框的头像与图标（`0` /
// `1毳06` / `14丐0` / `1400`）。它们本来会落进 `classify` 的气泡分支——
// 低彩度头像判成「对方说的」，彩度够高的头像判成「我说的」。
// 判成「我说的」尤其糟：面板上凭空多一句**用户自己没说过**的话。

/// 一块**花的**方块（头像/照片的替身）：坐标决定颜色，色种数上百。
fn noisy_block(px: &mut [u8], x0: u32, y0: u32, x1: u32, y1: u32) {
    for y in y0..y1 {
        for x in x0..x1 {
            let h = x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503);
            paint(
                px,
                x,
                y,
                [
                    (h & 0xff) as u8,
                    ((h >> 8) & 0xff) as u8,
                    ((h >> 16) & 0xff) as u8,
                ],
            );
        }
    }
}

/// 一张有头像、有对方气泡、有我方气泡的帧。
///
/// 头像放**左缘**：微信里对方头像在左、我方在右，而消息流的「谁说的」
/// 判据是底色彩度，与左右无关——所以头像放左边照样会污染消息流。
///
/// ⚠ 头像刻意做成「**一种主色占多数 + 少量散色**」：那样它的外圈众数占比
/// 仍够 `RING_PURITY_MIN` 的线，**旧闸放行**——票 41 的 ① 说的就是这一类。
/// 主色还特意挑了彩度 50（≥ `SATURATED_MIN_CHROMA = 30`），于是旧闸放行之后
/// 它会被判成**我方说的**。
fn frame_with_an_avatar() -> Frame {
    let mut px = Vec::with_capacity((W * H * 3) as usize);
    for _ in 0..(W * H) {
        px.extend_from_slice(&PANEL);
    }
    // 对方头像：36x36（微信头像的量级），~75% 主色 + 散色
    for y in 60u32..96 {
        for x in 16u32..52 {
            if (x * 40 + y) % 4 != 3 {
                paint(&mut px, x, y, [70, 80, 120]);
            } else {
                noisy_block(&mut px, x, y, x + 1, y + 1);
            }
        }
    }
    // 头像里被 OCR 读成「0」的那一坨
    fill(&mut px, 26, 68, 40, 84, [225, 228, 236]);
    // 对方气泡 + 白字
    fill(&mut px, 64, 60, 220, 96, THEIR_BUBBLE);
    stripes(&mut px, 74, 210, 70, THEIR_INK);
    // 我方气泡 + 深字
    fill(&mut px, 240, 180, 380, 216, MY_BUBBLE);
    stripes(&mut px, 250, 370, 190, DARK_INK);
    Frame::from_rgb(W, H, px).expect("frame")
}

/// **头像被读成文字时，绝不能变成消息。**
///
/// **自证会响**：把 `classify_in_frame` 里那段头像闸去掉 ⇒ 这一行外圈众数
/// 占比约 0.75，过得了 `RING_PURITY_MIN`；外圈底色 `rgb(70,80,120)` 彩度
/// 50 ≥ `SATURATED_MIN_CHROMA` ⇒ 判成 `Me` + `Text` ⇒ 进消息流 ⇒ 本条红。
#[test]
fn an_avatar_recognised_as_text_never_becomes_a_message() {
    let f = frame_with_an_avatar();
    let palette = Palette::learn(&f, chat_of(&f)).expect("配色");

    let avatar_box = Rect::new(26, 68, 40, 84);
    let probe = chatservant_perceive_probe(&f, avatar_box);
    assert!(
        probe.purity >= crate::attribute::RING_PURITY_MIN,
        "旧闸必须放行，否则这条守的不是新闸：purity={:.2}",
        probe.purity
    );
    assert!(
        chroma(probe.bg) >= SATURATED_MIN_CHROMA,
        "头像主色必须够饱和，旧闸放行之后它会被判成我方说的（实测彩度 {}）",
        chroma(probe.bg)
    );

    let lines = vec![
        // OCR 把头像里的一团形状读成了「0」
        line("0", avatar_box, THEIR_INK),
        // 对照：真的两句话
        line("他说的", Rect::new(70, 66, 214, 84), THEIR_INK),
        line("我说的", Rect::new(246, 186, 374, 204), DARK_INK),
    ];
    let attrs = classify_all(&lines, &f, &palette);
    assert_eq!(attrs.len(), 3, "不该有行被丢掉");

    assert_eq!(
        attrs[0].noise,
        NoiseClass::ImageText,
        "头像那一行必须判成图内字，依据：{}",
        attrs[0].evidence
    );
    assert_eq!(
        attrs[0].speaker,
        Speaker::System,
        "头像不是任何人说的话。判成 Me/Them 就是产品层面凭空造话"
    );
    assert_eq!(attrs[1].noise, NoiseClass::Text);
    assert_eq!(attrs[2].noise, NoiseClass::Text);

    let msgs = group(&attrs);
    assert_eq!(msgs.len(), 2, "头像那行混进了消息流：{msgs:?}");
    assert!(
        msgs.iter().all(|m| m.text != "0"),
        "头像进了消息流：{msgs:?}"
    );
}

/// **只有「框内填满」这一条能响**：头像背景是纯色 ⇒ 外圈只有 1 种色，
/// 色种数那条判据打不响；框内是一块实心图形 ⇒ 填满度 1.0。
///
/// 这条守的是「头像恰好是纯色/纯色块头像」那一类。
#[test]
fn a_flat_icon_is_caught_by_the_fill_rule_alone() {
    let mut px = Vec::with_capacity((W * H * 3) as usize);
    for _ in 0..(W * H) {
        px.extend_from_slice(&PANEL);
    }
    // 纯色头像：整块一个颜色
    fill(&mut px, 16, 60, 52, 96, [90, 95, 110]);
    // 里面一个实心图形
    fill(&mut px, 26, 68, 40, 84, [240, 240, 245]);
    let f = Frame::from_rgb(W, H, px).expect("frame");
    let palette = Palette { panel_bg: PANEL };

    let probe = chatservant_perceive_probe(&f, Rect::new(26, 68, 40, 84));
    assert_eq!(
        probe.ring_colors, 1,
        "这组夹具的外圈必须是 1 种色，否则色种数那条也响了，测的就不是它"
    );
    assert!(probe.fill >= 0.99, "实心图形的填满度应接近 1");

    let attrs = classify_all(
        &[line("0", Rect::new(26, 68, 40, 84), THEIR_INK)],
        &f,
        &palette,
    );
    assert_eq!(
        attrs[0].noise,
        NoiseClass::ImageText,
        "纯色头像里的实心图形不该被当成说话：{}",
        attrs[0].evidence
    );
}

/// **只有「外圈色种数」这一条能响**：外圈是「一种主色占 60% + 散色」的
/// 照片 ⇒ `purity = 0.6` 过得了 `RING_PURITY_MIN`（旧闸放行），
/// 但色种数上百；框内是稀疏笔画 ⇒ 填满度不到 0.4。
///
/// 这条守的是「头像里被识别出来的那一块本身不密」的情形，
/// 也正是票 41 说的「头像外圈众数占比常常还够线」。
#[test]
fn a_busy_surrounding_is_caught_by_the_ring_colour_rule_alone() {
    let mut px = Vec::with_capacity((W * H * 3) as usize);
    for _ in 0..(W * H) {
        px.extend_from_slice(&PANEL);
    }
    // 头像 = 80% 主色 + 20% 散色（purity 约 0.8，旧闸放行）
    for y in 60u32..96 {
        for x in 16u32..52 {
            if (x * 40 + y) % 5 < 4 {
                paint(&mut px, x, y, [70, 80, 120]);
            } else {
                noisy_block(&mut px, x, y, x + 1, y + 1);
            }
        }
    }
    // 框内只有稀疏笔画 ⇒ 填满度低
    fill(&mut px, 26, 70, 40, 71, [235, 235, 240]);
    fill(&mut px, 26, 74, 40, 75, [235, 235, 240]);
    fill(&mut px, 26, 78, 40, 79, [235, 235, 240]);
    let f = Frame::from_rgb(W, H, px).expect("frame");
    let palette = Palette { panel_bg: PANEL };

    let probe = chatservant_perceive_probe(&f, Rect::new(26, 68, 40, 84));
    assert!(
        probe.purity >= crate::attribute::RING_PURITY_MIN,
        "旧闸必须放行，否则测的不是「旧闸漏掉的那一类」：purity={:.2}",
        probe.purity
    );
    assert!(
        probe.ring_colors >= crate::media::MEDIA_MIN_COLORS,
        "外圈应当是花的，实测只有 {} 种色",
        probe.ring_colors
    );
    assert!(
        probe.fill < crate::attribute::ICON_FILL_MIN,
        "框内是稀疏笔画，填满度不该够阈值，实测 {:.2}",
        probe.fill
    );
    assert!(
        chroma(probe.bg) >= SATURATED_MIN_CHROMA,
        "主色刻意选成够饱和的：旧闸放行之后它会被判成**我方说的**（实测彩度 {}）",
        chroma(probe.bg)
    );

    let attrs = classify_all(
        &[line("0", Rect::new(26, 68, 40, 84), THEIR_INK)],
        &f,
        &palette,
    );
    assert_eq!(
        attrs[0].noise,
        NoiseClass::ImageText,
        "外圈是照片时不该被当成说话：{}",
        attrs[0].evidence
    );
    assert_eq!(attrs[0].speaker, Speaker::System);
}

/// 造一个「真正文」的探针读数（与判据同口径的 `RING_PAD`）。
fn chatservant_perceive_probe(f: &Frame, r: Rect) -> crate::pixels::BubbleHit {
    crate::pixels::probe_bubble_bg(f, r, 6).expect("应能读到像素")
}

/// **真实正文两个信号都必须低于阈值**——否则头像闸会误伤对话。
///
/// ⚠ 只查**气泡里那两行**：合成帧里面板上的灰字/标题是**实心色块**
/// （`fill` 画出来的，见 `two_sided_frame`），它的填满度恒为 1.0。
/// 那是夹具的画法，不是真实文字的样子——真实字是稀疏笔画，
/// 两行气泡里的 `stripes` 才是同量级的替身。
/// 灰字/标题那两行只查外圈色种数：它们本来就判成噪声，头像闸不参与，
/// 但外圈仍必须干净，否则阈值调低就会伤到面板上的字。
///
/// **自证会响**：把两个阈值调低（例如 `ICON_FILL_MIN = 0.3`），
/// 本条立刻红。
#[test]
fn real_text_lines_stay_below_both_icon_thresholds() {
    let f = two_sided_frame();
    let palette = Palette::learn(&f, chat_of(&f)).expect("配色");
    // 画成稀疏笔画的两行真正文
    let text_lines = vec![
        line("我说的", Rect::new(250, 50, 370, 64), DARK_INK),
        line("他说的", Rect::new(30, 130, 150, 144), THEIR_INK),
    ];
    // 面板上的灰字/标题：夹具里是实心块，只查外圈
    let panel_lines = [
        line("昨天 20:00", Rect::new(190, 182, 222, 190), GRAY_INK),
        line("群公告", Rect::new(30, 17, 110, 25), WHITE_INK),
    ];
    for l in text_lines.iter().chain(panel_lines.iter()) {
        let probe = chatservant_perceive_probe(&f, l.rect);
        assert!(
            probe.ring_colors < crate::media::MEDIA_MIN_COLORS,
            "{:?} 的外圈有 {} 种色，已经够头像阈值，正文会被误伤",
            l.text,
            probe.ring_colors
        );
    }
    for l in &text_lines {
        let probe = chatservant_perceive_probe(&f, l.rect);
        assert!(
            probe.fill < crate::attribute::ICON_FILL_MIN,
            "{:?} 的填满度 {:.2} 已经够头像阈值，正文会被误伤",
            l.text,
            probe.fill
        );
        // 顺带把「合成帧里真正文的填满度」钉成一个数：**实测 0.27**。
        // 它比实测过的最密汉字（12px 的「器」0.644）低一倍多，
        // 阈值 0.75 落在两者之外。夹具的笔画稀疏度一变（`stripes` 的条数
        // 或粗细），这个数就会动——那时要回来重标 `ICON_FILL_MIN`。
        assert!(
            probe.fill < 0.30,
            "合成帧里真正文的填满度实测 {:.2}，与上面写下的 0.27 不符；\
             阈值余量的依据变了，先重标 ICON_FILL_MIN 再改这里",
            probe.fill
        );
    }
    let attrs = classify_all(&text_lines, &f, &palette);
    assert_eq!(
        attrs.iter().filter(|a| a.noise == NoiseClass::Text).count(),
        2,
        "两句话都该还是正文：{attrs:?}"
    );
}

/// **真实布局夹具上的实测余量**：真的微信气泡里，一个「可能的文字框」
/// 外圈的色种数远低于 [`crate::media::MEDIA_MIN_COLORS`]。
///
/// 2026-10-15 本机在两份真实帧脱敏夹具上量过（沿气泡横向每 8px 摆一个
/// 100x12 的文字框）：
///
/// ```text
///                      采样数        外圈色种 min/中位/max
/// 我方绿气泡 2side      2381          1 / 2 / 5
/// 我方绿气泡 myside      532          1 / 2 / 5
/// 对方灰气泡 2side      7594          1 / 2 / 8
/// ```
///
/// 阈值 24 落在实测最大值 8 的 3 倍以外。
///
/// ⚠ 只查**气泡色**的段：面板底色上的行（时间戳、标题）压根不会判成
/// `Text`，头像闸不参与；而其中一段 `rgb(47,47,48)` 在 `myside` 里
/// 是聊天里的**照片**（黄金测试里记着它的 5509 px），不是气泡。
///
/// 夹具不在公开仓库里，所以**缺件即跳过**而不是失败。
#[test]
fn a_real_bubble_never_has_a_busy_ring() {
    use std::path::Path;

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../capture/fixtures");
    let mut sampled_total = 0usize;
    for name in ["wechat-dark-2side", "wechat-dark-myside"] {
        let path = dir.join(format!("{name}.png"));
        if !path.exists() {
            eprintln!("跳过 {name}：夹具不在本机（公开仓库不含真实会话的脱敏帧）");
            continue;
        }
        let img = image::open(&path).unwrap_or_else(|e| panic!("读夹具 {name} 失败：{e}"));
        let (w, h) = image::GenericImageView::dimensions(&img);
        let f = Frame::from_rgb(w, h, img.into_rgb8().into_raw()).expect("尺寸与缓冲区一致");
        let chat = Rect::new(304, 31, 1260, 683);

        // 我方气泡色（两份夹具都是它；灰色那一档在 myside 里是照片，见函数文档）
        let my_bubble: Rgb = [114, 205, 144];
        let mut worst = 0usize;
        let mut y = chat.y0;
        while y < chat.y1 {
            let mut x = chat.x0;
            while x < chat.x1 {
                if f.pixel(x as u32, y as u32) != Some(my_bubble) {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < chat.x1 && f.pixel(x as u32, y as u32) == Some(my_bubble) {
                    x += 1;
                }
                let mut bx = start + 6;
                while bx + 100 <= x - 2 {
                    let probe =
                        chatservant_perceive_probe(&f, Rect::new(bx, y - 6, bx + 100, y + 6));
                    worst = worst.max(probe.ring_colors);
                    bx += 8;
                }
            }
            y += 1;
        }
        sampled_total += 1;
        assert!(
            worst < crate::media::MEDIA_MIN_COLORS,
            "{name}：气泡里一个可能的文字框，外圈最多有 {worst} 种色，\
             已经够头像阈值（{}）——真正文会被误伤",
            crate::media::MEDIA_MIN_COLORS
        );
    }
    if sampled_total == 0 {
        eprintln!("跳过：两份夹具都不在本机");
    }
}
