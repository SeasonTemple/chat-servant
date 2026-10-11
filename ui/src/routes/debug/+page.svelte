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
    /**
     * 这一帧的开销账。**全部由 Rust 算好**，前端只显示——
     * 那个 `roiPct` 尤其不能自己除：它是「增量门控有没有在起作用」的判据。
     */
    cost: {
      ocrPixels: number;
      roiPixels: number | null;
      panePixels: number;
      ocrCalls: number;
      visionTokens: number;
      roiPct: number | null;
    };
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
      // ⚠ **整个链路包在 try 里。**
      //
      // `listen` 在没有 Tauri IPC 的环境里（浏览器预览、静态部署）
      // 读 `window.__TAURI_INTERNALS__.transformCallback` 会拿到 undefined，
      // 于是抛出一个**没人接的 Promise 拒绝**——组件卸载了、日志里也
      // 没有，看起来就像「这个窗口没反应」。
      //
      // `pull` 自己的 catch 救不了它：崩在后面这一步。
      let un: (() => void) | null = null;
      try {
        un = await listen('debug_pane', () => {
          void pull();
        });
      } catch (e) {
        err = String(e);
      }
      if (dead) {
        un?.();
        return;
      }
      stop = un;
    })();
    return () => {
      dead = true;
      stop?.();
    };
  });

  /** 万 → 「123.4 万」。像素数到百万级以后，裸数字没人读得出来。 */
  function wan(n: number): string {
    return n >= 10000 ? `${(n / 10000).toFixed(1)} 万` : String(n);
  }

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
    <!--
      ① 这一帧的开销账（user story 13）。
         ⚠ 三个像素数**不是一回事**，标题上写死了，别让人把它们当同一个数：
           · 「送进 OCR」= 放大后的**整帧**。恒是整帧，因为 WinRT OCR
             需要行周围有内容才收，裁 ROI 反而漏行（2026-10-07 实测）。
             所以**调滑块省不了 OCR**，省 OCR 只能靠「这一帧什么都没做」。
           · 「增量带」= 水位线以下那一段，也就是「哪些行算数」。
         把它们分开写，用户才知道自己该动哪一个。
    -->
    <section class="cs-cost" aria-label="这一帧的开销">
      <div class="cs-cost-row">
        <span class="cs-cost-k">这一帧</span>
        {#if pane.cost.ocrCalls === 0}
          <!--
            静默期。**单独一种说法**——它不是「读了 0 行」，
            是「这一帧一行都没读」。这是硬约束第 6 条唯一可信的证据。
          -->
          <strong class="cs-cost-quiet">什么都没读</strong>
          <span class="cs-cost-note">画面没变，0 次 OCR、0 token</span>
        {:else}
          <strong>{pane.cost.ocrCalls} 次 OCR</strong>
          <span class="cs-cost-note">
            送进 OCR {wan(pane.cost.ocrPixels)} 像素（放大后的整帧）
            {#if pane.cost.roiPct !== null}
              · 增量带占消息区 {pane.cost.roiPct.toFixed(0)}%
            {/if}
            · 视觉 {pane.cost.visionTokens} token
          </span>
        {/if}
      </div>
    </section>

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

  /* 开销账。等宽——这是机器读出来的数，字体分工和面板那边一致。 */
  .cs-cost {
    padding: 0.35rem 0.5rem;
    border: 1px solid var(--cs-border);
    border-radius: var(--cs-radius-sm);
    background: var(--cs-surface-raised);
  }

  .cs-cost-row {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 0.4rem;
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 0.68rem;
  }

  .cs-cost-k {
    color: var(--cs-ink-3);
  }

  .cs-cost-quiet {
    /* 静默期用铅笔色，不用绿。绿留给「有东西出去了」那一类信号。 */
    color: var(--cs-ink-2);
  }

  .cs-cost-note {
    color: var(--cs-ink-3);
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
