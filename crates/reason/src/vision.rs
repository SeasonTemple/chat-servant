//! VisionBridge——**只在回复模型没有视觉能力时才存在**。
//!
//! ## 这不是第三条平级链路
//!
//! 默认配置下进程里只有**两个**模型：判断用的 A、起草用的 B。
//! 图片本身**不算一个模型**——它要么随文本一起进 B（B 有视觉能力），
//! 要么由 B 顺带描述掉。
//!
//! 只有一种情况需要**额外的视觉模型**：用户把 B 换成了**纯文本** LLM，
//! 而对方又发来一张图。那时候这个视觉调用是 **B 的一个配置项**，
//! 不是系统常驻的第三个模型。
//!
//! ## 三条边界（ticket 15 的核心验收点）
//!
//! 1. **默认关闭。**
//! 2. **B 有视觉能力时，这个模块不存在**——不调用、不计数、不出现设置项。
//! 3. **它的输出永远不用于读气泡文字。** 气泡文字只走本地 OCR。
//!    [`Describer`] 的入参只有 [`ImagePayload`]——**它拿不到聊天窗格**，
//!    拿不到就看不见气泡上的字，这条是类型层面保证的，不是靠自觉。
//!
//! ## 降级不是可选的
//!
//! 视觉调用失败、没开、B 是纯文本——这三种情况一律降级成
//! [`PLACEHOLDER`]，**链路不断**。硬失败等于「不回复」，
//! 而对方发了一张图就完全不回，比回得平庸糟得多。

use std::collections::HashMap;

/// 图片消息降级成这一句。**必须是人话**，因为它会直接进模型上下文。
pub const PLACEHOLDER: &str = "对方发来一张图片";

/// 一张已经缩好、编好码的图。
///
/// **推理层不碰像素。** 缩放走 `core::scale` 的预算算术，编码在
/// 采集侧做完——这里只带结果。这么切是因为：
/// - `reason` 是核心 crate，不该依赖 `image`；
/// - 视觉 token 的账要能在**不发网络请求**的情况下算清楚并断言。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ImagePayload {
    /// `image/png` 或 `image/jpeg`。
    pub media_type: String,
    /// base64 编码后的字节。**不落盘、不进日志。**
    pub base64: String,
    /// 送出这一张要花多少视觉 token。**实际值，不是预算值。**
    pub tokens: u32,
    /// 缩放后的宽（像素）。
    ///
    /// 缩放预算的中间产物，**推理层不拿它算图**——
    /// 视觉 token 的账已经落在 [`ImagePayload::tokens`] 上，
    /// 宽高留着是为了让调用方能对账「这张图被缩到了多大」。
    pub width: u32,
    /// 缩放后的高（像素）。与 [`ImagePayload::width`] 一起构成缩放预算。
    pub height: u32,
}

impl ImagePayload {
    /// 内容指纹。用来缓存——同一张图重复出现不该重复调视觉模型。
    ///
    /// 用 FNV-1a 而不是任何哈希库：这里要的不是密码学强度，
    /// 只是「同一段字节算出来同一个值」。**不引新依赖。**
    pub fn fingerprint(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |b: u8| {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        };
        for b in self.media_type.as_bytes() {
            eat(*b);
        }
        eat(0xff); // 分隔符：避免 ("ab","c") 与 ("a","bc") 撞上
        for b in self.base64.as_bytes() {
            eat(*b);
        }
        h
    }
}

/// 模型 B 的视觉能力。**决定这一模块存不存在。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisionMode {
    /// B 能看图 → 图片**随文本一起进 B**，这个模块不参与，零调用。
    Multimodal,
    /// B 是纯文本 → 图要么交给视觉模型描述，要么降级成占位。
    TextOnly,
}

