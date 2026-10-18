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
 * @param bleed     **指示块向四周外扩的像素**（2026-10-18 加）。
 *
 *   为什么需要它：指示块原来与激活项**逐像素重合**（`left = offsetLeft`、
 *   `width = offsetWidth`）。那是「药丸」这个形状的前提——它靠描边与
 *   底色自己占住那一格。
 *
 *   ⚠ 改成**撕边纸片**之后就不成立了：撕边那 10px 会被吃掉，
 *   若仍按激活项的盒子落位，撕边会直接啃在文字上
 *   （「只填入」实测只剩半只「只」字）。
 *   ⇒ 落位要**外扩**：内衬盒仍与激活项重合，而撕边落在盒外，
 *   于是「选中」读成一片纸盖上去，而不是一个框贴在字旁边。
 */
export type SliderParts = { pill: HTMLElement; active: HTMLElement } | null;

export function slideTo(
  pick: (scope: HTMLElement) => SliderParts,
  duration: number,
  bleed = 0,
): Attachment<HTMLElement> {
  return (node) => {
    const found = pick(node);
    if (!found) return;
    const { pill, active } = found;
    const left = active.offsetLeft - bleed;
    const width = active.offsetWidth + bleed * 2;

    //**第一次落位不许补间**（票 37，2026-10-15 定位）。
    //
    // 药丸的初值是 CSS 里的 `left: 0; width: 0`，于是挂载那一刻
    // `offsetLeft/offsetWidth` 都是 0，而下面对「无位移」的判断要求
    // 两者都差 < 0.5 —— **第一次必然判成「有位移」，从 0 补间到目标**。
    // 但挂载时根本没有「从哪来」：那段 240ms 是药丸凭空长出来，不是滑动。
    //
    // 后果不只是「开场动画多余」，而是**无头出图会把这半截帧当成成品**。
    // 量法：`scripts/shoturl.ps1 -Scale 1`（1 px = 1 CSS px），数药丸
    // **上边线那一行**（y=104）的墨线跨度 —— 那一行是药丸顶边，两端被
    // 圆角切掉，所以跨度 = 药丸宽 − 2×4px（圆角 `calc(radius − 2px)` = 4）。
    // 同一张图上「只填入」按钮的自然宽度是 61px：
    //   `--virtual-time-budget` 4000 / 15000 / 40000 → 跨度 36 / 36 / 46
    //   （即药丸 44 / 44 / 54 px，**随预算单调变长** = 补间在跑、没跑完）
    // 真实 Chromium 里量同一页面的 DOM，内联值是 `width: 61px; left: 2px`，
    // 与激活项 `offsetWidth: 61` 逐像素相等。
    // ⇒ 偏差的成因是**补间中途被截图**，不是量错了，更不是窄视口改了布局。
    //
    // 修法：初值（offset 全 0）视为「还没落位」，直接落位。之后的切换
    // 仍然照常滑动 —— 那才是这个组件要表达的东西。
    const unplaced = pill.offsetLeft === 0 && pill.offsetWidth === 0;
    // 没有位移就不补间：否则一段无意义的 240ms 动画只会让人觉得卡
    if (
      unplaced ||
      (Math.abs(left - pill.offsetLeft) < 0.5 && Math.abs(width - pill.offsetWidth) < 0.5)
    ) {
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
