<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import { ArrowLeft, Settings, ChevronDown, ChevronUp } from '@lucide/svelte/icons';
  import WinCtl from '$lib/WinCtl.svelte';
  import MessageStream from '$lib/MessageStream.svelte';
  import PanelSettings from '$lib/PanelSettings.svelte';
  import { IS_DEMO as DEMO } from '$lib/runtime';
  import type { Candidate, DispatchReply, Message, Phase, Tier } from '$lib/types';

  /**
   * 头部那颗返回记号的规格。**和 WinCtl 的三颗一样**（14 / 1.75）。
   *
   * ⚠ 为什么是同一个数：`←` 挨着 `··· − ×` 站着，它们是**同一排记号**。
   * 差一号就会读成「这排是三颗，那颗是别的什么东西」。
   *
   * 形制上它取 `lucide` 的 `ArrowLeft` 而不是手搓 —— 见 `WinCtl.svelte`
   * 开头那条纪律：「先查库，再谈设计」。
   */
  const ICON_SIZE = 14;
  const ICON_SW = 1.75;

  /**
   * 脱离 Tauri 运行？**判据在 `$lib/runtime`** —— 「是不是演示模式」
   * 这件事只有那一个来源，`+layout.svelte` 也要读它。
   *
   * 早先是这里一个局部常量、那里又 `typeof window.__TAURI_INTERNALS__`
   * 探一次，于是 `+layout.svelte` 够不着这套判据、各写各的，
   * 演示页每次打开都抛一个没人接的 `transformCallback`。
   */

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
   * 会话身份闸现在是开着还是关着。**Rust 侧推过来，前端不自己判断。**
   *
   * 关着的时候面板要明说「为什么不动手」——硬约束第 1 条要求这道闸
   * 是**看得见**的；看不见的闸等于没有闸。
   */
  let gateOpen = $state(true);
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
   * 把一帧 readout 灌进界面状态。**三处读取（事件 / 快照 / 演示）共用这一个
   * 入口。**
   *
   * 早先这三处各写一遍解构，而且 `candidates.length === 0` 那种隐式判断
   * 在其中两处还写得不一样——**同一个状态有三种读法**。收进一个函数之后，
   * 判据只有一处。
   */
  /**
   * 一帧 readout 的完整形状。
   *
   * ⚠ `Phase & Extra` 会把判别字段摊平成 `'read' | 'drafting' | ...` 的
   * 联合，于是 `r.phase === 'drafted'` 时 `r.candidates` 就不存在了——
   * 交叉类型在判别联合上是错的形状。四个变体各自带上附加字段才对。
   */
  type FullReadout =
    | (Extract<Phase, { phase: 'read' }> & Extra)
    | (Extract<Phase, { phase: 'drafting' }> & Extra)
    | (Extract<Phase, { phase: 'drafted' }> & Extra)
    | (Extract<Phase, { phase: 'failed' }> & Extra);

  interface Extra {
    reason?: string;
    peer?: string;
    tier?: Tier;
    relock_pending?: boolean;
    /**
     * 会话身份闸开着还是关着。**Rust 侧算好推过来，不靠 reason 的关键词猜。**
     *
     * ⚠ 2026-10-11 实机踩过：闸关上时 reason 写着「本轮不动手」，
     * 而理由栏只放行含「失败/错误/抓不到/收收到/异常」的那几条，
     * 于是闸关了界面却一个字都不说——用户看到的还是「在等〈新会话〉的新消息」。
     */
    gate_open?: boolean;
  }

  function apply(r: FullReadout) {
    phase = r;
    messages = r.messages ?? [];
    candidates = r.phase === 'drafted' ? r.candidates : [];
    reason = r.reason ?? reason;
    peer = r.peer ?? '';
    if (r.tier) tier = r.tier;
    relockPending = r.relock_pending ?? false;
    gateOpen = r.gate_open ?? gateOpen;
    // 候选换了，旧装填自动作废——装填是**某一条具体候选**的状态。
    armedId = '';
  }

  /**
   * 读回「关闭时」和档位这两个**由 Rust 持有**的值。
   *
   * 界面不自己记：用户可以在设置页改，也可以被别的窗口改。显示的
   * 一律是程序认定的那一个，不是用户记得自己点过什么。
   */
  async function pullLocalPrefs() {
    if (DEMO) return;
    try {
      const raw = await invoke<'exit' | 'tray'>('get_close_behavior');
      closeBehavior = raw ?? 'exit';
    } catch (e) {
      // ⚠ 这里原来是一个空的 `catch {}`。
      //
      // 实测（2026-10-10）：整个「`−` 不出现」的排查之所以绕了这么大一圈，
      // 就是因为它在吞异常。写进 `reason` 是因为正文里已经有这条异常条的
      // 位置——读不到 Rust 的推送本来就要让人看见，不该多一种悄悄的失败。
      reason = `读不到「关闭时」：${String(e)}`;
      /* 读不到就按「退出」处理 —— 那是最不容易出事的那个 */
    }
  }

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
      if (DEMO) {
        // ⚠ **演示模式不走 IPC。**
        //
        // 早先这里直接 `await invoke(...)`，在浏览器里必然抛
        // 「__TAURI_INTERNALS__ undefined」，于是按钮点了之后 reason
        // 变成一串天书。而它旁边的按钮**全被禁用**——出错的界面上
        // 一个能按的东西都没有。
        //
        // 演示模式要说清楚它做了什么，**不假装真的重锁了**。
        reason = '演示模式：不会真的重锁。真机会锁定当前打开的那个会话。';
        return;
      }
      await invoke('relock_session');
      reason = '已请求重锁：下一轮起以当前这个会话为基准。';
    } catch (e) {
      reason = `重锁失败：${String(e)}`;
    } finally {
      locking = false;
    }
  }
  /**
   * 面板当前处于哪一阶段。**判断归 Rust**（`contract::Phase`），界面只渲染。
   *
   * ⚠ 早先是靠 `candidates.length === 0` 隐式区分「静默 / 出错 / 刚启动」
   * 三种完全不同的状态——那是界面在猜后端在干嘛。
   */
  let phase = $state<Phase>({ phase: 'read', messages: [] });

  /** 起草中。判据来自 phase，不是「候选为空」。 */
  const drafting = $derived(phase.phase === 'drafting');

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
   * **当前显示在主区的是哪一条。**
   *
   * 主区永远只显示**一条**（默认最看好的）。备选排成一行。
   * 点备选 = 切到这里，**不弹窗、不跳页**。
   *
   * `picked` 只在用户**真的点了备选**时非空；候选集换了一批时它
   * 可能指向一条已经不在的，所以下面用 `sorted.some(...)` 把它归零
   * —— **纯派生，无副作用**。（早先在 `$effect` 里写 state，是
   * 「用副作用同步两个 state」，本来就脆弱。）
   */
  let pickedId = $state('');
  const shownId = $derived(sorted.some((c) => c.id === pickedId) ? pickedId : bestId);
  /** 主区正在显示的那条。找不到时退回第一条，绝不返回 undefined。 */
  const shown = $derived(sorted.find((c) => c.id === shownId) ?? sorted[0]);
  /** 备选 = 除了主区那条以外的全部。 */
  const alts = $derived(shown ? sorted.filter((c) => c.id !== shown.id) : []);

  /** 完整消息列表是否展开。**默认 false** —— 证据是背景资料，不是产出。 */
  let showMessages = $state(false);

  /**
   * 「更多」浮层开没开。识别画布都收在这里，
   * 所以标题栏永远只有三颗记号 + 一个当前档位名。
   */
  let moreOpen = $state(false);

  /**
   * 面板当前是主区还是设置。**两者是同一块面板的两种显示，不是两个窗口。**
   *
   * 早先「设置」调 `open_settings` 弹出另一个 640 × 520 的应用窗口 ——
   * 在 380px 的窄面板里点一下，突然跳到宽一倍的独立窗口，空间关系是拧的。
   * 设置内容不少，但 `docs/面板设计.md` 第一条结构性决定是「修问题的按钮
   * 和问题在同一屏」：设置该用**分节滚动**装进这块面板，不是用开第二个窗口。
   *
   * ⚠ 那个独立窗口已经**删干净了**：`tauri.conf.json` 里不再有 `settings`
   * 条目，`routes/settings` 路由和 `park_settings` 一起删掉。
   * 留着它有两个真实代价，不只是「多一个文件」：
   * 1. 它在 `.setup()` 之前就被创建，于是启动时白跑一份完整的设置界面，
   *    还会**多问一次 `get_close_behavior`**——这正是「`−` 不出现」那次
   *    排查里最早让人搞错方向的那个诱饵（同一进程里它拿到 `tray`、
   *    面板拿到 `exit`）。
   * 2. 托盘菜单的「打开设置」还指着它，等于留了一条回到旧界面的路，
   *    一个程序里就有两套设置界面。
   */
  let settingsOpen = $state(false);

  /**
   * 进入 / 退出设置。
   *
   * ⚠ **进设置前先关掉浮层**：浮层开着的时候切视图，它会**留在设置这一屏上**
   * —— 它的 DOM 在主区里面，而主区被 `{#if}` 换掉了，两边不同步就成了一块
   * 点不掉的残留。
   */
  function toggleSettings(next: boolean) {
    moreOpen = false;
    settingsOpen = next;
  }

  /**
   * 托盘菜单「打开设置」走的事件。Rust 那边 `open_settings` 先把面板
   * show 出来，再 emit 这条。
   *
   * **必须在这里集中处理，别让 Rust 去开第二个窗口**：设置已经在面板里了，
   * Rust 能做的只有「把面板叫到前面」和「告诉它切屏」两件事。
   *
   * 顺带一提，Rust 侧那个事件名原本该进 `events` 常量表；
   * 这里保持字符串字面量，是因为该模块目前是 `pub mod events` 但前端
   * 这侧没有生成绑定。改的时候两边必须一起改。
   */
  onMount(() => {
    let un: (() => void) | undefined;
    let dead = false;
    void (async () => {
      const { listen } = await import('@tauri-apps/api/event');
      if (dead) return;
      un = await listen('menu:settings', () => toggleSettings(true));
    })().catch(() => {
      /* 演示模式没有 IPC。收不到这条事件只是托盘菜单点不到设置，
         面板自己的 `··· → 设置` 不受影响。 */
    });
    return () => {
      dead = true;
      un?.();
    };
  });

  /**
   * 「关闭时」的行为。
   *
   * `exit`（默认）→ WinCtl 只给 ×。**没有托盘时「收起」没有落点**：
   * 一个离屏 + `alwaysOnTop` + `skipTaskbar` 的程序就是用户看不见的僵尸，
   * 它还在替人说话。所以没有落点的动作不提供。
   * `tray` → 多一颗 −，面板离屏、程序继续跑。
   *
   * ⚠ 演示模式下没有 IPC，读不到这个 setting，按 `exit` 处理。
   */
  let closeBehavior = $state<'exit' | 'tray'>('exit');

  /** 档位的中文名。顶边那道手画勾太短，这里用文字补一次。 */
  const TIER_NAME: Record<Tier, string> = {
    Manual: '只填入',
    Confirm: '确认后发',
    Auto: '自动发送',
  };

  /** 档位从低到高。顶边三道短记号按这个顺序排，勾落在当前那一道上。 */
  const TIERS: Tier[] = ['Manual', 'Confirm', 'Auto'];

  /** 概率转整数百分数。Rust 给的是 0..1，界面不自己换算成别的口径。 */
  function pctOf(c: Candidate): number {
    return Math.round(Math.max(0, Math.min(1, c.probability)) * 100);
  }

  /**
   * 备选只给**前 16 字**——够认出「这条是不是我想说的」，
   * 又不至于让人开始读它（一读就变成视线转移）。
   *
   * ⚠ **按字截，不按字符。** `slice(0, 16)` 对中文恰好，但对中英混排
   * （`release-notes-2026-10-09.md`）会把一个单词劈两半——那正是
   * 「B0 被拼成 B 0」那类问题的视觉版。所以在标点或空格处断。
   */
  function preview(text: string): string {
    const LIMIT = 16;
    if (text.length <= LIMIT) return text;
    const head = text.slice(0, LIMIT + 4);
    const cut = Math.max(
      head.lastIndexOf('，'),
      head.lastIndexOf('。'),
      head.lastIndexOf(' '),
      head.lastIndexOf('、'),
      head.lastIndexOf('，'),
    );
    const body = cut > LIMIT / 2 ? head.slice(0, cut) : text.slice(0, LIMIT);
    return body.length >= text.length ? text : body + '…';
  }

  /** 对方的条数。证据条上「对方 6」的 6。 */
  const themCount = $derived(messages.filter((m) => m.speaker === 'Them').length);
  const mineCount = $derived(messages.filter((m) => m.speaker === 'Me').length);

  /**
   * **只在真出事时才说的话。**
   *
   * 原来有一整条状态栏常驻，显示「判断 0 · 起草 0 · 排序 0 · 视觉 0」
   * 和各种说明文字。它的问题是：**大部分时候说的都是「没事」**——
   * 一条一直显示「一切正常」的横幅，会让真的异常变得不显眼。
   *
   * 所以这里**只挑正文中没说过的那种错**：
   * - Rust 报的连接/读取错误 —— 它出现在所有状态里（连「静默」时也可能
   *   正在重试），S6 的正文只显示 `phase.error`，盖不住这一类。
   *
   * ⚠ **两件事已经不算了，删掉它们是这一版的一部分：**
   *
   * 1. 「没认出会话」。S1 用整屏说清了它：标题 + 为什么 + 怎么办的按钮。
   *    底部再挂一条红字，同一件事被说两遍，而且**两遍措辞不同**（一条
   *    「不动手」、一条「确认窗口开着」）—— 用户得自己判断哪句算数。
   *    同一个事实有两条不一致的来源，正是这个项目反复在修的「界面在说谎」。
   *
   * 2. `phase.error`。S6 正文里已经是大标题了，底部再重复一遍没有新信息。
   */
  const problem = $derived.by(() => {
    // 已在 S6 正文里说过了 → 不重复。
    if (phase.phase === 'failed') return '';
    // Rust 侧的连接/读取错误。正文里没有出口，只能挂在这儿。
    if (/失败|错误|抓不到|收不到|异常/.test(reason)) return reason;
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
        if (DEMO) {
          // 演示模式**不碰 Tauri 也不注册监听器**——浏览器里没有 IPC，
          // `probe_anchor` 第一步就会抛；就算它不抛，监听器也只是一串
          // 永不触发的回调，让人以为「界面卡住了」。
          await pullDemo();
          return;
        }
        const ping = await invoke<{ ok: boolean; anchor: string; detail: string }>('probe_anchor');
        if (cancelled) return;
        reason = ping.ok ? `锚点 ${ping.anchor}` : (ping.detail ?? '锚点推不出来');

        const { listen } = await import('@tauri-apps/api/event');
        const un = await listen<FullReadout>('readout', (e) => {
          apply(e.payload);
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
        await pullLocalPrefs();
        const ok = await pullSnapshot();
        if (!ok && !cancelled) reason = '观察循环还没跑出第一帧，稍等一下点「再试一次」。';
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
   *
   * ## `?demo=1`：脱离 Tauri 跑
   *
   * 浏览器里没有 `window.__TAURI_INTERNALS__`，`invoke` 必然抛错——
   * 部署出去的页面只会显示一句「收不到 Rust 的推送」，那对改界面毫无用处。
   *
   * 所以给了它一条**数据源**而不是一条**补丁**：`?demo=1` 时读
   * `/readout.demo.json`，面板的每一个组件、每一条样式、每一个状态
   * 都照常渲染。这样界面设计可以在浏览器里全速迭代，不必每次都去点微信。
   *
   * 演示数据里**不含任何真实聊天内容**（见该文件顶部的说明）——
   * 因为这个页面可能被部署到公开 URL 上。
   */
  async function pullSnapshot() {
    if (DEMO) return pullDemo();
    try {
      const r = await invoke<FullReadout | null>('latest_readout');
      if (!r) return false;
      apply(r);
      return true;
    } catch (e) {
      reason = `读快照失败：${String(e)}`;
      return false;
    }
  }

  /**
   * 演示数据源。**形状照真实 OCR 输出仿**，所以排版是可信的。
   *
   * 演示数据里**不含任何真实聊天内容** —— 这个页面可能被部署到公开 URL。
   */
  async function pullDemo() {
    try {
      // ⚠ `?state=` 选哪一态。**八个态各有自己的数据文件** ——
      // 以前只有一份默认数据，于是 `drafting` / `failed` / `armed`
      // 这些状态在演示模式里根本渲染不出来，而它们恰恰最需要被评审。
      const state = new URLSearchParams(location.search).get('state') ?? 'drafted';
      const r = await fetch(`/demo/${state}.json`)
        .then((x) => x.json())
        .catch(() => fetch('/readout.demo.json').then((x) => x.json()));
      const msgs = r.messages ?? [];
      const cands = r.candidates ?? [];
      // ⚠ **尊重数据里写的 `phase`，别自己从 candidates 推。**
      //
      // 早先这里只有 `cands.length ? 'drafted' : 'read'`——于是
      // `drafting` 和 `failed` 这两相在演示模式里**永远渲染不出来**，
      // 而它们恰恰是最需要被评审的两态（一个是等待中，一个是失败）。
      // 设计评审看不见的状态，等于没设计它。
      //
      // 没写 `phase` 时才回退到从 candidates 猜，兼容老数据。
      const demoPhase: Phase =
        r.phase === 'drafting'
          ? { phase: 'drafting', messages: msgs }
          : r.phase === 'failed'
            ? { phase: 'failed', messages: msgs, error: r.error ?? '演示错误' }
            : cands.length
              ? { phase: 'drafted', messages: msgs, candidates: cands }
              : { phase: 'read', messages: msgs };
      // 展开，不嵌套 —— 判别字段得留在最外层。
      apply({
        ...demoPhase,
        reason: r.reason ?? '',
        peer: r.peer ?? '',
        tier: r.tier ?? 'Manual',
        relock_pending: r.relock_pending ?? false,
      });
      // ⚠ **armed 靠 Tauri 事件推，演示模式里永远等不到。**
      // 所以让数据能直接写它 —— 否则 S5（最不可逆的一态）只能靠脑补。
      armedId = r.armed_id ?? '';
      return true;
    } catch (e) {
      reason = `演示数据读不到：${String(e)}`;
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
   * 「更多」浮层开没开。识别画布和设置都收在这里，
   * 所以标题栏永远只有三颗记号 + 一个当前档位名。
   *
   * 点外面关掉——浮层不封路，也不抢焦点。
   */
  function dismissMore(node: HTMLElement) {
    /** 点浮层**外面** → 关掉。 */
    const outside = (e: PointerEvent) => {
      if (node.contains(e.target as Node)) return;
      moreOpen = false;
    };
    // `true` = 捕获阶段。但**命中浮层内部时不能 stopPropagation** ——
    // 那会连按钮自己的 onclick 一起拦掉。所以这里只判断，不拦。
    node.ownerDocument.addEventListener('pointerdown', outside, true);
    // attachment 的返回函数：卸载时退订。少了它，浮层开合几次之后
    // document 上就攒了好几个监听器，**都还活着**——往一个没人读的
    // state 里写值。这正是 `+page.svelte` 注释里记过的那个老坑。
    return () => node.ownerDocument.removeEventListener('pointerdown', outside, true);
  }

  /**
   * ⚠ **这里曾经有过一个 `changeTier`，已经删掉。**
   *
   * 改档现在走 `PanelSettings` 的 `bind:tier`——设置搬进面板之后，
   * 面板自己不再持有改档入口。两处各写一遍 `set_tier` 就是两处可能
   * 写错的地方，而其中一处（这个）已经变成死代码、连 eslint 都在报。
   *
   * 保留的教训：**用 Rust 回传的真实生效值覆盖界面**。硬约束第 5 条
   * 「默认档最低」，Rust 有可能在解析失败时把档位夹回 Manual，
   * 界面必须显示程序认定的那个，不是用户点过的那个。
   * `PanelSettings` 里那份实现守的就是这条。
   */

  async function act(c: Candidate, action: 'fill' | 'confirm') {
    busy = true;
    if (DEMO) {
      // 演示模式没有后端。**不假装成功**——回一句实话，
      // 而不是让界面显示「已填入」然后什么都没有发生。
      reason = `演示模式：不会真的${action === 'fill' ? '填入' : '发送'}。这条候选是：${c.text}`;
      busy = false;
      return;
    }
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

<!--
  ════════════════════════════════════════════════════════════════
  六个状态。**这是本面板功能被交付的全部方式**，一个不多一个不少。

  `types.ts` 的 4 个 Phase × 3 个 Tier × 5 个 Outcome，加重锁、刷新、
  识别画布、设置、托盘，共十一个功能，下面每个都有落位。

  S1 没认出会话 · S2 静默 · S3 起草中 · S4 已起草 · S5 待发射 · S6 出错

  三条结构性决定，都不是配色：

  1. **修问题的按钮和问题在同一屏。** 重锁和刷新只出现在唯一需要它们的
     两个状态里（S1 / S6），做成正文中的墨底主按钮。标题栏因此只剩
     会话名 + 一组窗口控制。
     （原版正文写「点标题栏的锁」「点右上角的 ⟳」—— 把路指给用户走，
     而按钮在 13px 的角落。）

  2. **档位长在面板顶边上。** 它描述的是「你交出去多少控制权」，
     也就是这块面板和外面世界的关系，不是里面写了什么。所以它不该
     参与内容布局。该控件是从右侧 72px 竖柱（TierColumn）搬上来的——
     那根柱子在 400px 宽的面板上占 18%，而且面板一变宽它就跑到
     余光外了。边框不占宽度，再宽再窄都在视野最上沿。

  3. **armed（待发射）占满整个面板。** 这是全产品最不可逆的一瞬——
     已经有字躺在对方输入框里了。原版把它塞在按钮内的一行小字里
     （旧注释写着「必须显形」）。显形不够：它得占整条顶边。
     此刻**「先不发」和「发送」一样重**。
  ════════════════════════════════════════════════════════════════
-->

<main
  {@attach watchWidth}
  class="cs-panel"
  data-tier={tier}
  data-armed={armedId ? 'y' : 'n'}
  data-compact={compact ? 'y' : 'n'}
>
  <!--
    顶边 = 档位。**三道中性短记号 + 一支红铅笔画的勾**。

    为什么不是横条（2026-10-12 实测）：`--cs-rule` 在纸底
    rgb(243 230 192) 上只有 **1.05 : 1**，是**冷色相**——亮度不跳、
    色相跳，屏上读作一道**划痕**。而且只要它还是一条贯穿 316px 的
    连续直线，它就在跟撕边抢轮廓：撕边之所以成立，靠的正是轮廓被打断。

    为什么是「勾」而不是「圈」（4× 真实像素比对）：
      · 圈是**容器**，勾是**标记**。纸面上最强的形状语言已经是撕口，
        再加一条闭合轮廓 = 两个边界打架。
      · 圈要 28px 高才像个圈，勾只要 14px。560px 的面板上这是两倍的
        纵向代价，换来同样的信息量。
      · 圈的收笔在左下，那一小段过头的墨会跟记号叠在一起，1× 下视线
        先落在那个角上，而不是落在标记本身。勾没有这个瑕疵。

    颜色：这一支是**红铅笔**，不是 `--cs-live`。两者对比度几乎相同
    （5.90 : 1 vs 5.95 : 1）——所以「红 = 警告」这件事**不是对比度能
    定输赢的**，只能靠形（手画勾 vs 实心条）和质感来分。用实心红条
    画档位，它就变成警报；用蜡质红笔画一个勾，它是校样批注。
    armed 那一下才是真警报，那时才换成 `--cs-live`。

    编码：**位置** = 档位集合，**勾落在哪** = 当前档。
  -->
  <div class="cs-tier-edge" aria-hidden="true">
    {#each TIERS as t, i (t)}
      <span class="cs-tier-slot" data-tier-idx={i}>
        <svg class="cs-tier-anchor" viewBox="0 0 100 5" preserveAspectRatio="none" height="5">
          <path d="M35 2.8 C44 1.9 56 3.6 65 2.5 L65 3.9 C56 4.3 44 3.3 35 3.9 Z" />
        </svg>
        <svg class="cs-tier-check" viewBox="0 0 100 20" height="20">
          <path d="M39 10.5 C42 12.8 43.6 14.6 46 16 C50.5 12.8 55.5 8 61.5 4" />
        </svg>
      </span>
    {/each}
  </div>

  <!--
    头部 = 「我现在看着谁」+ 窗口控制。

    会话名是这一屏最重要的一个事实：程序只对它动手（硬约束第 1 条）。
    它同时是**拖拽区**——抓手就是这两个字。

    ⚠ 顶边那 3px 档位条太细，当不了抓手，所以拖拽从会话名开始、
    到按钮前为止。WinCtl 内部 `data-no-drag` 阻断冒泡，否则点 ×
    会变成拖窗口。
  -->
  <header class="cs-head" data-tauri-drag-region>
    <!--
      ⚠ **返回记号长在头部，不在设置内容里面。**

      三条理由，任意一条都够：

      1. 头部是 `flex: none`，**永远在滚动区之外**。设置是一整列要滚的
         内容，把 `←` 放在里面，它会跟着内容滚走 —— 而用户正是最需要
         它的时候（滚到一半想回去）在找不到它。
      2. 头部的**位置不随节数变化**。放在内容里的话，加一节它就挪一格，
         肌肉记忆永远对不上。
      3. 设置这一屏需要**一直看得见的出口**，不是要一个「取消本次改动」。
         这里所有改动都是即发的（Rust 侧改完就生效），所以那颗记号必须说
         「回到主区」，**不能**写成「取消」—— 界面不能承诺它做不到的事。

      `···` 也留着同样的入口：设置态下它那一条变成「回到主区」，
      两条路通同一个地方。
    -->
    {#if settingsOpen}
      <button
        class="cs-back"
        type="button"
        data-no-drag
        onclick={() => toggleSettings(false)}
        title="回到主区"
        aria-label="回到主区"
      >
        <ArrowLeft {ICON_SIZE} {ICON_SW} strokeWidth={ICON_SW} />
      </button>
      <span class="cs-view-title">设置</span>
    {:else}
      <span
        class="cs-peer"
        class:unknown={!peer}
        data-tauri-drag-region
        title={peer || '没认出会话'}
      >
        {peer || '没认出会话'}
      </span>
    {/if}

    <div class="cs-head-right">
      {#if !settingsOpen}
        <span class="cs-tier-name">{TIER_NAME[tier]}</span>
        <!--
          ⚠ 设置是**直接给出来**的，不藏在 `···` 里面。（2026-10-10 实测反馈）

          早先是 `··· → 设置` 两步：先点开一个浮层，**浮层本身还要再占一屏**，
          然后才点得到设置。一个一年可能用两次的入口，让人先开一次菜单、
          再在一组新出现的按钮里挑——这既慢，又让「设置在哪」变成一个需要
          记住的事。**常用度低不等于可以藏起来**：正因为低频，它才必须在原地
          等着，不能要求用户记得路径。

          所以：齿轮直接在头部，`···` 里**不再有**设置那一项。同一时刻只留
          一颗控制设置的记号——设置态下齿轮让位给头部那颗 `←`，位置不变。
          一颗键不能有两种含义，两颗键也不能管同一件事。

          位置在 WinCtl **左边**：窗口三键永远在最右，这是系统约定，
          别的键插到它左边才对。
        -->
        <!--
          ⚠ 图标必须用 `Settings`（齿轮），**不是 `Settings2`**。

          2026-10-11 实机截图：表头里那个记号是**两根调节滑块**——
          那是 lucide 的 `settings-2`，而 `settings` 才是齿轮。
          它紧挨着 SegSlider 摆着，一个滑块记号在那里会被读成「档位调节」，
          而它管的是「设置」。**代码自称齿轮、画面画的是滑块**，
          这正是本项目反复在查的那类不一致。

          一颗键不能有两种含义：这一颗进设置，档位是隔壁那颗滑块的事。
        -->
        <button
          class="cs-gear"
          class:on={settingsOpen}
          type="button"
          data-no-drag
          onclick={() => toggleSettings(true)}
          title="设置"
          aria-label="设置"
        >
          <Settings {ICON_SIZE} {ICON_SW} strokeWidth={ICON_SW} />
        </button>
      {/if}
      <WinCtl
        {closeBehavior}
        onquit={() => invoke('quit_app').catch(() => {})}
        onhide={() => invoke('hide_panel').catch(() => {})}
        onmore={() => (moreOpen = !moreOpen)}
      />
    </div>
  </header>

  <!--
    ⚠ **设置是一块互斥的主区，不是叠在主区上面的一层。**

    早先的 `open_settings` 是**另一个窗口**，于是设置和主区可以同时占着
    屏幕。现在它是同一块面板的另一种显示 —— 两个都显示会让「我现在看到
    的到底是不是正在替我说话的那一句」变得没法回答。

    所以这里用 `{#if}` 换，不是用 `position: absolute` 盖：
    盖住的话底层那份内容还在滚，滚轮会滚到看不见的那一层去。

    顶边那条 3px 档位条和头部**留在外面**：
    - 顶边 = 「你交出去多少控制权」，设置里也要改它，它得一直在。
    - 头部 = 窗口唯一的出口 + 返回主区的那颗记号，不能跟着内容消失。
  -->
  {#if settingsOpen}
    <!--
      ⚠ `···` 这一行**必须在 `PanelSettings` 之前**。

      `PanelSettings` 是 `flex: 1 1 auto` + 自己的滚动容器，它会把后面的
      兄弟节点顶到面板最底下 —— 浮层就会掉到所有设置项的下面去，
      变成「点 `···` 然后去屏幕最下面找」。放在前面它就贴在头部下面，
      和主区里的位置一致。
    -->
    {@render moreMenu()}
    <!--
      `tier` / `closeBehavior` 是 `$bindable` 双向绑：设置里改档，
      面板顶边那条 3px 和头部的 `−` 立刻跟着变。**不留第二份。**
    -->
    <PanelSettings bind:tier bind:closeBehavior />
  {:else}
    <!--
      ⚠ **状态是状态，按钮是按钮 —— 两者不能焊在一起。**

      早先这里是一个 `<button>` 把整条包住：状态文字「对方 2 · 我 1」和展开
      控件一起塞进可点击元素里，而那颗 button **没有任何按钮形态**——无边框、
      无底色、无 hover 态，`font-size` 还是 11px。三个后果叠在一起：

        · 这一行看起来就是一行字，用户不知道哪里能点；
        · 点下去下方长出消息列表，可那颗「按钮」自己什么都没变，
          于是下次还是不确定它到底管不管用；
        · 更要命的是**状态被迫伪装成可点的东西**。「对方 2 · 我 1」是
          **信息**，用户读到它只想知道读到哪了，不想点它。

      这与 `docs/面板设计.md` 结构性决定 ① 是同一条思路：动作和它回答的
      问题必须分得清，不能靠「把两者涂成同一个颜色」蒙混过关。

      所以现在左边那份是纯 `<span>`，不可点、不可聚焦；右边那颗独立按钮
      **只**回答「展开 / 收起」，形态和 `WinCtl` 三颗完全一致
      （同样的 `ICON_SIZE` / `ICON_SW`、同样的 hover 7% / active 12%）。
      可发现性靠**按钮自己的形态**，不靠把整行都涂成可点的。
    -->
    <div class="cs-strip">
      <span class="cs-strip-lead">
        {#if drafting}
          正在起草
        {:else if !peer}
          还没认出会话
        {:else}
          对方 {themCount} · 我 {mineCount}
        {/if}
      </span>

      <button
        class="cs-strip-toggle"
        type="button"
        title={showMessages ? '收起完整消息列表' : '展开完整消息列表'}
        aria-label={showMessages ? '收起完整消息列表' : '展开完整消息列表'}
        aria-expanded={showMessages}
        onclick={() => (showMessages = !showMessages)}
      >
        <!--
          换掉手搓的 `▴` 字符 + `rotate(180deg)`：那是个**字体**，
          不同机器上形状和位置都不一样（`▴` 在很多等宽字体里根本不是三角形）。
          lucide 的 chevron 在同一个 viewBox 里，三颗记号天然对齐。
        -->
        {#if showMessages}
          <ChevronUp {ICON_SIZE} {ICON_SW} strokeWidth={ICON_SW} />
        {:else}
          <ChevronDown {ICON_SIZE} {ICON_SW} strokeWidth={ICON_SW} />
        {/if}
      </button>
    </div>

    <!--
    展开时才出现的完整消息列表。**默认不占地方。**
    展开是把内容**往下推**，不是盖在台词上面 —— 实测台词从 y=107 挪到
    y=300。盖住不行：用户要一边看原话一边看拟的那句，那是这个动作的全部意义。
  -->
    {#if showMessages}
      <MessageStream {messages} />
    {/if}

    <!--
    「更多」浮层。**两种视图下都在**（snippet，两个地方渲染同一份）。

    早先它只写在主区里。设置搬进面板之后如果还留在那儿，设置态下点 `···`
    会**什么也不发生** —— 头部的记号还在，浮层不在。那是「点下去没反应」，
    正是这个项目反复在修的东西。所以这里提成一个 snippet，两边渲染同一份。

    它是**文档流里的一行**，不是浮在上面的层：浮层会盖住面板内容，
    而这一屏的记号只有三颗 + 一个档位名，不该再被一层临时的东西遮住。
  -->
    {@render moreMenu()}

    <!--
    主区。候选是**产出**，它会在用户没操作时自己出现，所以必须
    `aria-live`。`polite` 而非 `assertive`：候选不是错误，不该打断
    当前朗读。
  -->
    <section class="cs-body" aria-label="候选回复" aria-live="polite">
      {#if armedId && shown}
        <!--
        ══ S5 · 待发射 ══

        ⚠ **那句话必须留在屏上。**
        此刻它已经在对方输入框里了 —— 也就是说，它**已经离开了这个面板**，
        而用户正要决定要不要让它继续走下去。那句话正是他要判断的东西，
        把它拿掉等于让他凭记忆去撤回一句他看不见的话。

        所以 armed 态**不是另一套内容，是同一份内容加一层状态**：
        台词不变，下面多出两个并排的动作。

        ⚠ **两个按钮的措辞都改过一轮。**
        「发出去」是从**收信那侧**说的 —— 可用户就坐在微信前面，
        他才是发的那个；把视角写在别人位置上，一颗按钮就把用户的
        角色说反了。所以是「发送」。

        左键那颗更隐蔽：写「撤回」「拿回来」都在暗示**输入框被清空了**，
        可它只清掉「程序要替你发」这件事，那行字仍然在输入框里、
        用户自己在微信里看得见也能改。说自己做不到的事就是假话。
        「先不发」是用户犹豫时嘴里的话，也是真的。
      -->
        <!--
        **状态行说「在哪」，不说「会怎样」。**

        「按下去就发出去了」跟旁边那颗写着「发送」的按钮是同一句话，
        说两遍等于没说，还把红色从「一个信号」稀释成「一段话」。

        真正需要说清的是**它已经在哪儿了** —— 这一件事整屏没有任何
        别的地方写着，而它决定了用户此刻站在哪个环节上。
      -->
        <p class="cs-armed-where">在小周的输入框里</p>
        <p class="cs-line selectable">{shown.text}</p>

        <div class="cs-armed-row">
          <!--
          按钮写**动作**，而且是准确的那个动作。

          按下去清掉的只是「程序要替你发」这件事 —— 输入框里那行字
          **仍然在**，用户自己在微信里看得见、也能改能删。
          所以「撤回」「拿回来」都在暗示输入框被清空了，那是**假话**。

          「先不发」是用户犹豫时嘴里的话，也是真的：程序不再替你发。
        -->
          <button
            class="cs-btn cs-btn-ghost"
            type="button"
            onclick={() => (armedId = '')}
            disabled={busy || DEMO}
          >
            先不发
          </button>
          <button
            class="cs-btn cs-btn-live"
            type="button"
            onclick={() => act(shown, 'confirm')}
            disabled={busy || DEMO}
          >
            发送
          </button>
        </div>
      {:else if sorted.length > 0}
        <!-- ══ S4 · 已起草 ══ -->
        <p class="cs-caption">替你对{peer}说</p>
        <p class="cs-line selectable">{shown.text}</p>

        <div class="cs-meta">
          <span class="cs-meta-pct">把握 {pctOf(shown)}%</span>
          {#if alts.length > 0}
            <span class="cs-meta-alt">{alts.length} 个备选 ↑↓</span>
          {/if}
        </div>

        {#if alts.length > 0 && !armedId}
          <div class="cs-alts" role="group" aria-label="备选">
            {#each alts as c (c.id)}
              <button
                class="cs-alt"
                type="button"
                onclick={() => (pickedId = c.id)}
                title={`${c.text}（把握 ${pctOf(c)}%）`}
              >
                <span class="cs-alt-pct">{pctOf(c)}%</span>
                <span class="cs-alt-txt">{preview(c.text)}</span>
              </button>
            {/each}
          </div>
        {/if}

        <div class="cs-act-row">
          <span class="cs-act-label">填入输入框</span>
          <button
            class="cs-btn cs-btn-ink"
            type="button"
            onclick={() => act(shown, 'fill')}
            disabled={busy || DEMO}
          >
            填入
          </button>
        </div>
        <p class="cs-act-note">停在你微信的光标处，不发。</p>
      {:else if drafting}
        <!-- ══ S3 · 起草中 ══ -->
        <div class="cs-idle">
          <p class="cs-idle-lead cs-idle-busy">正在成形…</p>
          <p class="cs-idle-sub">通常几秒</p>
        </div>
      {:else if phase.phase === 'failed'}
        <!-- ══ S6 · 出错。修问题的按钮就在正文里 ══ -->
        <div class="cs-idle">
          <p class="cs-idle-lead">{phase.error}</p>
          <button
            class="cs-btn cs-btn-ink cs-idle-btn"
            type="button"
            onclick={refresh}
            disabled={refreshing}
          >
            {refreshing ? '正在读…' : '再试一次'}
          </button>
        </div>
      {:else if !peer}
        <!-- ══ S1 · 没认出会话。重锁按钮就在正文里 ══ -->
        <div class="cs-idle">
          <p class="cs-idle-lead">面板不认人，就不动手</p>
          <p class="cs-idle-sub">这是硬约束，不是故障。认错人 → 在错的聊天里替你说话。</p>
          <button
            class="cs-btn cs-btn-ink cs-idle-btn"
            type="button"
            onclick={relock}
            disabled={locking || relockPending}
          >
            {locking ? '正在锁…' : relockPending ? '已请求，下一轮生效' : '锁定当前会话'}
          </button>
        </div>
      {:else if !gateOpen}
        <!--
        ══ 闸关着。这不是「在等」，是「故意不动手」══
        ⚠ 2026-10-11 实机：这一支早先不存在，闸关着的时候界面显示的还是
        「在等〈新会话〉的新消息」——用户会以为程序正盯着那个新会话，
        而它其实什么都没做。硬约束第 1 条要求这道闸是**看得见**的。
      -->
        <div class="cs-idle">
          <p class="cs-idle-lead">不认这个人，就不动手</p>
          <p class="cs-idle-sub">{reason}</p>
          <button
            class="cs-btn cs-btn-ink cs-idle-btn"
            type="button"
            onclick={relock}
            disabled={locking || relockPending}
          >
            {locking ? '正在锁…' : relockPending ? '已请求，下一轮生效' : '锁定当前会话'}
          </button>
        </div>
      {:else}
        <!-- ══ S2 · 静默。这是面板 90% 时间的样子 ══ -->
        <div class="cs-idle">
          <p class="cs-idle-lead">在等{peer}的新消息</p>
          <p class="cs-idle-sub">发来了才起草，不会空转</p>
        </div>
      {/if}
    </section>

    <!--
    只在**真出事**时出现。正常时不占地方 —— 一条一直显示「一切正常」
    的横幅会把真的异常稀释掉。
  -->
    {#if problem && !drafting && sorted.length === 0}
      <p class="cs-alarm" role="alert">{problem}</p>
    {/if}
  {/if}
</main>

<!--
  「更多」浮层。定义在 `<main>` 之外、但**归属这个面板**。

  ⚠ 为什么不是组件：它只有两个按钮、四个状态（`moreOpen` / `settingsOpen`
  / `DEMO` / `dismissMore` attachment），而 `dismissMore` 要拿
  `node.ownerDocument`。抽成组件就得把这一整套当 props 递下去 ——
  那一层封装换来的只是文件变短，代价是多一层间接。
  Svelte 5 的 `{#snippet}` 就是为「同一段标记，两个地方渲染」准备的。

  「设置」这一项**从浮层里拿掉了**（2026-10-10 实测反馈）：设置改成头部
  的齿轮直达，浮层里再来一项就是同一件事有两个入口。设置现在是这一块面板
  的一屏，不是另一个窗口——Rust 侧的 `open_settings` 只服务托盘菜单，
  它把面板叫到前面再 emit `menu:settings` 让面板自己切屏。
-->
{#snippet moreMenu()}
  {#if moreOpen}
    <div class="cs-more" {@attach dismissMore}>
      <button
        class="cs-more-item"
        type="button"
        onclick={() => {
          moreOpen = false;
          invoke('open_debug').catch(() => {});
        }}
        disabled={DEMO}
      >
        识别画布
      </button>
    </div>
  {/if}
{/snippet}

<style>
  /* ══════════════════════════════════════════════════════════════
     这是一张纸。

     分组全靠**留白和字号**，一根边框都不画 —— 刻线 (#D3DADD,
     1.24 : 1) 只在确实需要一条线的地方出现，而且永远不会被看见。
     ══════════════════════════════════════════════════════════════ */

  .cs-panel {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    /* PROBE-SHEET 临时：纸由 SVG 自己画，边缘交给合成器抗锯齿 */
    background: url(/probe-sheet.svg?v=21) center / 100% 100% no-repeat;
    padding: 40px 32px 30px; /* PROBE-SHEET: 无顶部装饰，纸的撕口就是顶边（17.5px）+ 留白 */
    color: var(--cs-ink);
    overflow: hidden;
  }

  /* ── 顶边：档位 ───────────────────────────────────────────
   *
   * 三道短记号是**中性**的，不编码档位；档位由那支红铅笔的**勾**
   * 落在哪一道决定。理由记在 markup 注释里。
   *
   * ⚠ 勾是 SVG path，交给合成器抗锯齿。之前那道二值的 CSS 横条
   * 读作划痕，一半原因就是它没有中间灰阶。 */
  .cs-tier-edge {
    flex: none;
    display: flex;
    height: 20px;
  }

  .cs-tier-slot {
    position: relative;
    flex: 1;
  }

  /*  记号：短且居中。**长横线套一个勾会读成「0」**，
   *  而长横线配档位又会退回到那根贯穿纸面的机械条。 */
  .cs-tier-anchor {
    position: absolute;
    left: 0;
    top: 9px;
    width: 100%;
    height: 5px;
  }

  .cs-tier-anchor path {
    fill: var(--cs-tier-ink);
    opacity: 0.26;
  }

  .cs-tier-check {
    position: absolute;
    left: 0;
    top: 0;
    width: 100%;
    height: 20px;
    overflow: visible;
    opacity: 0;
    transition: opacity var(--cs-dur) var(--cs-ease);
  }

  .cs-tier-check path {
    fill: none;
    stroke: var(--cs-pencil-red);
    stroke-width: 2.1;
    stroke-linecap: round;
    stroke-linejoin: round;
    opacity: 0.88;
  }

  /*  勾只落在当前档那一道上。位置**不是**唯一线索——三道记号
   *  一模一样，所以「哪一道」只由勾的位置决定；好在勾本身是
   *  位置无关的形状，不依赖颜色分辨（WCAG 1.4.1）。 */
  .cs-panel[data-tier='Manual'] .cs-tier-slot[data-tier-idx='0'] .cs-tier-check,
  .cs-panel[data-tier='Confirm'] .cs-tier-slot[data-tier-idx='1'] .cs-tier-check,
  .cs-panel[data-tier='Auto'] .cs-tier-slot[data-tier-idx='2'] .cs-tier-check {
    opacity: 1;
  }

  /*  Auto：不可逆，勾从红铅笔换成**警报红**。
   *  这一步是红铅笔唯一让位给 `--cs-live` 的地方。 */
  .cs-panel[data-tier='Auto'] .cs-tier-slot[data-tier-idx='2'] .cs-tier-check path {
    stroke: var(--cs-live);
    opacity: 1;
  }

  /*  armed：这是**状态**，不是档位 ——
   *
   *  ⚠ 此刻面板在等一个不可逆的确认，勾必须盖住「红铅笔批注」
   *  那层日常语气，直接升成警报红。反过来只改 Auto 那一档是不够的：
   *  armed 可以发生在任何档位上。 */
  .cs-panel[data-armed='y'] .cs-tier-check path {
    stroke: var(--cs-live);
    opacity: 1;
    transition: none;
  }

  /* ── 头部 ─────────────────────────────────────────────── */
  .cs-head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 8px 6px 14px;
    /* 顶边那 3px 档位条上面留一点，让它读得出是独立的一条线 */
    margin-top: 10px;
  }

  .cs-peer {
    flex: 1 1 auto;
    min-width: 0;
    font-size: 14px;
    font-weight: 600;
    color: var(--cs-ink);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    cursor: grab;
  }

  .cs-peer.unknown {
    color: var(--cs-mark);
    font-weight: 400;
  }

  /* ── 返回主区（设置态）────────────────────────────────────────
   *
   * ⚠ **盘子、圆角、悬停底色逐项照抄 `.cs-win`（WinCtl 里那颗）。**
   * 它和 `··· − ×` 是同一排记号，一颗换了规格，这一排就散了。
   *
   * 唯一的两处差别，都有理由：
   * - **不悬停变红。** 红只给不可逆（发送、退出）。返回不是不可逆，
   *   它随时能再进来。红色一旦给了「可逆的导航」，它就不再是信号了。
   * - 悬停是墨的 7%，和 `···` 一样，不比 `×` 重 —— 「最容易误点的键
   *   不该长成主要动作的样子」这条已经写在 WinCtl 的注释里。 */
  .cs-back {
    display: grid;
    place-items: center;
    width: 28px;
    height: 32px;
    flex: none;
    margin-left: -4px;
    border-radius: var(--cs-radius-sm);
    color: var(--cs-mark);
    transition:
      background var(--cs-dur-fast) var(--cs-ease),
      color var(--cs-dur-fast) var(--cs-ease);
  }

  .cs-back:hover {
    background: rgb(20 26 31 / 7%);
    color: var(--cs-ink);
  }

  .cs-back:active {
    background: rgb(20 26 31 / 12%);
  }

  /* 齿轮。**和 `.cs-back` 同一副样子** —— 它们是同一件事的两种朝向：
     齿轮进设置，`←` 回主区。位置都在头部左边，形态就该一模一样，
     否则用户会以为它们是两个功能。 */
  .cs-gear {
    display: grid;
    place-items: center;
    width: 28px;
    height: 32px;
    border-radius: var(--cs-radius-sm);
    color: var(--cs-mark);
    flex: none;
    transition:
      background var(--cs-dur-fast) var(--cs-ease),
      color var(--cs-dur-fast) var(--cs-ease);
  }

  .cs-gear:hover {
    background: rgb(20 26 31 / 7%);
    color: var(--cs-ink);
  }

  .cs-gear:active {
    background: rgb(20 26 31 / 12%);
  }

  /* 设置态的标题。**不是 `<h1>`** —— 面板不是文档，这一屏没有标题层级，
   * 它只是「头部那两个字告诉你在哪」。14px / 600，与 `.cs-peer` 同规格：
   * 会话名和「设置」是同一个位置上的同一个角色。 */
  .cs-view-title {
    flex: 1 1 auto;
    min-width: 0;
    font-size: 14px;
    font-weight: 600;
    color: var(--cs-ink);
    white-space: nowrap;
    cursor: grab;
  }

  .cs-head-right {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  /* 当前档位名。**用文字再说一遍顶边的长度**，因为 3px 在
   * 窄面板上很难读 —— 冗余编码是故意的，不是浪费。

   ⚠ `docs/面板设计.md` 的字体分工把「档位名」和「证据条 / 把握 /
   * 备选%」并列在同一行，注明「这是机器读出来的」，对应**等宽**。
   * 这里原来没声明字体族，全局雅黑把它带过去了 —— 于是那条设计
   * 决定在这一行**静默失效**：`app.css` 明说字体分工是给色盲兜底的
   * 第二条通道，缺了它，这个产品对色盲用户就只剩颜色一条路了。 */
  .cs-tier-name {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 11px;
    color: var(--cs-mark);
    letter-spacing: 0.2px;
    white-space: nowrap;
    /* 档位名是**状态**，右边三颗是**控制**。这一行原来是统一 6px，
     * 于是「只填入」和齿轮挤成一团、还和后两颗同色（都走
     * `--cs-mark`），看上去像一个四元素按钮组。
     * 加 6px 把两组拉开：组间 12px，组内仍是 6px。
     * **不加竖线** —— 400px 宽的面板上，竖线是最贵的一种分隔，
     * 而这里的间距差已经够读出分组。 */
    margin-right: 6px;
  }

  .cs-panel[data-tier='Auto'] .cs-tier-name {
    color: var(--cs-live);
    font-weight: 600;
  }

  /* ── 证据条 ───────────────────────────────────────────── */
  /* 一行两件事，各占一头：左边是**信息**，右边是**动作**。
   * 早先是「信息裹在动作里」，于是那一行既像字又像按钮。 */
  .cs-strip {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 0 14px 6px;
    /* 机器认出来的字 → 等宽。这和 .cs-line 的衬线是同一件事的
     * 两面：一个说「这是机器读的」，一个说「这是替你写的」。 */
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 11px;
  }

  /* 状态本身**不可点**：不加 `cursor: pointer`，也不进 Tab 序。
   * 它只回答「读到哪了、轮到谁了」，用户读到就走。 */
  .cs-strip-lead {
    color: var(--cs-pencil);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* 展开控件是一颗**真按钮**：有盘子、有 hover、有 active。
   * 尺寸 / 描边 / 配色与 `WinCtl` 三颗一致，面板里于是只有一套记号的形态。 */
  .cs-strip-toggle {
    flex: none;
    display: grid;
    place-items: center;
    width: 28px;
    height: 24px;
    margin: -4px -6px -4px 0;
    border-radius: var(--cs-radius-sm);
    color: var(--cs-mark);
    transition:
      background var(--cs-dur-fast) var(--cs-ease),
      color var(--cs-dur-fast) var(--cs-ease);
  }

  /* 墨的 7% —— 是「这一格亮了」，不是「这是一颗按钮」。与 WinCtl 同值。 */
  .cs-strip-toggle:hover {
    background: rgb(20 26 31 / 7%);
    color: var(--cs-ink);
  }

  .cs-strip-toggle:active {
    background: rgb(20 26 31 / 12%);
  }

  /* ── 「更多」浮层 ─────────────────────────────────────── */
  .cs-more {
    flex: none;
    display: flex;
    gap: 4px;
    padding: 0 14px 8px;
  }

  .cs-more-item {
    font-size: 11.5px;
    color: var(--cs-pencil);
    padding: 5px 10px;
    border: 1px solid var(--cs-rule-strong);
    border-radius: var(--cs-radius-sm);
    transition:
      background var(--cs-dur-fast) var(--cs-ease),
      color var(--cs-dur-fast) var(--cs-ease);
  }

  .cs-more-item:hover:not(:disabled) {
    background: rgb(20 26 31 / 7%);
    color: var(--cs-ink);
  }

  .cs-more-item:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* ── 主区 ─────────────────────────────────────────────── */
  .cs-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    padding: 4px 14px 12px;
  }

  .cs-caption {
    margin: 0 0 6px;
    font-size: 11px;
    /* ⚠ **不能是标注档。**
     *
     * 它写着「替你对<某人>说」——**这句话是谁说的**，是这个面板最要紧的
     * 一条归属信息，字号还最小。`--cs-mark` 只有 3.81 : 1，而 11px
     * 属于「小字正文」，WCAG 要的是 4.5 : 1。
     *
     * 降档省下的是几乎看不出的差别，赔上的是读不清。
     */
    color: var(--cs-pencil);
  }

  /* 替你写下的那句 → 衬线。它是这一屏唯一的「人声」。 */
  .cs-line {
    margin: 0;
    /* ⚠ 字体栈必须**以 Windows 上真有的字体收尾**。
     *
     * 'Songti SC' / 'Source Han Serif' 都是 macOS / Linux 常见字。
     * 这是一个 Windows 独占应用，只写那两个的话在用户机器上会
     * 静默掉到浏览器默认无衬线 —— 衬线这个设计决定就整个丢了，
     * 而且**看不出哪里错了**。
     * SimSun / 宋体 是 Windows 一定有的，它兜底。 */
    font-family:
      'Songti SC', 'Source Han Serif SC', 'Noto Serif CJK SC', 'Noto Serif SC', 'SimSun', '宋体',
      serif;
    font-size: 19px;
    line-height: 1.5;
    color: var(--cs-ink);
  }

  .cs-meta {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin: 12px 0 0;
    font-size: 10.5px;
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
  }

  .cs-meta-pct {
    color: var(--cs-pencil);
  }

  .cs-meta-alt {
    color: var(--cs-mark);
  }

  /* ── 备选 ─────────────────────────────────────────────── */
  /* 同一句话的不同版本，**不是并列的几件事**。所以排成一行、
   * 给前 16 字，不做成卡片。 */
  .cs-alts {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 10px 0 0;
  }

  .cs-alt {
    display: flex;
    align-items: baseline;
    gap: 8px;
    text-align: left;
    padding: 5px 8px;
    border-radius: var(--cs-radius-sm);
    transition: background var(--cs-dur-fast) var(--cs-ease);
  }

  .cs-alt:hover {
    background: rgb(20 26 31 / 7%);
  }

  .cs-alt-pct {
    flex: none;
    width: 26px;
    font-size: 10px;
    color: var(--cs-mark);
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
  }

  .cs-alt-txt {
    flex: 1 1 auto;
    min-width: 0;
    font-size: 12px;
    color: var(--cs-pencil);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* ── 动作 ─────────────────────────────────────────────── */
  .cs-act-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 16px 0 0;
  }

  .cs-act-label {
    flex: 1 1 auto;
    font-size: 13px;
    color: var(--cs-pencil);
  }

  .cs-btn {
    flex: none;
    padding: 8px 18px;
    border-radius: var(--cs-radius-sm);
    font-size: 12.5px;
    font-weight: 600;
    transition:
      background var(--cs-dur-fast) var(--cs-ease),
      color var(--cs-dur-fast) var(--cs-ease),
      opacity var(--cs-dur-fast) var(--cs-ease);
  }

  .cs-btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  /* 主动作：墨底纸字。它是屏上唯一实心的东西。 */
  .cs-btn-ink {
    background: var(--cs-ink);
    color: var(--cs-paper-raised);
  }

  .cs-btn-ink:hover:not(:disabled) {
    background: var(--cs-ink-2);
  }

  /* 不可逆：红。**只有这一颗**，和顶边是同一个红。 */
  .cs-btn-live {
    background: var(--cs-live);
    color: #fff;
    /* ⚠ 这条**透明**边框是给尺寸对齐用的，不是装饰。
     *
     * 2026-10-10 本机实测：这一行里两颗按钮虽然都是 `flex: 1 1 0`，
     * 但 `box-sizing: border-box` 下 flex 均分的是**内容盒**——
     * `.cs-btn-ghost` 有 `border: 1px solid`，边框留在外框上，
     * 于是「先不发」外宽 173px、「发送」171px，差 2px。
     * 而上面那句注释写的是「撤回：和白底主按钮**同尺寸**」——
     * 设计意图是同尺寸，实测不是。这里补一条透明边框把外框补齐。
     *
     * 为什么用 transparent 而不是去掉 ghost 的边框：ghost 那颗的
     * 描边**就是它的样子**（白底 + 描边），去掉就不是那颗按钮了。 */
    border: 1px solid transparent;
  }

  .cs-btn-live:hover:not(:disabled) {
    background: var(--cs-live-dim);
  }

  /* 撤回：和白底主按钮**同尺寸**，因为此刻它和「发出去」一样重。
   * 做成更小的一颗 = 暗示可以随便撤回，那不是真的。 */
  .cs-btn-ghost {
    background: none;
    color: var(--cs-ink);
    border: 1px solid var(--cs-rule-strong);
  }

  .cs-btn-ghost:hover:not(:disabled) {
    background: rgb(20 26 31 / 7%);
  }

  .cs-armed-row {
    display: flex;
    gap: 8px;
    margin: 10px 0 0;
  }

  .cs-armed-row .cs-btn {
    flex: 1 1 0;
    text-align: center;
  }

  /* ⚠ **红色只有这一句。**
   *
   * 早先是两句红的：「已经填进 X 的输入框」+「现在按下去，这句话就发出去了」
   * 共 28 个红字。红色是**信号**，一句话说清就够；一旦写成段落，它就
   * 退化成「红色文字」，警告力归零。
   *
   * 前那一句还跟头部重复了两重：顶边已经整条红了（= armed），
   * 标题栏也写着是谁。这儿再说一遍是在同一屏里把同一件事讲第三遍。
   *
   * 所以只剩一句，说的是**下一个动作的后果**，不是当前状态
   * （状态由顶边说）。 */
  /* ⚠ **这里没有红色。**
   *
   * armed 态的红色出现两处就够：顶边整条（状态）、发送键（不可逆的动作）。
   *
   * 早先这里是**一句红字**「按下去就发出去了」—— 它跟旁边那颗写着
   * 「发送」的按钮是同一句话；再早先是**两句**、28 个字。红色是信号，
   * 写成段落它就退化成「红色文字」，警告力归零。
   *
   * 这一行是**机器状态**，跟证据条、把握、备选百分比同一档：
   * 等宽 + 铅笔色。它说「在哪」，不说「会怎样」。
   */
  .cs-armed-where {
    margin: 0 0 6px;
    font-size: 11px;
    color: var(--cs-pencil);
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
  }

  .cs-act-note {
    margin: 6px 0 0;
    font-size: 10.5px;
    color: var(--cs-mark);
  }

  /* ── 静默 / 出错：都要有实体，不能是一个空盒子 ─────────── */
  .cs-idle {
    margin: auto 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
  }

  .cs-idle-lead {
    margin: 0;
    font-size: 15px;
    color: var(--cs-ink);
  }

  /* 起草中：字是**灰的**，因为它还没定。
   * 不转圈 —— 转圈说的是「多久」，这里该说的是「正在成形」。
   *
   * ⚠ 但**灰只能是铅笔灰**（6.22 : 1），不是刻线色。
   * 原来用的是 `--cs-rule-strong`，实测 **1.20 : 1** —— 纸上几乎看不见，
   * 「正在成形…」变成一片空白。设计意图对，执行过头了：
   * 「未完成」不等于「看不清」。 */
  .cs-idle-busy {
    color: var(--cs-pencil);
  }

  .cs-idle-sub {
    margin: 0;
    font-size: 11px;
    /* 11px 是正文、不是标注，所以走 `--cs-pencil`（5.10 : 1）而不是
     * `--cs-mark`（3.00 : 1，达不到 AA 正文）。 */
    color: var(--cs-pencil);
  }

  .cs-idle-btn {
    margin-top: 10px;
  }

  /* ── 异常 ─────────────────────────────────────────────── */
  .cs-alarm {
    flex: none;
    margin: 0 0 10px;
    padding: 0 14px;
    font-size: 11.5px;
    color: var(--cs-live);
    line-height: 1.5;
  }

  /* ── 窄面板 ───────────────────────────────────────────── */
  /* 不靠媒体查询 —— 窗口可以被拖到任意宽度。用 ResizeObserver
   * 量面板自己（`watchWidth` attachment）。 */
  .cs-panel[data-compact='y'] .cs-tier-name {
    display: none;
  }

  .cs-panel[data-compact='y'] .cs-line {
    font-size: 17px;
  }
</style>