/// 描述一张图。实现方负责发请求。
///
/// ⚠ **拿不到聊天窗格，也拿不到 OCR 结果**——签名里只有一张图。
/// 这是「视觉输出永远不用于读气泡文字」的结构性保证。
pub trait Describer {
    /// 这个实现**真的会发请求吗？**
    ///
    /// 默认 `true`。关掉视觉桥的实现要返回 `false`。
    ///
    /// ## 为什么要有这个
    ///
    /// 没有它的话，一个「没开启」的桥如果返回 `Ok(占位)`，
    /// 调用方就会把它算成**一次成功的视觉调用**——界面于是显示
    /// 「视觉 1 次」，而实际上一个字节都没发出去。
    /// 这正是 ticket 15 第 2 条要禁的事：**默认配置下必须是 0，不是 1。**
    fn is_live(&self) -> bool {
        true
    }

    /// 把一张图折成一段文字。**入参只有一张图**——
    /// 拿不到聊天窗格、拿不到 OCR 结果，所以它的输出**在类型层面**
    /// 就不可能拿去读气泡文字。
    ///
    /// 返回的 `Err` 不会中断链路：[`VisionBridge::fold`] 收到错误就降级成
    /// [`PLACEHOLDER`]，并记一次降级。所以实现方报错时**别把异常吞了**，
    /// 把原因写进 `Err` 的文本——那是界面上唯一能解释
    /// 「为什么这张图变成了『对方发来一张图片』」的线索。
    fn describe(&mut self, img: &ImagePayload) -> Result<String, String>;
}

/// 默认实现：**不发任何请求**，直接降级成占位。
///
/// 它存在的意义是让「关掉」和「没配 key」走同一条代码路径——
/// 没有一个「关掉了但代码里还留着分支」的状态。
pub struct NoVision;

impl Describer for NoVision {
    fn is_live(&self) -> bool {
        false
    }

    fn describe(&mut self, _img: &ImagePayload) -> Result<String, String> {
        Ok(PLACEHOLDER.to_string())
    }
}

/// 视觉调用次数与缓存。**不是全局 static**——谁开谁负责。
pub struct VisionBridge {
    describer: Box<dyn Describer>,
    cache: HashMap<u64, String>,
    /// 真正发出去的视觉请求数。**缓存命中不计数**——
    /// 界面上显示的「几次视觉调用」要是真实开销。
    calls: u32,
    /// 降级次数。**单独计数**：它和成功次数要能在界面上分开看，
    /// 「调用了 3 次但 3 次都失败」和「调用了 3 次都成功」不是一回事。
    degraded: u32,
}

impl VisionBridge {
    /// 关闭状态。**零调用**，全部降级成占位。
    pub fn off() -> Self {
        Self::new(Box::new(NoVision))
    }

    /// 用一个描述器建桥。**描述器自己决定发不发请求**——
    /// 传 [`NoVision`] 就是关闭状态，与 [`VisionBridge::off`] 等价。
    ///
    /// 缓存和计数都是空的，全新开始。**计数从这一刻算起**，
    /// 因为 [`Describer`] 可能是个带自己缓存的实现，
    /// 想断言「视觉调用了几次」就得连描述器一起新建。
    pub fn new(describer: Box<dyn Describer>) -> Self {
        Self {
            describer,
            cache: HashMap::new(),
            calls: 0,
            degraded: 0,
        }
    }

    /// 这一轮实际发出的视觉请求数。
    pub fn calls(&self) -> u32 {
        self.calls
    }

    /// 降级成占位的次数。
    pub fn degraded(&self) -> u32 {
        self.degraded
    }

