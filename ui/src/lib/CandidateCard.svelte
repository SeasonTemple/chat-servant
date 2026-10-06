<script lang="ts">
  import type { Candidate } from './types';

  interface Props {
    c: Candidate;
    rank: number;
    busy: boolean;
    onfill: () => void;
    onconfirm: () => void;
  }

  const { c, rank, busy, onfill, onconfirm }: Props = $props();

  // 宽度直接就是概率：概率高的候选卡片更长，一眼能看出模型有多确定。
  // 不写死像素——用百分比，窗口随便拉。
  let pct = $derived(Math.round(Math.max(0, Math.min(1, c.probability)) * 100));
</script>

<article class="cs-cand" style:--pct={pct}>
  <div class="cs-bar" aria-hidden="true"></div>
  <div class="cs-body">
    <header class="cs-meta">
      <span class="cs-rank">候选 {rank}</span>
      <span class="cs-prob">{pct}%</span>
    </header>
    <p class="cs-text selectable">{c.text}</p>
  </div>
  <div class="cs-actions">
    <!-- Manual 档：只填入。默认动作，不发送。 -->
    <button class="cs-btn primary" disabled={busy} onclick={onfill} title="填入输入框，停在光标处">
      填入
    </button>
    <!-- 升到 Confirm 档必须人按。程序自己不许升档。 -->
    <button class="cs-btn" disabled={busy} onclick={onconfirm} title="填入后二次确认才发送">
      填入并确认发送
    </button>
  </div>
</article>

<style>
  .cs-cand {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    padding: 0.45rem 0.55rem 0.5rem;
    border-radius: var(--cs-radius-sm);
    background: var(--cs-surface-raised);
    border: 1px solid var(--cs-border);
    position: relative;
    overflow: hidden;
  }

  /* 概率条：贴在左侧，宽度 = 概率。 */
  .cs-bar {
    position: absolute;
    inset: 0 auto 0 0;
    width: calc(var(--pct) * 1%);
    background: linear-gradient(
      90deg,
      color-mix(in srgb, var(--cs-accent) 26%, transparent),
      transparent
    );
    pointer-events: none;
  }

  .cs-body {
    position: relative;
  }

  .cs-meta {
    display: flex;
    justify-content: space-between;
    font-size: 0.62rem;
    color: var(--cs-text-faint);
  }

  .cs-prob {
    font-variant-numeric: tabular-nums;
  }

  .cs-text {
    margin: 0.2rem 0 0;
    font-size: 0.78rem;
    line-height: 1.55;
    color: var(--cs-text);
  }

  .cs-actions {
    position: relative;
    display: flex;
    gap: 0.35rem;
  }

  .cs-btn {
    flex: 1;
    padding: 0.3rem 0.4rem;
    font-size: 0.7rem;
    border-radius: 7px;
    border: 1px solid var(--cs-border-strong);
    background: transparent;
    transition:
      background var(--cs-dur-fast) var(--cs-ease),
      border-color var(--cs-dur-fast) var(--cs-ease),
      transform var(--cs-dur-fast) var(--cs-ease);
  }

  .cs-btn:hover:not(:disabled) {
    background: rgb(255 255 255 / 6%);
    border-color: var(--cs-text-faint);
  }

  .cs-btn:active:not(:disabled) {
    transform: translateY(1px);
  }

  .cs-btn:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .cs-btn.primary {
    border-color: color-mix(in srgb, var(--cs-accent) 55%, transparent);
    color: var(--cs-accent);
  }
</style>
