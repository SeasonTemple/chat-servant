<script lang="ts">
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
</script>

<div class="cs-tier" role="group" aria-label="发送档位">
  <div class="cs-seg" role="radiogroup" aria-label="发送档位">
    {#each TIERS as t (t.id)}
      <button
        class="cs-seg-btn"
        class:on={tier === t.id}
        role="radio"
        aria-checked={tier === t.id}
        title={t.hint}
        onclick={() => (tier = t.id)}
      >
        {t.label}
      </button>
    {/each}
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

  .cs-seg {
    display: flex;
    padding: 2px;
    border-radius: 999px;
    background: rgb(0 0 0 / 28%);
    border: 1px solid var(--cs-border);
  }

  .cs-seg-btn {
    padding: 0.18rem 0.5rem;
    font-size: 0.66rem;
    border: none;
    border-radius: 999px;
    background: transparent;
    color: var(--cs-text-dim);
    transition:
      background var(--cs-dur) var(--cs-ease),
      color var(--cs-dur) var(--cs-ease);
  }

  .cs-seg-btn.on {
    background: color-mix(in srgb, var(--cs-accent) 22%, transparent);
    color: var(--cs-accent);
  }

  .cs-hint {
    margin: 0;
    font-size: 0.6rem;
    color: var(--cs-text-faint);
    max-width: 15rem;
    text-align: right;
  }
</style>
