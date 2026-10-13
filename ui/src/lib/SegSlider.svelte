<script lang="ts">
  /**
   * 分段选择器：选中态是**一颗会滑过去的块**，不是给按钮换个底色。
   *
   * ## 为什么要一个组件而不是两处各写一遍
   *
   * 设置页有两组「改一个设置」的选择——「发送档位」和「关闭窗口的行为」。
   * 早先档位那组有滑块（`TierSwitch`），窗口那组是静态 `.on` 底色，
   * 同一屏里两种做法。用户 2026-10-10 的原话：「按钮选中方式为何不做成
   * 表面一层的滑块？窗口关闭和档位切换不都可以用这个方案？」
   *
   * 两组本来就该一样：都是**改一个设置、都能改回来**，都不该用红色
   * （红色在这套色板里只给不可逆）。所以抽到这里，两边共用。
   *
   * ## 滑块而不是底色
   *
   * 静态底色的毛病是**选中态只有「有/没有」两帧**，眼睛要在两项之间
   * 重新找一次落点；滑块把「从哪来到哪去」显式画出来，切换时视线不用
   * 跳。而且它不依赖按钮尺寸相等——项宽不一样时（档位三项、窗口两项）
   * 照样对得齐。
   *
   * ⚠ 规格**逐项对齐** `TierSwitch` 那一版（33px 总高 / 0.78rem / 圆角
   * `--cs-radius`）。这个尺寸早先是给主面板 400px 宽定的，现在主面板
   * 的档位已经搬到**纸底**（`6f10f9f`），那个理由不成立了。
   *
   * ⚠ **墨底不是待清的债**（2026-10-14 复核）：票 23 当初否的是旧面板里
   * 316px 栏内**全宽**的红/黑色块互相压，那些按钮已随旧面板删除。
   * 现在这三颗是 `align-self: flex-start` 的自适应小药丸（约 40×33），
   * 既不撑满也不互相压；而「淡到 8% 的底撑不住『当前停在哪一档』」
   * 是 2026-10-10 实测否掉的替代方案，用户当时还主动问过
   * 「为什么不做成表面一层的滑块」。**别拿它当没审过的旧物。**
   */
  import { slideTo } from './attach';
  import { DUR, prefersReducedMotion } from './motion';

  interface Option<T extends string> {
    id: T;
    label: string;
  }

  interface Props<T extends string> {
    /** 当前值。**由 Rust 持有**，这里只负责显示与请求。 */
    value: T;
    options: Option<T>[];
    /** 无障碍名。`role="radiogroup"` 需要它。 */
    label: string;
    /** 请求改值。**前端只负责请求**，是否生效由 Rust 裁决后回填。 */
    onchange?: (v: T) => void;
  }

  let { value = $bindable(), options, label, onchange }: Props<string> = $props();

  /**
   * 指示块「滑过去」而不是「跳过去」。
   *
   * attachment 跑在 effect 里，`value` 被读进依赖 → 换值就自动重跑，
   * 不需要手写 `update()`，也不需要「记住上次值」的状态。
   */
  const move = (dep: string) =>
    slideTo(
      (scope: HTMLElement) => {
        const btn = scope.querySelector(`[data-val="${CSS.escape(dep)}"]`) as HTMLElement | null;
        const pill = scope.querySelector('.cs-pill') as HTMLElement | null;
        return btn && pill ? { pill, active: btn } : null;
      },
      prefersReducedMotion() ? 0 : DUR.base,
    );

  function pick(id: string) {
    if (id === value) return;
    // 只改本地显示，**由父组件回填 Rust 的真实生效值**。
    // Rust 若把值夹回去（比如解析失败），界面就显示夹后的那个——
    // 那才是程序真正的状态，而不是用户以为自己选上的。
    value = id;
    onchange?.(id);
  }

  /**
   * 方向键换档 —— `role="radiogroup"` 的**硬要求**，不是加分项。
   *
   * WAI-ARIA APG 规定 radiogroup 用方向键移动并**同时**改变选中项
   * （roving tabindex：整组只占一个 Tab 位）。只有 Tab 能用的话，
   * 读屏用户按 ↓ 什么都没发生——它以为那是 radio。
   *
   * ⚠ 焦点跟着走：`document.activeElement` 由浏览器维护，我们只用
   * `querySelector('[aria-checked=true]')` 拿元素再 `.focus()`。
   */
  function onKeydown(e: KeyboardEvent) {
    const step =
      e.key === 'ArrowRight' || e.key === 'ArrowDown'
        ? 1
        : e.key === 'ArrowLeft' || e.key === 'ArrowUp'
          ? -1
          : 0;
    if (step === 0) return;
    // Home/End 是 APG 里的可选增强，但两行的控件上很顺手
    if (e.key === 'Home' || e.key === 'End') {
      e.preventDefault();
      pick(options[e.key === 'Home' ? 0 : options.length - 1].id);
      focusChecked();
      return;
    }
    e.preventDefault();
    const i = options.findIndex((o) => o.id === value);
    const next = (i + step + options.length) % options.length;
    pick(options[next].id);
    focusChecked();
  }

  function focusChecked() {
    // 等 Svelte 把 aria-checked 更新完再取；tick 之后 DOM 已是新的。
    queueMicrotask(() => {
      document
        .querySelector<HTMLElement>(
          `[role="radiogroup"][aria-label="${CSS.escape(label)}"] [aria-checked="true"]`,
        )
        ?.focus();
    });
  }
