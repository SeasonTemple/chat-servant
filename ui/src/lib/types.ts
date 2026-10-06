// 前端的数据类型。
//
// **全部镜像 Rust 侧的 serde 类型**。Rust 是唯一事实来源，这里不允许
// 「前端自己算一个大概」——那会让两边的判据悄悄分叉。

/** 发送档位。与 `chatservant_core::dispatch::Dispatch` 一一对应。 */
export type Tier = 'Manual' | 'Confirm' | 'Auto';

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
  /** 胜出概率，0..1。界面按它排序，**不是**按生成顺序。 */
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
  rank: number;
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
