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
      // ⚠ 注册失败也要**看得见**。监听器静默挂掉时，界面会一直停在
      // `probe_anchor` 那一行，用户和开发者都看不出是「没收到」还是「收到但没变」。
      try {
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
          messages = e.payload.messages ?? [];
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
      } catch (e) {
        reason = `**收不到 Rust 的推送**：${String(e)}`;
      }
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
  <!-- 标题栏：只做「我是谁 + 两个次要入口」。
       ⚠ 发送档位**不在这里**（2026-10-07 改）——
       它是最需要慎重的一步，却原来和标题挤在一起、看着像随手点的。
       档位现在固定在**底部**，和候选区隔开，让「先看内容、再决定怎么发」
       成为视觉上的自然顺序。 -->
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
    </div>
  </header>

  <!-- 会话闸：只对锁定会话动手。这是**当前状态**，所以放在最上面做成卡片：
       用户任何时候一眼都能看出「它现在看着谁」，不对就会点下面的锁定。 -->
  <section class="cs-peer" class:unknown={!peer} aria-label="当前会话">
    <span class="cs-peer-bar" aria-hidden="true"></span>
    <span class="cs-peer-text">
      <span class="cs-peer-label">正在处理</span>
      <span class="cs-peer-name">{peer || '没认出来'}</span>
    </span>
    <div class="cs-peer-actions">
      <button
        class="cs-peer-lock"
        onclick={refresh}
        disabled={refreshing}
        title="手动读一次当前快照。面板是纯推送的，静态窗口里它不会自己说话。"
      >
        {refreshing ? '读取中' : '刷新'}
      </button>
      <button
        class="cs-peer-lock"
        onclick={relock}
        disabled={locking || relockPending}
        title="把当前这个会话锁成要处理的那个。程序只对它动手。"
      >
        {locking || relockPending ? '等下一轮' : '改锁这个'}
      </button>
    </div>
  </section>

  <!-- 为什么不动手：只在有事发生时说一句话。认不出会话的时候，
       它取代所有其它内容——那时候显示「候选」是在骗人。 -->
  {#if !peer && messages.length === 0 && candidates.length === 0}
    <p class="cs-note">{reason}</p>
  {:else}
    <ReasonBar {reason} {counts} />
  {/if}

  <MessageStream {messages} />

  <!-- 候选区。**它是这个面板的产出**，所以拿走所有剩余空间；
       没有候选时，空状态就落在这块地方里，不再另外插一段灰字。
       原来「还没有消息」「窗口静止时不会刷新」「没有新消息不会起草」
       三行灰字说的其实是同一件事，读起来像出了三个问题。 -->
  <section class="cs-cands" aria-label="候选回复">
    {#if sorted.length > 0}
      <h2 class="cs-section">候选回复</h2>
      {#each sorted as c, i (c.id)}
        <CandidateCard
          {c}
          rank={i + 1}
          {busy}
          recommended={c.id === bestId}
          onfill={() => act(c, 'fill')}
          onconfirm={() => act(c, 'confirm')}
        />
      {/each}
    {:else}
      <h2 class="cs-section">候选回复</h2>
      <div class="cs-empty">
        <p class="cs-empty-lead">
          {#if !peer}
            没认出会话，动不了手
          {:else if messages.length === 0}
            窗口静止时这里不会刷新
          {:else}
            等对方那条先到
          {/if}
        </p>
        <p class="cs-empty-sub">对方发来新消息才会起草。</p>
      </div>
    {/if}
  </section>

  <!-- 发送档位。固定在底部，和候选区之间有一条分隔线：
       「看完再决定怎么发」这个顺序要在视觉上成立。 -->
  <footer class="cs-foot">
    <span class="cs-foot-label">发送方式</span>
    <TierSwitch bind:tier onchange={changeTier} />
  </footer>
</main>

<style>
  .cs-panel {
    display: flex;
    flex-direction: column;
    gap: 0.55rem;
    height: 100%;
    padding: 0.75rem 0.75rem 0;
    /* ⚠ 底色必须**完全不透明**（2026-10-07 实测）。
       原来这里是 `rgb(28 28 30 / 88%)` + `backdrop-filter: blur(18px)`，
       结果整块底色不绘制——文字在、面板没了，底下桌面直接透进来，
       完全没法读。`backdrop-filter` 一并去掉。
       圆角不受影响：它来自**窗口的透明四角**，不来自面板。 */
    background: rgb(26 26 29);
    border: 1px solid var(--cs-border);
    border-radius: var(--cs-radius);
    box-shadow: var(--cs-shadow);
    overflow: hidden;
  }

  /* ── 标题栏：安静，让位给内容 ── */
  .cs-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    flex: none;
  }

  .cs-title {
    font-size: 0.78rem;
    font-weight: 600;
    letter-spacing: 0.14em;
    color: var(--cs-text-dim);
    white-space: nowrap;
  }

  .cs-head-tools {
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }

  .cs-tool {
    padding: 0.18rem 0.5rem;
    font-size: 0.68rem;
    border-radius: 7px;
    border: 1px solid var(--cs-border);
    background: var(--cs-surface-raised);
    color: var(--cs-text-dim);
    transition: background var(--cs-dur-fast) var(--cs-ease);
  }

  .cs-tool:hover {
    color: var(--cs-text);
    background: var(--cs-border);
  }

  /* ── 会话卡：当前状态，面板里最重要的一个「事实」 ── */
  .cs-peer {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    flex: none;
    padding: 0.5rem 0.6rem;
    border-radius: var(--cs-radius-sm);
    background: var(--cs-surface-raised);
    border: 1px solid var(--cs-border);
  }

  .cs-peer.unknown {
    border-color: var(--cs-danger);
  }

  .cs-peer-bar {
    flex: none;
    width: 3px;
    align-self: stretch;
    border-radius: 2px;
    background: var(--cs-accent);
  }

  .cs-peer.unknown .cs-peer-bar {
    background: var(--cs-danger);
  }

  .cs-peer-text {
    display: flex;
    flex-direction: column;
    gap: 0.05rem;
    min-width: 0;
    flex: 1;
  }

  .cs-peer-label {
    font-size: 0.6rem;
    letter-spacing: 0.1em;
    color: var(--cs-text-faint);
  }

  .cs-peer-name {
    font-size: 0.82rem;
    font-weight: 600;
    color: var(--cs-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cs-peer.unknown .cs-peer-name {
    color: var(--cs-danger);
  }

  .cs-peer-actions {
    display: flex;
    gap: 0.25rem;
    flex: none;
  }

  .cs-peer-lock {
    padding: 0.2rem 0.45rem;
    font-size: 0.64rem;
    border: 1px solid var(--cs-border-strong);
    border-radius: 7px;
    background: var(--cs-surface-sunken);
    color: var(--cs-text-dim);
    white-space: nowrap;
    transition: background var(--cs-dur-fast) var(--cs-ease);
  }

  .cs-peer-lock:hover:not(:disabled) {
    background: var(--cs-border);
    color: var(--cs-text);
  }

  .cs-peer-lock:disabled {
    opacity: 0.4;
    cursor: default;
  }

  /* ── 状态行 ── */
  .cs-note {
    margin: 0;
    padding: 0.5rem 0.6rem;
    font-size: 0.72rem;
    line-height: 1.5;
    color: var(--cs-danger);
    background: var(--cs-surface-raised);
    border: 1px solid var(--cs-border);
    border-radius: var(--cs-radius-sm);
  }

  /* 小节标题。让「候选回复」这几个字真正成为一个标题，
     而不是漂浮的又一个灰字。 */
  .cs-section {
    margin: 0.15rem 0 0;
    font-size: 0.62rem;
    font-weight: 600;
    letter-spacing: 0.14em;
    color: var(--cs-text-faint);
  }

  .cs-cands {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    overflow-y: auto;
    scrollbar-width: thin;
    /* 拿走全部剩余空间——面板的产出在这里，不该让中段空一大块。 */
    flex: 1 1 auto;
    min-height: 0;
  }

  /* 空状态：两行，一行说结论一行说下一步。居中占住这块地方，
     免得上面一条状态下面一大片黑，看着像坏了。 */
  .cs-empty {
    flex: 1 1 auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 0.3rem;
    margin: 0;
    padding: 1.2rem 0.8rem;
    border: 1px dashed var(--cs-border);
    border-radius: var(--cs-radius-sm);
    background: var(--cs-surface-sunken);
    text-align: center;
  }

  .cs-empty-lead {
    margin: 0;
    font-size: 0.78rem;
    color: var(--cs-text-dim);
  }

  .cs-empty-sub {
    margin: 0;
    font-size: 0.68rem;
    color: var(--cs-text-faint);
  }

  /* ── 底部：发送档位 ── */
  .cs-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    flex: none;
    margin-top: auto;
    padding: 0.55rem 0 0.65rem;
    border-top: 1px solid var(--cs-border);
  }

  .cs-foot-label {
    font-size: 0.62rem;
    letter-spacing: 0.1em;
    color: var(--cs-text-faint);
    white-space: nowrap;
  }

  /* 紧凑模式：面板窄的时候收紧间距与字号，
     而不是让文字挤成两行。挤成两行的可读性远低于更小但单行的字。 */
  .cs-panel.compact {
    padding: 0.55rem 0.55rem 0;
    gap: 0.4rem;
  }

  .cs-panel.compact .cs-title {
    font-size: 0.72rem;
  }

  .cs-panel.compact .cs-peer-name {
    font-size: 0.76rem;
  }

  .cs-panel.compact .cs-foot-label {
    display: none;
  }
</style>
