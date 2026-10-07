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
</script>

<section {@attach follow(messages.length)} class="cs-stream" aria-label="对话">
  {#each messages as m (m.id)}
    <!--
      入场动画只给**新增的那一条**：{#each} 的 keyed 渲染只重建变化项，
      所以这里给每条挂 enter 是安全的——旧消息的 DOM 没被重建，
      attachment 不会重跑，只有真正新插入的节点会闪一下。
    -->
    <article
      {@attach enter(enterFrom(6))}
      class="cs-msg"
      class:me={m.speaker === 'Me'}
      class:sys={m.speaker === 'System'}
    >
      <header class="cs-msg-head">
        <span class="cs-who">
          {m.speaker === 'Me' ? '我' : m.speaker === 'Them' ? '对方' : '系统'}
        </span>
        <!-- 判定依据常驻可见：认错了要能立刻发现，而不是去猜 -->
        <span class="cs-ev" title={m.evidence}>{m.evidence}</span>
      </header>
      <p class="cs-text selectable">{m.text}</p>
    </article>
  {:else}
    <p class="cs-empty">还没有消息</p>
  {/each}
</section>

<style>
  .cs-stream {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    padding: 0.5rem 0 0;
    overflow-y: auto;
    scrollbar-width: thin;
    min-height: 2.4rem;
    /* GSAP 改 scrollTop，这里不能再给 transition，否则会互相拖慢 */
  }

  /* 每条消息只用**左边一条 2px 的色条**区分说话人，不做气泡、不做头像。
     面板很窄，气泡会把正文的行宽压到读不下去；色条够了——
     「谁说的」是二元信息，不需要更多像素。 */
  .cs-msg {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    padding: 0.35rem 0.5rem;
    border-radius: var(--cs-radius-sm);
    background: var(--cs-surface-raised);
    border-left: 2px solid var(--cs-them);
  }

  .cs-msg.me {
    border-left-color: var(--cs-me);
  }

  .cs-msg.sys {
    opacity: 0.7;
    border-left-color: var(--cs-ink-3);
  }

  .cs-msg-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.4rem;
  }

  .cs-who {
    font-size: 0.62rem;
    color: var(--cs-ink-3);
    flex: 0 0 auto;
  }

  .cs-ev {
    font-size: 0.58rem;
    color: var(--cs-ink-3);
    opacity: 0.8;
    text-align: right;
    /* 依据可能很长：限制一行 + 保留 title。
       窄面板下宁可截断，也不要它把整条消息撑高。 */
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .cs-text {
    margin: 0;
    font-size: 0.78rem;
    line-height: 1.5;
    overflow-wrap: anywhere;
  }

  .cs-empty {
    margin: 0;
    padding: 0.75rem 0.25rem;
    font-size: 0.72rem;
    color: var(--cs-ink-3);
    text-align: center;
  }
</style>
