//! 增量识别——静默期零调用，新消息只读新增带。
//!
//! ## 这条硬约束是什么
//!
//! **没有新的对方消息时，一次 OCR 都不跑。** 十分钟没人说话就是十分钟
//! 零调用。这不是性能优化，是产品定义：它挂在你自己的聊天窗口旁边，
//! 不能因为你没在聊天就一直在烧机器。
//!
//! ## 做法
//!
//! 维护一条**已提交水位线**。新消息永远从底部出现，所以识别范围永远是
//! 「水位线以下、忽略区以下」的那一条新增带。
//!
//! 每来一帧问两个问题：
//!
//! 1. 整块消息区跟上一帧一模一样吗？→ [`Decision::Quiet`]，一个像素都不读。
//! 2. 不一样的话，**水位线以下**那一段也一样吗？→ 一样就说明变的全在
//!    水位线以上（滚出旧消息、顶部公告），仍然是 [`Decision::Quiet`]。
//!
//! ## 为什么必须排除顶部
//!
//! 置顶公告条会自己滚。把它算进 diff，画面就**永远**判定为变化，
//! 零调用这条约束当场作废。跳过多高的区域由 [`ChatSurface`] 给——
//! 那是「接一个新软件」要提供的东西之一，不该写死在引擎里。
//!
//! ## 水位线什么时候动
//!
//! **识别成功之后**，由调用方显式 [`PaneWatcher::commit`]。
//! 失败就不推进——下一帧会重试，而不是永久丢掉一条消息。
//!
//! ## 这层管不了的事
//!
//! **滚动翻出旧消息不算新消息**——像素上它和「来了新消息」一样都是内容上移，
//! 靠对比分不开。真正的判据是「这条消息是不是已经上报过」，
//! 那是会话层的 `Novelty`。这一层保证的是它的**必要前提**：
//! 识别范围永远在水位线以下，已经读过的那一段不会被重读。
//!
//! [`ChatSurface`]: ../../../../CONTEXT.md

use chatservant_core::frame::{Frame, Rect};

/// 一帧消息区看完之后，决定要不要读。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// 没变：一行都不读。这次观察的 OCR 调用数必须是 0。
    Quiet,
    /// 变了：只读这一个矩形。**它永远在水位线以下。**
    Read {
        /// 要读的增量带（上沿即已提交水位线，下沿到消息区底部）。
        ///
        /// 给整条矩形而不是「从 y 到 y」的偏移，是为了让调用方不必
        /// 再知道水位线是多少——那条常量只由 [`PaneWatcher`] 管。
        ///
        /// ## 坐标系是**消息区局部坐标**，不是整帧坐标
        ///
        /// `observe` 收的是**裁过的消息区**（`frame.crop(anchor.chat)`），
        /// 所以这里的 `x0` **恒为 0**、`x1` 恒等于消息区宽度。
        /// 要换回整帧坐标必须**加上** `chat.x0 / chat.y0`。
        ///
        /// 2026-10-15 实测（真机微信，量法见 `capture::anchor` 模块头）：
        /// 帧里 `chat.x0 = 316`、消息区 940×637，日志打的是
        /// `roi=Rect { x0: 0, y0: 171, x1: 940, y1: 637 }`。
        /// ⇒ **`x0: 0` 是正常的，不是锚点塌了**；票 36 就是把它读成了
        /// 整帧坐标才判成缺陷。判据是 `x1`：真塌了 `x1` 会宽到接近帧宽。
        roi: Rect,
    },
}

/// 消息区的增量观察器。
///
/// **一个会话一个实例。** 切走再切回同一个会话要用**同一个**实例——
/// 水位线是「这个会话读到哪了」的凭证，重建实例等于把读过的消息全忘掉，
/// 用户会看到重复上报。
#[derive(Debug)]
pub struct PaneWatcher {
    /// 顶部跳过多高。置顶公告条会自己滚，算进 diff 会让画面永远判定为变化。
    ignore_top: i32,
    /// 已提交水位线（含）。以上的内容都读过了。
    watermark: i32,
    /// 上一次要读、但**还没读到结果**的矩形。
    ///
    /// 有它是为了失败重试：没有它的话，「画面跟上一帧一模一样」会把重试
    /// 吞掉，那条消息就**永久丢了**——漏一条比多跑一次 OCR 严重得多。
    pending: Option<Rect>,
    /// 上一帧消息区。用来对比。
    last: Option<Frame>,
    /// 观察以来返回过几次 `Read`。回放验证「零调用」靠它。
    calls: u32,
}

