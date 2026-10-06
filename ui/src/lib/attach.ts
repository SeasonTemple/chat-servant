/**
 * Svelte 5 **attachment**：`{@attach fn}`。
 *
 * ## 与 action 的区别（这不是风格问题，是能力差异）
 *
 * | | `use:fn`（action） | `{@attach fn}`（attachment） |
 * |---|---|---|
 * | 触发 | 只在元素挂载时跑一次 | 在 **effect** 里跑 |
 * | 重跑 | 要手写 `update()` | **自动**：函数内读到的 state 变了就重跑 |
 * | 清理 | 返回 `{ destroy() {} }` | 返回 `() => {}`（普通函数） |
 * | 能挂组件 | 否 | 是（变成 Symbol prop） |
 *
 * 本项目要的三件事恰好都是「重跑」语义：
 * · 档位指示块：tier 变了要重新量并滑过去
 * · 理由栏：reason 变了要闪一下
 * · 消息流：新消息进来要滚到底
 * 全都要监听 prop/state 的变化——这正是 attachment 的设计目标。
 *
 * 官方文档：https://svelte.dev/docs/svelte/svelte-attachments（需 Svelte 5.29+）
 */
import { gsap } from 'gsap';
import type { Attachment } from 'svelte/attachments';

/** 入场动画。 */
export function enter(vars: gsap.TweenVars): Attachment<HTMLElement> {
  return (node) => {
    const tween = gsap.from(node, vars);
    return () => {
      tween.kill();
    };
  };
}

/**
 * 一次性闪一下。挂成 attachment 后，**函数体内读到的 state 变化就会重跑**——
 * 所以调用方只要在被闪的元素上再读一次那个 state 就行，
 * 不需要 `{#key}` 强制重建节点。
 *
 * 典型用法：
 * ```svelte
 * <p {@attach flash(reason)} class="cs-reason">{reason}</p>
 * ```
 * 这里 `{reason}` 被读了 → reason 变了就重新闪一次。
 */
export function flashOnce(
  dep: string,
  vars: { from: gsap.TweenVars; to: gsap.TweenVars; duration: number; ease: string },
): Attachment<HTMLElement> {
  return (node) => {
    // `void dep` 是刻意的：把参数读进依赖图，变化时这个 attachment 才重跑。
    // 不读的话参数只是个摆设。
    void dep;
    const tween = gsap.fromTo(node, vars.from, {
      ...vars.to,
      duration: vars.duration,
      ease: vars.ease,
    });
    return () => {
      tween.kill();
    };
  };
}

/**
 * 把指示块「滑过去」而不是「跳过去」。
 *
 * 用 left/width 的**像素值**：百分比要相对 pill 的定位父级算，
 * 而 pill 自己在移动，百分比会跟着漂——表现是滑到一半就偏了。
 *
 * @param pick      从容器里挑出当前激活项与指示块（返回 null 表示还没就位）
 * @param duration  补间时长；传 0 表示直接落位（尊重 reduced-motion）
 */
export type SliderParts = { pill: HTMLElement; active: HTMLElement } | null;

export function slideTo(
  pick: (scope: HTMLElement) => SliderParts,
  duration: number,
): Attachment<HTMLElement> {
  return (node) => {
    const found = pick(node);
    if (!found) return;
    const { pill, active } = found;
    const left = active.offsetLeft;
    const width = active.offsetWidth;

    // 没有位移就不补间：否则一段无意义的 240ms 动画只会让人觉得卡
    if (Math.abs(left - pill.offsetLeft) < 0.5 && Math.abs(width - pill.offsetWidth) < 0.5) {
      gsap.set(pill, { left, width });
      return;
    }
    const tween = gsap.to(pill, { left, width, duration, ease: 'power3.out', overwrite: 'auto' });
    return () => {
      tween.kill();
    };
  };
}

/**
 * 滚到底部。
 *
 * 调用方要在同一元素上读 `messages.length`，长度变了才重跑——
 * 这是「只在真的新增时滚」的落点：用户往上翻历史时来了新消息，
 * 把他拽回底部是最招人烦的行为。
 */
export function scrollToBottom(dep: number, duration: number): Attachment<HTMLElement> {
  return (node) => {
    void dep;
    const tween = gsap.to(node, {
      scrollTop: node.scrollHeight,
      duration,
      ease: 'power3.out',
      overwrite: 'auto',
    });
    return () => {
      tween.kill();
    };
  };
}