    /// 把图片折成**要追加进上下文的文本行**。
    ///
    /// ## 这是整个模块唯一的决策点
    ///
    /// - [`VisionMode::Multimodal`] → 返回空 Vec。图片由调用方直接挂到
    ///   [`crate::model::TurnContext`] 上随文本进 B，**这里一次请求都不发**。
    /// - [`VisionMode::TextOnly`] → 每张图要么命中缓存，要么调一次描述器。
    ///
    /// 返回的 Vec 里**每张图对应一行**，顺序与输入一致——
    /// 模型看到的图文顺序必须和聊天窗格里一致。
    pub fn fold(&mut self, images: &[ImagePayload], mode: VisionMode) -> Vec<String> {
        if mode == VisionMode::Multimodal {
            // B 自己能看。这条路径上**不允许**发出任何请求——
            // 「模型 B 多模态时 VisionBridge 零调用」这条要能直接断言。
            return Vec::new();
        }
        images
            .iter()
            .map(|img| {
                if !self.describer.is_live() {
                    // 没开启。**不计数**——界面上要显示的「视觉调用次数」
                    // 是用户实际掏的钱，一个字节没发就不能记一次。
                    self.degraded += 1;
                    return PLACEHOLDER.to_string();
                }
                match self.cache.get(&img.fingerprint()) {
                    Some(hit) => hit.clone(),
                    None => {
                        let out = match self.describer.describe(img) {
                            Ok(text) if !text.trim().is_empty() => text,
                            // 失败或空串 → 占位。**链路不断。**
                            Ok(_) | Err(_) => {
                                self.degraded += 1;
                                PLACEHOLDER.to_string()
                            }
                        };
                        self.calls += 1;
                        self.cache.insert(img.fingerprint(), out.clone());
                        out
                    }
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Candidate, DraftSet, Intent, TurnContext};

    fn img(seed: &str) -> ImagePayload {
        ImagePayload {
            media_type: "image/png".into(),
            base64: seed.into(),
            tokens: 1296,
            width: 1000,
            height: 1000,
        }
    }

    struct Counter {
        seen: Vec<String>,
        fail: bool,
    }

    impl Describer for Counter {
        fn describe(&mut self, i: &ImagePayload) -> Result<String, String> {
            self.seen.push(i.base64.clone());
            if self.fail {
                Err("上游 503".into())
            } else {
                Ok(format!("一张图：{}", i.base64))
            }
        }
    }

    /// **B 是多模态时，这个模块一次请求都不发。**
    ///
    /// 这是 ticket 15 的第 2 条验收点。多模态是**默认配置**，
    /// 所以默认下的调用次数必须是 0——不是「很少」，是 0。
    #[test]
    fn a_multimodal_draft_model_means_zero_vision_calls() {
        let mut b = VisionBridge::new(Box::new(Counter {
            seen: Vec::new(),
            fail: false,
        }));
        let lines = b.fold(&[img("a"), img("b")], VisionMode::Multimodal);
        assert!(lines.is_empty(), "多模态下不该折出任何文本行");
        assert_eq!(b.calls(), 0, "多模态下视觉调用必须是 0");
    }

    /// 纯文本 + **没开**视觉桥 → 降级成占位，**链路不断**。
    #[test]
    fn a_disabled_bridge_degrades_to_a_placeholder() {
        let mut b = VisionBridge::off();
        let lines = b.fold(&[img("a")], VisionMode::TextOnly);
        assert_eq!(lines, vec![PLACEHOLDER.to_string()]);
        assert_eq!(b.calls(), 0, "NoVision 不该发请求");
        assert_eq!(b.degraded(), 1, "但要计一次降级");
    }

    /// 纯文本 + 开了 → 每张图一行，顺序与输入一致。
    #[test]
    fn descriptions_keep_the_input_order() {
        let mut b = VisionBridge::new(Box::new(Counter {
            seen: Vec::new(),
            fail: false,
        }));
        let lines = b.fold(&[img("a"), img("b"), img("c")], VisionMode::TextOnly);
        assert_eq!(lines, vec!["一张图：a", "一张图：b", "一张图：c"]);
        assert_eq!(b.calls(), 3);
        assert_eq!(b.degraded(), 0);
    }

    /// **同一张图不重复调用。** 缓存命中不计数。
    #[test]
    fn the_same_image_is_described_once() {
        let mut b = VisionBridge::new(Box::new(Counter {
            seen: Vec::new(),
            fail: false,
        }));
        b.fold(&[img("a")], VisionMode::TextOnly);
        b.fold(&[img("a")], VisionMode::TextOnly);
        b.fold(&[img("a"), img("b")], VisionMode::TextOnly);
        assert_eq!(b.calls(), 2, "a 命中缓存，只该为 b 再调一次");
    }

    /// 视觉调用**失败**退回占位，不阻断整条链路。
    #[test]
    fn a_failing_describer_falls_back_to_the_placeholder() {
        let mut b = VisionBridge::new(Box::new(Counter {
            seen: Vec::new(),
            fail: true,
        }));
        let lines = b.fold(&[img("a")], VisionMode::TextOnly);
        assert_eq!(lines, vec![PLACEHOLDER.to_string()]);
        assert_eq!(b.degraded(), 1);
        assert_eq!(b.calls(), 1, "请求确实发出去了，要如实计数");
    }

    /// **返回空串也算降级。** 空的描述对模型等于没有内容。
    struct Empty;
    impl Describer for Empty {
        fn describe(&mut self, _: &ImagePayload) -> Result<String, String> {
            Ok("   ".into())
        }
    }
    #[test]
    fn a_blank_description_is_treated_as_a_failure() {
        let mut b = VisionBridge::new(Box::new(Empty));
        assert_eq!(
            b.fold(&[img("a")], VisionMode::TextOnly),
            vec![PLACEHOLDER.to_string()]
        );
        assert_eq!(b.degraded(), 1);
    }

    /// **指纹不能因为「拼在一起一样」而撞车。**
    #[test]
    fn the_fingerprint_separates_media_type_from_payload() {
        assert_ne!(img("ab").fingerprint(), img("ba").fingerprint());
        let mut a = img("x");
        let mut b = img("x");
        a.media_type = "image/png".into();
        b.media_type = "image/jpeg".into();
        assert_ne!(a.fingerprint(), b.fingerprint(), "换了编码格式就是另一张图");
    }

    /// 降级之后，**链路仍然能跑完并产出候选**。
    ///
    /// 这条是「链路不断」的真实含义：不只是 `fold` 返回了占位，
    /// 而是从头到尾 `turn()` 仍然给出可用的候选。
    #[test]
    fn a_degraded_turn_still_yields_usable_candidates() {
        let mut bridge = VisionBridge::off();
        let mut lines = bridge.fold(&[img("a")], VisionMode::TextOnly);
        let mut messages = vec!["对方：今晚有空吗".to_string()];
        messages.append(&mut lines);

        let mut r = crate::gate::GateReasoner::new();
        let ctx = TurnContext {
            messages,
            peer: String::new(),
            images: Vec::new(),
        };
        let t = crate::turn(&mut r, &ctx).expect("降级不该让整轮失败");
        assert!(!t.draft.candidates.is_empty(), "仍然要有候选");
        assert!(
            t.draft
                .candidates
                .iter()
                .all(|c: &Candidate| !c.text.is_empty())
        );
    }

    /// 占位文本必须真的进了模型上下文——不然「降级」只是自说自话。
    #[test]
    fn the_placeholder_reaches_the_model_context() {
        let mut bridge = VisionBridge::off();
        let lines = bridge.fold(&[img("a")], VisionMode::TextOnly);
        let ctx = TurnContext {
            messages: lines,
            peer: String::new(),
            images: Vec::new(),
        };
        assert!(
            ctx.messages.iter().any(|m| m == PLACEHOLDER),
            "占位没进上下文：{:?}",
            ctx.messages
        );
    }

    /// 编译期提醒：`DraftSet`/`Intent` 仍在本模块测试里用到。
    #[allow(dead_code)]
    fn _types_stay_used(_: &DraftSet, _: &Intent) {}
}
