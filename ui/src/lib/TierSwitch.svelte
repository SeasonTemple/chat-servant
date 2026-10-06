<script lang="ts">
  import { flashOnce, slideTo } from './attach';
  import { DUR, EASE, prefersReducedMotion } from './motion';
  import type { Tier } from './types';

  interface Props {
    tier: Tier;
  }

  let { tier = $bindable() }: Props = $props();

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
    tier = id;
    // Auto 是安全相关的档位，闪一次表示「我知道了」。
    // 不让用户以为「点了一下没反应」——那正是安全操作里最糟的体验。
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
  <!-- 档位是安全相关的决定，提示语必须一直可见，不能只在 tooltip 里。 -->
  <p class="cs-hint">{active.hint}</p>
</div>

<style>
  .cs-tier {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 0.15rem;
  }

  .cs-wrap {
    position: relative;
  }

  .cs-seg {
    display: flex;
    position: relative;
    padding: 2px;
    border-radius: 999px;
    background: rgb(0 0 0 / 28%);
    border: 1px solid var(--cs-border);
  }

  /* 滑动指示块。left/width 由 GSAP 写，这里不能给 transition——
     两个都动同一个属性会在补间结束瞬间抖一下。 */
  .cs-pill {
    position: absolute;
    top: 2px;
    bottom: 2px;
    left: 0;
    width: 0;
    border-radius: 999px;
    background: color-mix(in srgb, var(--cs-accent) 22%, transparent);
    pointer-events: none;
  }

  .cs-seg-btn {
    position: relative;
    z-index: 1;
    padding: 0.18rem 0.5rem;
    font-size: 0.66rem;
    border: none;
    border-radius: 999px;
    background: transparent;
    color: var(--cs-text-dim);
    transition: color var(--cs-dur) var(--cs-ease);
    white-space: nowrap;
  }

  .cs-seg-btn.on {
    color: var(--cs-accent);
  }

  /* 切到 Auto 时的红色警示闪。只闪一次且极快——
     用户要的是「我知道了」，不是动画欣赏。 */
  .cs-pulse {
    position: absolute;
    inset: -2px;
    border-radius: 999px;
    border: 1px solid var(--cs-danger);
    opacity: 0;
    pointer-events: none;
  }

  .cs-hint {
    margin: 0;
    font-size: 0.6rem;
    color: var(--cs-text-faint);
    max-width: 16rem;
    text-align: right;
    /* 窄面板下提示语要能换行，不能横向溢出 */
    overflow-wrap: anywhere;
  }
</style>
