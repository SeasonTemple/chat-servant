/**
 * 档位标记形状（勾 / 圈）。**值由 Rust 持有并落盘，这里只是一份镜像。**
 *
 * 为什么需要这个文件：设置页（`PanelSettings.svelte`）写、主面板
 * （`$lib/PaperPanel.svelte`）读，两边都要。放进模块而不是各自
 * `invoke` 一次，是因为**它们必须是同一份**——两处各读一次就会出现
 * 「设置页已经切成圈，面板还是勾」的分裂状态，而用户没有任何办法
 * 从界面上看出是哪里不同步了。
 *
 * ⚠ **不是安全相关值。** 发送档位走 Rust 的 `TierCell`（进程内、
 * 会话结束回落 `Manual`，硬约束第 5 条），外观偏好走 prefs 落盘。
 * 两种生命周期不能混：一个要在重启后记得，一个**必须**忘掉。
 *
 * 写失败时**读回真实值**，与 `set_close_behavior` 同一策略：宁可让开关
 * 停在程序并不认同的状态上（用户会发现），也不要停在一个程序其实没存
 * 的状态上（用户不会发现）。
 */
import { invoke } from '@tauri-apps/api/core';
import { HAS_TAURI } from './runtime';

export type MarkShape = 'tick' | 'ring';

/** 默认值与 Rust 的 `MarkShape::default()` 一致：勾。 */
export const DEFAULT_SHAPE: MarkShape = 'tick';

let shape = $state<MarkShape>(DEFAULT_SHAPE);

export const markShape = {
  get current(): MarkShape {
    return shape;
  },

  /**
   * 从后端读真值。
   *
   * 读不到就留在默认档：这是**失败方向偏保守**的一侧——标记长什么样
   * 错不了，但不能让一个读失败把界面卡在加载态。
   */
  async load(): Promise<MarkShape> {
    if (!HAS_TAURI) return shape;
    try {
      shape = await invoke<MarkShape>('get_mark_shape');
    } catch {
      // 演示模式与静态页面都走这里，保留默认值。
    }
    return shape;
  },

  /** 写后端，成功才改本地镜像。 */
  async set(next: MarkShape): Promise<{ ok: true } | { ok: false; note: string }> {
    if (!HAS_TAURI) {
      shape = next;
      return { ok: true };
    }
    try {
      await invoke('set_mark_shape', { shape: next });
      shape = next;
      return { ok: true };
    } catch (e) {
      const note = String(e);
      await this.load();
      return { ok: false, note };
    }
  },
};
