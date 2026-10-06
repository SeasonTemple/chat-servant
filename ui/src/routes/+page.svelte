<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import CandidateCard from '$lib/CandidateCard.svelte';
  import MessageStream from '$lib/MessageStream.svelte';
  import ReasonBar from '$lib/ReasonBar.svelte';
  import TierSwitch from '$lib/TierSwitch.svelte';
  import type { Candidate, DispatchReply, Message, Tier } from '$lib/types';

  /** Rust 侧推过来的消息流。**前端不自己算**——`Message[]` 是 Rust 判完发过来的。 */
  let messages = $state<Message[]>([]);
  let candidates = $state<Candidate[]>([]);
  let tier = $state<Tier>('Manual');
  let reason = $state('等待第一条消息');
  let busy = $state(false);
  /** 紧凑模式：面板窄于阈值时切。**不靠媒体查询**——
   *  窗口可以被用户拖到任意宽度，媒体查询只看视口。 */
  let compact = $state(false);

  /** 进入紧凑模式的宽度阈值。实测 320px 时三栏并排会挤成两行。 */
  const COMPACT_AT = 340;

  /**
   * 紧凑模式：面板窄于阈值时切。
   *
   * **不靠媒体查询**——窗口可以被用户拖到任意宽度，媒体查询只看视口，
   * 拖窄窗口时不会跟着变。用 `ResizeObserver` 量面板自己的宽度。
   *
   * 做成 attachment：`compact` 被写进 state，attachment 会在重跑前
   * 调清理函数断开旧的观察者，不会泄漏监听器。
   */
  function watchWidth(node: HTMLElement) {
    const ro = new ResizeObserver(([entry]) => {
      compact = entry.contentRect.width < COMPACT_AT;
    });
    ro.observe(node);
    return () => {
      ro.disconnect();
    };
  }

  const sorted = $derived(
    // 按概率降序。**不是**按生成顺序——模型的信心顺序才是有用的顺序。
    [...candidates].sort((a, b) => b.probability - a.probability),
  );
  const bestId = $derived(sorted[0]?.id ?? '');

  onMount(() => {
    // 一次 invoke 打通 Rust → 前端。**不轮询**：`readout` 是 Rust 推过来的。
    let dispose: (() => void) | null = null;
    let cancelled = false;

    void (async () => {
      const ping = await invoke<{ ok: boolean; anchor: string; detail: string }>('probe_anchor');
      if (cancelled) return;
      reason = ping.ok ? `锚点 ${ping.anchor}` : (ping.detail ?? '锚点推不出来');

      const { listen } = await import('@tauri-apps/api/event');
      const un = await listen<{
        messages: Message[];
        candidates: Candidate[];
        reason: string;
      }>('readout', (e) => {
        messages = e.payload.messages;
        candidates = e.payload.candidates ?? [];
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
      const r = await invoke<DispatchReply>('dispatch', {
        candidateId: c.id,
        action,
        tier,
      });
      reason = `${r.outcome}：${r.detail}`;
    } catch (e) {
      // Rust 侧返回的文本**已经脱敏**，可以直接显示。
      reason = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<main {@attach watchWidth} class="cs-panel" class:compact>
  <header class="cs-head">
    <span class="cs-title">副驾</span>
    <div class="cs-head-tools">
      <button
        class="cs-tool"
        title="打开识别画布：每个框按归类结果上色"
        onclick={() => invoke('open_debug').catch(() => {})}
      >
        画布
      </button>
      <button
        class="cs-tool"
        title="打开设置"
        onclick={() => invoke('open_settings').catch(() => {})}
      >
        设置
      </button>
      <TierSwitch bind:tier />
    </div>
  </header>

  <ReasonBar {reason} />

  <MessageStream {messages} />

  <section class="cs-cands" aria-label="候选回复">
    {#each sorted as c, i (c.id)}
      <CandidateCard
        {c}
        rank={i + 1}
        {busy}
        recommended={c.id === bestId}
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
    gap: 0.45rem;
    height: 100%;
    padding: 0.7rem;
    /* 半透明玻璃。底下是用户的聊天窗口，面板不能把它完全盖住。 */
    background: var(--cs-surface);
    backdrop-filter: blur(18px) saturate(1.2);
    border: 1px solid var(--cs-border);
    border-radius: var(--cs-radius);
    box-shadow: var(--cs-shadow);
    /* 面板可以被拖动——整个 main 都是拖动区，按钮自己排除。 */
    overflow: hidden;
  }

  .cs-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .cs-head-tools {
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }

  .cs-tool {
    padding: 0.15rem 0.4rem;
    font-size: 0.66rem;
    border-radius: 6px;
    border: 1px solid var(--cs-border);
    background: transparent;
    color: var(--cs-text-faint);
  }

  .cs-tool:hover {
    color: var(--cs-text);
    background: rgb(255 255 255 / 6%);
  }

  .cs-title {
    font-size: 0.8rem;
    letter-spacing: 0.08em;
    color: var(--cs-text-dim);
    white-space: nowrap;
  }

  .cs-cands {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    overflow-y: auto;
    scrollbar-width: thin;
    flex: 0 1 auto;
    min-height: 0;
  }

  .cs-empty {
    margin: 0;
    padding: 0.6rem 0.25rem;
    font-size: 0.72rem;
    color: var(--cs-text-faint);
    text-align: center;
  }

  /* 紧凑模式：面板窄的时候**收紧间距与字号**，
     而不是让文字挤成两行。挤成两行的可读性远低于更小但单行的字。 */
  .cs-panel.compact {
    padding: 0.5rem;
    gap: 0.3rem;
  }

  .cs-panel.compact .cs-title {
    font-size: 0.72rem;
  }
</style>