impl PaneWatcher {
    /// `ignore_top` 是消息区顶部要跳过的像素数，由 `ChatSurface` 给。
    pub fn new(ignore_top: i32) -> Self {
        Self {
            ignore_top,
            watermark: 0,
            pending: None,
            last: None,
            calls: 0,
        }
    }

    /// 已提交水位线，即「这个会话读到哪了」。
    ///
    /// 增量 ROI 从**它本身那一行**开始（`top.max(watermark)`），不是从下一行，
    /// 所以边界那一行会被重扫一次——宁可重扫，不可漏掉贴着水位线长出来的消息。
    ///
    /// 只认 [`PaneWatcher::commit`] 显式推进过的值：识别失败不推进，
    /// 下一帧才有机会重试——漏一条消息比多跑一次 OCR 严重得多。
    pub fn watermark(&self) -> i32 {
        self.watermark
    }

    /// 观察以来返回过几次「要读了」。静默期必须是 0。
    pub fn calls(&self) -> u32 {
        self.calls
    }

    /// 看一帧，决定这次要不要读。
    ///
    /// `pane` 是**裁过的消息区**，因此返回的 [`Decision::Read`] 里的
    /// 矩形是消息区局部坐标（`x0` 恒为 0）。
    ///
    /// 有未完成的读取时**无视画面变化直接重试**——这是「失败不丢消息」
    /// 的代价：反复失败时会一直重试，退避由调用方负责。
    pub fn observe(&mut self, pane: &Frame) -> Decision {
        let bottom = pane.height() as i32;
        let top = self.ignore_top.clamp(0, bottom);

        let decision = match self.pending {
            // 上一次没读到，重试同一段
            Some(roi) => Decision::Read { roi },
            None => match &self.last {
                None => Decision::Read {
                    roi: Rect::new(0, top, pane.width() as i32, bottom),
                },
                Some(prev) if same_pixels(prev, pane, 0, bottom) => Decision::Quiet,
                // 变的全在水位线以上：滚出旧消息、顶部公告条在滚。一样不读。
                Some(prev) if same_pixels(prev, pane, top.max(self.watermark), bottom) => {
                    Decision::Quiet
                }
                Some(_) => Decision::Read {
                    roi: Rect::new(0, top.max(self.watermark), pane.width() as i32, bottom),
                },
            },
        };

        if let Decision::Read { roi } = decision {
            self.calls += 1;
            self.pending = Some(roi);
        }
        // 无论读不读，都把参照系换成最新的一帧。
        self.last = Some(pane.clone());
        decision
    }

    /// 识别成功后推进水位线，并清掉待重试的那一段。
    ///
    /// **只在真的读完之后才调。** 失败不推进，下一帧会重试同一段。
    pub fn commit(&mut self, y: i32) {
        self.watermark = self.watermark.max(y);
        self.pending = None;
    }

    /// **把水位线退回顶部**，下一帧会重读整段消息区。
    ///
    /// ## 什么时候必须调
    ///
    /// 换会话。换完之后消息区里全是**从没读过的内容**，而水位线还停在
    /// 上一段对话的底部——于是增量 ROI 永远只剩贴着输入框的那一条缝，
    /// **一条消息都不会再进来**，候选也就永远出不来。
    ///
    /// 实测：点「锁」换到另一个会话后，会话名和闸都对，但候选一直是 0，
    /// 而且不会自愈。清历史、清摘要、清候选都做了，唯独漏了水位线——
    /// 它不在 `Scratch` 里，所以「把会话状态清干净」这件事天然漏掉它。
    ///
    /// `last` 也要丢掉：留着旧会话的画面会让「没变化」的判断继续成立。
    pub fn rewind(&mut self) {
        self.watermark = 0;
        self.pending = None;
        self.last = None;
    }
}

