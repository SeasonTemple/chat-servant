<!--
  面板内的设置视图。**它不是另一个窗口**，是这个 380 × 560 面板里的一屏。

  ## 为什么从独立窗口搬进来

  原来 `··· → 设置` 调 `open_settings`，弹出另一个 640 × 520 的常规应用
  窗口。在一个 380px 的窄面板里点一下，空间关系突然跳到另一个宽一倍的
  窗口——而且面板内部**完全没有导航**，`···` 纯粹是个窗口启动器。

  `docs/面板设计.md` 第一条结构性决定是「修问题的按钮和问题在同一屏」。
  设置内容不少，但**该用分节滚动解决，不是用开第二个窗口**。

  ## 一屏，不分两级

  ⚠ 这里试过「分节目录 → 单节详情」两层。**放弃的理由**：在 560px 高的
  面板里，目录页让人先点一层才够得着任何一个设置，而滚一屏就能全部够到。
  多一层导航换来的只是「少滚 400px」，不值。

  所以是**一列分节 + 整列滚动**：节与节之间靠留白和字号分组，
  和主面板是同一条排版语言。

  ## 材质：**与主面板同族**（2026-10-16 反转）

  ⚠ 独立设置窗口用的是 `--cs-surface`（中性白）。搬进来之后**不能跟着换**：
  在同一个窗口里铺一块白底会留下一道接缝——那正是这个项目一直在删的东西。

  ⚠ 反转前还有第二层：这一屏把整张撕口纸撤掉、铺 `--cs-paper` 冷灰
  （设计规范 §1「纸面是配置界面唯一的敌人」）。**那一条也作废了**，
  理由见下面 `<style>` 开头那段。现在这一屏直接坐在同一张纸上，
  字条用 `--cs-paper-raised`——同样指向纸族。

  语义没变的部分保持不变：红仍然只给不可逆（删 key、图片外发那句警示），
  机器读出来的数仍然走等宽。

  ## 返回的路在哪

  返回主面板的那颗 `←` 在**面板头部**，不在这一屏里 —— 见 `+page.svelte`
  的 `.cs-back`。头部是 `flex: none`、永远在滚动区之外，所以它**不会**被
  内容滚走，位置也不随节数变化。这是「修问题的按钮和问题在同一屏」的
  同一套道理：返回的路不该藏在滚动内容下面，也不该藏在浮层里。
-->
<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import TierSwitch from '$lib/TierSwitch.svelte';
  import SegSlider from '$lib/SegSlider.svelte';
  import { IS_DEMO as DEMO } from '$lib/runtime';
  import { markShape, type MarkShape } from '$lib/mark-shape.svelte';
  import type { BudgetView, Tier } from '$lib/types';
  import { TICK_NESTED, TICK_VIEW } from '$lib/marks';
  import PencilDefs from '$lib/PencilDefs.svelte';

  /**
   * 一个供应商在**某一组**里的 key 状态。**Rust 是唯一出处**，前端不缓存。
   *
   * ⚠ `providers` 现在是**「预设 + 自定义」**两类，而不只是几家。
   *   「OpenAI 兼容」被做成**协议**而不是一家供应商——所以「自定义端点」
   *   要单独选协议、单独填地址。见 `$lib` 下的 `Provider` 契约。
   */
  interface ProviderView {
    id: string;
    label: string;
    configured: boolean;
    from_env: boolean;
    /** 值来自旧版那个不带组别的凭据目标名。删它会连带另一组。 */
    legacy: boolean;
    masked: string;
    hint: string;
  }

  /** 当前生效的这一路。**界面只显示它，不自己存第二份。** */
  interface RouteView {
    provider: string;
    provider_label: string;
    protocol: string;
    protocol_label: string;
    /** 归一化后的真实端点。自定义时**就是用户填的那个**。 */
    base_url: string;
    /** 端点与模型名有没有在本机实测过。false ⇒ 界面上必须说「未核实」。 */
    measured: boolean;
  }

  /** 一组模型配置。两组**互不影响**。 */
  interface GroupView {
    id: string;
    label: string;
    role: string;
    providers: ProviderView[];
    route: RouteView;
    models: string[];
    model: string;
    model_note: string;
    /**
     * 与**哪一组**共用同一格 —— 值是**对方那一组的显示名**（Rust 侧给的）。
     *
     * ⚠ 2026-10-19：原先这里是 `shared_with_other: boolean`，
     * 而提示语把「回复模型」写死。共享是**有方向的**——
     * 只配了决策组时是回复组借决策组的格子，写死就会**指错组**。
     * ⇒ 判据改成后端下发对方的名字，`null` 就是各配各的。
     */
    shares_with: string | null;
  }

  interface ProxyView {
    url: string;
    redacted_in_logs: boolean;
  }

  interface ChoiceView {
    id: string;
    label: string;
  }

  interface SettingsView {
    groups: GroupView[];
    proxy: ProxyView;
    protocols: ChoiceView[];
  }

  /**
   * 「用哪个模型」那一栏。
   *
   * ⚠ `models` 是一份**只填实测能通的**清单（实测过的三家）或**抄自文档**
   *   的候选（其余几家）。空清单不是没做完——本机推理与自定义端点的
   *   模型名只能用户自己给。
   */
  interface ModelView {
    group: string;
    provider: string;
    models: string[];
    model: string;
  }

  /** 「参考上下文」设置。上下限也由 Rust 下发。 */
  interface RecentView {
    n: number;
    min: number;
    max: number;
    default: number;
  }

  interface Props {
    /**
     * 发送档位。**与主面板共用同一个值**，所以是 `$bindable`。
     *
     * ⚠ 早先两个页面各有一份本地 state —— 于是设置页点完档位切回面板，
     * 主面板上那道档位记号还是旧的，而 Rust 那边已经是新的。
     * **同一个事实两个来源，界面就在说谎。** 这一份绑回父组件。
     */
    tier: Tier;
    /**
     * 「关闭时」的行为。同样绑回父组件 —— 头部 WinCtl 的 `−` 就是靠它显形。
     * 在这一屏改成「收进托盘」，回到面板就该立刻多一颗 `−`。
     */
    closeBehavior: 'exit' | 'tray';
  }

  let { tier = $bindable(), closeBehavior = $bindable() }: Props = $props();

  /**
   * 关闭窗口的两项。**只有两个字**——原来第二项写着「（默认）」，
   * 同一组里两项长度差一倍，滑块行程跟着不对称（2026-10-10 删）。
   */
  const CLOSE_OPTIONS = [
    { id: 'exit', label: '退出' },
    { id: 'tray', label: '收进托盘' },
  ];

  let tierNote = $state<string | null>(null);
  let recentN = $state(12);
  /** 滑块上下限。**不写死在 HTML 里**——写死迟早和 Rust 侧的策略对不上。 */
  let recentMin = $state(4);
  let recentMax = $state(40);
  let recentNote = $state<string | null>(null);
  /** 单张图片的视觉 token 预算。**Rust 侧算好该不该出现**，前端不自己判断。 */
  let budget = $state<BudgetView | null>(null);
  let budgetNote = $state<string | null>(null);
  /** 图片外发总闸。**读不出来时按「关」处理**——失败方向必须偏保守。 */
  let imagesOut = $state(false);
  let imagesNote = $state<string | null>(null);
  let view = $state<SettingsView | null>(null);
  let closeNote = $state('');
  /**
   * 「模型名没设成」的错误，**按组分开**。
   *
   * ⚠ 2026-10-15 修一个真缺陷：两组结构重写时，展示错误的那段标记
   *   （`{#if modelNote}<p class="cs-fail">`）被删掉了，而两处赋值
   *   （`pickModel` / `commitModelName` 的 catch）留着 ⇒
   *   `set_model` 被 Rust 拒收时，错误被 catch 住存进状态，
   *   **界面上一个字都不显示**。
   *   症状是「点了没反应」，而真实原因（模型名不在实测清单里）被吞掉了。
   *
   *   为什么按组而不是一个 `modelNote`：现在有两组配置，
   *   报在「决策模型」下的错不该出现在「回复模型」那一节旁边。
   */
  let modelErr = $state<Record<string, string>>({});

  /**
   * 换模型。
   *
   * ⚠ **用回包覆盖**：Rust 会拒收不在实测清单里的名字，而界面必须显示
   * 拒收之后的真实状态。只显示用户点的那一格，就成了一个说谎的控件。
   */
  async function pickModel(g: GroupView, m: string) {
    delete modelErr[g.id];
    try {
      await invoke<ModelView>('set_model', { group: g.id, model: m });
      await refresh();
    } catch (e) {
      // 写失败就**读回真实值**，别让控件停在一个程序并不认同的状态上。
      await refresh();
      modelErr = { ...modelErr, [g.id]: String(e) };
    }
  }

  /**
   * 自由填模型名。**只在 Rust 说「这一路没有可选清单」时出现**——
   * 有清单却让你手打一个不在清单里的名字，那是教用户去撞 404。
   */
  async function commitModelName(g: GroupView) {
    delete modelErr[g.id];
    try {
      await invoke<ModelView>('set_model', { group: g.id, model: drafts[g.id] ?? '' });
      drafts[g.id] = '';
      await refresh();
    } catch (e) {
      await refresh();
      modelErr = { ...modelErr, [g.id]: String(e) };
    }
  }

  /**
   * 「展开输入框」的哨兵值。
   *
   * 早先这里塞的是一个**裸 NUL 字符**（`'\0show'` 的字面 NUL），
   * 用不可能出现在 key 里这一点当哨兵。它能跑，但代价很重：
   * git 因此把整个 `.svelte` 判成**二进制**，从此这个文件的 diff
   * 全失效，review 只能靠肉眼。用具名常量，哨兵的意图也看得见了。
   */
  const REVEAL = '__reveal__';
  /** 输入框内容。**只存在于组件内存里**，提交后立刻清空。
   *
   * ⚠ 键是 `组`，不是供应商——**一组只显示当前那一家的 key 卡片**。
   *   九家 × 两组 = 十八张卡，而它们绝大多数永远不会被用到；
   *   测连通走的也是「当前这一路」，所以卡片与它是同一个对象。
   */
  let drafts = $state<Record<string, string>>({});
  /** 自定义端点的地址草稿。**提交前不生效**——每一次换协议/换厂商都要重发。 */
  let endpoints = $state<Record<string, string>>({});
  let busy = $state<Record<string, boolean>>({});
  let notes = $state<Record<string, string>>({});
  let fail = $state<string | null>(null);
  let proxyDraft = $state('');
  let proxyNote = $state<string | null>(null);

  async function refresh() {
    try {
      view = await invoke<SettingsView>('get_settings');
      // ⚠ 成功时**必须清掉 `fail`**，否则「重试」成功了红色那一条还赖着不走。
      //   `fail` 之前是只写不清的（只有 DEMO 分支与 catch 分支在设它），
      //   那在「没有重试入口」时看不出来——因为一屏之内根本不会成功第二次。
      //   一旦给了重试按钮，不复位就等于**报错是假的**。
      fail = null;
      // 自定义地址的草稿**只在没有草稿时**从回包填。
      // 覆盖它会让用户正在输入的东西在每次失败提示后被冲掉。
      const next = view.groups.filter((g) => g.route.provider === 'custom');
      for (const g of next) {
        if (endpoints[g.id] === undefined) endpoints[g.id] = g.route.base_url;
      }
      proxyDraft = view.proxy.url;
    } catch (e) {
      // 这里的 e 是 Rust 侧已经**脱敏过**的文本，可以直接显示。
      fail = String(e);
    }
  }

  /** 当前这一组选中的那家供应商（**Rust 给的那一份**，前端不自己挑）。 */
  function currentProvider(g: GroupView): ProviderView | undefined {
    return g.providers.find((p) => p.id === g.route.provider);
  }

  /**
   * 换一家 / 换协议 / 换自定义端点。**一条命令三件事**——
   * 它们在出网时是同一个值（`Route`），拆成三次调用就会允许
   * 「显示的是 A、实际发的是 B」这种组合。
   */
  async function applyRoute(g: GroupView, provider: string, protocol?: string) {
    busy[g.id] = true;
    notes[g.id] = '';
    try {
      await invoke<GroupView>('set_route', {
        group: g.id,
        provider,
        protocol: protocol ?? g.route.protocol,
        base_url: provider === 'custom' ? (endpoints[g.id] ?? '') : null,
      });
      await refresh();
    } catch (e) {
      notes[g.id] = String(e);
    } finally {
      busy[g.id] = false;
    }
  }

  async function save(g: GroupView) {
    const p = currentProvider(g);
    if (!p) return;
    busy[g.id] = true;
    notes[g.id] = '';
    try {
      const wrote = await invoke<boolean>('save_setting', {
        group: g.id,
        provider: p.id,
        key: drafts[g.id] || null,
      });
      notes[g.id] = wrote
        ? '已存进 Windows 凭据管理器。**下一次生成立即生效，不用重启。**'
        : '留空 = 保留原值，没有改动。';
      // 提交后清掉输入框：明文不该在内存里多待一刻
      drafts[g.id] = '';
      await refresh();
    } catch (e) {
      notes[g.id] = String(e);
    } finally {
      busy[g.id] = false;
    }
  }

  async function forget(g: GroupView) {
    const p = currentProvider(g);
    if (!p) return;
    busy[g.id] = true;
    try {
      await invoke('forget_key', { group: g.id, provider: p.id });
      notes[g.id] = '已从凭据管理器删除。';
      drafts[g.id] = '';
      await refresh();
    } catch (e) {
      notes[g.id] = String(e);
    } finally {
      busy[g.id] = false;
    }
  }

  /**
   * 测连通性。
   *
   * 失败**必须显示**。早先的 `probe_key` 恒返回 false、前端又没有按钮，
   * 等于这个功能从来没存在过；现在它真的会发请求了，失败信息就是用户
   * 唯一能判断「是 key 不对还是网络不通」的依据。吞掉它等于白做。
   *
   * ⚠ 它测的是**这一组当前这一路**——包括用户自己填的端点。
   */
  async function probe(g: GroupView) {
    busy[g.id] = true;
    notes[g.id] = '';
    try {
      await invoke<boolean>('probe_key', { group: g.id });
      notes[g.id] = '连上了。走的是只读的 models 端点，没有产生任何 token 费用。';
    } catch (e) {
      notes[g.id] = `连不上：${String(e)}`;
    } finally {
      busy[g.id] = false;
    }
  }

  /**
   * 代理。**单独一栏，两组都生效。**
   *
   * ⚠ 不放进任何一组：代理是网络层的属性。放进去就意味着填两次，
   *   而两份迟早不一样——「一组走代理、一组不走」是个没人主动想要的状态。
   */
  async function saveProxy() {
    proxyNote = null;
    try {
      const p = await invoke<ProxyView>('set_proxy', { url: proxyDraft });
      proxyDraft = p.url;
      proxyNote = p.url ? '已设。**两组模型都走它**，下一次请求生效。' : '已清空，现在直连。';
    } catch (e) {
      // 写失败就**读回真实值** —— 别让输入框停在一个程序并不认同的状态上。
      proxyDraft = await invoke<ProxyView>('get_proxy')
        .then((p) => p.url)
        .catch(() => proxyDraft);
      proxyNote = String(e);
    }
  }

  /**
   * 改档。与主面板**共用 Rust 那一个格子**。
   *
   * 界面上显示的是 Rust 回传的真实生效值，不是用户点过的那个 ——
   * 写回 `$bindable` 的 `tier` 之后，主面板纸底那道档位记号立刻跟着变。
   */
  async function changeTier(t: Tier) {
    try {
      tier = await invoke<Tier>('set_tier', { tier: t });
      tierNote = null;
    } catch (e) {
      tierNote = String(e);
      try {
        tier = await invoke<Tier>('get_tier');
      } catch {
        tier = 'Manual';
      }
    }
  }

  /**
   * 「标记形状」。与主面板**共用 `$lib/mark-shape.svelte` 那一份**。
   *
   * ⚠ 不能各存一份：设置页切了、主面板没跟，就是「同一个事实两个来源」，
   * 而用户在界面上看不出哪里不同步。
   */
  let shape = $state<MarkShape>(markShape.current);
  let shapeNote = $state('');

  const SHAPE_OPTIONS = [
    { id: 'tick' as const, label: '勾' },
    { id: 'ring' as const, label: '圈' },
  ];

  async function setShape(next: MarkShape) {
    shapeNote = '';
    const r = await markShape.set(next);
    if (!r.ok) {
      shapeNote = `没存上：${r.note}`;
    }
    // 无论成败都读回镜像 —— 成功时它等于 next，失败时它等于程序认定的值。
    shape = markShape.current;
  }

  /**
   * 「关闭时」的行为。与主面板**共用 Rust 那一个 setting**。
   *
   * ⚠ Rust 侧会验证：托盘不存在时**拒收**「收进托盘」—— 那样关掉面板
   * 它就藏起来了。所以这里把错误原样显示，不假装成功。
   */
  async function setClose(next: 'exit' | 'tray') {
    closeNote = '';
    try {
      await invoke('set_close_behavior', { behavior: next });
      closeBehavior = next;
      closeNote =
        next === 'tray'
          ? '按 − 收进托盘。托盘图标在「会发出去」的档位下会带红点。'
          : '按 × 直接退出。程序不会在后台继续跑。';
    } catch (e) {
      // 写失败就**读回真实值** —— 别让开关停在一个程序并不认同的状态上。
      closeBehavior = await invoke<'exit' | 'tray'>('get_close_behavior').catch(() => 'exit');
      closeNote = String(e);
    }
  }

  /**
   * 切换图片外发总闸。
   *
   * ⚠ 这是整个设置里唯一一个**危险开关**：打开之后，对方发来的图片区域
   * 会被裁出、缩放、编码，然后发给第三方模型。媒体检测的准确率尚未在
   * 真实帧上完整验证，所以没有「自动」档，只有开和关。
   */
  async function toggleImagesOut(next: boolean) {
    try {
      imagesOut = await invoke<boolean>('set_images_out', { on: next });
      // 开关变了，预算滑块的可显示性也跟着变——**重新读一次**，
      // 不在前端自己推导。
      budget = await invoke<BudgetView>('get_token_budget');
      imagesNote = imagesOut
        ? '已打开：对方发来的图片会被裁出并发给模型。'
        : '已关闭：一张图都不会外发。';
    } catch (e) {
      imagesNote = String(e);
    }
  }

  /**
   * 提交图片预算。
   *
   * ⚠ 用回包覆盖：值被夹过的话界面必须显示夹完的数。
   * 只显示拖到的那格，界面就变成了一个说谎的控件——ticket 11 刚为
   * 同一类问题付过代价。
   */
  async function commitBudget() {
    if (!budget) return;
    try {
      budget = await invoke<BudgetView>('set_token_budget', { budget: budget.budget });
      budgetNote = `已设为每张 ${budget.budget} token，下一张图就会用上`;
    } catch (e) {
      budgetNote = String(e);
    }
  }

  /** 把 Rust 回的**真实生效值**铺到界面上。 */
  function apply(v: RecentView) {
    recentN = v.n;
    recentMin = v.min;
    recentMax = v.max;
  }

  // 滑块改动 → 立刻生效。`change` 而不是 `input`：拖动过程中不该
  // 一路发请求，松手时才提交一次。
  async function commitRecent() {
    try {
      const v = await invoke<RecentView>('set_recent_n', { n: recentN });
      // ⚠ 用回包覆盖：值被夹过的话，界面必须显示夹完的数。
      apply(v);
      recentNote = `已设为 ${v.n} 条，下一次生成就会用上`;
    } catch (e) {
      recentNote = String(e);
    }
  }

  onMount(() => {
    // ⚠ 演示模式**不走 IPC**。这一屏改的是真设置（key、档位、预算），
    // 浏览器里没有 `window.__TAURI_INTERNALS__`，`invoke` 必然抛。
    // 但**布局照常渲染** —— 这一屏的排版恰恰是最需要被评审的东西，
    // 不该因为拿不到数据就变成一个空白框。
    if (DEMO) {
      fail = '演示模式：这里显示的是排版，真实设置在 Tauri 窗口里读。';
      return;
    }
    void (async () => {
      try {
        apply(await invoke<RecentView>('get_recent_n'));
      } catch {
        // 读不到就用默认档，**不报错打断这一屏**。
        // 滑块仍可拖——写了就可能有值，读不到只是看不到当前值。
      }
      try {
        budget = await invoke<BudgetView>('get_token_budget');
      } catch {
        // 同上。预算读不到就整块不显示，别摆一个半死的滑块。
      }
      try {
        imagesOut = await invoke<boolean>('get_images_out');
      } catch {
        // 读不到就当关着。**失败方向要偏保守**——总闸读不出来时
        // 宁可当成关着，也不能当成开着。
      }
      try {
        // 档位也要**读回来**：界面显示的必须是程序认定的那个，
        // 不是用户上次点过的那个。写回 `$bindable` 让主面板跟着走。
        tier = await invoke<Tier>('get_tier');
      } catch {
        tier = 'Manual';
      }
      try {
        // 「关闭时」也要读回来。读不到就按 `exit` —— 那是**失败方向偏保守**
        // 的那个：宁可用户以为它会退出（然后发现托盘还在），也不要
        // 以为它会退出而它其实藏在后台。
        closeBehavior = await invoke<'exit' | 'tray'>('get_close_behavior').catch(() => 'exit');
      } catch {
        closeBehavior = 'exit';
      }
      // 标记形状也要读回来：界面显示的必须是程序认定的那个。
      shape = await markShape.load();
      // ⚠ **模型设置**不再单独读一条命令：它在 `get_settings` 那一屏里
      // （两组 + 端点 + 代理）。摆两个来源就等于同一个事实两处显示，
      // 而它们迟早不一致。
      await refresh();
    })();
  });

  /**
   * demo 夹具：模型配置两组（模板里 `{:else if DEMO}` 那段渲染它）。
   *
   * 形状与真实分支同构，刻意覆盖**四种按钮形态**：已配置组有 `primary`
   * 厂牌 chip + 「换新 key / 测连通 / 删除」，未配置组直接给输入框 + `primary` 保存。
   * 不含真实 key、不发任何 IPC。
   *
   * ⚠⚠⚠ **这两组的名字必须与 `ModelGroup::label()` 逐字相同**
   *   （`crates/reason/src/transport.rs`：`"决策模型"` / `"回复模型"`）。
   *   2026-10-18 我在这里写过「主模型 / 备用模型」，
   *   而用户当场问「**为何是主模型、备用模型？**」——问得对：
   *   **那是我编的，而且在界面上说了一个假架构。**
   *
   *   真实结构是**一条消息走两段流水线**：
   *     ① `Reasoner::judge` 判断对方意图（输出几十个字，配便宜模型就够）
   *     ② `Reasoner::draft` / `rank` 起草与排序（图片跟着这一段走）
   *   两段**每条消息都跑**（A×2 + B×1），不存在「主失败才启用备」。
   *   写成「主 / 备」会让人以为配了备模型就不用管主模型——
   *   **这个误解会直接导致配错**，而 demo 夹具是排版评审的基准，
   *   名字错了等于把错的架构钉在了每一张出图上。
   */
  const DEMO_GROUPS = [
    {
      name: '决策模型',
      configured: true,
      providers: ['anthropic', 'openai'],
      hint: '存进 Windows 凭据管理器，面板里读不到明文。',
    },
    {
      name: '回复模型',
      configured: false,
      providers: ['deepseek', 'ollama'],
      hint: '自定义端点：协议与厂商解耦，填 base_url 即可。',
    },
  ];
