<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import TierSwitch from '$lib/TierSwitch.svelte';
  import type { BudgetView, Tier } from '$lib/types';

  interface ProviderView {
    id: string;
    configured: boolean;
    from_env: boolean;
    masked: string;
    hint: string;
  }

  interface SettingsView {
    providers: ProviderView[];
  }

  /** 「参考上下文」设置。上下限也由 Rust 下发。 */
  interface RecentView {
    n: number;
    min: number;
    max: number;
    default: number;
  }

  let tier = $state<Tier>('Manual');
  /** 改档失败时的说明。空 = 没问题。 */
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
  /**
   * 改档。与主面板**共用 Rust 那一个格子**。
   * 界面上显示的是 Rust 回传的真实生效值，不是用户点过的那个。
   */
  /**
   * 「关闭时」的行为。与主面板**共用 Rust 那一个 setting**。
   *
   * ⚠ Rust 侧会验证：托盘不存在时**拒收**「收进托盘」—— 那样关掉面板
   * 它就藏起来了。所以这里把错误原样显示，不假装成功。
   */
  let closeBehavior = $state<'exit' | 'tray'>('exit');
  let closeNote = $state('');

  async function setClose(next: 'exit' | 'tray') {
    closeNote = '';
    try {
      await invoke('set_close_behavior', { behavior: next });
      closeBehavior = next;
      closeNote =
        next === 'tray'
          ? '按 − 收进托盘。托盘图标在自动发送档位下会带红点。'
          : '按 × 直接退出。程序不会在后台继续跑。';
    } catch (e) {
      // 写失败就**读回真实值** —— 别让开关停在一个程序并不认同的状态上。
      closeBehavior = await invoke<'exit' | 'tray'>('get_close_behavior').catch(() => 'exit');
      closeNote = String(e);
    }
  }

  async function changeTier(t: Tier) {
    try {
      tier = await invoke<Tier>('set_tier', { tier: t });
    } catch (e) {
      tierNote = String(e);
      try {
        tier = await invoke<Tier>('get_tier');
      } catch {
        tier = 'Manual';
      }
    }
  }

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

  async function close() {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      await getCurrentWindow().close();
    } catch {
      fail = '这个页面在浏览器里预览，关不掉窗口。';
    }
  }

  onMount(() => {
    void (async () => {
      try {
        apply(await invoke<RecentView>('get_recent_n'));
      } catch {
        // 读不到就用默认档，**不报错打断设置页**。
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
        // 档位也要**读回来**：两个页面各有一份本地 state，
        // 不读的话设置页显示的可能是用户上次在这页点过的那个，
        // 而程序实际认定的档位在别处。
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
    })();
  });

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
      // 只显示拖到的那格，界面就变成了一个说谎的控件。
      apply(v);
      recentNote = `已设为 ${v.n} 条，下一次生成就会用上`;
    } catch (e) {
      recentNote = String(e);
    }
  }
</script>

