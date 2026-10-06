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
