<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import CandidateCard from '$lib/CandidateCard.svelte';
  import TierSwitch from '$lib/TierSwitch.svelte';
  import type { Candidate, Message, Tier } from '$lib/types';

  /** Rust 侧推过来的消息流。**前端不自己算**——`Message[]` 是 Rust 判完发过来的。 */
  let messages = $state<Message[]>([]);
  let candidates = $state<Candidate[]>([]);
  let tier = $state<Tier>('Manual');
  let reason = $state('等待第一条消息');
  let busy = $state(false);

  onMount(() => {
    // 一次 invoke 打通 Rust → 前端。这是 ticket 12 的验收点之一。
    // **不轮询**：`readout` 是 Rust 推过来的，不是前端去问的。
    let dispose: (() => void) | null = null;
    let cancelled = false;

    void (async () => {
      const ping = await invoke<{ ok: boolean; anchor: string; detail: string }>('probe_anchor');
      if (cancelled) return;
      reason = ping.ok ? `锚点 ${ping.anchor}` : (ping.detail ?? '锚点推不出来');

      const { listen } = await import('@tauri-apps/api/event');
      const un = await listen<{ messages: Message[]; reason: string }>('readout', (e) => {
        messages = e.payload.messages;
        reason = e.payload.reason;
      });
      if (cancelled) {
        un();
        return;
      }
      dispose = un;
    })();

    return () => {
      cancelled = true;
      dispose?.();
    };
  });

  async function act(c: Candidate, action: 'fill' | 'confirm') {
    busy = true;
    try {
      // 回执**不是布尔值**。PossiblySent 之后禁止自动重放。
      const r = await invoke<{ outcome: string; detail: string }>('dispatch', {
        candidateId: c.id,
        action,
        tier,
      });
      reason = `${r.outcome}：${r.detail}`;
    } finally {
      busy = false;
    }
  }
</script>

<main class="cs-panel">
  <header class="cs-head">
    <span class="cs-title">副驾</span>
    <TierSwitch bind:tier />
  </header>

  <p class="cs-reason" title="Rust 侧每一步都会给出理由">{reason}</p>

  <section class="cs-stream" aria-label="对话">
    {#each messages as m (m.id)}
      <article class="cs-msg" class:me={m.speaker === 'Me'}>
        <span class="cs-who">{m.speaker === 'Me' ? '我' : '对方'}</span>
        <p class="cs-text selectable">{m.text}</p>
      </article>
    {:else}
      <p class="cs-empty">还没有消息。窗口静止时这里不会刷新。</p>
    {/each}
  </section>

  <section class="cs-cands" aria-label="候选回复">
    {#each candidates as c, i (c.id)}
      <CandidateCard
        {c}
        rank={i + 1}
        {busy}
        onfill={() => act(c, 'fill')}
        onconfirm={() => act(c, 'confirm')}
      />
    {:else}
      <p class="cs-empty">没有新消息时不会起草候选。</p>
    {/each}
  </section>
</main>

<style>
  .cs-panel {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    height: 100%;
    padding: 0.75rem;
    /* 半透明玻璃。底下是用户的聊天窗口，面板不能把它完全盖住。 */
    background: var(--cs-surface);
    backdrop-filter: blur(18px) saturate(1.2);
    border: 1px solid var(--cs-border);
    border-radius: var(--cs-radius);
    box-shadow: var(--cs-shadow);
  }

  .cs-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .cs-title {
    font-size: 0.8rem;
    letter-spacing: 0.08em;
    color: var(--cs-text-dim);
  }

  .cs-reason {
    margin: 0;
    font-size: 0.7rem;
    color: var(--cs-text-faint);
    /* 理由可能很长，限制一行并给 tooltip */
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .cs-stream,
  .cs-cands {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    overflow-y: auto;
    scrollbar-width: thin;
  }

  .cs-stream {
    flex: 1 1 auto;
    min-height: 3rem;
  }

  .cs-msg {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    padding: 0.4rem 0.55rem;
    border-radius: var(--cs-radius-sm);
    background: var(--cs-surface-raised);
    /* 对方在左、我方在右——和聊天软件的习惯一致，省一次心算 */
    border-left: 2px solid transparent;
  }

  .cs-msg.me {
    border-left-color: var(--cs-me);
  }

  .cs-who {
    font-size: 0.62rem;
    color: var(--cs-text-faint);
  }

  .cs-text {
    margin: 0;
    font-size: 0.78rem;
    line-height: 1.5;
  }

  .cs-empty {
    margin: 0;
    padding: 0.75rem 0.25rem;
    font-size: 0.72rem;
    color: var(--cs-text-faint);
    text-align: center;
  }
</style>