/// 两帧在 `[y0, y1)` 这段上逐字节是否完全一样。
///
/// 逐字节比，不采样、不哈希。采样会漏掉细窄的变化（参考项目就栽在
/// 「在原尺寸下碰巧成立」上——1 px 的边框线恰好能被抽样格点命中，
/// 换个窗口尺寸就漏检）。
fn same_pixels(a: &Frame, b: &Frame, y0: i32, y1: i32) -> bool {
    let h = a.height().min(b.height()) as i32;
    let w = a.width().min(b.width());
    if a.width() != b.width() || a.height() != b.height() {
        return false;
    }
    let y0 = y0.clamp(0, h);
    let y1 = y1.clamp(0, h);
    if y0 >= y1 {
        // 空区间：认为没变。这不是「随便返回 true」——
        // 水位线到底时「没有新增带」本来就等于不读。
        return true;
    }
    let bytes = (w as usize) * 3;
    let pa = a.as_bytes();
    let pb = b.as_bytes();
    (y0..y1).all(|y| {
        let s = (y as usize) * bytes;
        pa[s..s + bytes] == pb[s..s + bytes]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANEL: [u8; 3] = [30, 30, 31];
    const INK: [u8; 3] = [225, 225, 229];
    const W: u32 = 400;
    const H: u32 = 300;

    /// 造一帧消息区。`rows` 是 (y0, y1) 上画一道文字行的位置。
    fn pane(rows: &[(u32, u32)], ignore_band: Option<(u32, u32)>) -> Frame {
        let mut px = vec![0u8; (W * H * 3) as usize];
        for y in 0..H {
            for x in 0..W {
                let i = ((y * W + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&PANEL);
            }
        }
        let paint = |px: &mut [u8], y0: u32, y1: u32, c: [u8; 3]| {
            for y in y0..y1 {
                for x in 40..(W - 40) {
                    let i = ((y * W + x) * 3) as usize;
                    px[i..i + 3].copy_from_slice(&c);
                }
            }
        };
        if let Some((a, b)) = ignore_band {
            paint(&mut px, a, b, INK);
        }
        for &(a, b) in rows {
            paint(&mut px, a, b, INK);
        }
        Frame::from_rgb(W, H, px).expect("pane")
    }

    /// **重锁之后必须能重读整段消息区。**
    ///
    /// 2026-10-08 实机事故：换会话后水位线还停在上一段对话的底部，
    /// 增量 ROI 永远只剩贴着输入框的那条缝，于是**一条消息都进不来**、
    /// 候选永远是 0，而且不会自愈。清历史、清摘要、清候选都做了，
    /// 唯独漏了水位线——它不在会话状态里。
    ///
    /// 这条测试按那条真实路径走：A 读到顶 → 切到 B（画面全变）→ 重锁。
    #[test]
    fn a_relock_rereads_the_whole_pane() {
        let a = pane(&[(100, 112), (140, 152)], None);
        let mut w = PaneWatcher::new(0);
        assert!(matches!(w.observe(&a), Decision::Read { .. }));
        w.commit(200);
        assert_eq!(w.watermark(), 200);

        // 换成会话 B：整块画面都不同，但水位线还在底部。
        //
        // 注意这里**可能返回 Quiet**：水位线以下那段（A、B 都空）在字节上
        // 完全一样，`same_pixels` 会判「没变化」。真实系统里换会话是靠
        // **头部指纹**发现的（见 watch.rs 的会话闸），不是靠这里的 diff——
        // 闸关着的时候不会推进水位线，于是 B 的内容一条也进不来。
        let b = pane(&[(40, 52), (80, 92), (120, 132)], None);
        let _ = w.observe(&b);
        assert_eq!(w.watermark(), 200, "闸关着的时候水位线不动");

        // 重锁：水位线必须退回去，否则 B 的内容永远读不到。
        w.rewind();
        assert_eq!(w.watermark(), 0, "重锁没有把水位线退回顶部");
        let roi = match w.observe(&b) {
            Decision::Read { roi } => roi,
            Decision::Quiet => panic!("重锁后必须重读整段消息区"),
        };
        assert!(
            roi.y0 <= 1,
            "重锁后的 ROI 应当从消息区顶部开始，实际 {roi:?}"
        );
    }

    /// 静止回放：第一次读，之后每次都是零调用。
    #[test]
    fn a_static_pane_triggers_zero_ocr_calls() {
        let f = pane(&[(100, 112), (140, 152)], None);
        let mut w = PaneWatcher::new(0);

        // 第一次没有参照系，必须读一次——否则第一条消息永远读不到。
        assert!(matches!(w.observe(&f), Decision::Read { .. }));
        w.commit(200);

        for _ in 0..5 {
            assert_eq!(w.observe(&f), Decision::Quiet, "静止时不该再读");
        }
        assert_eq!(w.calls(), 1, "静止五分钟就是五次观察零次调用");
    }

    /// 新消息只触发一次增量识别。
    #[test]
    fn a_new_message_triggers_exactly_one_incremental_read() {
        let before = pane(&[(100, 112)], None);
        let mut w = PaneWatcher::new(0);
        w.observe(&before);
        w.commit(200);
        assert_eq!(w.calls(), 1);

        // 底部多来一条消息
        let after = pane(&[(100, 112), (200, 212)], None);
        let Decision::Read { roi } = w.observe(&after) else {
            panic!("底部新增消息必须触发识别")
        };
        assert!(roi.y0 >= 200, "新增带的顶边应在水位线之下");
        w.commit(260);

        // 再看同一帧：不该再读
        assert_eq!(w.observe(&after), Decision::Quiet);
        assert_eq!(w.calls(), 2, "一条新消息只该触发一次");
    }

    /// 识别范围永远在水位线以下——已经读过的那一段不会被重读。
    #[test]
    fn the_roi_never_reaches_into_already_read_pixels() {
        let mut w = PaneWatcher::new(0);
        w.observe(&pane(&[(100, 112)], None));
        w.commit(200);

        let cases: [&[(u32, u32)]; 2] = [
            &[(100, 112), (250, 262)],
            &[(100, 112), (250, 262), (280, 292)],
        ];
        for extra in cases {
            let Decision::Read { roi } = w.observe(&pane(extra, None)) else {
                panic!("水位线以下有新内容，应触发识别")
            };
            assert!(
                roi.y0 >= 200,
                "ROI 顶边 {} 越过了水位线 200——会重读已识别的内容",
                roi.y0
            );
            assert_eq!(roi.y1, H as i32, "ROI 底边应到消息区下沿");
            // 推进到新内容的**下沿**，不是 ROI 底边：真实聊天里新消息会把旧
            // 内容顶上去，但合成帧不会，线一旦推到 300 就再没有可读区域了。
            w.commit(extra.last().expect("非空").1 as i32);
        }
        assert_eq!(w.calls(), 3, "每次新增都只触发一次识别");
    }

    /// 票 36 的守卫：ROI 的**横轴**是消息区局部坐标。
    ///
    /// 上面那条只钉了纵轴（`y1 == 消息区高度`），横轴没人钉——
    /// 于是 2026-10-15 有人把日志里的 `roi.x0: 0` 当成整帧坐标，
    /// 判成「消息区左边界退化成 0、会话列表被框进 OCR」，票 36 整张票
    /// 都建在这个误读上（真机重放给出的 `chat.x0` 是 316，不是 0）。
    ///
    /// 这条把横轴钉死：`x0` 恒为 0、`x1` 恒等于消息区宽度。
    /// 哪天它俩变成整帧坐标（`x0` 非 0，或 `x1` 宽过消息区），这里立刻红。
    #[test]
    fn the_roi_is_in_pane_coordinates_not_frame_coordinates() {
        let mut w = PaneWatcher::new(0);
        for extra in [&[(100, 112)][..], &[(100, 112), (250, 262)][..]] {
            let Decision::Read { roi } = w.observe(&pane(extra, None)) else {
                panic!("首次必须读一次")
            };
            assert_eq!(
                roi.x0, 0,
                "ROI 横轴是消息区局部坐标，x0 恒为 0；非 0 说明坐标系变了"
            );
            assert_eq!(
                roi.x1, W as i32,
                "ROI 右边界应等于**消息区**宽度。真塌成 0 边界的形态是 \
                 x1 宽到接近帧宽——真机实测消息区 940px、帧 1253px"
            );
            w.commit(200);
        }
    }

    /// 顶部会自己滚的东西不算变化。
    ///
    /// 置顶公告条会滚动。把它算进 diff，画面就永远判定为变化——
    /// 「静默期零调用」当场作废。
    #[test]
    fn a_scrolling_pinned_notice_is_not_a_change() {
        let ignore_top = 60;
        let base = pane(&[(100, 112)], None);
        let mut w = PaneWatcher::new(ignore_top);
        w.observe(&base);
        w.commit(200);
        assert_eq!(w.calls(), 1);

        // 只有顶部那一带在动，位置每帧都不一样（模拟滚动）
        for top in [20u32, 24, 28, 32] {
            let f = pane(&[(100, 112)], Some((top, top + 12)));
            assert_eq!(
                w.observe(&f),
                Decision::Quiet,
                "顶部 {top} 处的公告条在滚，不该触发识别"
            );
        }
        assert_eq!(w.calls(), 1, "滚动公告条期间必须是零调用");
    }

    /// 忽略区即使真的触发了识别，也不能进 ROI。
    #[test]
    fn the_roi_excludes_the_ignored_top_band() {
        let ignore_top = 60;
        let mut w = PaneWatcher::new(ignore_top);

        // 第一次观察没有参照系，ROI 从忽略区下沿开始
        let Decision::Read { roi } = w.observe(&pane(&[(100, 112)], None)) else {
            panic!("首次必须读一次")
        };
        assert_eq!(roi.y0, ignore_top, "ROI 顶边必须从忽略区下沿开始");
        w.commit(200);

        // 水位线在忽略区之下时，取两者更大的那个
        let Decision::Read { roi } = w.observe(&pane(&[(100, 112), (250, 262)], None)) else {
            panic!("底部新增应触发识别")
        };
        assert!(roi.y0 >= 200, "ROI 顶边 {} 应在忽略区与水位线之下", roi.y0);
    }

    /// 识别失败**不推进水位线**，下一帧重试——不能永久丢消息。
    #[test]
    fn a_failed_read_does_not_advance_the_watermark() {
        let before = pane(&[(100, 112)], None);
        let mut w = PaneWatcher::new(0);
        w.observe(&before);
        w.commit(200);

        let after = pane(&[(100, 112), (250, 262)], None);
        // 识别失败：拿到 Read 之后没有 commit
        let Decision::Read { roi: first } = w.observe(&after) else {
            panic!("应触发识别")
        };
        assert_eq!(w.calls(), 2);

        // 下一帧同一张图：必须重试同一段，而不是当成「已经处理过」
        let Decision::Read { roi: second } = w.observe(&after) else {
            panic!("失败后必须重试")
        };
        assert_eq!(first, second, "重试的范围必须和上次一样");
        assert_eq!(w.calls(), 3, "失败要重试，不能靠「已经看过了」糊弄过去");
    }

    /// 一个会话一个观察器。切走再切回，用的还是同一个水位线。
    #[test]
    fn each_session_keeps_its_own_watermark() {
        let f = pane(&[(100, 112)], None);
        let mut a = PaneWatcher::new(0);
        let mut b = PaneWatcher::new(0);

        a.observe(&f);
        a.commit(200);
        b.observe(&f);
        b.commit(120);

        assert_ne!(a.watermark(), b.watermark(), "两个会话的水位线必须独立");

        // 切走再切回 A：它已经读到 200，同一帧不该再读
        let before = a.calls();
        assert_eq!(a.observe(&f), Decision::Quiet);
        assert_eq!(a.calls(), before, "切回已读的会话不该重复上报");
    }

    /// 水位线只进不退。重复 commit 旧的 y 不能把线拉回去。
    #[test]
    fn the_watermark_never_moves_backwards() {
        let mut w = PaneWatcher::new(0);
        w.commit(200);
        w.commit(120);
        assert_eq!(w.watermark(), 200);
    }
}
