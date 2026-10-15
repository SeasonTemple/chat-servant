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

  ## 材质：配色跟着面板走，但**整张纸会被换掉**

  ⚠ 独立设置窗口用的是 `--cs-surface`（中性白）。搬进来之后**不能跟着换**：
  在同一个窗口里铺一块白底会留下一道接缝——那正是这个项目一直在删的东西。
  底色改走 `--cs-paper`（#EDF0F1，中性），字条用 `--cs-paper-raised`。

  ⚠ 但也别读成「设置页还是那张撕口纸」：父级
  `.cs-sheet[data-settings='true']` 会把纸纹**整张撤掉**，
  这是设计规范 §1「纸面是配置界面唯一的敌人」定的，是有意的材质切换。

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

  interface ProviderView {
    id: string;
    configured: boolean;
    from_env: boolean;
    masked: string;
    hint: string;
    /** 这家实测能通的模型清单。**Rust 是唯一出处**，前端不自己列。 */
    models: string[];
    /** 当前选中的那一个（Rust 回的真实生效值）。 */
    model: string;
  }

  interface SettingsView {
    providers: ProviderView[];
  }

  /**
   * 「用哪个模型」那一栏。
   *
   * ⚠ `models` 是一份**只填实测能通的**清单，由 `Provider::models()` 给。
   * 它现在很短（一家一个），**这不是没做完**——填一个没验过的名字，
   * 症状是「设置成功、一调用就 404」，比明确不支持更难查。
   * 清单该变长的条件是「实测通了」，不是「文档上写了」。
   */
  interface ModelView {
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
  /** 「用哪个模型」。读不到时整块不显示——别摆一个选了也不生效的控件。 */
  let model = $state<ModelView | null>(null);
  let modelNote = $state<string | null>(null);

  /**
   * 换模型。
   *
   * ⚠ **用回包覆盖**：Rust 会拒收不在实测清单里的名字，而界面必须显示
   * 拒收之后的真实状态。只显示用户点的那一格，就成了一个说谎的控件。
   */
  async function pickModel(m: string) {
    modelNote = null;
    try {
      model = await invoke<ModelView>('set_model', { model: m });
    } catch (e) {
      // 写失败就**读回真实值**，别让控件停在一个程序并不认同的状态上。
      model = await invoke<ModelView>('get_model').catch(() => null);
      modelNote = String(e);
    }
  }

  /** 分段控件的选项。Rust 给什么就是什么，前端不排序也不改名。 */
  const modelOptions = $derived((model?.models ?? []).map((id) => ({ id, label: id })));

  /**
   * 「展开输入框」的哨兵值。
   *
   * 早先这里塞的是一个**裸 NUL 字符**（`'\0show'` 的字面 NUL），
   * 用不可能出现在 key 里这一点当哨兵。它能跑，但代价很重：
   * git 因此把整个 `.svelte` 判成**二进制**，从此这个文件的 diff
   * 全失效，review 只能靠肉眼。用具名常量，哨兵的意图也看得见了。
   */
  const REVEAL = '__reveal__';
  /** 每个供应商的输入框内容。**只存在于组件内存里**，提交后立刻清空。 */
  let drafts = $state<Record<string, string>>({});
  let busy = $state<Record<string, boolean>>({});
  let notes = $state<Record<string, string>>({});
  let fail = $state<string | null>(null);

  async function refresh() {
    try {
      view = await invoke<SettingsView>('get_settings');
    } catch (e) {
      // 这里的 e 是 Rust 侧已经**脱敏过**的文本，可以直接显示。
      fail = String(e);
    }
  }

  async function save(p: ProviderView) {
    busy[p.id] = true;
    notes[p.id] = '';
    try {
      const wrote = await invoke<boolean>('save_setting', {
        provider: p.id,
        key: drafts[p.id] || null,
      });
      notes[p.id] = wrote
        ? '已存进 Windows 凭据管理器。**下一次生成立即生效，不用重启。**'
        : '留空 = 保留原值，没有改动。';
      // 提交后清掉输入框：明文不该在内存里多待一刻
      drafts[p.id] = '';
      await refresh();
    } catch (e) {
      notes[p.id] = String(e);
    } finally {
      busy[p.id] = false;
    }
  }

  async function forget(p: ProviderView) {
    busy[p.id] = true;
    try {
      await invoke('forget_key', { provider: p.id });
      notes[p.id] = '已从凭据管理器删除。';
      drafts[p.id] = '';
      await refresh();
    } catch (e) {
      notes[p.id] = String(e);
    } finally {
      busy[p.id] = false;
    }
  }

  /**
   * 测连通性。
   *
   * 失败**必须显示**。早先的 `probe_key` 恒返回 false、前端又没有按钮，
   * 等于这个功能从来没存在过；现在它真的会发请求了，失败信息就是用户
   * 唯一能判断「是 key 不对还是网络不通」的依据。吞掉它等于白做。
   */
  async function probe(p: ProviderView) {
    busy[p.id] = true;
    notes[p.id] = '';
    try {
      await invoke<boolean>('probe_key', { provider: p.id });
      notes[p.id] = '连上了。走的是只读的 /v1/models，没有产生任何 token 费用。';
    } catch (e) {
      notes[p.id] = `连不上：${String(e)}`;
    } finally {
      busy[p.id] = false;
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
      try {
        // 模型要**读回来**：界面显示的必须是程序认定的那个。
        // 没接上真实传输时 Rust 会返回错误——那时候整块不显示，
        // 不用一个假的默认模型把这一节撑起来。
        model = await invoke<ModelView>('get_model');
      } catch {
        model = null;
      }
      await refresh();
    })();
  });
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
  <p class="cs-lead">全部改完立刻生效，不用重启面板。</p>

  <!-- ══ 发送档位 ══ -->
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
  -->
  <button class="cs-btn cs-btn-ink cs-onoff" onclick={() => toggleImagesOut(!imagesOut)}>
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
    ⚠ **这一节只在真的接上了传输时才出现。**
    没 key 时 Rust 不会造一个模型格子出来，前端也就读不到——
    摆一个「可选模型」但选了不生效，比不摆更糟：那正是这个项目
    一直在删的「调了没反应的控制项」。

    ⚠ **清单只有一条时不给分段控件。** 一颗滑块的控件只有一个选项，
    它回答不了任何问题，看着却像个选择器。此时只把当前值显示出来。
  -->
  {#if model}
    <h2 class="cs-h2">模型</h2>
    <!--
      ⚠ 2026-10-15 把「判断和起草用同一个模型——判断那段的输出只有几十个字」
      从下面那条独立说明**合并进名称**。
      理由：它是「为什么只有一个模型」的**理由**，不是另一个设置项；
      而它原来独占一行 `cs-note`，与真正需要独立成句的说明（隐私、风险）
      长得一模一样 ⇒ 一屏里全是同样重量的句子，就等于都没有重量。
    -->
    <p class="cs-label" title="判断与起草用同一个模型——判断那段的输出只有几十个字">
      {model.provider} 当前用的模型 · 判断与起草同一个
    </p>
    {#if modelOptions.length > 1}
      <SegSlider
        value={model.model}
        options={modelOptions}
        label="模型"
        onchange={(v) => pickModel(v)}
      />
    {:else}
      <p class="cs-note">{model.model}</p>
    {/if}
    {#if modelOptions.length < 2}
      <!--
        这一支只在「没有第二个可选项」时出现，而上面已经把模型名显示出来了。
        剩下要说的是**为什么不给你自由输入**——那是理由，不是当前状态，
        所以移进 `title`：需要的人悬停能看到，不占那一屏的版面。
      -->
      <p
        class="cs-label"
        title="这里只列实测能通的模型。填一个没验过的名字，表现会是「设置成功、一调用就 404」。"
      >
        只列实测能通的模型
      </p>
    {/if}
    {#if modelNote}
      <p class="cs-fail" role="alert">{modelNote}</p>
    {/if}
  {/if}

  <!-- ══ 模型 key ══ -->
  <h2 class="cs-h2">模型 key</h2>
  <!--
    ⚠ 2026-10-15 删掉了「key 存在 Windows 凭据管理器里，不落文件、不进日志。」
    它与下面每个供应商的 `missing_hint` **逐字重复**——而 hint 出现三次，
    也就是说这句承诺在一屏里本来就被说了四遍。
    隐私承诺跟着「未配置」那个状态出现，正好是它要生效的那一刻。
  -->

  {#if fail}
    <p class="cs-fail" role="alert">{fail}</p>
  {/if}

  {#each view?.providers ?? [] as p (p.id)}
    <section class="cs-prov">
      <div class="cs-prov-head">
        <span class="cs-prov-name">{p.id}</span>
        {#if p.configured}
          <span class="cs-badge ok" class:env={p.from_env}>
            {p.masked}{p.from_env ? '（来自环境变量）' : ''}
          </span>
        {:else}
          <span class="cs-badge miss">未配置</span>
        {/if}
      </div>

      {#if p.configured}
        <!-- 已配置时**不显示输入框**。要改才点「换一把」——
             这样明文 key 不会因为打开设置就躺在界面上。 -->
        <div class="cs-actions">
          <button
            class="cs-btn cs-btn-mini"
            onclick={() => (drafts[p.id] = REVEAL)}
            disabled={busy[p.id]}
          >
            换一把
          </button>
          <!-- 连通性测试。不调模型：走 /v1/models，只读、不花钱。 -->
          <button class="cs-btn cs-btn-mini" onclick={() => probe(p)} disabled={busy[p.id]}>
            {busy[p.id] ? '测试中…' : '测连通'}
          </button>
          <button class="cs-btn cs-btn-mini danger" onclick={() => forget(p)} disabled={busy[p.id]}>
            删除
          </button>
        </div>

        {#if drafts[p.id] === REVEAL}
          <div class="cs-form">
            <input
              class="cs-input"
              type="password"
              autocomplete="off"
              spellcheck="false"
              placeholder="粘贴 key"
              bind:value={drafts[p.id]}
            />
            <button
              class="cs-btn cs-btn-mini primary"
              onclick={() => save(p)}
              disabled={busy[p.id]}
            >
              {busy[p.id] ? '写入中…' : '保存'}
            </button>
          </div>
        {/if}
      {:else}
        <!-- p.hint 是 Rust 给的**可执行**指引：存哪、变量名叫什么。
             不能是空字符串或「暂无」。 -->
        <p class="cs-hint">{p.hint}</p>

        <!-- ⚠ **未配置时输入框必须直接显示**，不能套 `drafts[p.id] === REVEAL`
             那道门（2026-10-15 修的真机死路，见 `.scratch/…/40-*.md`）。

             那道门本来只为「已配置时不把明文 key 摆在界面上」而设，
             而 `REVEAL` 只有「换一把」按钮会去设——**而那个按钮在
             `{#if p.configured}` 分支里**。⇒ 未配置时 `drafts[p.id]` 永远是
             `undefined`，输入框永远出不来。

             后果不是「少个控件」：提示写着「在这个页面填」，而这一页**根本没有
             任何办法填**，全新安装只能去设环境变量。这条路是产品的第一步。

             未配置时界面上没有任何 key，**没有需要藏的东西**，所以直接显示。 -->
        <div class="cs-form">
          <input
            class="cs-input"
            type="password"
            autocomplete="off"
            spellcheck="false"
            placeholder="粘贴 key"
            bind:value={drafts[p.id]}
          />
          <button class="cs-btn cs-btn-mini primary" onclick={() => save(p)} disabled={busy[p.id]}>
            {busy[p.id] ? '写入中…' : '保存'}
          </button>
        </div>
      {/if}

      {#if notes[p.id]}
        <p class="cs-note">{notes[p.id]}</p>
      {/if}
    </section>
  {:else}
    <p class="cs-note">{DEMO ? '演示模式没有后端，这里不列供应商。' : '正在读设置…'}</p>
  {/each}
</div>

<style>
  /* ══════════════════════════════════════════════════════════════
     这一屏是**同一块面板的另一种显示**，配色仍走纸面板那套 token。

     ⚠ 但底色**不是那张撕口纸**：`.cs-sheet[data-settings='true']`
     在父级就把纸整张换成了 `--cs-paper`（#EDF0F1，中性）。
     那是设计规范 §1「纸面是配置界面唯一的敌人」定的，理由与
     「为什么换材质就是一句提示」写在 `PaperPanel.svelte` 那条注释里。

     ⚠ 真正被禁掉的是**独立设置窗口时代的 `--cs-surface`**（中性白，
       #FAFAF9）：在同一窗口里铺一块白底会留下一道接缝。字条用
       `--cs-paper-raised`——材质词汇与主面板一致。
     ══════════════════════════════════════════════════════════════ */

  .cs-set {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    padding: 0 14px 16px;

    /* ⚠ 滚动条是这一屏**最后一块 Web 默认外观**（票 42，2026-10-15）。
     *   系统那条滚动条会带自己的立体凹槽与箭头，在纸面上就是一块贴上去的
     *   异物 —— 与其余控件「1px 墨描边」的语言完全不搭。
     *
     *   两套属性都要写，缺一不可：
     *     · `scrollbar-width` / `scrollbar-color` 是**标准**属性，Firefox 走它；
     *     · `::-webkit-scrollbar` 是 Chromium 的。WebView2 是 Chromium，
     *       所以真正生效的是下面那段 `::-webkit-scrollbar`。
     *   ⚠ 只写标准属性在 WebView2 里**不生效**（非 `auto` 的 `scrollbar-width`
     *   在 Blink 里被忽略），反过来只写 `::-webkit` 则在 Firefox 里失效。
     *
     *   底槽透明：槽画出来会在纸上留一条比 `--cs-rule` 更重的痕，
     *   而这一屏本来就没有「装饰性横线」这个语言（分节全靠留白，见 `.cs-h2`）。 */
    scrollbar-width: thin;
    scrollbar-color: var(--cs-ink-2) transparent;
  }

  .cs-set::-webkit-scrollbar {
    width: 6px;
  }

  .cs-set::-webkit-scrollbar-track {
    background: transparent;
  }

  /* 滑块用**墨**，不用红、不用蓝（§5.4 同一条：状态色不兼职导航色）。
   * 常态压到铅笔档与墨之间，悬停才给足墨 —— 滚动条是「需要时才注意到」的东西。 */
  .cs-set::-webkit-scrollbar-thumb {
    background: color-mix(in srgb, var(--cs-ink-2) 42%, transparent);
    /* 纸是直角（app.css 的 --cs-radius 注释）。滚动条滑块不属于「字条」，
     * 也不属于「可点的按钮」，所以不给药丸圆角，给 1px 刚好收住边。 */
    border-radius: 1px;
  }

  .cs-set::-webkit-scrollbar-thumb:hover {
    background: var(--cs-ink-2);
  }

  .cs-lead {
    margin: 0 0 2px;
    font-size: 11.5px;
    color: var(--cs-pencil);
  }

  /* ── 分节标题 ────────────────────────────────────────────────
   *
   * 分组全靠**留白和字号**，不画分隔线 —— 和主面板同一条排版语言。
   * 上边距比一般间距大一倍，就是这一屏唯一的分组手段。 */
  .cs-h2 {
    margin: 0.85rem 0 0;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--cs-ink);
  }

  .cs-label {
    font-size: 0.78rem;
    font-weight: 600;
    color: var(--cs-ink);
  }

  /* ── 说明文字 ────────────────────────────────────────────────
   *
   * ⚠ 正文用**铅笔档**（6.22 : 1），不是标注档（3.81 : 1）。
   * 这些句子是要人读的，读不清等于没说。 */
  .cs-note {
    margin: 0;
    font-size: 0.7rem;
    /* 1.45 而不是 1.6（2026-10-10 实测反馈：说明喧宾夺主）。
       说明行距一大，它在那一屏里的**占位**就跟着变大——字号明明最小，
       视觉重量却是按行数×行距算的。1.6 在 11px 上是 18px 一行，
       一段三行的话就是 54px，比一个开关还高。 */
    line-height: 1.45;
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
    font-size: 0.68rem;
    line-height: 1.55;
    color: var(--cs-pencil);
    /* 指引可能有换行，要原样显示 */
    white-space: pre-line;
    overflow-wrap: anywhere;
  }

  .cs-fail {
    margin: 0;
    padding: 0.4rem 0.5rem;
    font-size: 0.7rem;
    line-height: 1.55;
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

  /* 有边框的那批（含 `.danger` 与 `.primary`）：只换线型，底色文字都不动。 */
  .cs-btn-mini:disabled {
    border-style: dashed;
  }

  /* ⚠ `.cs-btn-ink` 是本屏**唯一没有边框**的按钮（墨底药丸），
   *   所以 `border-style: dashed` 对它**完全不生效** —— 不能只写一条规则
   *   就以为全屏覆盖了。按 debug 页的先例改用「换底色」，
   *   底色与文字同步互换，两者都保持高对比。 */
  .cs-btn-ink:disabled {
    background: var(--cs-paper-sunken);
    color: var(--cs-ink);
  }

  /* 图片外发总闸。它是这一屏唯一的危险开关，也是**唯一一处实心**——
   * 主面板里「墨底」是当场的主动作，这里照用同一档墨，不引入新颜色。 */
  .cs-btn-ink {
    align-self: flex-start;
    padding: 7px 18px;
    font-size: 12.5px;
    background: var(--cs-ink);
    color: var(--cs-paper-raised);
  }

  .cs-btn-ink:hover:not(:disabled) {
    background: var(--cs-ink-2);
  }

  /* key 卡片里的三颗小按钮。**同高 30px**：它们是同一级控件，
   * 两种规格意味着用户点之前得先判断「这颗算不算主要控件」。 */
  .cs-btn-mini {
    padding: 6px 11px;
    font-size: 0.72rem;
    border: 1px solid var(--cs-rule-strong);
    background: transparent;
    white-space: nowrap;
  }

  .cs-btn-mini:hover:not(:disabled) {
    background: rgb(20 26 31 / 6%);
  }

  .cs-btn-mini.primary {
    border-color: color-mix(in srgb, var(--cs-live) 55%, transparent);
    color: var(--cs-live);
  }

  .cs-btn-mini.danger {
    border-color: color-mix(in srgb, var(--cs-live) 40%, transparent);
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

  /* 轨道 = 一道墨线，与按钮族的 1px 描边同一支笔。 */
  .cs-slider input[type='range']::-webkit-slider-runnable-track {
    height: 1px;
    background: var(--cs-ink);
  }

  /* 拇指 = 墨点。`margin-top` 把它压回轨道中线
   * （track 1px、thumb 14px ⇒ 上移 (14-1)/2 = 6.5，取 6px + 半像素视觉余量）。 */
  .cs-slider input[type='range']::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 14px;
    height: 14px;
    margin-top: -6px;
    border-radius: 50%;
    background: var(--cs-ink);
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
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--cs-ink);
    /* UA 默认把 output 渲染得像超链接，蓝色 + 下划线。去掉。 */
    text-decoration: none;
  }

  /* ── 供应商 ───────────────────────────────────────────────── */
  .cs-prov {
    padding: 0.5rem 0.6rem;
    border: 1px solid var(--cs-rule);
    border-radius: var(--cs-radius-sm);
    /* 字条落在纸上 —— 主面板里那张「递给你的条子」就是这一档。 */
    background: var(--cs-paper-raised);
  }

  .cs-prov + .cs-prov {
    margin-top: 0.35rem;
  }

  .cs-prov-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .cs-prov-name {
    font-size: 0.78rem;
    font-weight: 600;
  }

  .cs-badge {
    font-size: 0.65rem;
    padding: 0.1rem 0.4rem;
    border-radius: 999px;
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
   * 单给这一支加 1px 会让它比 .ok 高 2px。 */
  .cs-badge.miss {
    color: var(--cs-pencil);
    background: transparent;
    box-shadow: inset 0 0 0 1px var(--cs-line);
  }

  .cs-actions,
  .cs-form {
    display: flex;
    gap: 0.35rem;
    margin-top: 0.4rem;
  }

  .cs-input {
    flex: 1 1 auto;
    min-width: 0;
    padding: 0.35rem 0.45rem;
    font-size: 0.72rem;
    font-family: ui-monospace, 'Cascadia Mono', monospace;
    color: var(--cs-ink);
    /* 底色**完全不透明**（见 app.css 的注释）：透明窗口的分层合成会
     * 整层丢掉它，这里照面板的纸底走，不学独立窗口那套暗底。 */
    background: var(--cs-paper);
    border: 1px solid var(--cs-rule-strong);
    /* ⚠ 原来是硬编码的 `7px` —— 它**不对应任何一个 token**
     *   （`--cs-radius` 是 8px、`--cs-radius-sm` 是 5px），是 Web 默认圆角
     *   抄下来后随手改的数。同一屏的 `.cs-btn` 用的是 `--cs-radius-sm`，
     *   输入框与它的按钮差 2px，于是「一个框里一个输入框 + 三颗按钮」
     *   在同一行里显出两套曲率（票 42，2026-10-15）。
     *   ⇒ 归到同一支，输入框与它的按钮同族。 */
    border-radius: var(--cs-radius-sm);
    /* 浏览器给密码框可能挂 UA 的内阴影/背景图，一并清掉，
     * 免得在纸色底上出现一圈 Web 默认的灰边。 */
    appearance: none;
    box-shadow: none;
  }

  /* ⚠ `:focus` 已有墨色描边，而全局 `:focus-visible`（app.css）另有墨色环。
   *   这里**不**去覆盖 `:focus-visible` 的 outline：指针点击不该留环，
   *   键盘 Tab 才该留，两件事由 `:focus` / `:focus-visible` 自己分工。 */
  .cs-input:focus {
    outline: none;
    border-color: var(--cs-ink);
  }

  /* ── 危险开关下面那一句红。**只有这一句是红的。** ───────────── */
  .cs-warn-on {
    margin: 0.25rem 0 0;
    font-size: 0.72rem;
    font-weight: 600;
    line-height: 1.5;
    color: var(--cs-live);
  }
</style>
