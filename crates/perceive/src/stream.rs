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
//! 每来一帧问一个问题：
//!
//! **消息区（`ignore_top` 以下那一段）跟上一帧一模一样吗？**
//! → 一样 ⇒ [`Decision::Quiet`]，一个像素都不读；
//! → 不一样 ⇒ [`Decision::Read`]，**读整块消息区**。
//!
//! ⚠ **「读哪一段」不再由像素划出来**——见下面「底部对齐」那一节。
//! 「哪几条是新的」由**内容身份**判（[`crate::merge::novel_messages`]）。
//!
//! ## 为什么必须排除顶部
//!
//! 置顶公告条会自己滚。把它算进 diff，画面就**永远**判定为变化，
//! 零调用这条约束当场作废。跳过多高的区域由 [`ChatSurface`] 给——
//! 那是「接一个新软件」要提供的东西之一，不该写死在引擎里。
//!
//! ## ⚠⚠ 底部对齐的聊天窗：水位线**不能**锚在像素行上（2026-10-23 实测）
//!
//! 上面那条「新消息永远从底部出现 ⇒ 新增带在水位线以下」的推理，
//! **对微信是错的**，而微信就是本产品要接的那个软件。
//!
//! 微信的消息列表是**底部对齐**的：对话变长时**视口整体上移**，
//! 最新的那条始终贴在输入框上方。⇒ 对方又发了消息之后，
//! **新消息落在旧水位线之上**（旧水位线是「上一条消息的下沿」，
//! 而新消息是「上一条消息被顶上去之后腾出来的那一格」）。
//!
//! 真机实测（2026-10-23，同一会话的两张真机帧，夹具换文件模拟「新消息到达」）：
//!
//! ```text
//! 第 1 圈  roi y0=60   本轮行=14  消息=5      ← 首读，水位线推到 569
//! 换帧后  roi y0=569  本轮行=0   消息=0      ← 新增带是空的：一条新消息都没进来
//! ```
//!
//! 那一圈**什么都没读到**，而对方确实发了 4 条。⇒ 增量带这个模型
//! 在底部对齐的窗上**结构性地读不到新消息**。
//!
//! ⇒ 所以水位线**必须锚在内容上，不能锚在像素行上**：
//! 「读到了什么」由**内容身份**（说话人 + 正文）判定，
//! 「这段像素读过了没有」由**消息流**判定，与它在第几行无关。
//!
//! ## 「读到哪了」什么时候记
//!
//! **识别成功之后**，由调用方显式 [`PaneWatcher::commit`]。
//! 失败就不记——下一帧会重试，而不是永久丢掉一条消息。
//!
//! ⚠ 这个位置**只作诊断**（打进日志），**不再用来划 ROI**。
//!
//! ## 这层管不了的事
//!
//! **「哪几条是新的」不归这层管。** 像素上「来了新消息」和「滚动翻出旧消息」
//! 分不开（都是内容整体上移）。
//!
//! ⇒ 判据在**内容身份**（说话人 + 正文）：见 [`crate::merge::novel_messages`]。
//! 这一层只保证它的**必要前提**：画面没变就一个像素都不读。
//!
//! [`ChatSurface`]: ../../../../CONTEXT.md

use chatservant_core::frame::{Frame, Rect};