</script>

<div
  {@attach move(value)}
  class="cs-seg"
  role="radiogroup"
  aria-label={label}
  onkeydown={onKeydown}
  tabindex="-1"
>
  <div class="cs-pill" aria-hidden="true"></div>
  {#each options as o (o.id)}
    <button
      data-val={o.id}
      class="cs-seg-btn"
      class:on={value === o.id}
      role="radio"
      aria-checked={value === o.id}
      /* Roving tabindex（ARIA APG 对 radiogroup 的硬要求）：
         整组只占**一个** Tab 位，方向键在其内部移动。
         每个按钮都 tabindex="0" 会让读屏用户同时拿到「Tab 和方向键
         两套导航」——那是已知反模式，因为 role=radio 宣告的是后者。 */
      tabindex={value === o.id ? 0 : -1}
      onclick={() => pick(o.id)}
    >
      {o.label}
    </button>
  {/each}
</div>

<style>
  .cs-seg {
    display: flex;
    position: relative;
    padding: 2px;
    border-radius: var(--cs-radius);
    /* ⚠ **必须不撑满整行。**
     *
     * 设置页是 `display: flex; flex-direction: column`，子项默认
     * `align-items: stretch`——不写这一条，槽会被拉成一条横贯面板的空杠，
     * 两个选项缩在左端，看着像进度条而不是分段控件。
     * （实测：窗口那组就是这样，档位那组因为父级有 `align-items:flex-start`
     *  才没露馅。） */
    align-self: flex-start;
    max-width: 100%;
    border-radius: var(--cs-radius);
    /* 轨道应该是**几乎看不见的槽**——分段的边界靠文字和那颗指示块。 */
    background: var(--cs-surface-sunken);
    border: 1px solid var(--cs-rule);
  }

  /* 滑动指示块。left/width 由 GSAP 写，这里不能给 transition——
     两个都动同一个属性会在补间结束瞬间抖一下。 */
  .cs-pill {
    position: absolute;
    top: 2px;
    bottom: 2px;
    left: 0;
    width: 0;
    border-radius: calc(var(--cs-radius) - 2px);
    /* ⚠ **墨底反白，不是红色，也不是淡灰。**
     *
     * 红色：这套色板里红只给不可逆（发送、退出），而这两组都是在改一个
     * 能改回来的设置。原来档位那块淡红是 `--cs-brass` 全局换成
     * `--cs-live` 时被顺手带过来的，纯替换副作用。
     *
     * 墨底：「当前停在哪一档 / 关闭窗口是退出还是收托盘」必须一眼可见，
     * 淡到 8% 的底撑不住这件事。 */
    background: var(--cs-ink);
    pointer-events: none;
  }

  .cs-seg-btn {
    position: relative;
    z-index: 1;
    /* ⚠ padding 按「总高 33 − 容器上下 6px」倒推，不是照抄独立按钮的
       padding——照抄会矮 4px。（曾经照抄过，量出来 29，越调越偏。） */
    padding: 0.281rem 0.75rem;
    font-size: 0.78rem;
    border: none;
    border-radius: calc(var(--cs-radius) - 2px);
    background: transparent;
    color: var(--cs-ink-2);
    transition: color var(--cs-dur) var(--cs-ease);
    white-space: nowrap;
  }

  .cs-seg-btn.on {
    color: var(--cs-surface-raised);
    font-weight: 600;
  }

  /* ── forced-colors：给选中态补一条**颜色管不着**的通道 ──────────
   *
   *  ⚠ 为什么需要（2026-10-14 读代码 + 探针实测）：
   *    「选中」这一状态现在只由两样东西表达——药丸的满铺墨底
   *    （`background: var(--cs-ink)`）和 `font-weight: 600`。
   *    而在 Windows 高对比度下，浏览器**接管作者的 background-color**，
   *    墨底那条通道会整个失效；12.5px 的 400 / 600 在中文里差得很轻。
   *    ⇒ 只剩字重扛「我被选中了」。
   *
   *  `text-decoration` 的**存在与粗细不是颜色属性**，系统不会抹掉它。
   *  所以下划线是这里唯一能确定穿过接管的那条通道。
   *  常态不画：平时墨底已经说清楚了，再加一道是噪声。
   */
  @media (forced-colors: active) {
    .cs-seg-btn.on {
      text-decoration: underline;
      text-decoration-thickness: 2px;
      text-underline-offset: 3px;
    }
  }
</style>