<main class="cs-settings">
  <header class="cs-head">
    <h1>设置</h1>
    <button class="cs-close" onclick={close} title="关闭（窗口保留，下次直接开）">×</button>
  </header>

  <section class="cs-row">
    <span class="cs-label">发送档位</span>
    <div><TierSwitch bind:tier onchange={changeTier} /></div>
  </section>
  <!--
    ⚠ **说明文字只写「怎么用」。** 「早先两边各存一份、开关不起作用」
    这类是给接手程序员看的提交历史，不该出现在界面上——它对用户没有
    可操作性，只是让人以为自己用错过什么。历史留在注释里。
  -->
  <p class="cs-note">改完立刻生效。会话结束时回落到「只填入」—— 程序自己不会升档。</p>
  {#if tierNote}
    <p class="cs-fail" role="alert">{tierNote}</p>
  {/if}

  <h2 class="cs-h2">窗口</h2>
  <section class="cs-row">
    <span class="cs-label">按窗口控制键时</span>
    <div class="cs-seg" role="radiogroup" aria-label="关闭窗口的行为">
      <button
        class="cs-seg-item"
        class:on={closeBehavior === 'exit'}
        role="radio"
        aria-checked={closeBehavior === 'exit'}
        onclick={() => setClose('exit')}
      >
        退出（默认）
      </button>
      <button
        class="cs-seg-item"
        class:on={closeBehavior === 'tray'}
        role="radio"
        aria-checked={closeBehavior === 'tray'}
        onclick={() => setClose('tray')}
      >
        收进托盘
      </button>
    </div>
  </section>
  <p class="cs-note">
    主面板没有标题栏也不在任务栏里，这三个键是它<strong>唯一的出口</strong>。
    「退出」关掉窗口它就真停了；「收进托盘」则面板收起、<strong>程序继续跑</strong>。
  </p>
  {#if closeNote}
    <p class="cs-fail" role="alert">{closeNote}</p>
  {/if}

  <h2 class="cs-h2">上下文</h2>
  <p class="cs-note">
    送进模型的<strong>最近</strong>多少条原文。更早的压成摘要，所以调大<strong>不会</strong
    >等比变贵。
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

  <h2 class="cs-h2">图片</h2>
  <section class="cs-row">
    <span class="cs-label">把对方发的图片发给模型</span>
    <div>
      <!--
        ⚠ **按钮写动作，不写状态。**
        原来关着的时候显示「关闭」—— 可按下去是**打开**。
        「关闭」读起来像「当前状态是关闭」，而不是「按这个会关闭」。
        控件说的话和它要做的事对不上，用户得先猜一次。
      -->
      <button class="cs-btn" class:primary={imagesOut} onclick={() => toggleImagesOut(!imagesOut)}>
        {imagesOut ? '关掉' : '打开'}
      </button>
    </div>
  </section>
  <!--
    ⚠ 这段**必须短**，而且**只能红一句**。

    早先它是四行红字（`cs-warn` 整段都红），一个「关闭」开关底下
    挂着四行警告 —— 红占了整屏三分之一，**红色又退化成装饰了**。
    跟主面板 armed 态那两句红是同一个毛病，我在那边刚砍完就又犯在这边。

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
    图片 token 预算。<strong>`budget.visible` 由 Rust 读总闸算出来</strong>，前端不自己判断。
    总闸关着时没有一张图会外发，摆滑块就是「调了没反应的控制项」——
    ticket 11 刚拆掉过一个同类的。
  -->
  {#if budget?.visible}
    <h2 class="cs-h2">图片成本</h2>
    <p class="cs-note">
      送进模型的<strong>单张</strong>图片上限。超出就把图缩到预算内，小图不放大。
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
      <!-- 无解要<strong>明说</strong>。悄悄发一张然后超支，比报错糟得多。 -->
      <p class="cs-note cs-warn" role="alert">
        {budget.provider} 发任何一张图都要 {budget.provider_base} token 的底账， 预算 {budget.budget}
        无解——再调小也不会更省。
      </p>
    {:else if budget.provider_base > 0}
      <p class="cs-note">
        {budget.provider} 的固定底账是 {budget.provider_base} token，与图片大小无关； 剩下的是按图缩放后另外算的。
      </p>
    {/if}
    {#if budgetNote}
      <p class="cs-note" role="status">{budgetNote}</p>
    {/if}
  {/if}

  <h2 class="cs-h2">模型 key</h2>
  <p class="cs-note">
    key 存进 <strong>Windows 凭据管理器</strong>，不落任何普通文件、不进日志。 前端读不到 key
    本身——所有网络调用都在 Rust 侧。
  </p>

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
        <!-- 已配置时<strong>不显示输入框</strong>。要改才点「换一把」——
             这样明文 key 不会因为打开设置页就躺在界面上。 -->
        <div class="cs-actions">
          <button class="cs-btn" onclick={() => (drafts[p.id] = REVEAL)} disabled={busy[p.id]}>
            换一把
          </button>
          <!-- 连通性测试。不调模型：走 /v1/models，只读、不花钱。 -->
          <button class="cs-btn" onclick={() => probe(p)} disabled={busy[p.id]}>
            {busy[p.id] ? '测试中…' : '测连通'}
          </button>
          <button class="cs-btn danger" onclick={() => forget(p)} disabled={busy[p.id]}>
            删除
          </button>
        </div>
      {:else}
        <!-- p.hint 是 Rust 给的<strong>可执行</strong>指引：存哪、变量名叫什么。
             不能是空字符串或「暂无」。 -->
        <p class="cs-hint">{p.hint}</p>
      {/if}

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
          <button class="cs-btn primary" onclick={() => save(p)} disabled={busy[p.id]}>
            {busy[p.id] ? '写入中…' : '保存'}
          </button>
        </div>
      {/if}

      {#if notes[p.id]}
        <p class="cs-note">{notes[p.id]}</p>
      {/if}
    </section>
  {:else}
    <p class="cs-note">正在读设置…</p>
  {/each}
</main>

<style>
  .cs-settings {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    height: 100%;
    padding: 1rem 1.1rem;
    background: var(--cs-surface);
    overflow-y: auto;
  }

  .cs-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .cs-head h1 {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 600;
  }

  .cs-h2 {
    margin: 0.3rem 0 0;
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--cs-ink-2);
  }

  .cs-close {
    width: 26px;
    height: 26px;
    font-size: 1.05rem;
    line-height: 1;
    border: 1px solid var(--cs-border);
    border-radius: 7px;
    background: transparent;
    color: var(--cs-ink-2);
  }

  .cs-close:hover {
    background: rgb(255 255 255 / 6%);
    color: var(--cs-ink);
  }

  /* ── 一行设置项 ────────────────────────────────────────
   *
   * ⚠ **标题在上、控件在下，不是左右分列两端。**
   *
   * 原来是 `justify-content: space-between` —— 标签贴左边、控件甩到最右。
   * 700px 宽的窗口里中间空出 400px，两个东西看着像毫不相干的两块。
   * 而它们**本来就是一件事的两半**：这一项是什么，怎么改它。
   *
   * 上下排还有个实际好处：控件不再被窗口宽度摆布，
   * 窄一点也不会被挤到第二行。
   */
  .cs-row {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.4rem;
  }

  .cs-label {
    font-size: 0.78rem;
    font-weight: 600;
    color: var(--cs-ink);
  }

  .cs-note {
    margin: 0;
    font-size: 0.7rem;
    line-height: 1.6;
    /* ⚠ 正文用**铅笔档**（6.22 : 1），不是标注档。
     *
     * 早先这里是 `--cs-ink-3` —— 而那档只有 3.81 : 1，它被当正文用了。
     * 3.81 只够标注（WCAG AA 的 4.5 是给正文的）。
     * 「默认关闭。打开后…」这种句子是**要人读的**，读不清等于没说。 */
    color: var(--cs-pencil);
    white-space: pre-line;
    /* 一整段说明文字不该横跨 640px——行太长眼睛回行会累。 */
    max-width: 60ch;
  }

  /* 说明文字里的强调。**不是**加粗到极黑，用墨档里的一档，
   * 免得一段话里出现三个不同重量的字。 */
  .cs-note strong {
    color: var(--cs-ink);
    font-weight: 600;
  }

  .cs-hint {
    margin: 0.25rem 0 0;
    font-size: 0.68rem;
    line-height: 1.55;
    color: var(--cs-ink-2);
    /* 指引可能有换行，要原样显示 */
    white-space: pre-line;
  }

  .cs-fail {
    margin: 0;
    padding: 0.4rem 0.5rem;
    font-size: 0.7rem;
    border-radius: 7px;
    background: color-mix(in srgb, var(--cs-alarm) 18%, transparent);
  }

  /* 预算无解时的提示。用警告色而不是正文色——这不是「已保存」，
     是「你调的这个值不起作用」。 */
  .cs-warn {
    color: var(--cs-alarm);
  }

  .cs-prov {
    padding: 0.5rem 0.6rem;
    border: 1px solid var(--cs-border);
    border-radius: var(--cs-radius-sm);
    background: var(--cs-surface-raised);
  }

  .cs-prov-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .cs-prov-name {
    font-size: 0.78rem;
  }

  .cs-badge {
    font-size: 0.65rem;
    padding: 0.1rem 0.4rem;
    border-radius: 999px;
  }

  .cs-badge.ok {
    color: var(--cs-live);
    background: color-mix(in srgb, var(--cs-live) 16%, transparent);
  }

  .cs-badge.ok.env {
    color: var(--cs-ink-2);
    background: rgb(255 255 255 / 8%);
  }

  .cs-badge.miss {
    color: var(--cs-ink-3);
    background: rgb(255 255 255 / 6%);
  }

  .cs-actions,
  .cs-form {
    display: flex;
    gap: 0.35rem;
    margin-top: 0.4rem;
  }

  .cs-input {
    flex: 1;
    padding: 0.3rem 0.45rem;
    font-size: 0.72rem;
    font-family: ui-monospace, 'Cascadia Mono', monospace;
    color: var(--cs-ink);
    background: rgb(0 0 0 / 28%);
    border: 1px solid var(--cs-border-strong);
    border-radius: 7px;
  }

  .cs-input:focus {
    outline: none;
    border-color: color-mix(in srgb, var(--cs-live) 60%, transparent);
  }

  /* 独立按钮。**和两处分段控件同高 33px** —— 这一页上它们是
   * 同一级控件，三种规格（28 / 33 / 30）意味着用户点之前得先判断
   * 「这颗算不算主要控件」。
   *
   * 同样是量出来的，不是估的：原来 28px、圆角 7px、字号 0.7rem。 */
  .cs-btn {
    /* 33 − 2(border) = 31 给内容：行高 19 + 上下各 6 */
    padding: 0.406rem 0.7rem;
    font-size: 0.78rem;
    border-radius: var(--cs-radius-sm);
    border: 1px solid var(--cs-border-strong);
    background: transparent;
    white-space: nowrap;
  }

  .cs-btn:hover:not(:disabled) {
    background: rgb(255 255 255 / 6%);
  }

  .cs-btn:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .cs-btn.primary {
    border-color: color-mix(in srgb, var(--cs-live) 55%, transparent);
    color: var(--cs-live);
  }

  .cs-btn.danger {
    border-color: color-mix(in srgb, var(--cs-alarm) 45%, transparent);
    color: var(--cs-alarm);
  }

  /* ── 分段控件（「关闭时」用）──────────────────────────────
   *  两个选项都是一整个动作，<strong>不是轻/重</strong>。所以并列，不做主次。 */
  .cs-seg {
    display: inline-flex;
    gap: 2px;
  }

  .cs-seg-item {
    padding: 6px 12px;
    border: 1px solid var(--cs-rule-strong);
    border-radius: var(--cs-radius-sm);
    font-size: 12.5px;
    color: var(--cs-pencil);
    transition:
      background var(--cs-dur-fast) var(--cs-ease),
      color var(--cs-dur-fast) var(--cs-ease),
      border-color var(--cs-dur-fast) var(--cs-ease);
  }

  .cs-seg-item:hover {
    background: rgb(20 26 31 / 6%);
    color: var(--cs-ink);
  }

  /* 选中：墨底纸字。和 TierSwitch 同一套 —— 两个都是「改一个设置」，
   * 一个重一个轻才是真问题。红在这里永远不出现：红只给不可逆。 */
  .cs-seg-item.on {
    background: var(--cs-ink);
    border-color: var(--cs-ink);
    color: var(--cs-surface-raised);
    font-weight: 600;
  }

  .cs-seg-item.on:hover {
    background: var(--cs-ink-2);
  }

  /* 危险开关下面那一句红。**只有这一句是红的。** */
  .cs-warn-on {
    margin: 0.3rem 0 0;
    font-size: 0.72rem;
    font-weight: 600;
    color: var(--cs-live);
  }

  /* ── 滑块 ───────────────────────────────────────────────
   *
   * ⚠ **`<output>` 从来没被样式过。**
   *
   * 浏览器给它的默认样式是 `13.33px Arial` —— 比这一页正文（0.7rem
   * ≈ 11.2px）还大，而且带 UA 的蓝色。于是滑块旁边那个实时读数
   * 是全页最大最跳的一处字，比它要读的说明文字还显眼。
   *
   * 一个「12 条」不该比整段解释它为什么这么调的话还重。
   */
  .cs-slider {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    max-width: 30rem;
  }

  .cs-slider input[type='range'] {
    flex: 1 1 auto;
    min-width: 0;
    accent-color: var(--cs-ink);
  }

  .cs-slider output {
    flex: none;
    min-width: 5.5rem;
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    /* 机器读出来的数 → 等宽，和主面板的分工一致。 */
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--cs-ink);
    /* UA 默认把 output 渲染得像超链接，蓝色 + 下划线。去掉。 */
    text-decoration: none;
  }
</style>