/// 一帧消息区看完之后，决定要不要读。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// 没变：一行都不读。这次观察的 OCR 调用数必须是 0。
    Quiet,
    /// 变了：读这一个矩形。
    Read {
        /// 要读的那一块。
        ///
        /// ⚠ 2026-10-23：它现在覆盖**整块消息区**（`ignore_top` 以下）。
        /// 原来只取「已提交水位线以下」那条带子，而**底部对齐**的聊天窗
        /// （微信）里新消息落在**旧水位线之上** ⇒ 那条带子结构性读不到它。
        /// 见模块头「底部对齐」那一节的真机实测。
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
/// 「这个会话读到哪了」是它的凭证，重建实例等于把读过的消息全忘掉，
/// 用户会看到重复上报。
#[derive(Debug)]
pub struct PaneWatcher {
    /// 顶部跳过多高。置顶公告条会自己滚，算进 diff 会让画面永远判定为变化。
    ignore_top: i32,
    /// 本会话**消费到过的最深位置**（对方消息的下沿）。
    ///
    /// ⚠ **它不再用来划 ROI**（2026-10-23 改）——在底部对齐的聊天窗上
    /// 那个判据结构性失效（模块头有真机实测：新增的消息落在旧水位线**之上**）。
    ///
    /// 现在它是**诊断信息**：本轮消费到哪儿了，打进日志给人看。
    committed: i32,
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
            committed: 0,
            pending: None,
            last: None,
            calls: 0,
        }
    }

    /// 观察以来返回过几次「要读了」。静默期必须是 0。
    pub fn calls(&self) -> u32 {
        self.calls
    }

    /// 本会话**消费到过的最深位置**。**诊断用**（打进日志）。
    ///
    /// ⚠ 它**不是**下一轮的 ROI 上界——见 [`PaneWatcher`] 的字段说明。
    pub fn committed_bottom(&self) -> i32 {
        self.committed
    }

    /// 看一帧，决定这次要不要读。
    ///
    /// `pane` 是**裁过的消息区**，因此返回的 [`Decision::Read`] 里的
    /// 矩形是消息区局部坐标（`x0` 恒为 0）。
    ///
    /// 有未完成的读取时**无视画面变化直接重试**——这是「失败不丢消息」
    /// 的代价：反复失败时会一直重试，退避由调用方负责。
    ///
    /// ## 底部对齐的窗：ROI 覆盖**整块**消息区（2026-10-23 改）
    ///
    /// 原来这里给的是 `[水位线, 消息区底部)`。在**底部对齐**的聊天窗
    /// （微信就是）上这个判据**结构性地读不到新消息**——实测见模块头：
    /// 新消息落在旧水位线**之上**，那条带子里一条都没有。
    ///
    /// ⇒ 现在整块消息区都给，**「哪几条是新的」由内容身份判定**
    /// （见 [`crate::merge::novel_messages`]）。
    ///
    /// ## 为什么这不违反「静默期零调用」
    ///
    /// 静默期走的是**整块逐字节比对**：一模一样 ⇒ [`Decision::Quiet`]，
    /// 一行都不读、一像素都不送。ROI 大小只影响「变了之后读哪几行」，
    /// **不影响「变没变」**。
    ///
    /// ⚠ 而 OCR 送进去的画面**本来就不是 ROI**——
    /// `read_lines` 给的是整帧或整块消息区（见 `ocr_input_pixels` 的注释：
    /// 「ROI 只决定读哪几行，送进 OCR 的画面根本没变小」）。
    /// ⇒ ROI 变大**不增加 OCR 成本**，只增加「本轮行数」。
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
                // ⚠ **只比 `ignore_top` 以下那一段。**
                //   置顶公告条会自己滚，把它算进 diff，画面就永远判定为变化
                //   ⇒ 「静默期零调用」当场作废。
                //   （`ignore_top` 是**唯一**的顶部豁免机制——
                //   原来还有一条「变的全在水位线以上 ⇒ Quiet」的捷径，
                //   它在底部对齐的窗上判反，已删。）
                Some(prev) if same_pixels(prev, pane, top, bottom) => Decision::Quiet,
                Some(_) => Decision::Read {
                    roi: Rect::new(0, top, pane.width() as i32, bottom),
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

    /// **这一轮读完了，而且消费到了位置 `y`**：记下位置、清掉待重试的那一段。
    ///
    /// **只在真的读完之后才调。** 失败不调，下一帧会重试同一段。
    ///
    /// ## `y` 只是**记下来给人看**，不再划下一轮的 ROI
    ///
    /// 原来这里推进**像素行**的水位线，用来划下一轮的增量 ROI。
    /// 那个判据在**底部对齐**的聊天窗上结构性失效（见模块头）⇒
    /// 「读过没有」改由**上层的消息流**按内容身份承担
    /// （[`crate::merge::novel_messages`]）。
    pub fn commit(&mut self, y: i32) {
        self.committed = self.committed.max(y);
        self.pending = None;
    }

    /// **这一轮读完了，但没有可记的位置**（没读到对方说话）：清掉待重试的那一段。
    ///
    /// 与 [`PaneWatcher::commit`] 的差别只有「记不记位置」——
    /// 失败不调，下一帧会重试同一段。
    pub fn complete(&mut self) {
        self.pending = None;
    }

    /// **忘掉上一帧**，下一帧必然重读整段消息区。
    ///
    /// ## 什么时候必须调
    ///
    /// 换会话。换完之后消息区里全是**从没读过的内容**，而 `last` 还指着
    /// 上一段对话的画面——留着它，「画面跟上一帧一模一样」的判断会继续成立
    /// ⇒ 新会话的内容**一条都进不来**，候选也就永远出不来。
    ///
    /// 实测：点「锁」换到另一个会话后，会话名和闸都对，但候选一直是 0，
    /// 而且不会自愈。清历史、清摘要、清候选都做了，唯独漏了这个——
    /// 它不在 `Scratch` 里，所以「把会话状态清干净」这件事天然漏掉它。
    ///
    /// （原来这里还要退**水位线**。水位线已于 2026-10-23 拆掉 ——
    /// 它在底部对齐的聊天窗上结构性失效，见模块头。）
    pub fn rewind(&mut self) {
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

    /// **读完（哪怕没有对方说话）⇒ 下一帧必须是 `Quiet`，不是再读一次。**
    ///
    /// 这是硬约束第 6 条「静默期零调用」的守卫。
    ///
    /// ## 这条守的是什么（2026-10-19 修的真漏洞）
    ///
    /// 调用方原来只有 `if let Some(y) = newest { watcher.commit(y) }`，
    /// 而「读成功了但带子里只有我方消息」这条路 `newest == None`
    /// ⇒ 既不 commit 也不清 `pending`。
    /// 而 `observe` 里 `pending` 命中**先于像素比对**返回 `Decision::Read`
    /// ⇒ 之后**每一帧都重读同一段，永不停止**。
    ///
    /// 症状在真机上就是「对方不说话时程序仍在每秒做一次全量 OCR」——
    /// 静默期不是零调用，是持续满速。⚠ 而所有既有测试都**不会红**：
    /// 它们的调用方要么 commit、要么 rewind，**从没走过这条路径**。
    #[test]
    fn completing_a_read_with_no_message_from_them_goes_quiet() {
        // 一帧画面稳定不变（只有我方的行，对方没说话）。
        let pane = pane(&[(100, 112)], None);
        let mut w = PaneWatcher::new(0);

        // 第一帧：没有参照物，必然读。
        assert!(
            matches!(w.observe(&pane), Decision::Read { .. }),
            "第一帧没有参照物，必然要读"
        );

        // 读成功了，但带子里只有我方消息 ⇒ 调用方走 complete()。
        w.complete();

        // ⚠ 这一帧画面**逐字节没变** —— 本该走 Quiet。
        //   若 pending 还在，于是无视像素变化直接返回 Read，
        //   于是再下一帧还是 Read……永不停止。
        assert!(
            matches!(w.observe(&pane), Decision::Quiet),
            "静默期必须 Quiet —— 重新 Read 意味着对方不说话时每秒重读一次 \
             （硬约束第 6 条）。检查调用方是不是漏了 complete()。"
        );
    }

    /// **重锁之后必须能重读整段消息区。**
    ///
    /// 2026-10-08 实机事故：换会话后引擎还指着上一段对话的画面，
    /// 「画面跟上一帧一模一样」继续成立 ⇒ **一条消息都进不来**、
    /// 候选永远是 0，而且不会自愈。清历史、清摘要、清候选都做了，
    /// 唯独漏了这个——它不在会话状态里。
    ///
    /// 这条测试按那条真实路径走：A 读到顶 → 切到 B（画面全变）→ 重锁。
    #[test]
    fn a_relock_rereads_the_whole_pane() {
        let a = pane(&[(100, 112), (140, 152)], None);
        let mut w = PaneWatcher::new(0);
        assert!(matches!(w.observe(&a), Decision::Read { .. }));
        w.complete();

        // 换成会话 B：整块画面都不同。
        let b = pane(&[(40, 52), (80, 92), (120, 132)], None);
        let _ = w.observe(&b);

        // 重锁：必须丢掉上一帧的参照系，否则 B 的内容永远读不到。
        w.rewind();
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
        w.complete();

        for _ in 0..5 {
            assert_eq!(w.observe(&f), Decision::Quiet, "静止时不该再读");
        }
        assert_eq!(w.calls(), 1, "静止五分钟就是五次观察零次调用");
    }

    /// **底部对齐：新消息到了 ⇒ 必须触发一次识别。**
    ///
    /// ## 这条守的是什么（2026-10-23 的真缺陷）
    ///
    /// 微信的消息列表是**底部对齐**的：对话变长时**视口整体上移**，
    /// 最新的那条始终贴在输入框上方。⇒ 对方又发了消息之后，
    /// **新消息落在旧水位线之上**。
    ///
    /// 真机实测（同一会话的两张真机帧，夹具换文件模拟「新消息到达」）：
    ///
    /// ```text
    /// 第 1 圈  roi y0=60   本轮行=14  消息=5   ← 首读，水位线推到 569
    /// 换帧后  roi y0=569  本轮行=0   消息=0   ← 新增带是空的：一条都没进来
    /// ```
    ///
    /// 而换帧后**新增的那 5 条**在 y = 130..567 ⇒ **全在 569 之上**。
    ///
    /// ⇒ 原来那条「变的全在水位线以上 ⇒ Quiet」的捷径，
    /// 在这个软件上**恰好是最坏情况**：新消息到达时变动的正是
    /// 水位线以上的那一段 ⇒ 永远判 Quiet ⇒ 连读都不读。
    ///
    /// ## 这条测试把那个形态**原样复现**
    ///
    /// 新帧 = 旧内容**整体上移** + 底部多出一条 ⇒ 新消息落在旧消息**之上**。
    ///
    /// ⚠ **必须先读完一轮**（2026-10-23 补）：旧判据的「水位线以上 ⇒ Quiet」
    /// 只有在**水位线已经推进过**时才成立；单轮里水位线还是 0 ⇒
    /// 旧判据压根不触发 ⇒ 守卫会**假绿**（实测：那样写反证②不红）。
    #[test]
    fn a_bottom_anchored_new_message_triggers_a_read() {
        // 旧帧：三条消息（100..112 / 140..152 / 180..192）。
        let before = pane(&[(100, 112), (140, 152), (180, 192)], None);
        let mut w = PaneWatcher::new(0);
        assert!(matches!(w.observe(&before), Decision::Read { .. }));
        w.complete();
        assert_eq!(w.calls(), 1);

        // 新帧：**内容整体上移 20px**（底部对齐的视口滚动），
        // 底部**多出一条**。⇒ 新消息落在旧消息**之上**。
        let after = pane(&[(80, 92), (120, 132), (160, 172), (200, 212)], None);

        let Decision::Read { roi } = w.observe(&after) else {
            panic!(
                "底部对齐的窗里来了新消息，必须触发识别 —— \
                    旧判据（变的全在水位线以上 ⇒ Quiet）在这里恰好判反了"
            )
        };
        assert_eq!(roi.y0, 0, "ROI 顶边必须从消息区顶部开始（整块读）");
        assert_eq!(roi.y1, H as i32, "ROI 底边应到消息区下沿");
        assert_eq!(w.calls(), 2, "一条新消息只该触发一次");
    }

    /// **只比 `ignore_top` 以下那一段。**
    ///
    /// 置顶公告条会自己滚。把它算进 diff，画面就永远判定为变化——
    /// 「静默期零调用」当场作废。
    ///
    /// 这条与上面那条成对：上面守「底部新消息 ⇒ 要读」，
    /// 这条守「顶部滚动 ⇒ 不读」。**两条一起才完整**：
    /// 只放宽顶部而不放宽底部，会漏新消息；只放宽底部而不放宽顶部，
    /// 会一直读。
    #[test]
    fn only_the_ignored_top_band_is_excluded_from_the_diff() {
        let ignore_top = 60;
        let base = pane(&[(100, 112)], None);
        let mut w = PaneWatcher::new(ignore_top);
        w.observe(&base);
        w.complete();
        assert_eq!(w.calls(), 1);

        // 顶部那一带在动（模拟公告条滚动），消息区其余部分逐字节没变。
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

    /// **「只是滚动、不是新消息」不重复触发。**
    ///
    /// 旧判据靠「变的全在水位线以上」来挡滚动；在底部对齐的窗上那个判据
    /// 挡不住新消息，于是改成**内容身份**判（见 `merge::novel_messages`）。
    ///
    /// 这条钉住引擎这一侧仍然成立的部分：**像素逐字节没变 ⇒ 一定不读**。
    /// 「内容重复 ⇒ 不算新」是上层的事。
    #[test]
    fn an_unchanged_pane_never_triggers_a_read() {
        let f = pane(&[(100, 112)], None);
        let mut w = PaneWatcher::new(0);
        assert!(matches!(w.observe(&f), Decision::Read { .. }));
        w.complete();
        for _ in 0..3 {
            assert_eq!(w.observe(&f), Decision::Quiet);
        }
        assert_eq!(w.calls(), 1);
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
            w.complete();
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
        w.complete();
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
        w.complete();

        // 底部新增 ⇒ 整块读，ROI 顶边仍从忽略区下沿开始。
        let Decision::Read { roi } = w.observe(&pane(&[(100, 112), (250, 262)], None)) else {
            panic!("底部新增应触发识别")
        };
        assert_eq!(roi.y0, ignore_top, "ROI 顶边必须从忽略区下沿开始");
    }

    /// 识别失败**不推进状态**，下一帧重试——不能永久丢消息。
    #[test]
    fn a_failed_read_is_retried_on_the_next_frame() {
        let before = pane(&[(100, 112)], None);
        let mut w = PaneWatcher::new(0);
        w.observe(&before);
        w.complete();

        let after = pane(&[(100, 112), (250, 262)], None);
        // 识别失败：拿到 Read 之后没有 complete()
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

    /// 一个会话一个观察器。切走再切回，**各自的参照系互不干扰**。
    #[test]
    fn each_session_keeps_its_own_reference_frame() {
        let f = pane(&[(100, 112)], None);
        let mut a = PaneWatcher::new(0);
        let mut b = PaneWatcher::new(0);

        a.observe(&f);
        a.complete();
        b.observe(&f);
        b.complete();

        // 切走再切回 A：它已经读过这一帧，同一帧不该再读
        let before = a.calls();
        assert_eq!(a.observe(&f), Decision::Quiet);
        assert_eq!(a.calls(), before, "切回已读的会话不该重复上报");
    }
}
