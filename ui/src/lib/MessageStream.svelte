<script lang="ts">
  import { enter, scrollToBottom } from './attach';
  import { DUR, enterFrom } from './motion';
  import type { Message } from './types';

  interface Props {
    messages: Message[];
  }

  const { messages }: Props = $props();

  /**
   * 新消息到达时滚到底。
   *
   * `{@attach scrollToBottom(n)}` 里 `n = messages.length` 被读进依赖，
   * 长度变了 attachment 自动重跑 → 滚到底。**只在增长时滚**这件事
   * 由调用方保证（只在 `$derived` 里算出增长时才挂这个 attachment）。
   *
   * 用户往上翻看历史时来了新消息，把他拽回底部是最招人烦的行为——
   * 要去，也得他自己决定。
   */
  const follow = (n: number) => scrollToBottom(n, DUR.base);

  /** 展开了判据的那一条。同一时刻只开一条——这是诊断视图，不是产品。 */
  let opened = $state<string | null>(null);

  const speaker = (s: Message['speaker']) => (s === 'Me' ? '我' : s === 'Them' ? '对方' : '系统');
</script>

<section {@attach follow(messages.length)} class="cs-stream" aria-label="对话">
  {#each messages as m (m.id)}
    <!--
      入场动画只给**新增的那一条**：{#each} 的 keyed 渲染只重建变化项，
      所以这里给每条挂 enter 是安全的——旧消息的 DOM 没被重建，
      attachment 不会重跑，只有真正新插入的节点会闪一下。

      `m.id` 必须是真键。Rust 侧早期没这个字段，7 条消息的 key 全是
      undefined，Svelte 判成同一个节点——面板整块空白而后端毫无报错。
    -->
    <article
      {@attach enter(enterFrom(6))}
      class="cs-msg"
      class:me={m.speaker === 'Me'}
      class:sys={m.speaker === 'System'}
    >
      <button
        class="cs-who"
        title="点开看这条为什么被这样判"
        aria-expanded={opened === m.id}
        onclick={() => (opened = opened === m.id ? null : m.id)}
      >
        {speaker(m.speaker)}
      </button>
      <p class="cs-text selectable">{m.text}</p>
      {#if opened === m.id}
        <!-- 判据**按需展开**，不再常驻。
             它是 rgb()/对比度/彩度这一串诊断数字，常驻显示会把正文压到
             读不下去——上一版每条消息上面都挂着一整行这种数字，
             面板看上去像日志而不是聊天记录。诊断信息要能拿到，但不该占版面。 -->
        <p class="cs-ev selectable">{m.evidence || '（这条没有记录判据）'}</p>
      {/if}
    </article>
  {:else}
    <p class="cs-empty">还没有消息</p>
  {/each}
</section>

<style>
  .cs-stream {
    /* ⚠ **必须封顶并自己滚动。**
     *
     * 它是「背景资料」，候选才是这个面板的产出。消息一多（实测 9 条就
     * 把候选整块顶出可视范围），不封顶就等于把结果藏起来了。
     * `max-height` 用百分比——父级是定高的 flex 列容器，能解析。 */
    flex: 0 1 auto;
    max-height: 38%;
    display: flex;
    flex-direction: column;
    padding: 0.4rem 0 0;
    overflow-y: auto;
    scrollbar-width: thin;
    min-height: 2.2rem;
    /* GSAP 改 scrollTop，这里不能再给 transition，否则会互相拖慢 */
  }

  /* 一条消息 = **一行说话人 + 正文**。没有卡片、没有气泡、没有头像。
     这是仪表的读数区，不是第二个聊天软件：越平越好读，越不像微信
     就越不会被误认成微信的一部分。 */
  .cs-msg {
    display: grid;
    /* 说话人占一个固定窄列，正文吃掉剩下的。正文左边缘因此全线对齐，
       扫读时眼睛只需走一条直线。 */
    grid-template-columns: 2.1rem minmax(0, 1fr);
    column-gap: 0.45rem;
    align-items: baseline;
    padding: 0.16rem 0.1rem;
    border-radius: 4px;
    /* 上一版每条都有一块底色 + 左边框，一屏七八条时整片区域被切碎，
       反而看不出对话的节奏。改成**只有左边一条 2px 的说话人色**，
       并且**隔条才给一点底色**，密排时是安静的。 */
    border-left: 2px solid transparent;
  }

  .cs-msg:nth-child(even) {
    background: rgb(255 255 255 / 2.5%);
  }

  .cs-msg:hover {
    border-left-color: color-mix(in srgb, var(--cs-them) 70%, transparent);
  }
  .cs-msg.me:hover {
    border-left-color: color-mix(in srgb, var(--cs-me) 70%, transparent);
  }

  .cs-msg.me {
    color: var(--cs-ink-2);
  }

  .cs-msg.sys {
    color: var(--cs-ink-3);
  }

  /* 说话人用小号等宽，三种身份靠颜色区分：
     对方=蓝、自己=绿（沿用微信绿，但只用于这一条语义）、系统=灰。 */
  .cs-who {
    appearance: none;
    border: 0;
    background: none;
    padding: 0;
    margin: 0;
    font: inherit;
    font-size: 0.6rem;
    font-variant-numeric: tabular-nums;
    text-align: left;
    color: var(--cs-them);
    cursor: help;
    border-bottom: 1px dotted transparent;
  }
  .cs-who:hover {
    border-bottom-color: currentColor;
  }
  .cs-msg.me .cs-who {
    color: var(--cs-me);
  }
  .cs-msg.sys .cs-who {
    color: var(--cs-ink-3);
  }

  .cs-text {
    margin: 0;
    font-size: 0.76rem;
    line-height: 1.5;
    color: var(--cs-ink);
    overflow-wrap: anywhere;
  }

  .cs-ev {
    grid-column: 2;
    margin: 0.2rem 0 0.15rem;
    padding: 0.25rem 0.4rem;
    font-size: 0.62rem;
    line-height: 1.5;
    color: var(--cs-ink-3);
    background: var(--cs-surface-raised);
    border-radius: 4px;
    overflow-wrap: anywhere;
  }

  .cs-empty {
    margin: 0;
    padding: 0.6rem 0;
    text-align: center;
    font-size: 0.7rem;
    color: var(--cs-ink-3);
  }
</style>
