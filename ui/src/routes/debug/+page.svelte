<script lang="ts">
  // 调试画布：把每个识别框按**归类结果**上色。
  //
  // 这不是美陈。它回答的是「为什么这条没进消息流」——
  // 六种颜色各代表一种归类结果，认错了看一眼就知道是哪一步丢的。
  // 只给「归类/丢弃」两种颜色是不够的：看到框消失还是变色，
  // 判断不出是「太小」还是「当成图片」。
  import { onMount } from 'svelte';

  interface Attribution {
    rect: { x0: number; y0: number; x1: number; y1: number };
    text: string;
    evidence: string;
    css: string;
    label: string;
    is_text: boolean;
  }
  interface Pane {
    chat: { x0: number; y0: number; x1: number; y1: number };
    width: number;
    height: number;
    boxes: Attribution[];
  }

  let pane = $state<Pane | null>(null);
  let err = $state<string | null>(null);
  /** 只看被丢弃的。**默认开着**——调试时最常问的就是「这条去哪了」。 */
  let onlyDropped = $state(true);
  let picked = $state<Attribution | null>(null);

  onMount(() => {
    let stop: (() => void) | null = null;
    let dead = false;
    void (async () => {
      const { invoke } = await import('@tauri-apps/api/core');
      const { listen } = await import('@tauri-apps/api/event');
      const pull = async () => {
        try {
          pane = await invoke<Pane>('debug_pane');
        } catch (e) {
          err = String(e);
        }
      };
      await pull();
      const un = await listen('debug_pane', () => {
        void pull();
      });
      if (dead) {
        un();
        return;
      }
      stop = un;
    })();
    return () => {
      dead = true;
      stop?.();
    };
  });

  const shown = $derived((pane?.boxes ?? []).filter((b) => !onlyDropped || !b.is_text));
  const counts = $derived(
    (pane?.boxes ?? []).reduce<Record<string, { n: number; css: string }>>((acc, b) => {
      const e = acc[b.label] ?? { n: 0, css: b.css };
      e.n += 1;
      acc[b.label] = e;
      return acc;
    }, {}),
  );
</script>

<main class="cs-dbg">
  <header class="cs-head">
    <h1>识别画布</h1>
    <label class="cs-toggle">
      <input type="checkbox" bind:checked={onlyDropped} />
      只看被丢弃的
    </label>
  </header>

  {#if err}
    <p class="cs-fail" role="alert">{err}</p>
  {/if}

  {#if !pane}
    <p class="cs-empty">等观察循环跑出第一帧…</p>
  {:else}
    <div class="cs-legend">
      {#each Object.entries(counts) as [label, v] (label)}
        <span class="cs-chip" style:--c={v.css}>
          <i></i>{label}
          {v.n}
        </span>
      {/each}
    </div>

    <div class="cs-canvas" style:aspect-ratio="{pane.width} / {pane.height}">
      {#each shown as b, i (i)}
        <button
          class="cs-box"
          style:left="{((b.rect.x0 - pane.chat.x0) / pane.width) * 100}%"
          style:top="{((b.rect.y0 - pane.chat.y0) / pane.height) * 100}%"
          style:width="{((b.rect.x1 - b.rect.x0) / pane.width) * 100}%"
          style:height="{((b.rect.y1 - b.rect.y0) / pane.height) * 100}%"
          style:--c={b.css}
          onclick={() => (picked = b)}
          title={b.evidence}
        >
          <span class="cs-tag">{b.label}</span>
        </button>
      {/each}
    </div>

    {#if picked}
      <aside class="cs-detail">
        <strong>{picked.label}</strong>
        <p class="cs-evidence">{picked.evidence}</p>
        <p class="cs-text selectable">{picked.text || '（没有文字）'}</p>
        <button class="cs-close" onclick={() => (picked = null)}>关掉</button>
      </aside>
    {/if}
  {/if}
</main>

<style>
  .cs-dbg {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    height: 100%;
    padding: 0.75rem;
    background: var(--cs-surface);
    color: var(--cs-ink);
  }

  .cs-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .cs-head h1 {
    margin: 0;
    font-size: 0.85rem;
    font-weight: 600;
  }

  .cs-toggle {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.7rem;
    color: var(--cs-ink-2);
  }

  .cs-legend {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }

  .cs-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.1rem 0.35rem;
    border-radius: 999px;
    border: 1px solid var(--cs-border);
    font-size: 0.65rem;
    color: var(--cs-ink-2);
  }

  .cs-chip i {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--c);
  }

  /* 画布用**棋盘格**底——纯色上看不出框的边界在哪 */
  .cs-canvas {
    position: relative;
    flex: 1;
    min-height: 0;
    border: 1px solid var(--cs-border-strong);
    border-radius: 6px;
    background-color: #101012;
    background-image:
      linear-gradient(45deg, #17171a 25%, transparent 25%),
      linear-gradient(-45deg, #17171a 25%, transparent 25%),
      linear-gradient(45deg, transparent 75%, #17171a 75%),
      linear-gradient(-45deg, transparent 75%, #17171a 75%);
    background-size: 16px 16px;
    background-position:
      0 0,
      0 8px,
      8px -8px,
      -8px 0;
  }

  .cs-box {
    position: absolute;
    padding: 0;
    border: 1.5px solid var(--c);
    background: color-mix(in srgb, var(--c) 18%, transparent);
    cursor: pointer;
    transition: background var(--cs-dur-fast) var(--cs-ease);
  }

  .cs-box:hover {
    background: color-mix(in srgb, var(--c) 40%, transparent);
  }

  .cs-tag {
    position: absolute;
    top: -1.5px;
    left: -1.5px;
    padding: 0 2px;
    font-size: 9px;
    line-height: 1.3;
    color: var(--c);
    background: #101012;
    white-space: nowrap;
  }

  .cs-detail {
    padding: 0.4rem 0.5rem;
    border: 1px solid var(--cs-border);
    border-radius: var(--cs-radius-sm);
    background: var(--cs-surface-raised);
    font-size: 0.7rem;
  }

  .cs-evidence {
    margin: 0.2rem 0;
    color: var(--cs-ink-2);
  }

  .cs-text {
    margin: 0.2rem 0;
  }

  .cs-close,
  .cs-empty,
  .cs-fail {
    font-size: 0.7rem;
    color: var(--cs-ink-3);
  }

  .cs-fail {
    color: var(--cs-alarm);
  }
</style>
