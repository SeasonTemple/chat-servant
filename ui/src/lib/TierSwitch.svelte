<script lang="ts">
  import { flashOnce, slideTo } from './attach';
  import { DUR, EASE, prefersReducedMotion } from './motion';
  import type { Tier } from './types';

  interface Props {
    /**
     * 档位。**值由 Rust 持有**，这里只负责显示。
     *
     * 父组件把 `onchange` 接到 `set_tier`，再用 Rust 回传的真实生效值
     * 更新它——所以界面上显示的一律是**程序认定的档位**，
     * 不是用户记得自己点过什么。
     */
    tier: Tier;
    /** 请求改档。**前端只负责请求**，升档这件事由 Rust 裁决。 */
    onchange?: (t: Tier) => void;
  }

  let { tier = $bindable(), onchange }: Props = $props();

  const TIERS: { id: Tier; label: string; hint: string }[] = [
    { id: 'Manual', label: '只填入', hint: '默认。填进输入框，停在光标处，不发送。' },
    { id: 'Confirm', label: '确认后发', hint: '填入后还要你再确认一次才发送。' },
    { id: 'Auto', label: '自动发送', hint: '填入即发送，不打断。每条都可见可打断。' },
  ];

  const active = $derived(TIERS.find((t) => t.id === tier) ?? TIERS[0]);

  /**
   * 指示块「滑过去」而不是「跳过去」。
   *
   * attachment 跑在 effect 里，`tier` 被读进依赖 → 切档就自动重跑，
   * 不需要手写 `update()`，也不需要「记住上次 tier」的状态。
   */
  const move = (dep: Tier) =>
    slideTo(
      // 把「找指示块与激活项」从 attachment 里拆出来，attachment 只负责
      // 「量到了就滑过去」。
      (scope: HTMLElement) => {
        const btn = scope.querySelector(`[data-tier="${dep}"]`) as HTMLElement | null;
        const pill = scope.querySelector('.cs-pill') as HTMLElement | null;
        return btn && pill ? { pill, active: btn } : null;
      },
      prefersReducedMotion() ? 0 : DUR.base,
    );

  /** 切到 Auto 时的一次性警示闪。dep=tier 保证只在切档时闪，不在挂载时闪。 */
  const flashPulse = (dep: Tier) =>
    flashOnce(dep, {
      from: { opacity: 0.9 },
      to: { opacity: 0 },
      duration: prefersReducedMotion() ? 0 : DUR.slow,
      ease: EASE.out,
    });

  function pick(id: Tier) {
    if (id === tier) return;
    // 只改本地显示，**由父组件回填 Rust 的真实生效值**。
    // Rust 若把值夹回 Manual（比如解析失败），界面就会显示 Manual——
    // 那才是程序真正的档位，而不是用户以为自己选上的那个。
    tier = id;
    onchange?.(id);
  }
</script>

