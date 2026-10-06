<script lang="ts">
  import { enter } from './attach';
  import { enterFrom } from './motion';
  import type { Candidate } from './types';

  interface Props {
    c: Candidate;
    rank: number;
    busy: boolean;
    /** 是不是模型最看好的一条。推荐条要置顶并标注。 */
    recommended: boolean;
    onfill: () => void;
    onconfirm: () => void;
  }

  const { c, rank, busy, recommended, onfill, onconfirm }: Props = $props();

  /** 胜出概率。宽度直接就是概率——一眼看出模型有多确定，不靠读数字。 */
  const pct = $derived(Math.round(Math.max(0, Math.min(1, c.probability)) * 100));

  /** 概率越高底色越实。低于 40% 明显发虚——模型自己都不确定。 */
  const confidence = $derived(pct >= 60 ? 'high' : pct >= 35 ? 'mid' : 'low');
</script>

<!--
  卡片本身**不是**可点的——可点的是下面两个按钮。
  早期版本给 <article> 加 onclick 做按压反馈，a11y 立刻报两条：
  「可��见的非交互元素带 click 事件必须配键盘事件」。
  那两条是对的：整张卡片可点 = 用户会以为点哪都能填入，
  而实际只有按钮行能点。所以按压反馈挂在按钮上，卡片只做入场。

  入场位移刻意小——面板是悬浮在别人聊天窗口上的，大幅飞入会遮挡
  正在发生的事，也会让人分不清是新内容还是整个面板在动。
-->
<article
  {@attach enter(enterFrom(10))}
  class="cs-cand"
  class:rec={recommended}
  class:busy
  data-conf={confidence}
>
  <div class="cs-bar" style:--pct={pct} aria-hidden="true"></div>

  <div class="cs-body">
    <header class="cs-meta">
      <span class="cs-rank">
        {#if recommended}
          <span class="cs-star" title="模型最看好的一条">推荐</span>
        {:else}
          候选 {rank}
        {/if}
      </span>
      <span class="cs-prob" title="模型给的胜出概率">{pct}%</span>
    </header>
    <p class="cs-text selectable">{c.text}</p>
  </div>

  <div class="cs-actions">
    <!-- Manual 档：只填入。默认动作，不发送。 -->
    <button
      class="cs-btn primary"
      disabled={busy}
      onclick={() => {
        onfill();
      }}
      title="填入输入框，停在光标处"
    >
      填入
    </button>
    <!-- 升到 Confirm 档必须人按。程序自己不许升档。 -->
    <button
      class="cs-btn"
      disabled={busy}
      onclick={() => {
        onconfirm();
      }}
      title="填入后二次确认才发送"
    >
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
    /* 位移动效交给 GSAP。这里只留 border/color 的颜色过渡，
       否则两者会打架——CSS transition 和 GSAP 同时改一个属性，
       结果是动效结束瞬间跳一下。 */
    transition:
      border-color var(--cs-dur-fast) var(--cs-ease),
      box-shadow var(--cs-dur-fast) var(--cs-ease);
  }

  .cs-cand:hover {
    border-color: var(--cs-border-strong);
  }

  /* 推荐条：视觉上要比其它候选更突出，但**不靠更亮**——
     面板是半透明的，亮色会盖住底下用户的聊天窗口。 */
  .cs-cand.rec {
    border-color: color-mix(in srgb, var(--cs-accent) 45%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--cs-accent) 14%, transparent);
  }

  .cs-cand.busy {
    opacity: 0.6;
  }

  .cs-bar {
    position: absolute;
    inset: 0 auto 0 0;
    width: calc(var(--pct) * 1%);
    background: linear-gradient(
      90deg,
      color-mix(in srgb, var(--cs-accent) 24%, transparent),
      transparent
    );
    pointer-events: none;
    transition: width var(--cs-dur) var(--cs-ease);
  }

  .cs-body {
    position: relative;
  }

  .cs-meta {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.62rem;
    color: var(--cs-text-faint);
  }

  .cs-star {
    color: var(--cs-accent);
    /* 星标不用图标字体，直接一个字更省，也不会有字体缺失的方块 */
    font-weight: 600;
  }

  .cs-prob {
    font-variant-numeric: tabular-nums;
  }

  /* 概率低时数字也发虚——模型不确定时不该看起来很笃定 */
  .cs-cand[data-conf='low'] .cs-prob {
    opacity: 0.55;
  }

  .cs-text {
    margin: 0.2rem 0 0;
    font-size: 0.78rem;
    line-height: 1.55;
    color: var(--cs-text);
    /* 面板可以很窄。宁可换行，不要横向溢出或截断——
       截断的候选没法判断要不要用。 */
    overflow-wrap: anywhere;
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
    color: var(--cs-text-dim);
    /* 按下反馈留给 GSAP，这里不做 transform——两者同时写会互相打断 */
    transition:
      background var(--cs-dur-fast) var(--cs-ease),
      color var(--cs-dur-fast) var(--cs-ease),
      border-color var(--cs-dur-fast) var(--cs-ease);
    white-space: nowrap;
  }

  .cs-btn:hover:not(:disabled) {
    background: rgb(255 255 255 / 6%);
    color: var(--cs-text);
  }

  .cs-btn:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .cs-btn.primary {
    border-color: color-mix(in srgb, var(--cs-accent) 55%, transparent);
    color: var(--cs-accent);
  }

  .cs-btn.primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--cs-accent) 12%, transparent);
  }
</style>
