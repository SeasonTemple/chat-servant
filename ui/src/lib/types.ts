// 前端的数据类型。
//
// **全部镜像 Rust 侧的 serde 类型**。Rust 是唯一事实来源，这里不允许
// 「前端自己算一个大概」——那会让两边的判据悄悄分叉。

/**
 * 发送档位。与 `chatservant_core::dispatch::Dispatch` 一一对应。
 *
 * **只有两档。** 2026-10-15 砍掉 `Confirm`（二次确认），保留 `Auto`（自动发送），
 * 见 `docs/设计规范.md` §10 第 3b 条与根 `AGENTS.md` 硬约束第 5 条。
 * 加第三个值之前先读那两处——它动的是安全模型，不是排版。
 */
export type Tier = 'Manual' | 'Auto';

/** 一次投递的回执。**不是布尔值**——`PossiblySent` 禁止自动重放。 */
export type Outcome = 'NotSent' | 'Sent' | 'PossiblySent' | 'Failed' | 'TimedOut';

export interface Message {
  id: string;
  speaker: 'Me' | 'Them' | 'System';
  text: string;
  /** 消息区坐标系里的矩形。**几何判定在 Rust 侧**，前端只渲染。 */
  rect: { x0: number; y0: number; x1: number; y1: number };
  /** 这条为什么被这样判。误判必须可见。 */
  evidence: string;
}

export interface Candidate {
  id: string;
  text: string;
  /**
   * 起草段（`SYS_DRAFT`）让模型自评的「你认为用户会选哪条」，0..1。
   *
   * ⚠ **它现在是面板的排序依据**（2026-10-25 订正，与前一句相反）。
   * 这条注释原来写的是「它不是面板的排序依据」——那一版**是对的**，
   * 因为当时排序问的是模型第三遍（`SYS_RANK` 挑一个编号），
   * 而概率是起草段的自评 ⇒ **同一道题问了两次，答案可以对不上**。
   * ⇒ 第三遍已取消（票 94 §5）：排序改成**纯本地按 probability 降序**
   * ⇒ 现在**只有这一个答案**：第一行 = 概率最高的那条 = 首选。
   *
   * ⚠ 面板**当前选中的那条不显示这个数**（那一格放的是那枚手绘勾）
   * —— 这一条**没变**（票 94 §5）：勾说「用哪条」，数说「备选之间差多少」。
   */
  probability: number;
}

/** `dispatch` 命令的回执。 */
export interface DispatchReply {
  outcome: Outcome;
  detail: string;
}

/** 分段调用计数。与 Rust 侧 `watch::CallCounts` 镜像。 */
export interface CallCounts {
  judge: number;
  draft: number;
  /**
   * 额外视觉模型的调用次数。
   *
   * **默认配置下恒为 0**（模型 B 多模态，图片随文本进 B）。不显示这个 0
   * 用户会以为没统计——ticket 15 明确要求界面上能看出它是 0。
   */
  vision: number;
}

/** 单张图片的视觉 token 预算。与 Rust 侧 `commands::BudgetView` 镜像。 */
export interface BudgetView {
  budget: number;
  min: number;
  max: number;
  /**
   * 滑块**该不该出现**。
   *
   * false = 这一轮没有图会外发（模型 B 多模态、视觉桥关闭）。此时摆滑块
   * 就是「调了没反应的控制项」——Rust 侧已经把 `visible` 算好，
   * 前端不要自己猜。
   */
  visible: boolean;
  /** 供应商固定底账。预算低于它就无解，界面要如实显示。 */
  provider_base: number;
  provider: string;
}

/**
 * 面板当前处于哪一阶段。与 `contract::Phase`（Rust 侧）严格对应。
 *
 * **判断归 Rust，界面只渲染。**
 *
 * 早先界面靠 `candidates.length === 0` 隐式区分「静默 / 出错 / 刚启动」
 * 三种完全不同的状态——那是界面在猜后端在干嘛，而且同一个判断在三处
 * 各写一遍、其中两处还写得不一样。序列化形状是**带判别字段**的
 * （`{ phase: 'drafting', messages: [...] }`），Rust 侧有一条测试
 * `phase_serializes_with_a_discriminant_field` 把它测死了。
 */
export type Phase =
  | { phase: 'read'; messages: Message[] }
  | { phase: 'drafting'; messages: Message[] }
  | { phase: 'drafted'; messages: Message[]; candidates: Candidate[] }
  | { phase: 'failed'; messages: Message[]; error: string };

/**
 * 推送载荷。阶段走 {@link Phase}，另带面板顶部那一句「正在干什么」。
 *
 * 字段名是 **snake_case**：`Readout` 那个结构体上没有 `rename_all`，
 *   所以 JSON 里就是 `status_hint`。这跟 `Action`（有 `camelCase`）不一样，
 *   两处都是刻意选的，改一处记得看另一处。
 *
 * `status_hint` 是 Rust `Phase::status_hint()` 的快照。**界面不许自己写死
 * 这句**——早先 `(panel)/+page.svelte` 把「正在成形…」「通常几秒」硬编码成
 * 标签，Rust 侧改了口径它也不会跟着变，同一句话有两份。
 */
export type ReadoutPayload = Phase & {
  status_hint?: string | null;
  /**
   * 当前窗口里的会话名。**空串 = 没认出来**（`watch::Readout::peer`）。
   *
   * 必须显示，理由不是「好看」：会话名闸是「只对锁定会话动手」那道闸，
   *   而闸的判据看不见的话，用户只知道程序不动，不知道为什么。
   *   2026-10-13 补：纸面板的归属行要它，之前 `types.ts` 漏了这个字段，
   *   于是那一行在真机上只能显示占位符 `—`。
   */
  peer?: string;
  /** 会话名闸开没开。关着时不该有任何候选，界面也不该假装在写。 */
  gate_open?: boolean;
  /**
   * 这一轮为什么这么判。带来源戳（`source.stamp`），误判要看得见。
   * 与每条 `Message.evidence` 同一条纪律：判据不可见 = 缺陷不可查。
   */
  reason?: string;
};
