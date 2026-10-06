/**
 * 动效时长与缓动。**集中在这一处**，组件里不散落魔数。
 *
 * 为什么要集中：这些值不是审美偏好，是**和 CSS 变量一一对应**的
 * （见 app.css 的 --cs-dur-*）。两处各写一遍，改一处忘一处，
 * 面板就会出现「有的动效快、有的慢」的割裂感。
 *
 * 数值取自 app.css：
 *   fast 140ms · base 240ms · slow 420ms
 *   ease  cubic-bezier(0.22, 1, 0.36, 1)  —— 收尾快、起手稳
 */
export const DUR = {
  /** 按钮按下、悬停这类即时反馈 */
  fast: 0.14,
  /** 卡片入场、档位切换 */
  base: 0.24,
  /** 面板展开/收起、debug 窗口淡入 */
  slow: 0.42,
} as const;

/**
 * 与 CSS `--cs-ease` 等价的 GSAP 缓动。
 *
 * cubic-bezier(0.22, 1, 0.36, 1) 是 CSS 的写法；GSAP 要 CustomEase 或
 * 等价曲线。`power3.out` 在视觉上非常接近——都是「起手慢、收尾快」，
 * 且不需要额外插件与字体文件。
 *
 * 为什么不用 CustomEase：它要把 SVG path 内联进产物，为 0.02 秒的
 * 视觉差异多背一个运行时依赖不划算。真要完全一致，CustomEase 能做到，
 * 但要在注释里说清差异来自哪。
 */
export const EASE = {
  out: 'power3.out',
  in: 'power2.in',
  inOut: 'power2.inOut',
} as const;

/**
 * 用户是否要求减少动效。
 *
 * **这不是可选项，是无障碍要求。** `prefers-reduced-motion` 是系统
 * 设置里用户明确表达的偏好，前庭功能障碍的人靠它避免眩晕。
 *
 * 尊重它的方式不是「把时长缩到 0.1 秒」（那还是动），
 * 而是**直接把位移取消，只留不引起位移的淡入**。
 */
export function prefersReducedMotion(): boolean {
  if (typeof window === 'undefined' || !window.matchMedia) return false;
  return window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

/**
 * 入场动画的统一参数。
 *
 * 关键在 `y: 8` —— 位移很小。面板是贴着别人聊天窗口的悬浮层，
 * 大幅飞入会遮挡它正在发生的事，也会让人分不清「是新内容」
 * 还是「整个面板在动」。
 */
export function enterFrom(y = 8) {
  if (prefersReducedMotion()) {
    // 只淡入，不位移。
    return { opacity: 0, duration: DUR.base, ease: EASE.out };
  }
  return {
    opacity: 0,
    y,
    scale: 0.985,
    duration: DUR.base,
    ease: EASE.out,
  };
}
