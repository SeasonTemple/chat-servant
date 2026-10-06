<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import CandidateCard from '$lib/CandidateCard.svelte';
  import MessageStream from '$lib/MessageStream.svelte';
  import ReasonBar from '$lib/ReasonBar.svelte';
  import TierSwitch from '$lib/TierSwitch.svelte';
  import type { CallCounts, Candidate, DispatchReply, Message, Tier } from '$lib/types';

  /** Rust 侧推过来的消息流。**前端不自己算**——`Message[]` 是 Rust 判完发过来的。 */
  let messages = $state<Message[]>([]);
  let candidates = $state<Candidate[]>([]);
  let tier = $state<Tier>('Manual');
  let reason = $state('等待第一条消息');
  /** 分段调用次数。**Rust 侧算好推过来**，前端不自己累加——
   *  四个数是「静默期零调用」那条硬约束的唯一可视证据，自己数一遍就可能数错。 */
  let counts = $state<CallCounts | null>(null);
  /** 当前窗口里的会话名。
   *
   * 这是**会话身份闸**的判据，必须显眼：程序只对锁定会话动手，
   * 而「为什么不动」这件事用户要能一眼看到，不能只在 reason 里猜。
   * 空串 = 没认出来，那同样是不动手。
   */
  let peer = $state('');
  /**
   * 重锁请求**还在等着被下一轮取走**。
   *
   * 按钮点了之后真正生效要等下一轮（最多一个 `TICK`）。不把这个状态
   * 显出来，用户点了只看到请求成功，却不知道它还没起作用——
   * 而「点了没反应」正是这个项目反复在修的东西。
   */
  let relockPending = $state(false);
  /** 手动刷新中。按钮要有状态。 */
  let refreshing = $state(false);
  /** 重锁请求进行中。**按钮要有状态**——点了没反馈等于没点。 */
  let locking = $state(false);

  /**
   * 把当前这个会话锁成要处理的那个。
   *
   * 指纹闸在第一帧读到头部时就建了锁，所以「启动时开着的会话」
   * 就是被锁的那个。这是**唯一的修正手段**——没有它，用户在错的会话上
   * 启动了应用就只能关掉重开。
   */
  async function relock() {
    locking = true;
    try {
      await invoke('relock_session');
      reason = '已请求重锁：下一轮起以当前这个会话为基准。';
    } catch (e) {
      reason = `重锁失败：${String(e)}`;
    } finally {
      locking = false;
    }
  }
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
        counts: CallCounts | null;
        /** 当前窗口里的会话名。**空串 = 没认出来**。 */
        peer: string;
        /** 重锁请求是否还在等着被下一轮取走。 */
        relock_pending: boolean;
      }>('readout', (e) => {
        messages = e.payload.messages;
        candidates = e.payload.candidates ?? [];
        reason = e.payload.reason;
        counts = e.payload.counts ?? null;
        peer = e.payload.peer ?? '';
        relockPending = e.payload.relock_pending ?? false;
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

  /**
   * 手动读一次当前快照。
   *
   * 面板是**纯推送**的：循环只在有变化时 emit，静态窗口里它一直沉默。
   * 用户怀疑漏了消息时没有补救手段——这个按钮是那条出路，
   * 也让 `latest_readout` 不再是一个没人调用的命令。
   */
  async function refresh() {
    refreshing = true;
    try {
      const r = await invoke<{
        messages: Message[];
        candidates: Candidate[];
        reason: string;
        peer: string;
        counts: CallCounts | null;
      } | null>('latest_readout');
      if (!r) {
        reason = '观察循环还没跑出第一帧。';
        return;
      }
      messages = r.messages ?? [];
      candidates = r.candidates ?? [];
      reason = r.reason;
      peer = r.peer ?? '';
      counts = r.counts ?? null;
    } catch (e) {
      reason = `读快照失败：${String(e)}`;
    } finally {
      refreshing = false;
    }
  }

  /**
   * 改档。**用 Rust 回传的真实生效值覆盖**——
   * 硬约束第 5 条「默认档最低」，所以 Rust 有可能在解析失败时把档位
   * 夹回 Manual。界面必须显示程序认定的那个，不是用户点过的那个。
   */
  async function changeTier(t: Tier) {
    try {
      tier = await invoke<Tier>('set_tier', { tier: t });
    } catch (e) {
      reason = `改档失败：${String(e)}`;
      // 读回真实值，别让界面停在一个程序并不认同的档位上。
      try {
        tier = await invoke<Tier>('get_tier');
      } catch {
        tier = 'Manual';
      }
    }
  }

  async function act(c: Candidate, action: 'fill' | 'confirm') {
    busy = true;
    try {
      // 回执**不是布尔值**。PossiblySent 之后禁止自动重放。
      // ⚠ 不传 tier：档位由 Rust 自己持有并读取。
      // 早先这里传的是前端 `tier`，那等于把「会不会真发出去」
      // 交给一个界面可以随便改的字符串。
      const r = await invoke<DispatchReply>('dispatch', {
        candidateId: c.id,
        action,
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
      <TierSwitch bind:tier onchange={changeTier} />
    </div>
  </header>

  <!-- 会话闸：只对锁定会话动手。徽标 + 重锁按钮都要显眼——
       程序不动的时候，用户要能立刻看出「它看到的是哪个会话」，
       并且能当场纠正，而不是只能关掉重开。 -->
  <div class="cs-peer">
    <span class="cs-peer-name" class:unknown={!peer}>
      {#if peer}
        会话：{peer}
      {:else}
        会话：没认出来
      {/if}
    </span>
    <button
      class="cs-peer-lock"
      onclick={refresh}
      disabled={refreshing}
      title="手动读一次当前快照。面板是纯推送的，静态窗口里它不会自己说话。"
    >
      {refreshing ? '读取中…' : '刷新'}
    </button>
    <button
      class="cs-peer-lock"
      onclick={relock}
      disabled={locking || relockPending}
      title="把当前这个会话锁成要处理的那个。程序只对它动手。"
    >
      {#if locking || relockPending}
        等下一轮…
      {:else}
        锁这个会话
      {/if}
    </button>
  </div>

  <ReasonBar {reason} {counts} />

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
  /* 会话徽标 + 重锁按钮。认不出来时用警告色——那意味着程序**不会动手**，
     得让人一眼看出来，而不是当成一个还没加载完的空白。 */
  .cs-peer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    margin: 0 0 0.2rem;
  }

  .cs-peer-name {
    font-size: 0.66rem;
    letter-spacing: 0.02em;
    color: var(--cs-text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cs-peer-name.unknown {
    color: var(--cs-danger);
  }

  /* 重锁是个**小的、明确的**动作，不是主按钮——
     免得被误当成「开始工作」。 */
  .cs-peer-lock {
    flex: none;
    padding: 0.12rem 0.4rem;
    font-size: 0.62rem;
    border: 1px solid var(--cs-border-strong);
    border-radius: 6px;
    background: transparent;
    color: var(--cs-text-dim);
  }

  .cs-peer-lock:hover:not(:disabled) {
    background: rgb(255 255 255 / 6%);
    color: var(--cs-text);
  }

  .cs-peer-lock:disabled {
    opacity: 0.45;
    cursor: default;
  }

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