<div class="cs-tier">
  <div class="cs-wrap">
    <div {@attach move(tier)} class="cs-seg" role="radiogroup" aria-label="发送档位">
      <div class="cs-pill" aria-hidden="true"></div>
      {#each TIERS as t (t.id)}
        <button
          data-tier={t.id}
          class="cs-seg-btn"
          class:on={tier === t.id}
          role="radio"
          aria-checked={tier === t.id}
          title={t.hint}
          onclick={() => pick(t.id)}
        >
          {t.label}
        </button>
      {/each}
    </div>
    <!-- 警示闪只在 Auto 档时存在。dep 传 tier：挂载时 tier 已是 Auto，
         闪一次；之后切走再切回来，tier 变了，attachment 重跑又闪一次。 -->
    {#if tier === 'Auto'}
      <div {@attach flashPulse(tier)} class="cs-pulse" aria-hidden="true"></div>
    {/if}
  </div>
  <!--
    档位是安全相关的决定，所以这句话**必须一直可见**，不能塞进 tooltip。
    （tooltip 要悬停、要等、要会消失；这是「按下去会发生什么」的说明。）

    但它原来 0.6rem、右对齐、缩在右下角，像脚注 —— 而它讲的是**后果**。
    后果不该是脚注。跟正文一个字号档，跟正文一个左边线。
  -->
  <p class="cs-hint">{active.hint}</p>
</div>

<style>
  /* ⚠ **靠左，不靠右。**
   *
   * 整组是 `align-items: flex-end`，于是控件和它的说明都缩在右边 ——
   * 而设置页每一项的标题、控件、说明**都是从同一条左边线起的**。
   * 这一项单独飘到右边，读起来像另外一栏。
   */
  .cs-tier {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.15rem;
  }

  .cs-wrap {
    position: relative;
  }

  /* ── 规格：与设置页的 `.cs-seg-item` 逐项一致 ─────────────
   *
   * ⚠ 原来是 48 × 21 / 10.56px / 圆角 999px，设置页那个是
   *   101 × 33 / 12.5px / 圆角 5px —— **同一页两组分段控件两种
   *   规格**。量出来才发现的，不是看出来的。
   *
   * 这个尺寸早先是给主面板定的：400px 宽、必须省地方。
   * 现在它**只剩设置页在用**（主面板的档位搬到顶边了），
   * 那个理由不成立了。21px 高连鼠标都难瞄。
   */
  .cs-seg {
    display: flex;
    position: relative;
    padding: 2px;
    border-radius: var(--cs-radius);
    /* 28% 黑在白底上是一片中灰，整颗控件看着像默认样式。
     * 轨道应该是**几乎看不见的槽**——分段的边界靠文字和那颗指示块。 */
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
     * 红色：整套色板里红只给不可逆（发送、退出），而切档位是在改一个
     * 设置 —— 按错了也能改回来。原来那块淡红是 `--cs-brass` 全局换成
     * `--cs-live` 时被顺手带过来的，纯替换副作用。
     *
     * 墨底：档位是主面板上最重要的控件之一（硬约束第 5 条「默认档最低」），
     * 当前停在哪一档必须一眼可见。淡到 8% 的底撑不住这件事，
     * 而**设置页的「关闭时」用同一套** —— 两个都是「改一个设置」，
     * 一个重一个轻才是真问题。 */
    background: var(--cs-ink);
    pointer-events: none;
  }

  .cs-seg-btn {
    position: relative;
    z-index: 1;
    /* ⚠ **对齐的是「整组控件的总高 33px」，不是这个按钮自己 33px。**
     *
     * 这个控件是容器包着成员：容器 2px padding + 1px border 上下共吃 6px，
     * 成员撑满剩下的。所以成员的 padding 要按「33 − 6」倒推，
     * 不是照抄独立按钮的 padding—— 照抄会矮 4px，量出来 29。
     * （我就是这么错的：先照抄，量出 29，再瞎调成 28.875，越调越偏。
     * 该做的是先量容器盒模型。）
     */
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

  /* 切到 Auto 时的红色警示闪。只闪一次且极快——
     用户要的是「我知道了」，不是动画欣赏。 */
  .cs-pulse {
    position: absolute;
    inset: -2px;
    border-radius: var(--cs-radius);
    border: 1px solid var(--cs-alarm);
    opacity: 0;
    pointer-events: none;
  }

  .cs-hint {
    /* 后果说明用**正文档**（6.22 : 1），不是标注档。
     * 它要读，读不清等于没说。 */
    margin: 0.1rem 0 0;
    font-size: 0.72rem;
    line-height: 1.5;
    color: var(--cs-pencil);
    max-width: 34rem;
    text-align: left;
    /* 左边线：这是「后果」，不是脚注 */
    padding-left: 0.55rem;
    border-left: 2px solid var(--cs-rule);
    /* 窄面板下提示语要能换行，不能横向溢出 */
    overflow-wrap: anywhere;
  }
</style>
