<script lang="ts">
  import { enter } from './attach';
  import { enterFrom } from './motion';
  import { CornerDownLeft, SendHorizontal } from '@lucide/svelte/icons';
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

  /** 概率越低，概率条越短越淡。模型自己都不确定时不该看起来很笃定。 */
  const confidence = $derived(pct >= 60 ? 'high' : pct >= 35 ? 'mid' : 'low');
</script>

<!--
  设计上这是一条**建议**，不是一个卡片。

  上一版是「带边框的卡 + 两条占满宽度的按钮」，结果是：正文被两个按钮
  挤到只剩两行，三条候选在 560px 高的面板里立刻撑爆，用户必须滚动才能
  看完模型到底说了什么。卡片 + 满宽按钮是「聊天软件」的语汇，
  而这里要表达的是「仪表给出一个数，你按不按」。

  改法：
  · 去掉卡片边框和底色，只留一条**按概率长短的水平线**当置信度条
    ——它是数据，不是装饰。
  · 正文是这个组件里**最大的字**，按钮反而最小。产出优先于操作。
  · 动作收成两个方钮（↵ 填入 / ➤ 确认发送），图标带 tooltip；
    「填入」是默认动作，用黄铜实底表示，不靠文字强调。
  · 推荐与否由**左边一条 2px 竖线**表示，不写「推荐」两个字——
    满屏的推荐徽章等于没有推荐。
-->
<article
  {@attach enter(enterFrom(10))}
  class="cs-cand"
  class:rec={recommended}
  class:busy
  data-conf={confidence}
>
  <header class="cs-meta">
    <span class="cs-rank">{recommended ? '推荐' : rank}</span>
    <span class="cs-meter" aria-hidden="true"><i style:--pct={pct}></i></span>
    <span class="cs-prob" title="模型给的胜出概率">{pct}%</span>
  </header>

  <p class="cs-text selectable">{c.text}</p>

  <div class="cs-actions">
    <!-- Manual 档：只填入。默认动作，不发送。 -->
    <button
      class="cs-act primary"
      disabled={busy}
      onclick={() => {
        onfill();
      }}
      title="填入输入框，停在光标处（不发送）"
      aria-label="填入输入框，不发送"
    >
      <CornerDownLeft size={13} strokeWidth={2} aria-hidden="true" />
      <span>填入</span>
    </button>
    <!-- 升到 Confirm 档必须人按。程序自己不许升档。 -->
    <button
      class="cs-act"
      disabled={busy}
      onclick={() => {
        onconfirm();
      }}
      title="填入后二次确认才发送"
      aria-label="填入并确认发送"
    >
      <SendHorizontal size={13} strokeWidth={2} aria-hidden="true" />
      <span>确认发送</span>
    </button>
  </div>
</article>

<style>
  .cs-cand {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    padding: 0.5rem 0.55rem 0.55rem;
    /* 左边一条按推荐与否变色的竖线。整个组件的「推荐」表达就是它。 */
    border-left: 2px solid var(--cs-border);
    border-radius: 0 var(--cs-radius-sm) var(--cs-radius-sm) 0;
    background: var(--cs-surface-raised);
    transition:
      border-color var(--cs-dur-fast) var(--cs-ease),
      background var(--cs-dur-fast) var(--cs-ease);
  }

  .cs-cand:hover {
    background: color-mix(in srgb, var(--cs-surface-raised) 70%, var(--cs-ink) 4%);
  }

  .cs-cand.rec {
    border-left-color: var(--cs-brass);
  }

  .cs-cand.busy {
    opacity: 0.55;
  }

  .cs-meta {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.62rem;
    color: var(--cs-ink-3);
  }

  .cs-rank {
    min-width: 1.9rem;
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--cs-ink-3);
  }

  .cs-cand.rec .cs-rank {
    color: var(--cs-brass);
  }

  /* 置信度条：宽度就是概率，不是装饰。 */
  .cs-meter {
    flex: 1;
    height: 2px;
    border-radius: 2px;
    background: var(--cs-border);
    overflow: hidden;
  }
  .cs-meter i {
    display: block;
    width: calc(var(--pct) * 1%);
    height: 100%;
    background: var(--cs-brass);
    transition: width var(--cs-dur) var(--cs-ease);
  }
  .cs-cand[data-conf='low'] .cs-meter i {
    background: var(--cs-ink-3);
  }

  .cs-prob {
    font-variant-numeric: tabular-nums;
  }
  .cs-cand[data-conf='low'] .cs-prob {
    opacity: 0.55;
  }

  /* 正文是这个面板的产出，用**最大的字**。上一版正文 0.78rem、
     按钮 0.7rem——把最重要的东西排成了最小的。 */
  .cs-text {
    margin: 0;
    font-size: 0.86rem;
    line-height: 1.55;
    color: var(--cs-ink);
    overflow-wrap: anywhere;
  }

  .cs-actions {
    display: flex;
    gap: 0.3rem;
  }

  /* 方钮：图标 + 短标签。比满宽按钮省地方，也更像仪表的操作键。 */
  .cs-act {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.22rem 0.45rem;
    font-size: 0.68rem;
    border-radius: 7px;
    border: 1px solid var(--cs-border-strong);
    background: transparent;
    color: var(--cs-ink-2);
    white-space: nowrap;
    /* 按下反馈留给 GSAP，这里不做 transform——两者同时写会互相打断 */
    transition:
      background var(--cs-dur-fast) var(--cs-ease),
      color var(--cs-dur-fast) var(--cs-ease),
      border-color var(--cs-dur-fast) var(--cs-ease);
  }

  .cs-act:hover:not(:disabled) {
    background: rgb(255 255 255 / 6%);
    color: var(--cs-ink);
  }

  .cs-act:disabled {
    opacity: 0.45;
    cursor: default;
  }

  /* 默认档是「只填入」，所以它才是实底的——用户最常按的就是它。 */
  .cs-act.primary {
    border-color: color-mix(in srgb, var(--cs-brass) 55%, transparent);
    background: color-mix(in srgb, var(--cs-brass) 14%, transparent);
    color: var(--cs-brass);
  }

  .cs-act.primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--cs-brass) 22%, transparent);
  }
</style>
