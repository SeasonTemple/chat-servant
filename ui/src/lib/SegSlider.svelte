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
   * ⚠ **选中态 2026-10-15 从「实墨药丸」改成「墨线 + 底部短横」**，
   *   与动作行的墨线按钮族（`docs/设计规范.md` §4.2c）统一。
   *   原判断与推翻它的理由都留在 `.cs-pill` 的注释里——
   *   **别把它当没审过的旧物再翻回去**。
   */
  import { onMount } from 'svelte';
  import type { Attachment } from 'svelte/attachments';
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

  /**
   * ⚠ **药丸是量出来的覆盖层，所以布局一变它就是陈的**（2026-10-15）。
   *
   * `slideTo` 读激活项的 `offsetLeft/offsetWidth` 写进药丸，写完就不再跟着。
   * attachment 只在 `value` 变化时重跑，而字体、容器宽度、探针帧注入
   * 都会在挂载**之后**改掉按钮宽度。
   *
   * ⇒ 这里补两条重量时机：字体集就绪、以及**每个按钮自身的尺寸**。
   *   只盯容器没用——容器宽度全程没变，变的只有按钮。
   *
   * ⚠⚠ **这两条并没有修掉实测到的那个偏差，别当成已解决。**
   *   加上它们前后，`shoturl.ps1 -Scale 1` 出的图**字节数一模一样**
   *   （22926 B），药丸仍然只有 36px 而「只填入」的自然宽度约 55~61px，
   *   最后一个字挂在框外。
   *   · 已排除：`flex` 收缩（加 `flex: none` 后字节数不变）
   *   · 已排除：字体就绪时机（`document.fonts.ready` 后不变）
   *   · **在应用内浏览器里复现不出来**：那里量到药丸 rect 与激活项 rect
   *     **逐像素相等**（只填入 61/61、勾·圈 36/36、退出 49/49），
   *     而那边视口 671、这边 520 ⇒ 差异随视口宽度变。
   *   ⇒ 下一手该查的是「什么在窄视口下把 seg 那一行排窄了」，
   *     不是继续在这个组件里加钩子。已开票跟踪。
   */
  const reflow: Attachment<HTMLElement> = (node) => {
    // 读一次 `value` 把它挂进依赖图：换档时这个 attachment 也会重跑。
    void value;
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => move(value));
    ro.observe(node);
    // ⚠ 按钮是 `{#each}` 渲染的，attachment 跑的时候可能还没有子节点。
    //   补一帧再盯一次，否则第一屏量到的是「空槽」。
    const raf = requestAnimationFrame(() => {
      for (const btn of node.querySelectorAll('.cs-seg-btn')) ro.observe(btn);
    });
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
    };
  };

  onMount(() => {
    void document.fonts?.ready.then(() => move(value));
  });

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
  {@attach reflow}
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
    /* ⚠ **墨线槽，不是浅灰白 + 灰边框**（2026-10-15 真机指出后修）。
     *
     * ⚠ 这个组件原先用 `--cs-surface-sunken` + `--cs-rule`，
     *   而 `PanelSettings.svelte` 顶部明写着「独立设置窗口用的 `--cs-surface*`
     *   **这里不能用**」——设置页在那一轮已经改到同一张纸，
     *   **唯独这个被复用的组件漏了**，于是三分之二的控件还停在旧皮肤上，
     *   读成「同一个应用里塞了两套 UI」。
     *
     * 现在与动作行的「墨线」按钮族（`docs/设计规范.md` §4.2c）同一套语言：
     * 透明底 + 1px 墨描边，槽不再自己装成一张白卡。
     * 选中的那颗仍然是墨底药丸，在墨线槽里读作「墨压在墨线上」——
     * 那正是「当前停在哪一档」该有的重量。 */
    background: transparent;
    border: 1px solid var(--cs-ink);
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
    /* ⚠⚠ **不再满铺墨底**（2026-10-15 改，推翻下面那段 2026-10-14 的复核）。
     *
     *  为什么翻：那一轮把「墨底」当成**已审过的**决定留着，理由是
     *  「当前停在哪一档必须一眼可见，淡到 8% 的底撑不住」。但**同一天
     *  用户在动作行上明确否掉了实墨黑块**（原话「你就如此热爱？」），
     *  理由是量级与它表示的风险不匹配。同一个应用里两处选中态
     *  一处墨线竖笔、一处实墨黑块，读成两套 UI。
     *  ⇒ 统一到 §4.2c 的墨线语言，**不牺牲可见度**：
     *    墨线内描边（几何，不是色块）+ 底部一道短横（划重点）+ 字重 600。
     *
     *  用 `box-shadow` 而不是 `border`：药丸的 left/width 由 GSAP 写，
     *  `border` 会在动画期间把内容挤动 1px。
     *
     *  ⚠ 2026-10-14 那段复核保留在下面，不删——它记的是**为什么曾经
     *    是实墨**，而删掉它就会有人把它当没审过的旧物再翻回去。
     */
    background: transparent;
    box-shadow: inset 0 0 0 1px var(--cs-ink);
    pointer-events: none;
  }

  /* 底部短横：与动作行的「竖笔」同一个词汇——**划重点，不是铺满**。
     常态可见，因为高对比度下 `box-shadow` 会被系统抹掉。 */
  .cs-pill::after {
    content: '';
    position: absolute;
    left: 28%;
    right: 28%;
    bottom: 3px;
    height: 2px;
    border-radius: 1px;
    background: var(--cs-ink);
  }

  /* ── 2026-10-14 的原判断（已被上面推翻，留档） ──────────────────
   *  墨底：「当前停在哪一档 / 关闭窗口是退出还是收托盘」必须一眼可见，
   *  淡到 8% 的底撑不住这件事。
   *  红色：这套色板里红只给不可逆（发送、退出），而这两组都是在改一个
   *  能改回来的设置——原来档位那块淡红是 `--cs-brass` 全局换成
   *  `--cs-live` 时被顺手带过来的，纯替换副作用。
   *  ⚠ 真正该保留的是**「红色只给不可逆」**那半句。
   * ─────────────────────────────────────────────────────────── */

  .cs-seg-btn {
    position: relative;
    z-index: 1;
    /* ⚠ **不许被 flex 压扁**（2026-10-15）。
     *
     * 槽是 `display: flex`，按钮默认 `flex-shrink: 1`。容器一旦比两项的
     * 自然宽度窄，按钮就会被压到比文字还窄，而药丸量的是 `offsetWidth`
     * ——**药丸会忠实地把被压过的宽度再画一遍**，不会自己修正。
     *
     * ⚠ 这条**不是当前那个偏差的成因**：加上 `flex: none` 前后出的图
     *   字节数完全一样（22926 B），按钮并没有被压。留它是因为这个默认
     *   值本身就该关掉，而理由要连着「药丸是量出来的」这件事一起记。 */
    flex: none;
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
    /* ⚠ 墨色正文而不是反白——没有满铺底了，「反白」会读成文字缺失。 */
    color: var(--cs-ink);
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
    /*  药丸**不能靠满铺底**在 HC 下表达选中。
     *
     *  实测（2026-10-14，高对比度 #1 深色）：
     *    `background: var(--cs-ink)` → `CanvasText` = **白**，
     *    于是药丸成了一块白底；而文字色也指向系统前景色，
     *    文字与药丸底实测**对比度只有 1.67 : 1**，整个选中项读不出来。
     *    同一屏里未选中项与标题都是 15.13 : 1，所以这不是主题的问题，
     *    是**满铺底 + 同源前景色**这个组合撞了。
     *
     *  修法：底退回 `Canvas`（与页面同色，即"不强调"），
     *  强调交给**描边**——描边是几何不是色块，系统不会把它连坐。
     *  不需要 `forced-color-adjust: none`：这里用的已经是系统色本身，
     *  让浏览器照常接管即可。
     */
    .cs-pill {
      background: Canvas;
      border: 1px solid CanvasText;
    }

    .cs-seg-btn.on {
      color: CanvasText;
      text-decoration: underline;
      text-decoration-thickness: 2px;
      text-underline-offset: 3px;
    }
  }
</style>
