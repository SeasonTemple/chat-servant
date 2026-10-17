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
    /** 与另一组指向同一路？（决策组没单独配 key 时就是） */
    shared_with_other: boolean;
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
   * 厂牌 chip + 「换一把 / 测连通 / 删除」，未配置组直接给输入框 + `primary` 保存。
   * 不含真实 key、不发任何 IPC。
   */
  const DEMO_GROUPS = [
    {
      name: '主模型',
      configured: true,
      providers: ['anthropic', 'openai'],
      hint: '存进 Windows 凭据管理器，面板里读不到明文。',
    },
    {
      name: '备用模型',
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
<div class="cs-set">
  <!--
    一次就说清最要紧的那件事：**改完不用重启**。
    早先是挂在「发送档位」下面那一行，只有走过那一节的人才看得到，
    而「图片」「上下文」两节的改动同样立刻生效。
  -->
  <!-- ══ 发送档位 ══ -->
  <h2 class="cs-h2">发送档位</h2>

  <!--
    「改完不用重启」这句原先是**屏顶独占一行**：左边没有控件、右边没有分节，
    它不属于任何东西，读起来像一行飘着的脚注。
    用户 2026-10-16 原话：「为何说明要单独一行？」

    ⇒ 挪到**第一节标题之下**，并给左边线（与 `TierSwitch` 的后果说明同一支），
      让它明确挂在「发送档位」这一节下面。
      档位是安全相关、也最需要先知道「改了立刻生效」的那一项，
      所以放这里不是就近，是**该被看见的位置**。
  -->
  <p class="cs-lead">全部改完立刻生效，不用重启面板。</p>

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
      滑块放在**第一屏**。2026-10-16 改完「灰轨 + 墨点」之后它落在
      两组模型配置下面，出图只到「示例：数值滑块」那个标签就被纸底切掉 ——
      **看不见等于没改**。这正是本节开头写的那条教训（新增控件形态必须
      同时加进 demo 夹具，并且要放在纸够得着的位置）的第二次犯。
    -->
    <div>
      <span class="cs-label">示例：数值滑块</span>
      <div class="cs-slider">
        <input type="range" min="1" max="40" value="12" aria-label="示例数值" />
        <output>12 条</output>
      </div>
    </div>
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
          {#each g.providers as p (p)}
            <button class="cs-btn cs-btn-mini primary">{p}</button>
          {/each}
        </div>
        {#if g.configured}
          <div class="cs-actions">
            <button class="cs-btn cs-btn-mini">换一把</button>
            <button class="cs-btn cs-btn-mini">测连通</button>
            <button class="cs-btn cs-btn-mini danger">删除</button>
          </div>
        {:else}
          <div class="cs-form">
            <input
              class="cs-input"
              type="password"
              autocomplete="off"
              spellcheck="false"
              placeholder="粘贴 key"
            />
            <button class="cs-btn cs-btn-mini primary">保存</button>
          </div>
        {/if}
      </section>
    {/each}
  {/if}
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
  <p class="cs-label">按窗口控制键时</p>
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
    label="关闭窗口的行为"
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
      oninput={(e) => (recentN = Number(e.currentTarget.value))}
      onchange={commitRecent}
    />
    <output>{recentN} 条</output>
  </div>
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
  <p class="cs-warn-on">画面切分只在 1 张真实样本上验证过 —— 认错框，屏幕就出去了</p>
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
    <p class="cs-fail" role="alert">{fail}</p>
    <!--
      ⚠ **2026-10-18 加：失败后的重试入口。**

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

        {#if g.shared_with_other}
          <!-- ⚠ 必须**明说**。决策组没单独配 key 时它与回复组是同一格，
               而用户改这一节的端点会连带改掉另一组——不说就是坑人。 -->
          <p class="cs-note cs-warn">这一组还没有自己的 key，当前与「回复模型」是同一路。</p>
        {/if}

        <!-- 厂商。**九项不是分段控件能装的**（380px 面板里一颗滑块放不下），
             所以用一组同族的小按钮换行排列——仍是这一屏已有的那支笔。 -->
        <p class="cs-label">走哪一家</p>
        <div class="cs-chips">
          {#each g.providers as p (p.id)}
            <button
              class="cs-btn cs-btn-mini"
              class:primary={p.id === g.route.provider}
              onclick={() => applyRoute(g, p.id)}
              disabled={busy[g.id]}
            >
              {p.label}
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
          <p class="cs-note cs-warn">这一家的端点与模型名抄自官方文档，未在本机实测。</p>
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
                换一把
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
                 明文 key 摆在界面上」而设，而 `REVEAL` 只有「换一把」
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
    <p class="cs-note">两组模型与「测连通」都走它。账号密码那部分在日志里会被抹掉。</p>
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

    /* ⚠⚠ **`margin-bottom: 28px`**（2026-10-16，与主面板档位行同一个病根）。
     *
     *   `.cs-sheet` 的内容盒底是 540，而**纸底在 512**
     *   （`probe-sheet.svg`：画布 560，纸 `height=512` 且 `translate(16,0)`，
     *    下面 48px 是 `sheetDrop` 的落影余量）。
     *   ⇒ 这一列原来有 **28px 画在纸外的透明区上**。
     *   在 Tauri 透明窗口里那不是「空白」，是**字浮在桌面上**。
     *
     *   出图证据：`set-after3` 那张，滚到底时最后一行
     *   「演示模式没有后台，这里不列模型配置。」整行落在纸边之下。
     *
     *   ⚠ 这条与主面板「档位名骑在撕口上」是**同一个错误**：
     *     两处都把「元素底 560」当成了「纸底」。
     *   ⇒ `budget.html` 的 `set_overflow` 现在也报 `bottom` 与 `paperBottom`。
     */
    margin-bottom: 28px;

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

  /* 「改完不用重启」这句。左边线是它**依附于某一节**的标记——
   * 原先它在屏顶独占一行，两边都没有东西，读成一节浮着的脚注。
   * 与 `TierSwitch` 的 `.cs-hint` 用同一支左边线：凡是解释「这一节会发生什么」
   * 的句子都长这样，不解释的就别加线。 */
  .cs-lead {
    margin: 0.1rem 0 0.15rem;
    font-size: 11.5px;
    color: var(--cs-pencil);
    padding-left: 0.55rem;
    border-left: 2px solid var(--cs-rule);
    overflow-wrap: anywhere;
  }

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
   *     12.8px  分组标题
   *     12.5px  小节标题 / 字段标签 / 供应商名 / 「打开」按钮
   *     12px    数值读数（滑块右侧的 30s / 90s）
   *     11.5px  说明正文 / 指引 / 错误 / 小按钮 / 输入框
   *     10.4px  徽标（badge）——唯一一档，因为它标的是**状态**不是句子
   *
   * 行高统一 **1.5**：原先说明类三处各写一个（1.45 / 1.5 / 1.55），
   * 而 14 个选择器根本没写、吃的是浏览器默认 `normal`——
   * `normal` 随字体不同而不同，等于**每个选择器的行高都不确定**。
   *
   * 字重只剩 **400 / 600**：700 原本只有 `.cs-group-name` 一处，
   * 是同屏唯一的孤例，已并入 600。 */
  .cs-h2 {
    margin: 0.85rem 0 0;
    font-size: 12.8px;
    font-weight: 600;
    color: var(--cs-ink);
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

  .cs-fail {
    margin: 0;
    padding: 0.4rem 0.5rem;
    font-size: 11.5px;
    line-height: 1.5;
    border-radius: var(--cs-radius-sm);
    /* 失败方向偏保守的一段话，用极淡的红底托一下。 */
    background: color-mix(in srgb, var(--cs-live) 12%, transparent);
    overflow-wrap: anywhere;
  }

  /* 预算无解时的提示。用警告色而不是正文色——这不是「已保存」，
     是「你调的这个值不起作用」。 */
  .cs-warn {
    color: var(--cs-live);
  }

  /* ── 按钮 ─────────────────────────────────────────────────── */
  .cs-btn {
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
    margin: -8px 0;
    padding: 7px 18px;
    font-size: 12.5px;
    color: var(--cs-ink);
    /* 抬起 / 压回是 transform，得自己有 transition；
     * `.cs-btn` 里那三条（background / color / opacity）在撕边之后已经
     * 没有可过渡的对象了 —— 纸条不变色、不换底。 */
    transition:
      transform var(--cs-dur-fast) var(--cs-ease),
      filter var(--cs-dur-fast) var(--cs-ease);
    /* ⚠ 与 `.cs-act` 逐项一致：同一张 `strip.png`、同一套 slice / width。
     *   写 16px 是因为撕口的纸芯最早只在内缩 ~5.5 CSS px 处（`stripgen.py`
     *   实测最浅一档），再薄就会啃进纸。**这一对是一套，别单独改一个。** */
    border: 16px solid transparent;
    border-image: url('/strip.png') 32 fill / 16px / 0 stretch;
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
   * `16px` 与 `32` 都与 `.cs-btn-ink` 逐字绑定，别单独改一个。 */
  .cs-btn-mini {
    padding: 6px 11px;
    font-size: 11.5px;
    white-space: nowrap;
    border: 16px solid transparent;
    border-image: url('/strip.png') 32 fill / 16px / 0 stretch;
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
   * `border-color` 就不再被画出来（`.cs-btn-ink` 的禁用态注释记着同一件事）。 */
  .cs-btn-mini.primary {
    color: var(--cs-live);
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

  /* 分组标题。这一屏的标题档是 **12.8px / 600**，与 `.cs-h2` 同值；
   * 原来它独自写 `0.8rem`（12.8px）却配 **700** —— 全屏唯一的第三档字重，
   * 一个标题比同屏其它所有 600 的东西都重，读起来像另一个层级的东西。 */
  .cs-group-name {
    font-size: 12.8px;
    font-weight: 600;
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

  /* 已配置 = 常态。**不给红也不给绿** —— 绿在这个色板里是「对方」的颜色，
   * 用在这里等于把说话人的身份色借去表示状态。 */
  .cs-badge.ok {
    color: var(--cs-ink-2);
    background: rgb(20 26 31 / 7%);
  }

  .cs-badge.ok.env {
    color: var(--cs-pencil);
    background: rgb(20 26 31 / 5%);
  }

  /* 未配置：给铅笔灰，不给红。
   * docs/面板设计.md 的不变量是「红只给不可逆」——看到红就是有东西出去过，
   * 或者要出去。缺 key 时一个字都还没出去，也不构成一次不可逆动作，
   * 穿红字会把唯一的语义色稀释掉。真要走不可逆语义，红留给发送与退出。
   * 靠**底色**和已配置区分，不加边框：底类 .cs-badge 没有边框，
   * 单给这一支加 1px 会让它比 .ok 高 2px。
   *
   * ⚠ 2026-10-16 修一个真 bug：这里原来写的是 `var(--cs-line)`，
   *   而**`--cs-line` 在全仓从未定义过**。刻线那支叫 `--cs-rule`；
   *   而 `.cs-line`（带点）**也不是**正文那个 class —— 2026-10-16 实测
   *   它同样全仓零命中，正文那条是 `.cs-draft-text`。
   *   ⚠ **两处「听起来合理」的引用都指向不存在的东西**，
   *     所以这个 bug 能在样式表里躺着过好几轮：看起来像引用了一个
   *     真实存在的 token，而 svelte-check / eslint 都不报
   *     「自定义属性未定义」——**这一类缺口门禁抓不到**。
   *   引用一个不存在的自定义属性 ⇒ 该值在**计算时** invalid
   *   ⇒ **整条 `box-shadow` 声明作废**
   *   ⇒ 「未配置」徽标那圈描边**从来没渲染出来过**，它一直是光秃秃一个铅笔灰字。
   *   顺带这处也是漏接管的同类：它在 HC 那段里没有任何对应。
   *   ⇒ 改用 `--cs-edge`（功能性边界，3 : 1，与纸同源）。
   *   判据：`ui/scripts/verify-edge.mjs` 问渲染后的 `box-shadow`
   *   是不是 `none`（改回去验过：确实是 `none`）。 */
  .cs-badge.miss {
    color: var(--cs-pencil);
    background: transparent;
    box-shadow: inset 0 0 0 1px var(--cs-edge);
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
    padding: 0.35rem 0.45rem;
    font-size: 11.5px;
    font-family: ui-monospace, 'Cascadia Mono', monospace;
    color: var(--cs-ink);
    /* 底色**完全不透明**（见 app.css）：透明窗口分层合成会整层丢掉半透明底。
     * 槽底 = **纸本身**。⚠ 试过 `--cs-paper-raised`（比纸亮）又退回：
     *   比纸亮读成「白板」，而纸被压实应是**略深**。深度交给内阴影，底色不参与。 */
    background: var(--cs-paper);
    border: none;
    /* 弧边。⚠ 原是硬编码 `7px`，票 42 归到 `--cs-radius-sm`(5px)，理由
     *   「与同屏 `.cs-btn` 同族」——**那个理由已作废**：同屏按钮全换成撕边
     *   纸条，`border-image` 一生效 `border-radius` 根本不画。
     *   ⇒ 现在独立取 `--cs-radius`(8px)，按「弧边」这条自己的理由。 */
    border-radius: var(--cs-radius);
    appearance: none; /* 清 UA 阴影/背景图与默认外观 */
    box-shadow:
      inset 0 2px 4px color-mix(in srgb, var(--cs-tier-ink) 22%, transparent),
      inset 2px 0 3px color-mix(in srgb, var(--cs-tier-ink) 11%, transparent),
      inset 0 -2px 3px color-mix(in srgb, #fff 42%, var(--cs-paper)),
      inset -2px 0 2px color-mix(in srgb, #fff 30%, var(--cs-paper));
  }

  .cs-input:focus {
    outline: none;
    box-shadow:
      inset 0 2px 5px color-mix(in srgb, var(--cs-tier-ink) 30%, transparent),
      inset 2px 0 3px color-mix(in srgb, var(--cs-tier-ink) 15%, transparent),
      inset 0 -2px 4px color-mix(in srgb, #fff 52%, var(--cs-paper)),
      inset -2px 0 3px color-mix(in srgb, #fff 36%, var(--cs-paper));
  }

  /* ── 危险开关下面那一句红。**只有这一句是红的。** ───────────── */
  .cs-warn-on {
    margin: 0.25rem 0 0;
    font-size: 11.5px;
    font-weight: 600;
    line-height: 1.5;
    color: var(--cs-live);
  }
</style>
