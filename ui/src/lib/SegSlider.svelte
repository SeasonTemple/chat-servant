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
   * 规格**逐项对齐** `TierSwitch` 那一版（33px 总高 / 0.78rem / 圆角
   * `--cs-radius`）。这个尺寸早先是给主面板 400px 宽定的，现在主面板
   * 的档位已经搬到**纸底**（`6f10f9f`），那个理由不成立了。
   *
   * 选中态的载体**换过三次**，最后一次是 2026-10-16：
   *   实墨药丸（10-14 复核定）→ 墨线 + 底部短横（10-15）→ **红铅笔勾**。
   *   前两次的理由都保留在下面，但**它们已经不生效了** ——
   *   因为它们选错了载体：同一个应用里主面板档位行用的是手绘勾，
   *   设置屏用黑框 + 黑下划线，两处选中态读成两套 UI。
   *   用户 2026-10-16 原话：「有什么必要还需要加个黑色下划线来表示选中？」
   *   「最极端来说应该参照档位的设计来做这些按钮、切换的选中效果和展示？」
   *
   *  **别把下面 10-14 / 10-15 那两段当没审过的旧物再翻回去**，
   *     也**别因为它们写得详尽就以为它们赢过**——
   *     它们赢的是「选中必须一眼可见」这个结论，输的是「用黑框黑线来表达」。
   *   换载体之后这个结论照样成立：勾盖在字上，比下划线重得多。
   */
  import { onMount } from 'svelte';
  import type { Attachment } from 'svelte/attachments';
  import { slideTo } from './attach';
  import { DUR, prefersReducedMotion } from './motion';
  import { TICK_NESTED, TICK_VIEW } from '$lib/marks';
  import PencilDefs from '$lib/PencilDefs.svelte';
  // ⚠⚠ **这里曾经 import `TICK_NESTED_BIG` / `TICK_VIEW` 并在药丸里画勾**，
  //   2026-10-18 删掉了。理由见 `.cs-pill` 的注释与本文件留档段，
  //   **不是**「手绘不如墨洗」这种口味问题，是实测三张图都读不出字。
  //   ⇒ 主面板档位行的手绘勾在 `PaperPanel.svelte` 里，**没有被动过**。

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
      // ⚠ 10 = 撕边宽度，必须与下面 `.cs-pill` 的 `border` 一致。
      //   改了撕边忘了改这里 ⇒ 撕边啃在文字上（实测「只填入」只剩半只「只」）。
      10,
    );

  /**
   * **药丸是量出来的覆盖层，所以布局一变它就是陈的**（2026-10-15）。
   *
   * `slideTo` 读激活项的 `offsetLeft/offsetWidth` 写进药丸，写完就不再跟着。
   * attachment 只在 `value` 变化时重跑，而字体、容器宽度、探针帧注入
   * 都会在挂载**之后**改掉按钮宽度。
   *
   * ⇒ 这里补两条重量时机：字体集就绪、以及**每个按钮自身的尺寸**。
   *   只盯容器没用——容器宽度全程没变，变的只有按钮。
   *
   * **票 37 的偏差已定位并修掉，但不在这个组件里**（2026-10-15）。
   *   偏差不是「量错」：同一页面在真实 Chromium 里量，药丸与激活项
   *   逐像素相等（内联 `width: 61px; left: 2px` 对 `offsetWidth: 61`）。
   *   出图之所以偏窄，是**无头截图把 GSAP 那 240ms 的补间截在半路**——
   *   量法：`shoturl.ps1 -Scale 1`，数药丸**上边线那一行**（y=104）的
   *   墨线跨度，`--virtual-time-budget` 取 4000 / 15000 / 40000 时依次为
   *   36 / 36 / 46 px（随预算单调变长 = 补间在跑）。修法在 `attach.ts`：
   *   **第一次落位直接落位，不补间**（初值 0 从来不是「从哪来」）。
   *   下面这两条重量时机仍然留着——药丸是量出来的，这件事本来就该有
   *   它们，只是不该被当成本票的解。
   */
  const reflow: Attachment<HTMLElement> = (node) => {
    // 读一次 `value` 把它挂进依赖图：换档时这个 attachment 也会重跑。
    void value;
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => move(value));
    ro.observe(node);
    // 按钮是 `{#each}` 渲染的，attachment 跑的时候可能还没有子节点。
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
   * 焦点跟着走：`document.activeElement` 由浏览器维护，我们只用
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

<PencilDefs />

<div
  {@attach move(value)}
  {@attach reflow}
  class="cs-seg"
  role="radiogroup"
  aria-label={label}
  onkeydown={onKeydown}
  tabindex="-1"
