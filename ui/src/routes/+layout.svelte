<script lang="ts">
  import '../app.css';
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';

  let { children } = $props();

  /** Rust 侧推过来的帧计数——用来证明「不是轮询」。 */
  let framesSeen = $state(0);
  let lastError = $state<string | null>(null);

  onMount(() => {
    // 只监听，不轮询。Rust 那边什么时候 emit，前端就什么时候更新。
    let dispose: (() => void) | null = null;
    let cancelled = false;

    void (async () => {
      const un = await listen<{ frames: number }>('telemetry', (e) => {
        framesSeen = e.payload.frames;
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
</script>

{@render children?.()}

{#if lastError}
  <div class="cs-error" role="alert">{lastError}</div>
{/if}

<!-- 静默期指示。Rust 说「这一帧没有任何新增」，前端就只是把点熄掉。 -->
<div class="cs-telemetry" aria-live="off">
  <span class="cs-dot" class:idle={framesSeen === 0}></span>
  <span class="cs-telemetry-text">{framesSeen}</span>
</div>

<style>
  .cs-error {
    position: fixed;
    inset: auto 0 0 0;
    padding: 0.5rem 0.75rem;
    background: color-mix(in srgb, var(--cs-danger) 22%, transparent);
    color: var(--cs-text);
    font-size: 0.75rem;
  }

  .cs-telemetry {
    position: fixed;
    top: 0.5rem;
    right: 0.5rem;
    display: flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.2rem 0.45rem;
    border-radius: 999px;
    background: var(--cs-surface);
    border: 1px solid var(--cs-border);
    font-size: 0.7rem;
    color: var(--cs-text-faint);
    pointer-events: none;
  }

  .cs-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--cs-accent);
    box-shadow: 0 0 6px var(--cs-accent);
  }

  /* 静默期：点变暗。这是最直接的用户可观察证据——
     「没有新消息时它就不动」。 */
  .cs-dot.idle {
    background: var(--cs-text-faint);
    box-shadow: none;
  }

  .cs-telemetry-text {
    font-variant-numeric: tabular-nums;
  }
</style>
