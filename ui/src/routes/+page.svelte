<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import { Lock, RefreshCcwDot, ScanSearch, Settings2 } from '@lucide/svelte/icons';
  import CandidateCard from '$lib/CandidateCard.svelte';
  import MessageStream from '$lib/MessageStream.svelte';
  import TierSwitch from '$lib/TierSwitch.svelte';
  import type { Candidate, DispatchReply, Message, Tier } from '$lib/types';

  /** Rust 侧推过来的消息流。**前端不自己算**——`Message[]` 是 Rust 判完发过来的。 */
  let messages = $state<Message[]>([]);
  let candidates = $state<Candidate[]>([]);
  let tier = $state<Tier>('Manual');
  let reason = $state('等待第一条消息');
  /** 分段调用次数。**Rust 侧算好推过来**，前端不自己累加——
   *  四个数是「静默期零调用」那条硬约束的唯一可视证据，自己数一遍就可能数错。 */
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
  /**
   * **已装填的那条候选。** Confirm 档第一次点击之后 Rust 会推 `armed` 事件。
   *
   * 这是面板里唯一一个「再按一次就真的发出去」的按钮，所以必须显形：
   * 用户得知道现在按下去会发生什么。事件早先发出去但**没有任何监听者**，
   * 于是装填既不生效也不可见。
   */
  let armedId = $state('');
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

  /**
   * **只在真出事时才说的话。**
   *
   * 原来有一整条状态栏常驻，显示「判断 0 · 起草 0 · 排序 0 · 视觉 0」
   * 和各种说明文字。它的问题是：**大部分时候说的都是「没事」**——
   * 一条一直显示「一切正常」的横幅，会让真的异常变得不显眼。
   *
   * 所以这里只挑真正要人动手的情况：
   * - 认不出会话 → 程序不会动手，硬约束第 1 条生效中
   * - Rust 说推过来的那一轮出了错
   * - 前端这头断了（监听没建起来 / 快照读不回来）
   *
   * ⚠ **报错优先于「没认出会话」。**
   * 早先是 `if (!peer) return '没认出会话…'` 排在最前面，于是只要会话名
   * 是空的，`reason` 里真正的错误（读快照失败、监听被拒）就被盖掉——
   * 界面上永远是那一句「确认微信窗口开着」，把人引到完全错误的方向。
   * 这正是本项目反复在修的那类「界面在说谎」。
   */
  const problem = $derived.by(() => {
    if (/失败|错误|抓不到|收不到|异常/.test(reason)) return reason;
    if (!peer) return '没认出会话，本轮不动手 —— 确认微信窗口开着、没有被缩到托盘';
    return '';
  });

  onMount(() => {
    // 一次 invoke 打通 Rust → 前端。**不轮询**：`readout` 是 Rust 推过来的。
    //
    // 多个监听器要**分别**退订。早先只有一个 `dispose` 变量时，
    // 后注册的监听器会把前面的覆盖掉——组件卸载后那些监听器还活着，
    // 往一个已经没人读的 state 里写值。列表是从头就位是对的。
    const disposables: (() => void)[] = [];
    let cancelled = false;

    void (async () => {
      // ⚠ **整个链路必须在同一个 try 里。**
      //
      // 早先只有 `listen` 那一步包了 catch，`probe_anchor` 和动态 import
      // 在外面裸奔。任何一步抛错都会变成一个**没人接的 Promise 拒绝**——
      // `onMount` 里没人 `await` 它，于是监听器永远不会被注册，
      // 界面就停在初始状态，而后端日志一切正常。
      // 未捕获的异常在生产环境里是完全看不见的。
      try {
        const ping = await invoke<{ ok: boolean; anchor: string; detail: string }>('probe_anchor');
        if (cancelled) return;
        reason = ping.ok ? `锚点 ${ping.anchor}` : (ping.detail ?? '锚点推不出来');

        const { listen } = await import('@tauri-apps/api/event');
        const un = await listen<{
          messages: Message[];
          candidates: Candidate[];
          reason: string;
          /** 当前窗口里的会话名。**空串 = 没认出来**。 */
          peer: string;
          /** 重锁请求是否还在等着被下一轮取走。 */
          relock_pending: boolean;
        }>('readout', (e) => {
          messages = e.payload.messages ?? [];
          candidates = e.payload.candidates ?? [];
          reason = e.payload.reason;
          peer = e.payload.peer ?? '';
          relockPending = e.payload.relock_pending ?? false;
          // 候选换了，旧装填自动作废——装填是**某一条具体候选**的状态。
          armedId = '';
        });
        if (cancelled) {
          un();
          return;
        }
        disposables.push(un);
        // ⚠ **订阅之后必须立刻补拉一次快照。**
        //
        // Rust 的循环在应用启动那一刻就开始跑了，而前端要等
        // `probe_anchor` 返回、动态 import 完成，才走到这里注册监听器。
        // 这中间如果有内容变化，那一条推送就**发在我们订阅之前**——
        // 事件不会补发，界面从此停在空白，而 Rust 那边一切正常。
        //
        // 实测就是这样：后端日志 `peer="G8W" messages=2 candidates=3`，
        // 面板却显示「没认出会话 / 还没有消息」。
        //
        // 「先订阅、再补拉」是这类推送的标准接法：订阅保证不漏后面的，
        // 补拉保证不漏之前的。少任何一半都会出现「偶尔空白且不自愈」。
        const ok = await pullSnapshot();
        if (!ok && !cancelled) reason = '观察循环还没跑出第一帧，稍等一下或点「新」再试。';
        // 装填事件。它是 Confirm 档「再按一次就发」的那个状态，
        // 必须让按钮显形——不可见的状态等于没有状态。
        const unArmed = await listen<string>('armed', (e) => {
          armedId = e.payload;
        });
        disposables.push(unArmed);
      } catch (e) {
        // 这一条必须**显眼**：它意味着界面和 Rust 之间根本没连上，
        // 面板上看到的一切都不可信。
        reason = `**收不到 Rust 的推送**：${String(e)}`;
      }
    })();

    return () => {
      cancelled = true;
      // ⚠ **逐个退订**，不是只退最后一个——早先只有一个 `dispose` 变量，
      // 后注册的监听器会把前面的覆盖掉，组件卸载后它们还活着，
      // 往一个已经没人读的 state 里写值。
      for (const d of disposables.splice(0)) d();
    };
  });

  /**
   * 拉一次当前快照，灌进界面状态。
   *
   * 两个调用方：手动刷新按钮，以及**注册完监听器之后立刻补一次**
   * （见上面的 catch-up）。
   */
  async function pullSnapshot() {
    try {
      const r = await invoke<{
        messages: Message[];
        candidates: Candidate[];
        reason: string;
        peer: string;
      } | null>('latest_readout');
      if (!r) return false;
      messages = r.messages ?? [];
      candidates = r.candidates ?? [];
      reason = r.reason;
      peer = r.peer ?? '';
      return true;
    } catch (e) {
      reason = `读快照失败：${String(e)}`;
      return false;
    }
  }

  /**
   * 手动读一次当前快照。
   *
   * 面板是**推送为主**的：循环只在有变化时 emit，静态窗口里它一直沉默。
   * 用户怀疑漏了消息时没有补救手段——这个按钮是那条出路。
   */
  async function refresh() {
    refreshing = true;
    try {
      if (!(await pullSnapshot())) reason = '观察循环还没跑出第一帧。';
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
  <span class="cs-rail" aria-hidden="true"></span>
  <!-- 标题栏 = 「我现在看着谁」。
       会话名是这一屏最重要的一个事实：程序只对它动手（硬约束第 1 条）。
       原来它缩在一张小卡片里、字号还比旁边按钮小，「我看的到底是谁」
       这件事被埋掉了。现在它就是标题本身。 -->
  <header class="cs-head">
    <span class="cs-dot" class:idle={!peer} aria-hidden="true"></span>
    <span class="cs-peer" class:unknown={!peer} title={peer || '没认出会话'}>
      {peer || '没认出会话'}
    </span>
    <div class="cs-head-tools">
      <button
        class="cs-icon"
        title={relockPending ? '已请求重锁，下一轮起生效' : '把当前这个会话锁成要处理的那个'}
        aria-label="重新锁定当前会话"
        onclick={relock}
        disabled={locking || relockPending}
      >
        <Lock size={13} strokeWidth={2} aria-hidden="true" class={locking ? 'spin' : ''} />
      </button>
      <button
        class="cs-icon"
        title="识别画布：每个框按归类结果上色"
        aria-label="打开识别画布"
        onclick={() => invoke('open_debug').catch(() => {})}
      >
        <ScanSearch size={13} strokeWidth={2} aria-hidden="true" />
      </button>
      <button
        class="cs-icon"
        title="设置"
        aria-label="打开设置"
        onclick={() => invoke('open_settings').catch(() => {})}
      >
        <Settings2 size={13} strokeWidth={2} aria-hidden="true" />
      </button>
      <button
        class="cs-icon"
        title="手动读一次快照"
        aria-label="手动读一次快照"
        onclick={refresh}
        disabled={refreshing}
      >
        <RefreshCcwDot
          size={13}
          strokeWidth={2}
          aria-hidden="true"
          class={refreshing ? 'spin' : ''}
        />
      </button>
    </div>
  </header>

  <!-- 只在**真出事**时出现：认不出会话、抓不到帧、推理失败。
       正常时不占地方——一个一直挂在那儿刷新的状态条，会把「真的
       出事了」这件事稀释掉。这正是硬约束第 1 条要求的可见性。 -->
  {#if problem}
    <p class="cs-alarm">{problem}</p>
  {/if}

  <MessageStream {messages} />

  <!-- 候选是这个面板的**产出**。没有候选时不该有一大块空白顶在中间，
       看起来像界面坏了——空状态只占它该占的那点地方。

       上面加一条**带标签的分界线**是刻意的：消息流是「读到的证据」，
       候选是「给出的建议」，两者是不同性质的信息，不该糊在一起。
       上一版两者之间什么都没有，滚动时根本分不清哪块是哪块。 -->
  <section class="cs-cands" aria-label="候选回复">
    <div class="cs-cands-head">
      <span class="cs-cands-title">候选回复</span>
      {#if sorted.length > 0}
        <span class="cs-cands-n">{sorted.length} 条 · 按把握排序</span>
      {/if}
    </div>
    {#if sorted.length > 0}
      {#each sorted as c, i (c.id)}
        <CandidateCard
          {c}
          rank={i + 1}
          {busy}
          recommended={c.id === bestId}
          armed={c.id === armedId}
          onfill={() => act(c, 'fill')}
          onconfirm={() => act(c, 'confirm')}
        />
      {/each}
    {:else}
      <div class="cs-empty">
        {#if !peer}
          <span class="cs-empty-lead">没认出会话，不动手</span>
          <span class="cs-empty-sub">点上面的锁，换成你正在看的那个会话</span>
        {:else}
          <span class="cs-empty-lead">对方还没发新消息</span>
          <span class="cs-empty-sub">发来了才会起草候选</span>
        {/if}
      </div>
    {/if}
  </section>

  <!-- 发送档位固定在**底部**：它是这个界面上最需要慎重的一步，
       不该和标题挤在一起、看着像随手点的一档。 -->
  <footer class="cs-foot">
    <span class="cs-foot-label">发送方式</span>
    <span class="cs-switch" data-tier={tier}><TierSwitch bind:tier onchange={changeTier} /></span>
  </footer>
</main>

<style>
  /* ══════════════════════════════════════════════════════════════
     设计定位：**仪表，不是第二个聊天软件**。

     它浮在用户自己的聊天窗口旁边。如果做得像微信，它就会被
     误读成微信的一部分——所以底色用**暖石墨**（#1B1A18），
     刻意偏离微信那种冷调近黑，两块窗口并排时能一眼分清。

     强调色用**黄铜**（#D9A441），不用微信绿——绿要留给「我发的」
     这一条语义，界面自身的强调色不能抢掉它。

     面板唯一的「签名元素」是**左侧导轨**：从顶到底一条 3px 的竖条，
     顶端是状态点，通体带当前档位。它不是装饰——它编码的是
     「程序现在是活着 / 在等 / 可能发出去」这个事实。
     ══════════════════════════════════════════════════════════════ */

  .cs-panel {
    /* 导轨是绝对定位的，需要面板自己当包含块。 */
    position: relative;
    /* ⚠ **必须用 flex，不能用 grid 的位置行。**
     *
     * 原来这里是 `grid-template-rows: auto auto minmax(0,1fr) auto`，
     * 按「标题 / 异常条 / 消息 / 候选 / 档位」四行 + 一行隐式行来排。
     * 但**异常条是条件渲染的**：正常时它根本不在 DOM 里，于是所有孩子
     * 往上错一格——消息流落到 `auto` 行，按内容撑到多高算多高，
     * 候选被顶到屏幕外面去。
     *
     * 实测 2026-10-08：9 条消息时整个候选区被挤出可视范围，
     * 而「候选」正是这个面板的产出。flex + 显式 flex 值不受
     * 有无兄弟节点影响，这类错位就不存在了。 */
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 0 0.7rem;
    background: var(--cs-surface);
    border: 1px solid var(--cs-border);
    border-radius: var(--cs-radius);
    box-shadow: var(--cs-shadow);
    overflow: hidden;
  }

  /* ── 导轨 ───────────────────────────────────────────────── */
  .cs-rail {
    position: absolute;
    inset: 0 auto 0 0;
    width: 3px;
    background: var(--cs-rail);
    transition: background var(--cs-dur) var(--cs-ease);
  }
  /* 档位越高，导轨越亮——「可能真的发出去」要有物理上的警示。 */
  .cs-panel:has(.cs-switch[data-tier='Confirm']) .cs-rail {
    background: var(--cs-brass);
  }
  .cs-panel:has(.cs-switch[data-tier='Auto']) .cs-rail {
    background: var(--cs-alarm);
  }

  /* ── 标题栏 = 「我现在看着谁」 ─────────────────────────────── */
  .cs-head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 0.45rem;
    /* 标题栏自身带导轨的宽度，避免文字贴到竖条上 */
    padding: 0.6rem 0 0.45rem 0.6rem;
    border-bottom: 1px solid var(--cs-border);
  }

  .cs-dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--cs-brass);
    box-shadow: 0 0 0 3px rgb(217 164 65 / 14%);
  }
  .cs-dot.idle {
    background: var(--cs-ink-3);
    box-shadow: none;
  }

  /* 会话名是这一屏最重要的一个「事实」：程序只对它动手。
     所以它最大、最重；原来是 0.82rem 的小字，缩在一张卡片里。 */
  .cs-peer {
    flex: 1;
    min-width: 0;
    font-size: 1.02rem;
    font-weight: 600;
    letter-spacing: 0.01em;
    color: var(--cs-ink);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cs-peer.unknown {
    color: var(--cs-ink-3);
    font-weight: 500;
  }

  .cs-head-tools {
    display: flex;
    gap: 0.2rem;
    flex: none;
  }

  /* 图标按钮。**不用汉字**。
     上一版是「锁 / 画 / 设 / 新」四个字，理由写的是「一个字比图标更容易
     在这么窄的面板里辨认，也省掉一整套图标依赖」——那是给自己找的借口：
     单字图标在 22px 里既不像锁也不像画，四个并排时用户要逐字读，
     而真正的锁形/放大镜形在小尺寸下是一眼可辨的。
     @lucide/svelte 是 ISC 许可，符合本仓库的许可门槛；只取 4 个图标。 */
  .cs-icon {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    color: var(--cs-ink-3);
    background: transparent;
    border: 1px solid transparent;
    border-radius: 7px;
    transition:
      color var(--cs-dur-fast) var(--cs-ease),
      border-color var(--cs-dur-fast) var(--cs-ease),
      background var(--cs-dur-fast) var(--cs-ease);
  }
  .cs-icon:hover:not(:disabled) {
    color: var(--cs-ink);
    border-color: var(--cs-border);
    background: rgb(255 255 255 / 4%);
  }
  .cs-icon:disabled {
    opacity: 0.35;
    cursor: default;
  }

  /* 「正在做」用**旋转**表示，而不是把图标换成省略号——
     换形状会让按钮在 140ms 里跳一下，旋转则是在原地发生。 */
  .cs-icon :global(svg.spin) {
    animation: cs-spin 0.9s linear infinite;
  }

  @keyframes cs-spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .cs-icon :global(svg.spin) {
      animation: none;
    }
  }

  /* ── 异常条：只在真出事时出现 ─────────────────────────────── */
  .cs-alarm {
    flex: none;
    margin: 0.5rem 0 0;
    padding: 0.45rem 0.55rem;
    font-size: 0.7rem;
    line-height: 1.5;
    color: var(--cs-alarm);
    background: rgb(224 104 92 / 10%);
    border: 1px solid rgb(224 104 92 / 34%);
    border-radius: 8px;
  }

  /* ── 候选 ─────────────────────────────────────────────────── */
  /* ⚠ **候选区必须永远占住地方**，哪怕消息很多。
   *
   * 候选是这个面板的产出；它一旦被消息流顶出可视范围，用户就既看不到
   * 也没法点，产品等于没有输出。消息流是「背景资料」，候选是「结果」，
   * 所以消息流封顶 40%、自己滚动，剩下的都给候选。 */
  .cs-cands {
    flex: 1 1 0;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    min-height: 0;
    padding-top: 0.55rem;
    overflow-y: auto;
    scrollbar-width: thin;
    /* 没有候选时让空状态**在候选区里垂直居中**。
       之前它顶在框的上沿、下面空一大片，看着像界面只加载了一半。 */
    justify-content: flex-start;
  }

  .cs-cands:has(.cs-empty) {
    justify-content: flex-start;
  }

  /* 分界标签。用字距拉开的小号字，不是粗体——它在说「下面是另一块」，
     不是在喊「注意这里」。 */
  .cs-cands-head {
    flex: none;
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.45rem 0.1rem 0.4rem;
    margin-bottom: 0.3rem;
    border-top: 1px solid var(--cs-border-strong);
  }

  .cs-cands-title {
    font-size: 0.62rem;
    font-weight: 600;
    letter-spacing: 0.16em;
    color: var(--cs-ink-2);
  }

  .cs-cands-n {
    font-size: 0.6rem;
    color: var(--cs-ink-3);
  }

  /* 空状态只占它该占的那点地方。之前它 `flex:1` 撑满中段，
     看起来像界面坏了——其实只是「还没有候选」。 */
  .cs-empty {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    align-items: center;
    justify-content: center;
    padding: 1.1rem 0.6rem;
    text-align: center;
    border: 1px dashed var(--cs-border);
    border-radius: var(--cs-radius-sm);
  }
  .cs-empty-lead {
    font-size: 0.76rem;
    color: var(--cs-ink-2);
  }
  .cs-empty-sub {
    font-size: 0.66rem;
    color: var(--cs-ink-3);
  }

  /* ── 档位 = 推子 ──────────────────────────────────────────── */
  .cs-foot {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.5rem 0 0.6rem;
    border-top: 1px solid var(--cs-border);
  }
  .cs-foot-label {
    font-size: 0.6rem;
    letter-spacing: 0.12em;
    color: var(--cs-ink-3);
    white-space: nowrap;
  }

  /* ── 紧凑模式：面板窄的时候收紧，不让文字挤成两行 ─────────── */
  .cs-panel.compact {
    padding: 0 0.5rem;
  }
  .cs-panel.compact .cs-head {
    padding-left: 0.5rem;
  }
  .cs-panel.compact .cs-peer {
    font-size: 0.9rem;
  }
  .cs-panel.compact .cs-foot-label {
    display: none;
  }
</style>