>
  <div class="cs-pill" aria-hidden="true">
    <!--
      ⚠⚠ **红铅笔勾回来了**（2026-10-18，当天删、第二天加回）。
      删的理由是「56×34 的勾盖住 12.48px 的小字，字被吞」——
      那个判断**方向对、尺子用错**：它是拿**原尺寸**去量一枚
      本来就不该按原尺寸用的标记。
      ⇒ 现在缩到 **22×13.4**（保持 `TICK_VIEW` 的 56:34），
        并压在药丸**左端内侧**、只碰第一个字的一角。

      顺带把主面板那一支也统一到同一个缩放档（原来是 30×18）——
      **同一个概念必须同一支笔、同一把尺**，这是这一轮全部返工的根因。

      药丸本身仍由 GSAP 动 `left/width`，勾在药丸**内部**绝对定位，
      所以滑块走到哪，勾就跟到哪，不需要第二套状态。
    -->
    <svg class="cs-pill-tick" viewBox="0 0 {TICK_VIEW.w} {TICK_VIEW.h}" filter="url(#pencil)">
      {#each TICK_NESTED as layer (layer.d)}
        <path d={layer.d} fill-opacity={layer.o} />
      {/each}
    </svg>
  </div>
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
    /* **必须不撑满整行。**
     *
     * 设置页是 `display: flex; flex-direction: column`，子项默认
     * `align-items: stretch`——不写这一条，槽会被拉成一条横贯面板的空杠，
     * 两个选项缩在左端，看着像进度条而不是分段控件。
     * （实测：窗口那组就是这样，档位那组因为父级有 `align-items:flex-start`
     *  才没露馅。） */
    align-self: flex-start;
    max-width: 100%;
    border-radius: var(--cs-radius);
    /* **墨线槽，不是浅灰白 + 灰边框**（2026-10-15 真机指出后修）。
     *
     * 这个组件原先用 `--cs-surface-sunken` + `--cs-rule`，
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
    /***删掉了 `border: 1px solid var(--cs-ink)`**（2026-10-16）。
     *
     *   这圈纯黑框 + 药丸的黑内描边 + 药丸底部的黑下划线，
     *   是设置屏上**唯一**还停在 Web 按钮语言上的「选中」表达。
     *   用户 2026-10-16 的原话：「有什么必要还需要加个黑色下划线来表示选中？」
     *
     *   这三样**不是随手加的**，它们各自顶掉过一个反方案（见下面留档的
     *     2026-10-14 / 10-15 两段）。但它们被顶掉的理由是
     *     「实墨黑块量级与它表示的风险不匹配」——**结论对，选中的载体选错了**：
     *     同一屏里主面板用的是红铅笔勾，两处选中态于是读成两套 UI。
     *   ⇒ 保留「选中必须一眼可见」这个结论，换成与档位行同一支笔。
     */
    border: none;
  }

  /* 滑动指示块。left/width 由 GSAP 写，这里不能给 transition——
     两个都动同一个属性会在补间结束瞬间抖一下。

     ⚠⚠⚠ **它现在是撕边纸片，不是描边药丸**（2026-10-18）。
     实测打出来的（`target/probe/set-geom.mjs`）：

     | | `.cs-btn-mini`（按钮族） | 改前的 `.cs-pill` |
     |---|---|---|
     | `border-image` | `strip.png` | **none** |
     | `border-top-width` | 10px | 0 |
     | `box-shadow` | none | **`inset 0 0 0 1px`** |
     | 高 | 46 | 27.7 |

     ⇒ 那一块圆角描边是**这一屏唯一的 Web 控件**，用户红框指的就是它。
     ⇒ 现在与 `.cs-btn-mini` 逐项同源：同一张 `strip.png`、同一个
       `slice 32 / width 10px`、同一条落影（`drop-shadow` 而非 `box-shadow`
       ——后者会从撕口的透明区漏出矩形直边，见 `.cs-btn-mini` 那段的实测）。

     ⚠ `top/bottom: -10px` 与 `slideTo(..., 10)` 的 bleed **必须成对**：
       药丸的 border-box 要比激活项每边大 10px，内衬盒才与它重合；
       不成对就是撕边啃字（实测过一次：「只填入」只剩半只「只」）。 */
  .cs-pill {
    position: absolute;
    top: -10px;
    bottom: -10px;
    left: 0;
    width: 0;
    border: 10px solid transparent;
    border-image: url('/strip.png') 32 fill / 10px / 0 stretch;
    filter: drop-shadow(0 2px 4px rgb(24 18 10 / 0.16));
    pointer-events: none;
  }

  /* ── 以下是 2026-10-18 摘掉的「12% 墨洗 + 1px 暖墨描边」，留档 ──────
   *  它是被**材质**换掉的，不是被算掉的：那一层墨洗 + 描边是 Web 控件的
   *  语言（圆角矩形 + 1px 边），而同屏按钮是撕边纸片。
   *  2026-10-10 / 10-14 / 10-15 三轮算的混比（8% / 12% / 20% / 24%）
   *  全部作废——那几轮解决的是「墨洗多深才撑得住」，
   *  而真正的问题是「为什么这里要一个墨洗块」。
   *
   *  | 混比   | 底 vs 纸 | 选中字 vs 底 |
   *  |--------|----------|--------------|
   *  | 8%     | 1.163:1  | 12.13 : 1    | ← 2026-10-10 判掉的那个
   *  | 12%    | 1.258:1  | 11.22 : 1    | ← 当了三天选中态的载体
   *  | 20%    | 1.468:1  |  9.61 : 1    | ← 出图后退回（读成灰绿 chip）
   *  | 24%    | 1.599:1  |  8.82 : 1    |
   *
   *  ⚠ 那一轮也留下一条仍然有效的教训：**数字达标 ≠ 读起来对**
   *    （20% 数值最显眼，出图却读成一块灰绿 chip）。
   *  ⚠ 以及「红只给不可逆」——发送档位改的是能改回来的设置。
   *  ─────────────────────────────────────────────────────────── */

  /* 药丸里的那枚勾。⚠ 只有 fill，不设 fill-opacity 的兜底：
     五层嵌套的每层透明度是**笔迹数据**（`marks.js` 里每层自带 `o`），
     CSS 一兜底就把五层压成单层平涂，勾会立刻变成一个生硬的矢量对勾
     ——而这一族的整套材质靠的就是「芯重边虚」（`PaperPanel` 同款警告）。

     ⚠⚠⚠ **`translateX(-0.73em)` 已撤（2026-10-18 真机）**。
     那三个 em 常量是从主面板**一个样本**量出来的：`-0.73em` 在
     「只填入」这种三字标签上看着还行，换成别的长度就散架。
     真机实测（`target/probe/chip-tick-geom.mjs`，设置屏三个 SegSlider）：

     | 标签 | 字宽 | 勾心 − 字心 | 换算成字宽的倍数 |
     |---|---|---|---|
     | 只填入 | 37.5 | −12.9px | **−0.34** |
     | 退出   | 25.0 | −12.7px | **−0.51** |
     | 勾     | 12.5 | −12.9px | **−1.04** ← 勾整个落在字的左边 |

     ⇒ **那不是「一条关系」，是一个常量**。用户读成「勾选在文字最前面」
       说的就是它，而它正是从主面板抄来的。
     ⇒ 真正的关系只有一条：**勾心 = 字心**。`.cs-pill` 盖在激活项上，
       段内文字居中 ⇒ `left: 50%` **本来就是字心**，不需要再挪。

     ⚠⚠ **尺寸也不再用 em**（同一天实测）。`em` 各按自己继承到的字号解算，
       药丸继承 `.cs-seg` 的 1rem = 16px、芯片继承 11.5px
       ⇒ 同一屏里两个记号差了 1.39 倍（58.5 vs 42.1）。
       统一走 `--cs-stamp-w`（`app.css`，取景框宽 44 ⇒ 墨宽 37.3 ≈ 字宽）。 */
  .cs-pill-tick {
    position: absolute;
    left: 50%;
    top: 50%;
    /* 高由 56 : 34 的取景框比例推出，不另写一个数 */
    height: calc(var(--cs-stamp-w) * 34 / 56);
    width: var(--cs-stamp-w);
    transform: translate(-50%, -50%);
    overflow: visible;
  }

  .cs-pill-tick path {
    fill: var(--cs-pencil-red);
  }

  /* ── 2026-10-16 的红铅笔勾：2026-10-18 摘掉，留档 ────────────────
   *  当时的理由：「同一屏里主面板用的是红铅笔勾，两处选中态读成两套 UI」
   *  ⇒ 方向对，**尺子用错了地方**。实测（`target/probe/shot-seg.mjs`
   *  裁出的三张图，逐像素看过）：
   *
   *    1. `TICK_VIEW` 是 56×34，比整组（高 31.7）还高，字号只有 12.48px。
   *       「只填入」三字被盖到只剩半只「只」和一道划穿「入」的笔画；
   *       单字那组（「勾」/「圈」）**一个字都读不出来**。
   *    2. 缩到字高以下也不成立——`.cs-tick` 原注释自己量过：
   *       五层嵌套的羽化按 56 宽调，缩到 20 宽只剩一抹极淡的红。
   *       ⇒ **这一枚勾没有一个能用的尺寸**。
   *    3. 红色与「红只给不可逆」直接冲突（见下面 2026-10-14 那段）。
   *
   *  「盖章式」本身没被推翻：**主面板档位行的勾一个字没动**
   *  （`PaperPanel.svelte` 的 `.cs-pick-slot` / `.cs-tick`），
   *  那里文字是 20px 上下、勾有自己的取景框。差别是那里字大到盖得住。
   *  ─────────────────────────────────────────────────────────── */

  /* ── 2026-10-14 的原判断 ──────────────────────────────────────
   *  墨底：「当前停在哪一档 / 关闭窗口是退出还是收托盘」必须一眼可见，
   *  淡到 8% 的底撑不住这件事。
   *  红色：这套色板里红只给不可逆（发送、退出），而这两组都是在改一个
   *  能改回来的设置——原来档位那块淡红是 `--cs-brass` 全局换成
   *  `--cs-live` 时被顺手带过来的，纯替换副作用。
   *  真正该保留的是**「红色只给不可逆」**那半句。
   *
   *  ⚠ 2026-10-18 订正：这三条**到今天全部成立**，而当时只落地了第三条。
   *    「8% 撑不住」被读成「任何淡底都不行」，于是直接跳到黑块、再跳到
   *    红勾，中间那个档位没人算过——现在 `.cs-pill` 补上了
   *    （12% 墨洗 1.258 : 1 + `--cs-edge` 描边 3.04 : 1）。
   *  ─────────────────────────────────────────────────────────── */

  .cs-seg-btn {
    position: relative;
    z-index: 1;
    /* **不许被 flex 压扁**（2026-10-15）。
     *
     * 槽是 `display: flex`，按钮默认 `flex-shrink: 1`。容器一旦比两项的
     * 自然宽度窄，按钮就会被压到比文字还窄，而药丸量的是 `offsetWidth`
     * ——**药丸会忠实地把被压过的宽度再画一遍**，不会自己修正。
     *
     * 这条**不是票 37 那个偏差的成因**：加上 `flex: none` 前后出的图
     *   逐字节相同（修好之前那张是 22926 B），按钮并没有被压。留它是因为这个默认
     *   值本身就该关掉，而理由要连着「药丸是量出来的」这件事一起记。 */
    flex: none;
    /* padding 按「总高 33 − 容器上下 6px」倒推，不是照抄独立按钮的
       padding——照抄会矮 4px。（曾经照抄过，量出来 29，越调越偏。）

       ⚠⚠ 左右各多留 **10px = 撕边宽度**（2026-10-18）。
       按钮自己**没有**撕边（撕边在药丸上），而药丸的 border-box
       比按钮每边大 10px ⇒ 撕口正好落在按钮自己的左右 padding 里。
       实测第一次就这样：「只填入」的「只」字被撕口啃掉半边。
       ⇒ 按钮的文字必须**缩进到撕边内侧**，撕口才落在字外。
       ⚠ 这个 10 与 `.cs-pill` 的 `border-width` 和 `slideTo(..., 10)`
         是**同一个数**，三处必须一起改。 */
    padding: 0.281rem calc(0.75rem + 10px);
    font-size: 0.78rem;
    border: none;
    border-radius: calc(var(--cs-radius) - 2px);
    background: transparent;
    color: var(--cs-ink-2);
    transition: color var(--cs-dur) var(--cs-ease);
    white-space: nowrap;
  }

  .cs-seg-btn.on {
    /* 墨色正文而不是反白——没有满铺底了，「反白」会读成文字缺失。 */
    color: var(--cs-ink);
    font-weight: 600;
  }

  /* ── forced-colors：给选中态补一条**颜色管不着**的通道 ──────────
   *
   *  为什么需要（2026-10-14 读代码 + 探针实测）：
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
      /* 这一圈描边是**备份通道**（2026-10-18 订正）。
       *   主通道现在是常态那条墨洗底 + `.cs-seg-btn.on` 的下划线；
       *   而 HC 恰好接管 `background-color`，所以常态那条在这里**直接失效**，
       *   真正扛住的是 `.cs-seg-btn.on` 那道 `text-decoration`
       *   （存在与粗细都不是颜色属性）。药丸这圈描边留作第二道，
       *   因为描边是几何、系统不连坐。 */
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