</script>

<!--
  ⚠ 滚动容器是这一屏的 root。面板本身 `overflow: hidden`，所以**必须**
  自己声明 `flex: 1 1 auto; min-height: 0; overflow-y: auto` ——
  少了 `min-height: 0`，flex 子项不肯小于内容高度，整屏会被撑出面板外。
-->
<PencilDefs />

<div class="cs-set">
  <!--
    一次就说清最要紧的那件事：**改完不用重启**。
    早先是挂在「发送档位」下面那一行，只有走过那一节的人才看得到，
    而「图片」「上下文」两节的改动同样立刻生效。
  -->
  <!--
    ══ 屏头 ══

    ⚠⚠ **这一屏原先一个标题都没有**：直接从「发送档位」开始，
    而「全部改完立刻生效」那句灰字孤零零挂在第一个小节下面。

    那句 2026-10-16 被从屏顶挪到这里（当时的理由是「离它描述的东西近」），
    但它描述的是**整屏**——「图片」「上下文」「外观」几节改了也一样立刻生效，
    挂在「发送档位」底下等于**归属说错了**：用户会以为只有档位不用重启。

    ⇒ 给这一屏一个屏头（`设置` + 那句话作为副标题），
      它是**关于这一屏**的话，就该有**关于这一屏**的位置。
      顺带解决用户 2026-10-16 与 2026-10-18 两次说的
      「为何说明要单独一行？」——那次的修法是挪位置，
      而**根因是它不属于任何控件**；给屏头它就不再是一行孤字，
      而是一块有标题的说明区。
  -->
  <header class="cs-set-head">
    <h1 class="cs-set-title">设置</h1>
    <p class="cs-set-sub">改完立刻生效，不用重启面板。</p>
  </header>

  <!--
    ⚠⚠ 「发送档位」这个小节标题原先写在**这里**（demo 夹具之上），
    而它真正的控件 `<TierSwitch>` 写在夹具**下面**（2026-10-18 截图抓到）。
    ⇒ 标题与控件被整块模型配置隔开约 400px，屏幕上读成
      「发送档位」这一节是空的，后面凭空多出一组控件。
      这是「标题/说明意义不明」最重的一档：**归属被拆开了**。
    ⇒ 标题已移到 `<TierSwitch>` 紧上方（标题与控件必须相邻）。
      demo 夹具留在屏头之后——它是要给人看出图的，不属于任何小节。
  -->

  <!--
    demo 夹具（2026-10-16 加），**只在演示模式渲染**，真实运行时不存在。

    **为什么必须有**：这一屏的按钮族（`.cs-btn-mini` 及其 `primary` /
    `danger` / `disabled` 四种形态）原先在演示模式下**整节不渲染**——
    只留一句「演示模式没有后端，这里不列模型配置。」
    于是任何对按钮的改动都**看不见**，只能盲改。
    用户 2026-10-16 的原话：「而不是拿之前的设计敷衍」——
    而「看不见」正是敷衍的温床：我上一轮就改了这族却出不了图。

    **为什么放在第一屏顶部而不是它在真实布局里的位置**：
    设置屏比那张纸高（`.cs-set` 要滚动），模型配置节在很下面，
    出图截不到 ⇒ 盲区会以另一种形式回来。
    demo 只用于**排版评审**，顺序不同不影响评审；
    真实运行时的顺序不受影响（`{#if DEMO}` 包住整段）。

    它**不做任何 IPC、不含真实 key**，按钮点了只是没反应。
    形状与真实分支同构：一组「已配置」（徽标 + 三颗按钮）、
    一组「未配置」（直接给输入框）。

    ⚠ 别把它当成「简化版」。以后**新增按钮形态就要往这里加一份**，
      否则它会再次变成一个只能靠读代码判断的盲区。
  -->
  {#if DEMO}
    <!--
      ⛔ **2026-10-18 删掉夹具里那份「送进模型的最近原文条数」滑块。**

      它在这里与「上下文」小节里**同一个设置出现两次**（出图确认：
      第 1 屏顶部一颗无标题的滑块、第 3 屏「上下文」下面一颗），
      而真实分支只有后者 ⇒ **夹具与真实不同构**，正是本段自己写的
      「形状与真实分支同构」那条被自己破掉。

      ⚠⚠ 它当初为什么被搬到第一屏，理由是**量具 artifact**：
        「出图只到那个标签就被纸底切掉」。2026-10-18 实测，
        那是 `measure-settings.mjs` 把滚动容器展开（`height:auto` +
        `overflow:visible`）再 `fullPage` 截图的副作用——
        纸面位图在大约 y=1400 处**断掉**，而同一棵树用逐屏滚动截图
        （`target/probe/roll-settings.mjs`）拍出来好好的，在第 3 屏中部。
      ⇒ **「看不见」是量法的锅，不是布局的锅。** 真要评第一屏的排版，
        用滚动截图，别再往夹具里塞一份副本。
    -->
    {#each DEMO_GROUPS as g (g.name)}
      <section class="cs-group">
        <div class="cs-prov-head">
          <span class="cs-group-name">{g.name}</span>
          <span class="cs-badge {g.configured ? 'ok' : 'miss'}">
            {g.configured ? '已配置' : '未配置'}
          </span>
        </div>
        <p class="cs-hint">{g.hint}</p>
        <div class="cs-chips">
          {#each g.providers as p, i (p)}
            <!-- ⚠ 2026-10-18 修一处**夹具与真实不同构**：
                 真实分支是 `class:primary={p.id === g.route.provider}`
                 ⇒ **只有当前选中的那一颗**是 primary（红）。
                 而这里**每颗都硬写 primary** ⇒ 出图评审看到「四颗全红」，
                 那在真机上根本不存在。
                 ⇒ 只让第一颗 primary。夹具自己的承诺就是「形状与真实分支
                   同构」（见上面那段注释），这一条得守住。 -->
            <button class="cs-btn cs-btn-mini" class:primary={i === 0} aria-pressed={i === 0}>
              {p}{#if i === 0}<span class="cs-chip-ring" aria-hidden="true">
                  <svg viewBox="0 0 {TICK_VIEW.w} {TICK_VIEW.h}" filter="url(#pencil)">
                    {#each TICK_NESTED as layer (layer.d)}
                      <path d={layer.d} fill-opacity={layer.o} />
                    {/each}
                  </svg>
                </span>{/if}
            </button>
          {/each}
        </div>
        {#if g.configured}
          <div class="cs-actions">
            <button class="cs-btn cs-btn-mini">换新 key</button>
            <button class="cs-btn cs-btn-mini">测连通</button>
            <button class="cs-btn cs-btn-mini danger">删除</button>
          </div>
        {:else}
          <div class="cs-form">
            <!-- ⚠ placeholder 原先是「粘贴 key」——「key」是什么、
               贴的是什么格式、贴进去会存到哪，这一屏没一处说。
               现在把「API」写进去：与下面那行「存入 Windows 凭据管理器」
               合起来才读得出「这是一把模型 API key，会存进系统凭据管理器」。
               `aria-label` 与 placeholder 同义：**读屏用户拿不到 placeholder**
               （它不是可访问名称），所以这一条必须写。 -->
            <input
              class="cs-input"
              type="password"
              autocomplete="off"
              spellcheck="false"
              aria-label="模型 API key"
              placeholder="粘贴 API key"
            />
            <button class="cs-btn cs-btn-mini primary">保存</button>
          </div>
        {/if}
      </section>
    {/each}
  {/if}
  <!-- ══ 发送档位 ══ -->
  <!-- ⚠ 标题必须与 `<TierSwitch>` 相邻：中间夹任何东西，
       它读起来就变成「下一组控件的标题」而不是「这一节的标题」。 -->
  <h2 class="cs-h2">发送档位</h2>
  <TierSwitch bind:tier onchange={changeTier} />
  {#if tierNote}
    <p class="cs-fail" role="alert">{tierNote}</p>
  {/if}

  <!-- ══ 外观 ══ -->
  <h2 class="cs-h2">外观</h2>
  <!--
    ⚠ 这句文案 2026-10-14 从「顶边」改成「纸底」。

    `6f10f9f` 把档位从顶边挪到纸底（盖章式，见设计规范 §10 第 1 条），
    但这一直写着「顶边标出当前档位的那一笔」。**漂移的是界面上的字，
    不是注释**——用户照着这句去找标记，会去纸的上沿找，而那里现在只有撕口。

    教训同 `docs/设计规范.md` 那条「挪位置 ≠ 改形态」：挪位置时，
    **所有描述旧位置的文案都要一起改**，否则定案改了、界面还在教旧位置。
  -->
  <!--
    ⚠ 2026-10-15 删掉了这里那行「纸底标出当前档位的那一笔」。
    理由：它是**说明**而不是**名称**，而且说明的是**就在它正下方 20px 处的那一笔**
    ——标记是画出来的，看得见的东西不需要再写一遍。
    这一节真正的名称是上面那个 `外观`。
    留着的代价：它占了 `cs-label` 的位置（与「按窗口控制键时」这类名称同级），
    于是「名称」和「说明」两种东西在排版上分不出来。
  -->
  <!--
    勾 / 圈是**审美偏好**，所以交给用户选，不在这里替他们定。
    两支笔在 `$lib/marks` 里本来就都生成好了，切换只是换一组 path，
    笔性一致（同为「中心线 + 宽度剖面」算出的轮廓）。

    ⚠ 这是**外观偏好**，不是发送档位：它落盘、跨会话保留。
    档位走 Rust 的 `TierCell`，会话结束必须回落 `Manual`（硬约束第 5 条）。
    两种生命周期不能混 —— 一个要在重启后记得，一个**必须**忘掉。

    ⚠ 写失败时 `markShape.set` 会读回真实值，所以这里的绑定不会停在
    一个程序其实没存的状态上（与「关闭时」同一策略）。
  -->
  <SegSlider
    bind:value={shape}
    label="标记形状"
    options={SHAPE_OPTIONS}
    onchange={(v) => void setShape(v as MarkShape)}
  />
  {#if shapeNote}
    <p class="cs-note" role="status">{shapeNote}</p>
  {/if}

  <!-- ══ 窗口 ══ -->
  <h2 class="cs-h2">窗口</h2>
  <!--
    ⚠⚠ **这里原先是两行标签说同一件事**（2026-10-18 用户第三次提「说明文字
    意义不明的单独占一行」，指的就是这类）：
      <p class="cs-label">按窗口控制键时</p>     ← 单独一行
      <SegSlider label="关闭窗口的行为">          ← 控件自己又带一个标签
    两行的字面意思重叠，而**「名称」和「说明」在排版上分不出来**
    （它们是同一个 12.5px / 600）——这才是问题，不是哪一句不好看。

    ⇒ 只留一个标签，且用**用户会说的那句**（「按窗口控制键时」），
      因为它回答的是「我在按哪个键」，而「关闭窗口的行为」是界面自述。
  -->
  <!--
    两个选项并列、不分主次：它们是同一件事的两半，不是「轻/重」。
    选中用**墨底滑块**，与「发送档位」是同一个 `SegSlider` 组件——
    两组都是「改一个设置、都能改回来」，做法不该有两套
    （用户 2026-10-10：「窗口关闭和档位切换不都可以用这个方案？」）。
    红在这里永远不出现：红只给不可逆。

    ⚠ 选项文字里的**「（默认）」删掉了**（2026-10-10）。它是在用文字
    标记「哪一项是初始值」，可这是一组控件的默认值、不是某一枚按钮的属性，
    而且它把「退出」两个字撑成了「退出（默认）」五个字——
    同一组里两项长度差一倍，滑块的行程也就跟着不对称。
  -->
  <SegSlider
    bind:value={closeBehavior}
    label="按窗口控制键时"
    options={CLOSE_OPTIONS}
    onchange={(v) => setClose(v as 'exit' | 'tray')}
  />
  <!--
    ⚠ 这里原来有一段两行的说明：「主面板没有标题栏也不在任务栏里，这三个键是
    它唯一的出口……」（2026-10-10 实测反馈：说明文字喧宾夺主，又臭又长）

    删掉的理由不是它写得不好，是**它在复述控件**。上面两个选项的字面就是
    「退出」「收进托盘」，控件本身已经把话说完了；再补一段解释，是让说明
    替控件干活。

    判断标准：**一句话能删就删，除非它说了控件里看不到的东西。**
    这一屏保留下来的说明只有三类：
      1. 后果不在控件上——「调大不会等比变贵」「超出就缩图，不放大」；
      2. 隐私与代价——「整块画面会发给第三方模型」、凭据管理器、图片底账；
      3. **出错时**——无解的预算、读设置失败。
    其余一律删。
  -->
  {#if closeNote}
    <p class="cs-note" role="status">{closeNote}</p>
  {/if}

  <!-- ══ 上下文 ══ -->
  <h2 class="cs-h2">上下文</h2>
  <!--
    ⚠ 2026-10-15 改成名称 + `title`。这一句回答的是「这一节在设什么」
    （那是**名称**），而「更早的压成摘要」是**理由**——理由不该和名称
    平级站在屏幕上。
  -->
  <p class="cs-label" title="送进模型的最近多少条原文。更早的压成摘要，调大不会等比变贵。">
    送进模型的最近原文条数
  </p>
  <div class="cs-slider">
    <input
      type="range"
      min={recentMin}
      max={recentMax}
      step="2"
      value={recentN}
      aria-describedby="cs-recent-scale"
      aria-valuetext={`${recentN} 条，可调 ${recentMin} 到 ${recentMax} 条`}
      oninput={(e) => (recentN = Number(e.currentTarget.value))}
      onchange={commitRecent}
    />
    <output>{recentN} 条</output>
  </div>
  <!--
    ⚠ 量程两端。**没有它们，「12 条」这个数是没有参照的**——
    拇指位置只说得出「大概在中间」，说不出能拖到 2 还是 40。
    用户想知道的是「调到 30 会怎样」与「已经到顶了吗」，这两问都要两端。

    ⚠ 10.4px 是**徽标档**（`--cs-mark` 只有 3.51 : 1，不够正文），
      所以这里用 `--cs-pencil`（5.73 : 1），字号同样是 10.4。
      **字号小不等于对比度就可以不要**——它读的是数字，不是徽标。
  -->
  <p class="cs-scale" id="cs-recent-scale">
    <span>{recentMin} 条</span>
    <span>{recentMax} 条</span>
  </p>
  {#if recentNote}
    <p class="cs-note" role="status">{recentNote}</p>
  {/if}

  <!-- ══ 图片 ══ -->
  <h2 class="cs-h2">图片</h2>
  <p class="cs-label">把对方发的图片发给模型</p>
  <!--
    ⚠ **按钮写动作，不写状态。**
    原来关着的时候显示「关闭」—— 可按下去是**打开**。
    「关闭」读起来像「当前状态是关闭」，而不是「按这个会关闭」。
    控件说的话和它要做的事对不上，用户得先猜一次。

    ⚠ 原来的 class 里还有一个 `cs-onoff`（2026-10-16 删）。
    它**全仓没有任何样式规则**，而状态已由上面那句文案表达 ——
    于是一个「有名字没实现」的类：留着会让下一个人以为
    「哦这里本来该有个 on/off 的样式，只是没写」。
    ⇒ 真要给开/关加视觉区分，就在 `.cs-btn-ink` 上加，
      别把类名留着当占位符。
    ⚠ 同款坑：`svelte-check` **不会**因为「类名在标记里、样式表里没有」
      而报错（它只报反过来的 `css_unused_selector`）——
      所以这一类缺口靠门禁抓不到，只能靠数。
  -->
  <button class="cs-btn cs-btn-ink" onclick={() => toggleImagesOut(!imagesOut)}>
    {imagesOut ? '关掉' : '打开'}
  </button>
  <!--
    ⚠ 这段**必须短**，而且**只能红一句**。

    早先它是四行红字，一个「关闭」开关底下挂着四行警告 —— 红占了整屏
    三分之一，**红色又退化成装饰了**。

    危险开关要说清三件事，但**不用红字说**：会做什么、可能错在哪、
    错了什么后果 —— 三件都在，只是压在正常的铅笔色正文里。
    红色只留给那句「在你没预期的时候它会这么做」。
  -->
  <p class="cs-note">对方发来的<strong>整块画面</strong>会发给第三方模型。</p>
  <!-- ⚠ 2026-10-18：原文「画面切分只在 1 张真实样本上验证过 —— 认错框，
       屏幕就出去了」。前半句是在交代**我们**的验证状态（用户拿它无法行动），
       后半句「认错框」是内部术语——用户读到「框」不会想到那是截图里
       被框出来的一块。
       ⇒ 换成用户能懂的**后果**。「该红」不变：那一条守的是上面
         L859-861 的定案（红只留给「在你没预期的时候它会这么做」）。 -->
  <p class="cs-warn-on">截图可能认错 —— 认错一张就等于把整屏发了出去</p>
  {#if imagesNote}
    <p class="cs-note" role="status">{imagesNote}</p>
  {/if}

  <!--
    图片 token 预算。**`budget.visible` 由 Rust 读总闸算出来**，前端不自己判断。
    总闸关着时没有一张图会外发，摆滑块就是「调了没反应的控制项」。
  -->
  {#if budget?.visible}
    <h2 class="cs-h2">图片成本</h2>
    <!--
      ⚠ 2026-10-15 压成名称 + `title`。原来是一句两行的独立说明：
      「单张图片上限。超出就缩图到预算内，小图不放大。」——
      后半句是**行为**（滑块拖到哪会怎样），控件旁已经有实时的
      `{budget.budget} token` 在报数；真正需要解释的是「为什么小图不放大」，
      那是理由，悬停能看到就够。
    -->
    <p class="cs-label" title="超出预算就缩图到预算内；小图不放大——放大只会更贵，不会更清楚。">
      单张图片上限
    </p>
    <div class="cs-slider">
      <input
        type="range"
        min={budget.min}
        max={budget.max}
        step="32"
        value={budget.budget}
        oninput={(e) => (budget = budget && { ...budget, budget: Number(e.currentTarget.value) })}
        onchange={commitBudget}
      />
      <output>{budget.budget} token</output>
    </div>
    {#if budget.provider_base > budget.budget}
      <!-- 无解要**明说**。悄悄发一张然后超支，比报错糟得多。 -->
      <p class="cs-note cs-warn" role="alert">
        {budget.provider} 发任何一张图都要 {budget.provider_base} token 的底账，预算 {budget.budget}
        无解——再调小也不会更省。
      </p>
    {:else if budget.provider_base > 0}
      <p class="cs-note">
        {budget.provider} 的固定底账是 {budget.provider_base} token，与图片大小无关；剩下的是按图缩放后另外算的。
      </p>
    {/if}
    {#if budgetNote}
      <p class="cs-note" role="status">{budgetNote}</p>
    {/if}
  {/if}

  <!-- ══ 模型 ══ -->
  <!--
    ⚠ **这一屏永远是两组，不是一组**（票 43）。用户要的是
    「1 个决策模型的配置 + 1 个 LLM 的」：判断对方意图那一段可以配一个
    便宜的模型，起草那两段配一个好的。合成一个列表时，这件事
    **在数据结构上就表达不了**，而这里必须两节都摆出来才不算偷懒。

    ⚠ **没接上真实传输时这一屏仍然要显示。** 早先的版本在没 key 时
    整块不渲染——而没 key 正是用户来这一屏的原因。那条「没 key 就不列模型」
    的规则只适用于**模型清单**，不适用于**配置入口**。
  -->
  <h2 class="cs-h2">模型</h2>

  {#if fail}
    <!--
      ⚠ 2026-10-18：错误文案与它的按钮**必须包在同一个 flex 行里**。
        原来两者都是 `.cs-set`（列向 flex，`align-items` 拉伸）的直接子元素
        ⇒ 按钮被拉成**横贯整屏的整宽块**，而同一屏的「打开」是内容宽度自适应
        （实测两个动作按钮：97×46 与 288×46，同类控件两种宽度）。
        「重试」按动作读，**整宽会让它读成一块分区**，不是一颗按钮。
    -->
    <div class="cs-retry">
      <p class="cs-fail" role="alert">{fail}</p>
      <!--
        ⚠ **失败后的重试入口。**

        原来 `refresh()` 只在打开面板时调一次，于是「凭据管理器读不到」「自定义
        路由坏了」这类**一次性**故障只能靠关掉面板再打开——
        而那正好是**你最不想丢当前输入**的时候。

        ⚠ `view` 为 `null` 时下面整块设置体**不渲染**，所以这不是「在默认值上
        叠一条红字」：**失败屏上除了这条红字没有别的** ⇒ 不给重试按钮，
        用户在这一屏里无事可做，只能退出。

        ⚠ 按钮复用既有的 `.cs-btn cs-btn-mini`（与「测连通」同一族），
        **不新造一个按钮样式**。
      -->
      <button class="cs-btn cs-btn-mini" onclick={() => void refresh()}>重试</button>
    </div>
  {/if}

  {#if view}
    {#each view.groups as g (g.id)}
      <!--
        ⚠ `{@const}` 必须**紧贴** `{#each}` 块，不能隔着 `<section>`——
          Svelte 报 `const_tag_invalid_placement`。所以它放在标签的最前面，
          而不是跟着「这一组的 key」那段注释走。
      -->
      {@const p = currentProvider(g)}
      <section class="cs-group">
        <div class="cs-group-head">
          <span class="cs-group-name">{g.label}</span>
          <span class="cs-badge miss">{g.route.provider_label}</span>
        </div>
        <p class="cs-hint" title="这一组干什么用的">{g.role}</p>

        {#if g.shares_with}
          <!-- ⚠ 必须**明说**。这一组没单独配 key 时它与另一组是同一格，
               而用户改这一节的端点会连带改掉那一组——不说就是坑人。
               ⚠ 2026-10-18：原文「这一组还没有自己的 key，当前与「回复模型」
               是同一路。」后半句「同一路」是内部说法，用户读到只会问
               「哪一路」；而**真正要紧的后果**（改这节会连带改另一组）
               被压在前半句后面当补充说明。
               ⇒ 直接说后果，一句话说完。
               ⚠ 2026-10-19：对方的名字改成由后端下发（`g.shares_with`）。
               共享**有方向**——只配了决策组时是回复组借决策组的格子，
               把「回复模型」写死会在那一种配置下**指错组**。 -->
          <p class="cs-note cs-warn">没单独的 key，改这节会连带改掉「{g.shares_with}」。</p>
        {/if}

        <!-- 厂商。**九项不是分段控件能装的**（380px 面板里一颗滑块放不下），
             所以用一组同族的小按钮换行排列——仍是这一屏已有的那支笔。 -->
        <p class="cs-label">走哪一家</p>
        <div class="cs-chips">
          {#each g.providers as p (p.id)}
            <button
              class="cs-btn cs-btn-mini"
              class:primary={p.id === g.route.provider}
              aria-pressed={p.id === g.route.provider}
              onclick={() => applyRoute(g, p.id)}
              disabled={busy[g.id]}
            >
              {p.label}{#if p.id === g.route.provider}<span class="cs-chip-ring" aria-hidden="true">
                  <svg viewBox="0 0 {TICK_VIEW.w} {TICK_VIEW.h}" filter="url(#pencil)">
                    {#each TICK_NESTED as layer (layer.d)}
                      <path d={layer.d} fill-opacity={layer.o} />
                    {/each}
                  </svg>
                </span>{/if}
            </button>
          {/each}
        </div>

        {#if g.route.provider === 'custom'}
          <!--
            ⚠ **「OpenAI 兼容」是协议，不是供应商。** 绝大多数第三方服务
            只兼容它，所以协议单独一选、地址自己填。
            ⚠ 协议一改，端点文本框就在上面被 Rust 重写过一次——用户看得见
            那一次改动，不会有一个「界面上看着是 A、实际发的是 B」的组合。
          -->
          <p class="cs-label">协议</p>
          <SegSlider
            value={g.route.protocol}
            options={view.protocols}
            label="协议"
            onchange={(v) => applyRoute(g, 'custom', v)}
          />
          <p class="cs-label">端点地址</p>
          <div class="cs-form">
            <input
              class="cs-input"
              type="text"
              autocomplete="off"
              spellcheck="false"
              placeholder="https://…/v1/"
              bind:value={endpoints[g.id]}
            />
            <button
              class="cs-btn cs-btn-mini primary"
              onclick={() => applyRoute(g, 'custom')}
              disabled={busy[g.id]}
            >
              {busy[g.id] ? '写入中…' : '用它'}
            </button>
          </div>
          <!--
            ⚠ **这一句是隐私承诺，必须在这一屏而不是文档里。**
            端点可换之后「请求会发给第三方模型」这句话已经不准确了——
            它可能是任何一台机器。所以这里显示**真实的目标地址**。
          -->
          <p class="cs-note">
            <strong>聊天内容与图片会发到这个地址：</strong>{g.route.base_url}
          </p>
        {:else}
          <p class="cs-note" title="这一组当前请求发去的地方">
            请求发往 {g.route.base_url}
          </p>
        {/if}

        {#if !g.route.measured}
          <!-- 端点与模型名抄自文档而没在本机跑过。**必须说出来**：
               症状是「设置成功、一调用就 404」，不说就变成玄学。 -->
          <!-- ⚠ 2026-10-18：原文「这一家的端点与模型名抄自官方文档，未在本机
               实测。」要紧的是**症状**（设置成功、一调用就 404），而原文把
               「抄自官方文档、未在本机实测」放在最前面——那是在交代我们的
               工作状态，用户拿它无法行动，反而看不见真正会发生什么。
               ⇒ 把症状提到最前面。上面「不说就变成玄学」的判断不变。 -->
          <p class="cs-note cs-warn">没在本机验证过：填错的话保存照样成功，一调用才报错。</p>
        {/if}

        <!-- 模型名。清单只有一条时不给分段控件（一颗滑块只有一个选项，
             它回答不了任何问题，看着却像个选择器）；清单为空时给输入框。 -->
        <p class="cs-label" title={g.model_note}>模型名</p>
        {#if g.models.length > 1}
          <SegSlider
            value={g.model}
            options={g.models.map((id) => ({ id, label: id }))}
            label="模型名"
            onchange={(v) => pickModel(g, v)}
          />
        {:else if g.models.length === 1}
          <p class="cs-note">{g.model || g.models[0]}</p>
        {:else}
          <div class="cs-form">
            <input
              class="cs-input"
              type="text"
              autocomplete="off"
              spellcheck="false"
              placeholder="模型名要你自己填"
              bind:value={drafts[g.id]}
            />
            <button
              class="cs-btn cs-btn-mini primary"
              onclick={() => commitModelName(g)}
              disabled={busy[g.id]}
            >
              用它
            </button>
          </div>
          <p class="cs-hint">{g.model_note}</p>
        {/if}

        <!-- 模型名没设成时的错误。**必须在这里显示**——见上面 `modelErr`
             的注释：赋值留着而展示被删过一次，报错等于被吞掉。 -->
        {#if modelErr[g.id]}
          <p class="cs-fail" role="alert">{modelErr[g.id]}</p>
        {/if}

        <!-- ══ 这一组的 key ══ -->
        {#if p}
          <div class="cs-prov-head">
            <span class="cs-prov-name">{p.label} 的 key</span>
            {#if p.configured}
              <span class="cs-badge ok" class:env={p.from_env}>
                {p.masked}{p.from_env ? '（来自环境变量）' : ''}
              </span>
            {:else}
              <span class="cs-badge miss">未配置</span>
            {/if}
          </div>

          {#if p.configured}
            <div class="cs-actions">
              <button
                class="cs-btn cs-btn-mini"
                onclick={() => (drafts[g.id] = REVEAL)}
                disabled={busy[g.id]}
              >
                换新 key
              </button>
              <!-- 连通性测试。不调模型：走只读的 models 端点，不花钱。
                   它测的是**当前这一路**，包括自己填的端点。 -->
              <button class="cs-btn cs-btn-mini" onclick={() => probe(g)} disabled={busy[g.id]}>
                {busy[g.id] ? '测试中…' : '测连通'}
              </button>
              <button
                class="cs-btn cs-btn-mini danger"
                onclick={() => forget(g)}
                disabled={busy[g.id]}
              >
                删除
              </button>
            </div>

            {#if drafts[g.id] === REVEAL}
              <div class="cs-form">
                <input
                  class="cs-input"
                  type="password"
                  autocomplete="off"
                  spellcheck="false"
                  placeholder="粘贴 key"
                  bind:value={drafts[g.id]}
                />
                <button
                  class="cs-btn cs-btn-mini primary"
                  onclick={() => save(g)}
                  disabled={busy[g.id]}
                >
                  {busy[g.id] ? '写入中…' : '保存'}
                </button>
              </div>
            {/if}
          {:else}
            <!-- p.hint 是 Rust 给的**可执行**指引：存哪、变量名叫什么。
                 不能是空字符串或「暂无」。 -->
            <p class="cs-hint">{p.hint}</p>

            <!-- ⚠ **未配置时输入框必须直接显示**，不能套
                 `drafts[g.id] === REVEAL` 那道门（2026-10-15 修的真机死路，
                 见 `.scratch/…/40-*.md`）。那道门本来只为「已配置时不把
                 明文 key 摆在界面上」而设，而 `REVEAL` 只有「换新 key」
                 按钮会去设——**而那个按钮在 `{#if p.configured}` 分支里**
                 ⇒ 未配置时输入框永远出不来。

                 后果不是「少个控件」：这是产品的第一步。
                 未配置时界面上没有任何 key，**没有需要藏的东西**。 -->
            <div class="cs-form">
              <input
                class="cs-input"
                type="password"
                autocomplete="off"
                spellcheck="false"
                placeholder="粘贴 key"
                bind:value={drafts[g.id]}
              />
              <button
                class="cs-btn cs-btn-mini primary"
                onclick={() => save(g)}
                disabled={busy[g.id]}
              >
                {busy[g.id] ? '写入中…' : '保存'}
              </button>
            </div>
          {/if}

          {#if p.legacy}
            <p class="cs-note cs-warn">
              这一把来自旧版那个不带组别的存档，两组共用。删除它对<strong>两组都生效</strong>。
            </p>
          {/if}
        {/if}

        {#if notes[g.id]}
          <p class="cs-note">{notes[g.id]}</p>
        {/if}
      </section>
    {/each}

    <!-- ══ 代理 ══ -->
    <h2 class="cs-h2">代理</h2>
    <!--
      ⚠ **单独一栏，不放进任何一组。** 代理是网络层的属性，
      两组模型都走它；放进去就意味着填两次，而两份迟早不一样——
      「一组走代理、一组不走」是个没人主动想要却真实会发生的状态。

      ⚠ 地址里常有 `user:pass@`。**它会进日志**，所以 Rust 那边对它
      走的是与 key 同一条脱敏路径（只抹凭据段，host 与端口留着）。
    -->
    <p class="cs-label" title="两组模型与「测连通」都走它。留空 = 直连。">出站代理</p>
    <div class="cs-form">
      <input
        class="cs-input"
        type="text"
        autocomplete="off"
        spellcheck="false"
        placeholder="http://127.0.0.1:7897（留空 = 直连）"
        bind:value={proxyDraft}
      />
      <button class="cs-btn cs-btn-mini primary" onclick={saveProxy}
        >{view.proxy.url ? '改用它' : '用它'}</button
      >
    </div>
    <!-- ⚠ 2026-10-18：原文「两组模型与「测连通」都走它。账号密码那部分在
         日志里会被抹掉。」前半句与上面 `cs-label` 的 title **逐字重复**——
         而 title 只在 hover 可见，等于占了一行常驻文字去重复一句看不见的
         内容；后半句才是这一屏为数不多的隐私承诺。
         ⇒ 去掉重复的前半句，把「留空 = 直连」这个后果提到正文。 -->
    <p class="cs-note">留空就是直连。代理里的账号密码不会进日志。</p>
    {#if proxyNote}
      <p class="cs-note" role="status">{proxyNote}</p>
    {/if}
  {:else}
    <p class="cs-note">正在读设置…</p>
  {/if}
</div>

<style>
  /* ══════════════════════════════════════════════════════════════
     这一屏与主面板**同族**：同一张纸、同一套墨与红铅笔（2026-10-16 反转）。

     ⚠ 反转前这里铺的是 `--cs-paper` 的冷灰 #EDF0F1，整张撕口纸被父级
     `.cs-sheet[data-settings='true']` 撤掉。两条理由让它作废：
     ① 同一窗口里一半纸一半冷白，统一感断掉；
     ② `--cs-ink` / `--cs-pencil` 的对比度是按暖黄纸重算的，
        放冷灰上等于按 A 纸的合格线用 B 纸。

     ⇒ 三支 `--cs-paper*` 现在都指向纸族（见 `app.css` 2026-10-16 的注释），
       字条仍用 `--cs-paper-raised`——材质词汇与主面板一致。

     ⚠ 独立设置窗口时代的 `--cs-surface`（中性白 #FAFAF9）仍然禁掉：
       在同一窗口里铺一块白底会留下一道接缝，而接缝正是这项目一直在删的。
     ══════════════════════════════════════════════════════════════ */

  .cs-set {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    padding: 0 14px 16px;

    /* ⚠ `margin-bottom` 的推导**在下面**（`padding` 之后、`line-height` 之前
     *   那一段），它记着「为什么是 44 而不是 28」——别在这里再写一份。 */
    margin-bottom: 44px;

    /* 行高在这里**设一次**，不靠每个选择器各写一个。
     *
     * 2026-10-16 实测：这一屏原先有 14 个选择器**根本没写 `line-height`**，
     * 吃的是浏览器默认 `normal` —— 而 `normal` 是「按字体算的」，
     * 不是某个数，于是**每一个选择器的实际行高都不同**，而且没人知道是多少。
     * 另有三处各写一个数（1.45 / 1.5 / 1.55）。
     *
     * ⇒ 统一 1.5 写在这里一次，全屏继承。
     *   下方少数选择器仍显式写着同一个值，是为了让它们**自解释**；
     *   数值必须与这里一致，改一处就要改另一处。 */
    line-height: 1.5;

    /* ⚠⚠ **`margin-bottom: 44px`**（票 60，2026-10-18 订正 2026-10-16 的 28px）。
     *
     *   ⚠⚠⚠ **2026-10-16 的方向是对的，参照物错了一个数。**
     *   它推的是「`.cs-sheet` 内容盒底 540 − 纸底 512 = 28」。
     *   `540` **是对的**（`.cs-sheet` 自己有 20px 下 padding，560 − 20），
     *   错的是把 `probe-sheet.svg` 里那张纸的**标称矩形 512** 当成了纸底：
     *   纸的下沿是**撕口**，2026-10-18 台架把它画进 canvas 量了**画出来的边**——
     *   实心底 **496**，撕口吃掉 16px。
     *   ⇒ 正确的差是 **540 − 496 = 44**。
     *
     *   ⚠ 中间走过一段弯路，记下来是因为它有普遍性：
     *   我一度以为「要减的只是撕口那 16px」，把 28 改成 16 ——
     *   可视区底反而从 512 **涨到 524**，比原来更糟。
     *   ⇒ **28 不是「多留了 12px」，它是「差值按错的参照物算小了 16」**。
     *   同一个动作在两个方向上都动过，而两次都只验了「值变了」，
     *   没验「**可视区底落在哪**」——值变不等于落点变。
     *
     *   ⚠ `padding-bottom` 救不了这一点：它只在**滚到底**时起作用，
     *   而用户看到的第一屏是 `scrollTop = 0`，那一屏的**可视区底**本身
     *   就在实心纸底下 ⇒ 要改的是可视高度，只能用 `margin-bottom`。
     *
     *   `.cs-sheet` 的内容盒底是 540（它自己有 20px 下 padding，560 − 20），
     *   而**纸的实心底是 496**（标称 512 − 撕口 16）。
     *   ⇒ 这一列原来有 **44px 的可视区落在纸外的透明区上**。
     *   在 Tauri 透明窗口里那不是「空白」，是**字浮在桌面上**。
     *
     *   出图证据：`set-after3` 那张，滚到底时最后一行
     *   「演示模式没有后台，这里不列模型配置。」整行落在纸边之下。
     *
     *   ⚠ 这条与主面板「档位名骑在撕口上」是**同一个错误**：
     *     两处都把一个**不是纸的边**当成了纸底（那边是元素底 560，
     *     这边是标称矩形 512）。
     *   ⇒ `budget.html` 现在拿 `PAPER_INK.bottomFrac`（画出来的那条边）
     *     当限值，并且量的是**真正画出来的像素**（`elementFromPoint` 打靶），
     *     不再量布局框——布局框既会被祖先 `overflow` 裁掉，
     *     也可能属于一个自己不画东西的容器。
     */
    margin-bottom: 44px;

    /* ⚠ **滚动条的样子不在这里**（2026-10-16 提到 `app.css`）。
     *
     *   这一屏原先是全项目**唯一**样式过的滚动容器，而同一天主面板的
     *   候选区也变成了滚动容器（票 44）却没人把那三行搬过来 ——
     *   于是那边亮出一条带 ▲▼ 箭头的原生滚动条。
     *   用户原话：「滚动条也是，你就这么喜欢黑色/原生样式？」
     *
     *   ⇒ 病根是「做法允许忘」，所以收进全局（`app.css` 的 `button` 之后），
     *   与 `.cs-actions` 一起管。判据：槽宽 6px 而非原生 15px，
     *   由 `budget.html` 量 `offsetWidth - clientWidth`。 */
  }

  /* 原来这里有一条 `.cs-lead`：「改完不用重启」那句。它现在是**屏头的副标题**
   * （`.cs-set-sub`），所以这条规则连同它的选择器一起删了——留着的样式
   * svelte 会报 `Unused CSS selector`，而门禁里那道 autofixer 正是这么抓到它的。
   *
   * 「解释某一句会发生什么」的那支左边线，现在只剩 `TierSwitch` 的 `.cs-hint`
   * 在用，规则在那边。凡是解释**整屏**的句子归屏头，不占一节。 */

  /* ── 分节标题 ────────────────────────────────────────────────
   *
   * 分组全靠**留白和字号**，不画分隔线 —— 和主面板同一条排版语言。
   * 上边距比一般间距大一倍，就是这一屏唯一的分组手段。 */
  /* ── 字号阶梯（2026-10-16 收敛）───────────────────────────────
   *
   * 用户原话：「设置页的字体不统一？」——实测确认不统一，而且不是错觉：
   * 收敛前这一屏有 **9 个 font-size**（10.4 / 10.88 / 11.2 / 11.5 / 11.52 /
   * 12 / 12.48 / 12.5 / 12.8 px）、**4 个 line-height**（默认 normal、1.45、
   * 1.5、1.55）、**3 个 font-weight**（400 / 600 / 700）。
   * 其中 11.5 与 11.52 差 0.02px、12.48 与 12.5 差 0.02px ——
   * **肉眼绝无分别，却各自写着**，这正是「不统一」的真正来源：
   * 不是差得多，是**没有一档是共用的**。
   *
   * ⇒ 收成 5 档，每一档都有多个元素共用：
   *     12.5px  字段标签 / 供应商名 / 「打开」按钮 / 分组内的小节标题
   *     12px    数值读数（滑块右侧的 30s / 90s）
   *     11.5px  说明正文 / 指引 / 错误 / 小按钮 / 输入框
   *     10.4px  徽标（badge）——唯一一档，因为它标的是**状态**不是句子
   *
   * ⚠⚠ **2026-10-18：分组标题从阶梯里拿出来了**，因为上面那条
   *   「分组全靠留白和字号」的机制**实测是空转的**（`target/probe/rhythm.mjs`）：
   *     `.cs-h2` 12.8px/600  vs  `.cs-label` 12.5px/600
   *     ⇒ **字号差 0.3px**（肉眼绝无分别，正是本段开头批评的那种病）
   *     留白差：h2 上方 20.8~22.8 / label 上方 19.7 ⇒ **差 1.1px**
   *     更糟的是**方向反了**：h2 到**标签**的间距 19.7，比到**控件**的 7.2 大，
   *     于是「标题」和「标题下面第一个字段名」之间的缝最宽。
   *   ⇒ 「不画分隔线」这条**不动**（它的由来是分节曾被套上描边卡片，
   *     一条平规则不是那回事），改的是让它**依赖的量真的分得开**：
   *     标题单独一档 13.6px + 字距，上边距加大、下边距给一点，
   *     并把紧跟标题的字段标签的上边距清零。
   *
   * 行高统一 **1.5**：原先说明类三处各写一个（1.45 / 1.5 / 1.55），
   * 而 14 个选择器根本没写、吃的是浏览器默认 `normal`——
   * `normal` 随字体不同而不同，等于**每个选择器的行高都不确定**。
   *
   * 字重只剩 **400 / 600**：700 原本只有 `.cs-group-name` 一处，
   * 是同屏唯一的孤例，已并入 600。 */
  .cs-h2 {
    /* ⚠ 分组标题档 = **13.6px**，不进正文阶梯。
     *   它标的是「这一整块在设什么」，与正文句不是同一类东西，
     *   放进正文阶梯只会逼它和字段标签挤到差 0.3px。
     *   字距是第二重信号：中文没有词间空格，标题靠**加宽**比靠加大
     *   字号更容易被认出来，而 0.08em 在 4 字标题上是 ~4.4px，肉眼可见。 */
    font-size: 13.6px;
    letter-spacing: 0.08em;
    margin: 1.35rem 0 0.28rem;
    font-weight: 600;
    color: var(--cs-ink);
  }

  /* ⚠ 紧跟分组标题的字段标签，**上边距清零**（2026-10-18 实测）。
   *   改之前：标题到**标签** 19.7px，到**控件**只有 7.2px ——
   *   也就是标题与它下面第一个字段名之间的缝最宽，
   *   读起来像「标题属于上一块，标签才是这一块的开头」。
   *   清零之后两者都贴着标题，这一块是什么一眼就定得下来。 */
  .cs-h2 + .cs-label {
    margin-top: 0;
  }

  /* ── 屏头（2026-10-18 新增）───────────────────────────────────────
   *
   * 这一屏原先**一个标题都没有**，直接从「发送档位」的小节开始。
   * 而「改完立刻生效」那句灰字只能挂在某个小节底下——挂着就变成
   * 「关于发送档位的话」，而它其实关于**整屏**。
   *
   * ⇒ 给屏头一个位置。这不是为了好看，是给**归属**：
   *   一句话该挂在哪个标题下，取决于它描述的是谁。
   *
   * `margin-bottom` 是唯一与 `.cs-h2` 不同的地方：屏头是这一屏的第一块，
   * 它下面紧跟第一个小节标题，两个 0.85rem 会让「设置」和「发送档位」
   * 看起来是**平级**的。 */
  .cs-set-head {
    margin-bottom: 0.35rem;
  }

  .cs-set-title {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    line-height: 1.4;
    color: var(--cs-ink);
  }

  .cs-set-sub {
    margin: 0.1rem 0 0;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--cs-pencil);
  }

  .cs-label {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--cs-ink);
  }

  /* 说明文字：正文用**铅笔档**（6.22 : 1），不是标注档（3.81 : 1）。
   * 这些句子是要人读的，读不清等于没说。 */
  .cs-note {
    margin: 0;
    font-size: 11.5px;
    /* 行距一大的代价是**占位**：字号明明最小，视觉重量却按行数×行距算。
     * 11.5px × 1.5 = 17.25px 一行，一段三行就是 51.75px，比一个开关还高。
     * （这里原先是 0.7rem + 1.45，2026-10-16 随字号阶梯一并归到 11.5 / 1.5。） */
    line-height: 1.5;
    color: var(--cs-pencil);
    white-space: pre-line;
    /* 一整段说明文字不该横跨整个面板 —— 窄屏上回行本来就密，
     * 再加 `max-width` 只会让它变成一小块。限制的是**行内的连续长度**：
     * `anywhere` 保证供应商名、长 URL 这类无空格串不会撑出横向滚动条。 */
    overflow-wrap: anywhere;
  }

  .cs-note strong {
    color: var(--cs-ink);
    font-weight: 600;
  }

  .cs-hint {
    margin: 0.25rem 0 0;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--cs-pencil);
    /* 指引可能有换行，要原样显示 */
    white-space: pre-line;
    overflow-wrap: anywhere;
  }

  /* 错误 + 它的重试按钮。**同一行、同一左边线**——
   * 分成两行时那条按钮会被列向 flex 拉成整宽块（2026-10-18 实测 288px），
   * 而同屏「打开」是 97px 自适应 ⇒ 同类动作两个宽度。
   * ⚠ 错误文案**可换行**且通常不止一行，所以 `min-width: 0` 必需，
   *   否则 flex 子项按内容宽度撑开，按钮会被挤出可视区。 */
  .cs-retry {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
  }

  .cs-fail {
    flex: 1 1 auto;
    min-width: 0;
    margin: 0;
    padding: 0.4rem 0.5rem;
    font-size: 11.5px;
    line-height: 1.5;
    border-radius: var(--cs-radius-sm);
    /* 失败方向偏保守的一段话，用极淡的红底托一下。
     * ⚠⚠ **混纸，不混 `transparent`**（2026-10-18）。`app.css` 顶头那条
     *   「所有底色必须完全不透明」是硬约束：Tauri 透明窗口走 WebView2
     *   分层合成，子元素上的半透明底色**整层不绘制**（2026-10-07 实测）。
     *   浏览器里分毫不差，真机窗口里**可能整块消失**——
     *   而 `?demo=1` 的出图评审全在浏览器里做，**这类错浏览器查不出来**。
     *   ⚠ 混比 12% 只有 1.184 : 1（读不出「这一块是坏的」），
     *   **20% → 1.326 : 1**，ink 字仍有 10.64 : 1。
     *   ⚠ **不是靠它承载正文**（正文色是 `--cs-ink`，不红），
     *     它只是「整块是坏的」这一层分区信号——所以宁可弱也别抢。
     *   ⚠ 语义边界：demo 模式那句「演示模式：这里显示的是排版」
     *     也走这一格（同一个 `fail` 字段），而它不是错误；
     *     真机上那个位置渲染的是 Rust 传来的内容。 */
    background: color-mix(in srgb, var(--cs-live) 20%, var(--cs-paper));
    overflow-wrap: anywhere;
  }

  .cs-retry .cs-btn {
    flex: none;
  }

  /* 配置**当前状态**的提醒（无解预算 / 没单独 key / 没在本机验证过 /
   * 旧存档两组共用）——**不是「出事了」**。
   *
   * ⚠⚠ **2026-10-18：从 `--cs-live` 改回墨色**，两条理由各自独立成立：
   *   ① **语义**：本文件「红色只留给那句『在你没预期的时候它会这么做』」
   *      那条定案，讲的是**后果突然发生**；这四句讲的全是**配置的事实**
   *      （这个值不起作用 / 这两组共用一格 / 这家没验证过），
   *      不符合那句话的形状。符合的是 `.cs-warn-on`。
   *   ② **对比度**：`--cs-live` 在暖黄纸上只有 **4.28 : 1**，
   *      11.5px 常规字重不达 AA 正文 4.5（大字豁免要 ≥18.66px 粗体，这里够不上）。
   *
   * 「不是已保存，是这个值不起作用」这个区别**由字重扛**，不靠颜色。 */
  .cs-warn {
    font-weight: 600;
    color: var(--cs-ink);
  }

  /* ── 按钮 ─────────────────────────────────────────────────── */
  .cs-btn {
    /* ⚠⚠ **两族按钮与输入框共用同一个内容盒高度**（2026-10-18）。
     *   改之前：`.cs-btn-ink` 是 `padding: 7px 18px`（约 31px）、
     *   `.cs-btn-mini` 是 `padding: 6px 11px`（约 29px），
     *   各自按自己的字号算，**两族差 2px**；加上同样的撕边之后，
     *   真机实测同一屏出现 62 / 58.5 / 55.5 / 46 / 44 **五种高度**。
     *
     *   ⇒ 不再用 padding 推高度，统一 `min-height` + flex 居中。
     *     ⚠⚠ **`box-sizing: border-box` 下 `min-height` 量的是总高（含撕边），
     *     不是内容盒**——2026-10-18 之前这里写「min-height: 30px + 16px 边 ⇒
     *     逐项相等」，那句话两处都推不出来：30px 是**总高**，
     *     而撕边本身就有 32px，于是这个 `min-height` 一直是**空转的**
     *     （实测三档 62 / 58.5 / 55.5，各族各按自己的 padding 走）。
     *     现在写成真值 46px：撕边 10×2 = 20 + 内容 26。
     *
     *   ⚠ 前提是这一族都写着 `class="cs-btn ..."` —— 三族
     *     （`.cs-btn-ink` / `.cs-btn-mini` / 未来的）都走这条，
     *     谁漏了 `cs-btn` 谁就掉出这条对齐。 */
    box-sizing: border-box;
    min-height: 46px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    /* ⚠ `line-height: 1`：flex 居中之后，行高再参与一次垂直布局，
     *   会把文字顶上去 1–2px。按钮里的文字不需要行距。 */
    line-height: 1;
    border-radius: var(--cs-radius-sm);
    font-weight: 600;
    transition:
      background var(--cs-dur-fast) var(--cs-ease),
      color var(--cs-dur-fast) var(--cs-ease),
      opacity var(--cs-dur-fast) var(--cs-ease);
  }

  /* ⚠ **禁用态不降透明度。**`disabledsep.py` 实算：
   *   `opacity: .34` 把**整个元素**一起稀释——文字、边框、底色全淡。
   *   `.cs-btn-mini` 的边框正常态就只有 1.57:1，被淡一次掉到 **1.16:1**，
   *   而「禁用」恰恰是最该一眼看出来的状态。`.45` 也只有 1.21:1。
   *   ⇒ 改用「换线型 / 换底色」，对比度与正常态保持一致。
   *   ⚠ WCAG 1.4.3 / 1.4.11 对 disabled 是豁免的，这里量的是设计可读性。
   */
  .cs-btn:disabled {
    cursor: not-allowed;
  }

  /* 有边框的那批（含 `.danger` 与 `.primary`）：只换线型，底色文字都不动。
   *
   * 2026-10-16 **这条已随撕边作废**：`.cs-btn-mini` 改用 `border-image` 之后，
   * `border-style` 画不出来（与 `.cs-btn-ink:disabled` 记的是同一件事）。
   * 禁用表现在统一走「收掉投影 + 铅笔灰」，见下面 `.cs-btn-mini:disabled`。 */
  .cs-btn-mini:disabled {
    cursor: not-allowed;
    /* 禁用读成「一片没落在纸上的纸」——与 `.cs-btn-ink:disabled` 同一条。
     * 不用透明底：WebView2 分层合成会把整层低 alpha 整层不绘制，那会让按钮
     * **整个消失**。所以只动影与字色。 */
    color: var(--cs-pencil-2);
    filter: none;
  }

  /* ⚠ 原先这里有一条 `.cs-btn-ink:disabled` 用「换底色」表达禁用
   *   （`background: var(--cs-paper)` + 文字转墨色），**已于 2026-10-16 删除**。
   *
   *   它是**实心墨底**时代的做法：没有底色可换时，就换成纸色。
   *   现在这颗按钮是撕边纸条 —— 纸由 `border-image … fill` 给，
   *   `background` 画在它下面，`background-color` 已经不是它的表面，
   *   换它等于换一张画在撕口下面的纸，读不出来。
   *   ⇒ 禁用改用**收掉投影**（与主面板 `.cs-act:disabled` 同一条），
   *   新规则在下面 `.cs-btn-ink` 那段之后。
   *
   *   ⚠ 当年那条注释里「纸族上红铅笔没有余量」的实测（红 4.56:1）
   *     依然成立，只是它守护的对象已经不存在了 —— 纯黑底没了。 */

  /* 图片外发总闸。⚠⚠ **从「实心墨底药丸」改成撕边纸条**（2026-10-16）。
   *
   *  ## 为什么它是全屏最后一块纯黑
   * `background: var(--cs-ink)` 是一块**纯黑实心块 + 反白字**。
   * 用户 2026-10-16 的原话：「为何还有这种纯黑的0设计按钮存在？」
   * ——「0 设计」说得准确：它没有任何材质、没有笔、没有撕口，
   * 就是浏览器按钮换了个颜色。同一屏其余控件都已经是纸，
   * 只有这一颗还停在 Web 上。
   *
   *  ## 为什么不是「换个浅色底」
   * 改底色只是把黑块换成灰块，**载体仍然是色块**。
   * 而票 44 已经在主面板上判过一次：候选动作行用实心色块，
   * 读出来就是「按钮」，不是「从这张纸上撕下来的一片」。
   * ⇒ 这里直接用同一套撕边（`.cs-act` 的 `border-image` / `strip.png`），
   * 同一个材质源、同一套参数，不是「类似风格」。
   *
   *  ## 为什么它仍然可以是「开关」
   * 撕边说的是「这是一片纸」，不规定它是「做什么的」。
   * 开/关由**字**说（「打开」/「关闭」），不由颜色说 ——
   * 红在这套色板里只给不可逆（app.css 文件头），而这一项恰恰**可逆**。
   */
  .cs-btn-ink {
    align-self: flex-start;
    position: relative;
    /* ⚠ 这个负外边距是为了抵消撕边带来的额外高度（`.cs-act` 同一手法）。
     *   撕边 16 → 10px 后从 `-8px` 收到 `-5px`，别留着旧的，
     *   否则这一行会比同行多出 6px 的空当。 */
    margin: -5px 0;
    padding: 7px 18px;
    font-size: 12.5px;
    color: var(--cs-ink);
    /* 抬起 / 压回是 transform，得自己有 transition；
     * `.cs-btn` 里那三条（background / color / opacity）在撕边之后已经
     * 没有可过渡的对象了 —— 纸条不变色、不换底。 */
    transition:
      transform var(--cs-dur-fast) var(--cs-ease),
      filter var(--cs-dur-fast) var(--cs-ease);
    /* ⚠⚠ **撕边 16px → 10px**（2026-10-18）。
     *
     *   原注释说「写 16px 是因为撕口的纸芯最早只在内缩 ~5.5 CSS px 处
     *   （`stripgen.py` 实测最浅一档），再薄就会啃进纸」。
     *   那个 16 是**照抄 `.cs-act`（主面板动作行）来的**，
     *   而主面板那一行的字是 20px 上下、内容盒 40px——16px 撕边占 40%。
     *   设置页这一族内容盒只有 30px，**撕边占了 53%**：
     *   输入框因此是 205 × 62（**高宽比 1 : 3.3**，出图确认读成一块
     *   空荡荡的横板，不像能写字的地方），同屏按钮还因为 flex 压缩
     *   分裂成 62 / 58.5 / 55.5 / 46 / 44 五种高度。
     *
     *   ⇒ 按内容盒重算：撕边 10px + 内容 26px = **46px**，
     *     高宽比 205 : 46 = **1 : 4.5**。
     *   `10` 与 `slice 32` 的换算关系不变（slice 量的是源图像素，
     *   width 量的是画到边框盒多宽），**不需要重跑 `stripgen.py`**。
     *   ⚠ 「纸芯内缩 5.5px」那条推导是相对 16px 说的（5.5/16 = 34%），
     *   按同比例换算 10px 下是 3.4px；**这一格是出图看的，不是算出来的**，
     *   若撕口开始啃进内容区就往回调。 */
    border: 10px solid transparent;
    border-image: url('/strip.png') 32 fill / 10px / 0 stretch;
    /* 投影跟着 alpha 走：`box-shadow` 画的是边框盒，
     * 撕口的透明区会漏出它的直边，在纸上就是一圈生硬的方框
     * （票 44 实测：同一列 x=203 连续暗 185~205，邻列 198/227）。
     *
     * 2026-10-16 用户红框指出「顶部为何要有这条黑边」，实测同主面板：
     * `drop-shadow(0 2px 3px …)` 的核覆盖 `[-1.5, +3.5]`，**越进元素内侧
     * 0.5px**，那半份影子从半透明撕口底下透出来 ⇒ 上沿浮出一条直线。
     * ⇒ 判据是 `dy ≥ blur/2`，不靠"看着还行"。 */
    filter: drop-shadow(0 3px 6px rgb(24 18 10 / 0.2));
  }

  .cs-btn-ink:hover:not(:disabled) {
    color: var(--cs-ink-2);
    transform: translateY(-1px);
    filter: drop-shadow(0 6px 11px rgb(24 18 10 / 0.28));
  }

  .cs-btn-ink:active:not(:disabled) {
    transform: translateY(1px);
    filter: drop-shadow(0 2px 4px rgb(24 18 10 / 0.14));
  }

  /* ⚠ 禁用态读成「一片没有落在纸上的纸」——收掉投影，
   *   与主面板 `.cs-act:disabled` 同一条（那里记着为什么不能用虚线边框：
   *   `border-image` 一生效，`border-style` 就画不出来）。
   *   ⚠ 不能用透明底：WebView2 分层合成会把整层 `rgb(... / 低alpha)` 整层
   *   不绘制（app.css `--cs-paper` 的注释，2026-10-07 实测），
   *   那会让按钮**整个消失**。所以只动 filter。 */
  .cs-btn-ink:disabled {
    color: var(--cs-pencil-2);
    filter: none;
  }

  /* key 卡片里的小按钮。**与 `.cs-btn-ink` 逐项同源**：同一张 `strip.png`、
   * 同一套 slice / width、同一条「核不上沿」判据（见 `.cs-btn-ink` 那段的实测）。
   *
   * 2026-10-16 用户原话：「让你参考档位是指像现在打开按钮这样去做**风格统一的
   * 按钮样式**，而不是拿之前的设计敷衍。」⇒ 这一族原来还是 `1px solid` 描边，
   * 与刚改好的「打开」并排时就是两个时代的东西。
   *
   * 代价是**变高**：撕边上下各 16px，30px 的按钮变成约 61px。这是照着用户
   * 指定的方向换材质必然的尺寸变化，不是顺手调出来的。
   * ⚠ 2026-10-18：撕边 16 → 10、内容盒 30 → 26，总高 62 → **46**，
   *   与 `.cs-btn-ink` / `.cs-input` 同高。三个值**必须一起改**。 */
  .cs-btn-mini {
    padding: 6px 11px;
    font-size: 11.5px;
    white-space: nowrap;
    border: 10px solid transparent;
    border-image: url('/strip.png') 32 fill / 10px / 0 stretch;
    /* 影子随 alpha 走（同 `.cs-act`）：`box-shadow` 画边框盒，会从撕口的透明区
     * 漏出矩形直边。`dy ≥ blur/2` ⇒ 高斯核不越进元素内侧，顶部不会浮出黑边。 */
    filter: drop-shadow(0 2px 4px rgb(24 18 10 / 0.16));
    transition:
      filter var(--cs-dur-fast) var(--cs-ease),
      transform var(--cs-dur-fast) var(--cs-ease);
  }

  /* 悬停 / 聚焦：**拿起来**——与主面板 `.cs-act` 同一个动作，只是抬 1px
   * （小按钮抬 2px 会顶到相邻行）。键盘 Tab 到这里也走同一条。 */
  .cs-btn-mini:hover:not(:disabled),
  .cs-btn-mini:focus-visible {
    transform: translateY(-1px);
    filter: drop-shadow(0 5px 9px rgb(24 18 10 / 0.24));
  }

  /* 按下：**压回纸面**。抬起与按下必须成对，否则反馈只剩「换了个底色」——
   * 而这片纸不会变色。 */
  .cs-btn-mini:active:not(:disabled) {
    transform: translateY(1px);
    filter: drop-shadow(0 1px 2px rgb(24 18 10 / 0.12));
  }

  /* 撕边之后**「主要 / 危险」只能靠字色说**：`border-image` 一生效，
   * `border-color` 就不再被画出来（`.cs-btn-ink` 的禁用态注释记着同一件事）。
   *
   * ⚠⚠ **2026-10-18：`.primary` 原来也是 `var(--cs-live)`，改回墨。**
   *   理由不是审美，是**硬约束第 4 条 + app.css 文件头「红只给不可逆」**：
   *   全文 8 处 `.primary`（保存 / 应用路由 / 提交模型名 / 存供应商 / 用它）
   *   **全部可逆**，而 `.danger`（删除 / 忘记）才是不可逆的那个。
   *   两族共用一支红 ⇒ 同一行里「保存」与「删除」红得一模一样，
   *   **可逆与不可逆在界面上读不出区别**（2026-10-18 出图确认：
   *   那一行只有「保存」是红的，红色已经不再表示危险）。
   *
   *   「主要」与「普通」现在只差字重（`.cs-btn` 已统一 600），**故意的**：
   *   这一屏每个动作都写着自己的字（保存 / 删除 / 测连通），
   *   靠字就认得出来，不需要颜色再分一遍。 */
  .cs-btn-mini.primary {
    color: var(--cs-ink);
  }

  .cs-btn-mini.danger {
    color: var(--cs-live);
  }

  /* ⚠ 原来这里有一整段 `.cs-seg` / `.cs-seg-item` / `.cs-seg-item.on` 的样式
   * （2026-10-10 删）。「关闭窗口的行为」现在用 `SegSlider` 组件，
   * 与「发送档位」是同一套——滑块由组件自己带。
   *
   * 顺带修掉一个**规格不一致**：旧的 `.cs-seg-item` 是 `padding 6px 12px /
   * 12.5px / 圆角 --cs-radius-sm`，而 `TierSwitch` 那一版是
   * `0.281rem 0.75rem / 0.78rem / --cs-radius`。同一屏两组分段控件
   * 两种规格——而档位那组的 33px 总高是**按容器盒模型倒推**出来的，
   * 照抄独立按钮的 padding 会矮 4px（实测 29px）。 */

  /* ── 滑块 ───────────────────────────────────────────────────
   * ⚠ `<output>` **必须**样式过：浏览器默认给它 13.33px Arial + 蓝色下划线，
   * 比这一屏正文还大还跳。一个「12 条」不该比解释它为什么这么调的话还重。 */
  .cs-slider {
    display: flex;
    align-items: center;
    gap: 0.7rem;
  }

  .cs-slider input[type='range'] {
    flex: 1 1 auto;
    min-width: 0;
    accent-color: var(--cs-ink);
    /* ⚠ **必须自己画轨道**（2026-10-15 真机指出后修）。
     * `accent-color` 只换拇指颜色，轨道仍是 WebView 的浅灰槽——
     * 于是读成「一条白底窄杠上放个黑点」，与同一屏的墨线控件不是一个东西。
     * `appearance: none` 是前提：不关掉它，上面那两条伪元素不生效。 */
    -webkit-appearance: none;
    appearance: none;
    background: transparent;
  }

  /* 轨道 = **铅笔灰**，不是墨。
   *
   * 依据是这一套色板自己的分工（app.css 文件头）：**墨 = 你写下的，
   * 灰 = 机器认出来的**。轨道是「可以走的那条路」——还没写；
   * 而拇指是「你把它放在哪」——那是你写的。
   * ⇒ 灰轨 + 墨点，而不是同一种颜色画两个东西。
   * 原来轨与点都是 `--cs-ink` 纯墨，14px 的实心黑点在暖黄纸上比同一屏
   * 任何东西都重，而它表达的只是「一个数值」。**量级与它代表的风险不匹配**
   * （与票 44 否掉「首选=实墨」是同一条理由）。 */
  .cs-slider input[type='range']::-webkit-slider-runnable-track {
    height: 1px;
    /* 轨道要**明显退到背景**，但不能退成冷灰蓝。
     * 三个候选都实测过：`--cs-ink`(20,26,31) 太黑、`--cs-pencil-2`(66,80,89)
     * 只淡一点点、仍读成一条深灰线；`--cs-rule`(211,218,221) 又太淡而且是冷色，
     * 与暖黄纸不同源。
     * ⇒ 铅笔灰与纸对半调，得到一个**暖调中灰**，与纸同源。 */
    background: color-mix(in srgb, var(--cs-pencil) 52%, var(--cs-paper));
  }

  /* 拇指 = 墨点，外加一圈纸色环，让它从轨道上**浮起来**而不是嵌进去。
   * `margin-top` 把它压回轨道中线
   * （track 1px、thumb 13px ⇒ 上移 (13-1)/2 = 6，取 6px）。 */
  .cs-slider input[type='range']::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 13px;
    height: 13px;
    margin-top: -6px;
    border-radius: 50%;
    background: var(--cs-ink);
    box-shadow: 0 0 0 2px var(--cs-paper);
    cursor: pointer;
  }

  .cs-slider input[type='range']:focus-visible {
    outline: 2px solid var(--cs-ink);
    outline-offset: 3px;
  }

  /* 量程两端。**左右端必须与轨道对齐**，否则两行数字看着是歪的。
   * ⚠ 那个 `2px` 不是玄学：`input[type=range]` 的 UA 样式自带
   *   `margin: 0 2px`，所以它的盒子比 flex 行窄 4px（实测行 46..334、
   *   轨道 48..237.6）。不补这 2px，右端会差 2px。
   * ⚠ 右边那坨 = 读数 `min-width` 5.2rem + 行 `gap` 0.7rem + 上面那 2px。
   * ⚠ 不用 `--cs-mark`：那支只有 3.51 : 1，标的是状态不是句子；
   *   这里读的是数字，10.4px 也够不上大字豁免，所以用 `--cs-pencil` 5.73。 */
  .cs-scale {
    display: flex;
    justify-content: space-between;
    margin: 0.15rem calc(5.2rem + 0.7rem + 2px) 0 2px;
    font-size: 10.4px;
    line-height: 1.5;
    color: var(--cs-pencil);
  }

  .cs-slider output {
    flex: none;
    min-width: 5.2rem;
    /* 机器读出来的数 → 等宽，和主面板的分工一致。 */
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 12px;
    font-weight: 600;
    color: var(--cs-ink);
    /* UA 默认把 output 渲染得像超链接，蓝色 + 下划线。去掉。 */
    text-decoration: none;
  }

  /* ── 两组模型配置（票 43） ─────────────────────────────────────
   *
   * ⚠ 这一段是 2026-10-15 新写的，而配套样式当时**一条都没写**——
   *   标记在、样式不在，于是两组区块渲染成完全裸的块级元素：
   *   没有边框、没有底色、没有分组间距。svelte-check **不会**因为
   *   「类名在标记里但样式表里没有」而报错（它只报反过来的
   *   `css_unused_selector`），所以这种缺口能一路躲过 type check。
   *
   * 几何沿用这一屏既有的那一支：`.cs-prov` 那种「字条落在纸上」的卡片
   * （`--cs-rule` 1px 描边 + `--cs-paper-raised` 底 + `--cs-radius-sm`）。
   * 换句话说：**两组 = 两张字条**，这是这一屏已经有的语言，
   * 不为「两组」另发明一套。
   */
  /* 两组之间靠**留白 + 标题**分开，与主面板同一条排版语言
   * （`.cs-h2` 的注释：「分组全靠留白和字号，不画分隔线」）。
   * 这里原先还给每组套了 1px 描边 + 圆角 + 抬升底色 ——
   * 那是「卡片」，是另一套材质语言，而且它把里面已经是撕边纸条的按钮
   * 又关进一个描边盒子里，等于同一个控件上了两层框。 */
  .cs-group {
    padding: 0;
  }

  /* 组间距要**看得见地分开**：它们是两份互不影响的配置，
   * 挤在一起会被读成一张卡的上下两段。 */
  .cs-group + .cs-group {
    margin-top: 0.9rem;
  }

  .cs-group-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
  }

  /* 分组标题。与 `.cs-h2` **同一档**（2026-10-18 跟着改的）。
   *
   * ⚠ 这里原来写 12.8px 并注释「与 `.cs-h2` 同值」——`.cs-h2` 改到 13.6px
   *   加字距之后这句就假了，于是同一屏出现两种分节标题：
   *   「主模型 / 备用模型」一种、「发送档位 / 外观 / 窗口」另一种。
   *   两者在结构上是**同一级**，必须同值。
   *   （原来它独自写 `0.8rem` 却配 700 —— 全屏唯一的第三档字重，
   *   2026-10-16 已并入 600。） */
  .cs-group-name {
    font-size: 13.6px;
    font-weight: 600;
    letter-spacing: 0.08em;
  }

  /* 厂商选择：九项在 380px 的面板里**换行排**，不做分段控件
   * （票 43 原话「一颗滑块放不下」）。仍是 `.cs-btn-mini` 那一族，
   * 所以选中态直接借用 `.primary`，不另造一颗药丸。 */
  .cs-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    margin-top: 0.3rem;
  }

  /* ⚠⚠ 厂商芯片的选中态：**一枚红铅笔勾**（2026-10-18，票 61）。
   *
   *   ⚠⚠ **上一版（2026-10-18 当天）是错的，已撤。** 那一版给选中那颗
   *   加了一层 `12% tier-ink 混纸` 的底色，想跟 `SegSlider` 的药丸对齐。
   *   用户的判词是「毛坯」「模型选中的阴影又是什么个意思」。
   *   两条批评都成立，根因是**它把两件事混成了一件**：
   *
   *   ① `.cs-btn-mini` 全族本来就有 `drop-shadow(0 2px 4px …)` ——
   *      那不是「选中态的阴影」，那是**材质语言**：这是一片贴在纸上的小纸片，
   *      所以它有落影。选中与落影本来就不该有关系。
   *   ② 我加的淡墨洗底**把落影顶出来了** ⇒ 读成「这枚按钮的阴影更重」，
   *      而人真正要读的是「这家被选中了」。**状态被材质盖住了。**
   *
   *   ⇒ 现在的做法与主面板档位行**同一条语言**：
   *     空着就是空着，**标记的位置本身就是编码**——
   *     选中那颗右上角盖一枚圈，没选的什么都不画（`PaperPanel` 那段原话）。
   *
   *   ⚠ 为什么是**勾**而不是圈：主面板档位行在 Manual / Auto 上就是勾
   *     （`mark-shape` 只在 big 档才给 ring），所以这里跟**最常出现的那一档**
   *     同一支笔。票 57 判「勾没有能用的尺寸」是在 `SegSlider` 那个
   *     56×34 药丸上判的（它要**盖住 12.48px 的小字**）；
   *     而这里是**右上角一枚独立标记**、不压字，30×18 完全放得下。
   *   ⚠ 标记必须挂 `filter="url(#pencil)"`，而那个滤镜**原来只在
   *     `PaperPanel` 和两个 studio 页里**。设置屏没有 ⇒ `url(#id)`
   *     找不到目标时 SVG 规范是**整个元素不渲染**，
   *     症状是「勾整个不见了」而不是「没滤镜效果」。
   *     ⇒ 现在统一从 `$lib/PencilDefs.svelte` 取（全仓库唯一一份）。
   */
  /* ⚠⚠⚠ **勾的位置不是「设计」出来的，也不是从主面板抄来的**（2026-10-18 真机订正）。
   *
   *   用户两次指认「勾选为何要放到文字最前面」——判对。票 63 把它改成
   *   「量出来的主面板几何」，**真机照样错**，因为抄的是**一个样本的常量**。
   *
   *   ### 当时的推理（错在哪一步）
   *   实测 `target/probe/main-tick-geom.mjs`（Chromium 380×560）：
   *     字 45.5 宽 · 勾 54.4 宽 · 勾心比字心偏左 0.73em
   *   ⇒ 写成 `3.66em / 2.22em / −0.73em` 三个常量，芯片与药丸共用。
   *
   *   ### 那个 −0.73em 不是关系
   *   真机 + 量具复测（`target/probe/chip-tick-geom.mjs`）证明它是**常量**，
   *   而「勾心该在哪」取决于**标签有多长**：
   *
   *     | 标签 | 字宽 | 勾心 − 字心 | 换算成字宽的倍数 |
   *     |---|---|---|---|
   *     | 只填入 | 37.5 | −12.9px | **−0.34** |
   *     | 退出   | 25.0 | −12.7px | **−0.51** |
   *     | 勾     | 12.5 | −12.9px | **−1.04** ← 勾整个落在字的左边 |
   *
   *   三行的 px 几乎一样（偏移是常量），倍数却从 0.34 跑到 1.04。
   *   ⇒ 一字标签上那枚勾**根本没碰到那个字**，它读成「排在前面的勾选框」——
   *     正是用户两次指认的那件事。**照抄一个样本 = 把巧合当规范。**
   *
   *   ### 真正的关系只有一条：勾心 = 字心
   *   芯片的 `left: 50%` 就是字心（芯片宽度由文字决定、文字居中），
   *   不需要再往左挪。药丸那一支同样，`SegSlider.svelte` 一起改。
   *
   *   ⚠ 尺寸也不用 em 了：`em` 各按自己继承到的字号解算，同屏两处
   *     差 1.39 倍（芯片 42.1 vs 药丸 58.5）。统一走 `--cs-stamp-w`。
   */
  .cs-chip-ring {
    position: absolute;
    left: 50%;
    top: 50%;
    height: calc(var(--cs-stamp-w) * 34 / 56);
    width: var(--cs-stamp-w);
    transform: translate(-50%, -50%);
    pointer-events: none;
    overflow: visible;
  }
  .cs-chip-ring svg {
    width: 100%;
    height: 100%;
    display: block;
  }

  /* 只有 fill、不设 fill-opacity 的兜底：透明度是**笔迹数据**的一部分
     （`marks.js` 每层自带 `o`）。CSS 兜底会把三道叠画压成单层平涂，
     圈会立刻变成一个生硬的矢量圆环——见 `PaperPanel` 的同款警告。 */
  .cs-chip-ring path {
    fill: var(--cs-pencil-red);
  }

  /* 选中那颗本来就没有底色（`primary` 只改字重），这条只负责让勾有定位父级。
     ⚠ 不给 `padding-left`：勾是**盖章**，压在字上，不是排成勾选框那一列。
     第一版给整族加 `padding-left: 30px` 是想「让四颗左对齐线不跳」，
     而盖章不需要槽位——它本来就盖在字上，选中前后文字位置不动。 */
  .cs-chips .cs-btn-mini {
    position: relative;
  }

  /* 供应商卡片那一支已随两组结构退场，只留仍在用的 head / name。
   * `.cs-prov` 与 `.cs-prov + .cs-prov` 2026-10-15 删除：
   * 新标记里不再有 `.cs-prov` 卡片（svelte-check 报 css_unused_selector）。 */
  .cs-prov-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .cs-prov-name {
    font-size: 12.5px;
    font-weight: 600;
  }

  /* 徽标是**状态**，不是句子 ⇒ 单独一档 10.4px，比正文还小一档。
   *
   * 圆角从 999px（药丸）改成 **2px**：药丸是 Web 控件语汇，
   * 在这一屏里与撕边纸条、墨线、铅笔灰都不是一族。
   * 2px 是「一张小纸片」的最小圆角——撕边的毛边本来就是方的，
   * 药丸却把它整个圆掉，等于说「这不是纸」。 */
  .cs-badge {
    font-size: 10.4px;
    padding: 0.1rem 0.4rem;
    border-radius: 2px;
    overflow-wrap: anywhere;
  }

  /* ⚠⚠⚠ **它不该是一个「tag」。**（2026-10-18 第三轮改）
   *
   *   用户把真机裁图贴回来问「这三个组件符合统一设计吗？」——
   *   图里「已配置」是一个**圆角矩形灰底小块**，而同屏按钮是撕边纸片。
   *   ⇒ 底色 + 圆角 = **Web 控件皮肤**：这块纸上一件 Web 控件都不该有。
   *
   *   两轮改法都是错的，值得记下来：
   *     ① 12% / 20% 墨洗（同底不同深浅）——**仍然是一个灰底小块**，
   *        只是深浅不同，形状没变 ⇒ 原问题原样留着。
   *     ② `inset 1px --cs-edge` 描边——那是**表单框**，更 Web。
   *
   *   ⇒ 现在**去掉底色**：它是组标题行右端的一个**注记**，
   *     靠位置 + 字色 + 字重说状态，和主面板那些说明是同一种东西。
   *     真要一眼分得出，靠的是 `已配置`（墨 600）与 `未配置`（铅笔灰 400）
   *     的**字**，不是一块底。
   */
  .cs-badge {
    font-size: 10.4px;
    padding: 0;
    overflow-wrap: anywhere;
    color: var(--cs-pencil);
    background: none;
  }

  /* 已配置 = 常态。**不给红也不给绿** —— 绿在这个色板里是「对方」的颜色，
   * 用在这里等于把说话人的身份色借去表示状态。 */
  .cs-badge.ok {
    color: var(--cs-ink-2);
    font-weight: 600;
  }

  .cs-badge.ok.env {
    color: var(--cs-pencil);
    font-weight: 600;
  }

  /* 未配置：给铅笔灰，不给红。
   * docs/面板设计.md 的不变量是「红只给不可逆」——看到红就是有东西出去过，
   * 或者要出去。缺 key 时一个字都还没出去，也不构成一次不可逆动作，
   * 穿红字会把唯一的语义色稀释掉。真要走不可逆语义，红留给发送与退出。
   *
   * ⚠⚠⚠ **2026-10-18 记两条别回退的东西**：
   *   ① 这一支原先写的是 `var(--cs-line)`，而那个自定义属性**全仓从未定义**
   *      （刻线那支叫 `--cs-rule`）——引用不存在的属性 ⇒ 计算时 invalid
   *      ⇒ 整条 `box-shadow` 作废 ⇒ 那圈描边**从来没渲染出来过**，
   *      而 svelte-check / eslint 都不报「自定义属性未定义」。
   *      **「名字看起来合理」不等于它存在**，这一类缺口门禁抓不到。
   *   ② 中间试过「12% / 20% 墨洗同底不同深浅」当统一解 —— **也是错的**，
   *      形状仍然是灰底小块。见上面 `.cs-badge` 那段。 */
  .cs-badge.miss {
    color: var(--cs-pencil);
    font-weight: 400;
  }

  /* ⚠⚠ 危险动作**推到行尾**（2026-10-18）。
   *
   *   实测过：同一行里三颗按钮的几何**完全一致**（撕边、高 46、圆角 5、
   *   间距 5.6、基线齐），宽 88.4 / 76.5 / 65 只是文字长短。
   *   唯一差别是「删除」穿红——而它被两个中性按钮**夹在中间**，
   *   那一抹红就落在三颗的正中央，读成「这三颗里有一颗特别」。
   *
   *   红**不能撤**：这套色板里红只给不可逆，而删除是唯一不可逆的那个
   *   （硬约束第 4 条）。但「用不用红」和「站在哪」是两件事。
   *   ⇒ `margin-left: auto` 把它推到行尾，于是这一行读成
   *     **两个维护动作 + 一个拿走东西的动作**，而不是三颗同级按钮。
   */
  .cs-actions .cs-btn-mini.danger {
    margin-left: auto;
  }

  .cs-actions,
  .cs-form {
    display: flex;
    gap: 0.35rem;
    margin-top: 0.4rem;
  }

  /* ══ 输入槽：**内陷 + 弧边**（2026-10-16 第十轮定） ══
   *
   * ## 形态的由来（三轮才对上，过程记在票 45 第八~十轮）
   *
   * ① 用户：「设置页居然还是原生的边框、白底」⇒ 上一轮把 1px 描边
   *    从冷色调到 3.04 : 1。**这一轮错了整个方向** ——
   *    用户追问「**这个边框更淡有什么用？该有吗？**」：1.14:1 太淡认不出，
   *    3.04:1 也仍是个 Web 输入框，**浓淡根本不是那个变量**。
   * ② 用户：「按钮是纸片、候选项是纸片，那么输入框…**不应该是类似从纸面
   *    抠出来的内陷的区域**」⇒ 撕边**语义**就错：撕边说「可以拿起来」，
   *    而输入槽不是可拿起的零件，它是**你往里写字的地方**。
   * ③ 用户：「就是要内陷啊？**但也不应边框这么周正吧？**」
   *    （⚠ 我上一轮把「不应该是」读反了，白做四版候选才发现。）
   *
   * ## 形态
   *
   * - **内陷**：`box-shadow: inset` 四段，**上/左暗、下/右亮** ——
   *   真实压痕的物理：凹进去那侧受光少，凸起挡光那侧反光。
   * - **弧边**：`border-radius: var(--cs-radius)`（8px，裁出来的圆角纸片；
   *   不是撕的锯齿、也不是直角）。⚠ **无 `border`** ——
   *   内陷的东西不该有「外」的边，那道边正是它读成 Web 控件的原因。
   *
   * ## 颜色：全部从**暖墨**与**纸**混出来
   *
   * ⚠ 亮边**必须与纸混，不能 `#fff … transparent`**：
   *   纯白半透明落暖黄纸必然显冷（2026-10-16 实测那版出来一圈灰蓝，
   *   **破了同源**）。真实压痕的亮边是「**纸被照亮**」，不是「加了一层白」。
   *
   * ⚠⚠ **天花板，说在前面**：`box-shadow: inset` **永远是矩形渐变**
   *   ⇒ 内陷那条边仍然周正。真正做到「内陷也不周正」要一张
   *   **「撕边洞」遮罩**（`stripgen.py` 派生），见票 45 第十轮。
   *   2026-10-16 用户拍板：**先接受这一版**。 */
  .cs-input {
    flex: 1 1 auto;
    min-width: 0;
    /* ⚠⚠ **高度与同屏按钮对齐**（2026-10-18）。
     *   改之前这一屏实测：输入框 **24px** 高，右边那颗「保存」**62px** 高
     *   ——输入框贴在两颗撕边按钮中间，读成「一个洞旁边立着两块纸」。
     *   现在与两族按钮同高（内容盒 26 + 撕边 10×2 = **46**），
     *   长宽比 205 : 46 = **1 : 4.5**，出图确认不再是空横板。
     *   ⚠ 这一格此前写的是「内容盒都是 30px，加上 16px 撕边 ⇒ 三者逐项相等」，
     *     同样推不出来：border-box 下 `min-height` **含**撕边，
     *     产不出 30px 的内容盒，而实测三档是 62 / 58.5 / 55.5。
     *     `slice 32` / `width 10px` 必须与两族按钮**逐字绑定**，
     *     别单独改一个。 */
    box-sizing: border-box;
    /* ⚠ 同 `.cs-btn`：`box-sizing: border-box` 下 `min-height` 量的是**总高**。
     *   写 26 时它是空转的（光撕边就有 20px），所以输入框的真实高度一直是
     *   `line-height 26 + 撕边 20` = 46；现在把 46 写出来，是为了让
     *   「同屏等高」由**这一条声明**保证，而不是由两处数值恰好相同。 */
    min-height: 46px;
    /* ⚠ 左内缩比按钮多 6px：撕边吃掉一圈，文字要回到原来那个视觉位置。
     * 垂直内缩是 0 —— 靠 `line-height` 居中，padding 会把单行输入框撑歪。 */
    padding: 0 12px;
    font-size: 11.5px;
    line-height: 26px;
    font-family: ui-monospace, 'Cascadia Mono', monospace;
    color: var(--cs-ink);
    background: var(--cs-paper);
    /* ⚠⚠ **从「无边框」改成撕边纸**（2026-10-18，用户原话：
     *   「输入框意义不明的要显示完整的边框」）。
     *
     *   原来这里是 `border: none` + 四条 inset 阴影，理由写在 app.css 里：
     *   「3:1 暖色描边不会读成 Web 输入框」。**那是对「像 Web 输入框」的
     *   防御，不是对「读不出来」的防御**——实测真机上它读成的是一个**洞**：
     *   没有边框的矩形在纸上看不出是控件，用户不知道那里能点、能写。
     *
     *   ⇒ 改成与两族按钮**同一套撕边**（同 `strip.png`、同 slice / width），
     *     于是输入框与它旁边的按钮是同一种东西的两片，不是一个洞配一块纸。
     *     边框「完整」是这一版的硬要求；辨识度靠**材质同族**，
     *     不靠「压淡到看不见」。
     *
     *   底色**仍然完全不透明**（见 app.css）：透明窗口分层合成会整层丢掉半透明底。 */
    border: 10px solid transparent;
    border-image: url('/strip.png') 32 fill / 10px / 0 stretch;
    appearance: none; /* 清 UA 阴影/背景图与默认外观 */
    /* 影子随 alpha 走（同两族按钮）：`box-shadow` 画边框盒，会从撕口的透明区
     * 漏出矩形直边。`dy ≥ blur/2` ⇒ 高斯核不越进元素内侧，顶部不浮黑边。 */
    filter: drop-shadow(0 2px 4px rgb(24 18 10 / 0.16));
    transition:
      filter var(--cs-dur-fast) var(--cs-ease),
      border-color var(--cs-dur-fast) var(--cs-ease);
  }

  .cs-input::placeholder {
    color: var(--cs-pencil-2);
    /* ⚠ placeholder 用**与正文同一支**无衬线，不用输入框自己的 monospace：
     *   等宽字体里「粘贴 API key」会按等宽格子排，宽度比实际字面多一大截，
     *   读起来像一段被拉长的代码，而不是一个提示。
     *   （这与 `.cs-draft-text` 的判断同族：**该像正文的地方别像代码**。） */
    font-family: 'Microsoft YaHei UI', 'PingFang SC', 'Segoe UI', system-ui, sans-serif;
  }

  .cs-input:hover:not(:disabled) {
    filter: drop-shadow(0 4px 8px rgb(24 18 10 / 0.22));
  }

  .cs-input:focus {
    outline: none;
    /* 聚焦态**只抬亮**，不换形：撕边纸条换了形就掉回 Web 输入框那一套了。 */
    filter: drop-shadow(0 5px 10px rgb(24 18 10 / 0.28));
  }

  /* ── 危险开关下面那一句红。**只有这一句是红的。** ─────────────
   *
   * ⚠ 2026-10-18：`--cs-live` → `--cs-live-dim`，**只为了让对比度过 AA**。
   *   `--cs-live` 在暖黄纸上 4.28 : 1，11.5px/600 够不上「大字」豁免
   *   （要 ≥18.66px 粗体或 ≥24px 常规），差 0.22。
   *   `--cs-live-dim` 是**同一支红更深一档**，实测 **5.15 : 1** ✅，
   *   语义不变（还是「这句红只给不可逆 / 出事了」），不用改文案也不用改版式。
   *   ⚠ 别拿它去填大色块——它是给**纸上小字**用的那一档。 */
  .cs-warn-on {
    margin: 0.25rem 0 0;
    font-size: 11.5px;
    font-weight: 600;
    line-height: 1.5;
    color: var(--cs-live-dim);
  }
</style>
