<script lang="ts">
  import { flashOnce } from './attach';
  import SegSlider from './SegSlider.svelte';
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
    { id: 'Auto', label: '自动发送', hint: '点一下就填入并发送，不再二次确认。' },
  ];

  /** 给 `SegSlider` 的只有 id + label；hint 由下面的 `<p>` 单独渲染。 */
  const OPTIONS = TIERS.map((t) => ({ id: t.id, label: t.label }));

  const active = $derived(TIERS.find((t) => t.id === tier) ?? TIERS[0]);

  /**
   * 切到**最高档**时的一次性警示闪。dep=tier 保证只在切档时闪，不在挂载时闪。
   *
   * ⚠ 2026-10-15 从 `Confirm` 改挂回 `Auto`：砍掉二次确认那一档之后，
   * `Auto` 是唯一「会把话发出去」的档，**警示该跟着最高档走**，
   * 而不是跟着某个具体档名走。删掉它等于取消升档提示，那是另一回事。
   */
  const flashPulse = (dep: Tier) =>
    flashOnce(dep, {
      from: { opacity: 0.9 },
      to: { opacity: 0 },
      duration: prefersReducedMotion() ? 0 : DUR.slow,
      ease: EASE.out,
    });
</script>

<div class="cs-tier">
  <div class="cs-wrap">
    <SegSlider
      bind:value={tier}
      label="发送档位"
      options={OPTIONS}
      onchange={(v) => onchange?.(v as Tier)}
    />
    <!-- 警示闪只在**最高档**（Auto）时存在。dep 传 tier：挂载时 tier 已是 Auto，
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

  /* 切到最高档时的红色警示闪。只闪一次且极快——
     用户要的是「我知道了」，不是动画欣赏。
     `inset: -2px` 贴着 SegSlider 那圈 1px 边框 + 2px padding。 */
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
