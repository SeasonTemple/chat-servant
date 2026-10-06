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
    <div><TierSwitch bind:tier /></div>
  </section>
  <p class="cs-note">
    档位只在**你按了之后**才生效。会话结束时一律回落到「只填入」，程序自己不会升档。
  </p>

  <h2 class="cs-h2">上下文</h2>
  <p class="cs-note">
    送进模型的**最近**多少条原文。超出后更早的部分压成一段摘要——
    摘要是按块算的（攒够一整块才调一次模型），所以这个数调大**不会**等比变贵。
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
      <button class="cs-btn" class:primary={imagesOut} onclick={() => toggleImagesOut(!imagesOut)}>
        {imagesOut ? '已打开' : '关闭'}
      </button>
    </div>
  </section>
  <p class="cs-note cs-warn">
    <strong>默认关闭。</strong>打开后，对方发来的图片区域会被裁出、缩放、编码，
    然后发给第三方模型。图片区域的识别目前**只在 1 张真实样本上验证过**，
    其余情况可能误判——误判的代价是把你看到的画面交给外部模型。
  </p>
  {#if imagesNote}
    <p class="cs-note" role="status">{imagesNote}</p>
  {/if}

  <!--
    图片 token 预算。**`budget.visible` 由 Rust 读总闸算出来**，前端不自己判断。
    总闸关着时没有一张图会外发，摆滑块就是「调了没反应的控制项」——
    ticket 11 刚拆掉过一个同类的。
  -->
  {#if budget?.visible}
    <h2 class="cs-h2">图片成本</h2>
    <p class="cs-note">
      送进模型的**单张**图片上限。超出就把图缩到预算内——小图不放大， 放大只会更贵更糊。
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
    key 存进 **Windows 凭据管理器**，不落任何普通文件、不进日志。 前端读不到 key
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
        <!-- 已配置时**不显示输入框**。要改才点「换一把」——
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
        <!-- p.hint 是 Rust 给的**可执行**指引：存哪、变量名叫什么。
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
    color: var(--cs-text-dim);
  }

  .cs-close {
    width: 26px;
    height: 26px;
    font-size: 1.05rem;
    line-height: 1;
    border: 1px solid var(--cs-border);
    border-radius: 7px;
    background: transparent;
    color: var(--cs-text-dim);
  }

  .cs-close:hover {
    background: rgb(255 255 255 / 6%);
    color: var(--cs-text);
  }

  .cs-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
  }

  .cs-label {
    font-size: 0.78rem;
    color: var(--cs-text-dim);
  }

  .cs-note {
    margin: 0;
    font-size: 0.7rem;
    line-height: 1.6;
    color: var(--cs-text-faint);
    white-space: pre-line;
  }

  .cs-hint {
    margin: 0.25rem 0 0;
    font-size: 0.68rem;
    line-height: 1.55;
    color: var(--cs-text-dim);
    /* 指引可能有换行，要原样显示 */
    white-space: pre-line;
  }

  .cs-fail {
    margin: 0;
    padding: 0.4rem 0.5rem;
    font-size: 0.7rem;
    border-radius: 7px;
    background: color-mix(in srgb, var(--cs-danger) 18%, transparent);
  }

  /* 预算无解时的提示。用警告色而不是正文色——这不是「已保存」，
     是「你调的这个值不起作用」。 */
  .cs-warn {
    color: var(--cs-danger);
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
    color: var(--cs-accent);
    background: color-mix(in srgb, var(--cs-accent) 16%, transparent);
  }

  .cs-badge.ok.env {
    color: var(--cs-text-dim);
    background: rgb(255 255 255 / 8%);
  }

  .cs-badge.miss {
    color: var(--cs-text-faint);
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
    color: var(--cs-text);
    background: rgb(0 0 0 / 28%);
    border: 1px solid var(--cs-border-strong);
    border-radius: 7px;
  }

  .cs-input:focus {
    outline: none;
    border-color: color-mix(in srgb, var(--cs-accent) 60%, transparent);
  }

  .cs-btn {
    padding: 0.3rem 0.7rem;
    font-size: 0.7rem;
    border-radius: 7px;
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
    border-color: color-mix(in srgb, var(--cs-accent) 55%, transparent);
    color: var(--cs-accent);
  }

  .cs-btn.danger {
    border-color: color-mix(in srgb, var(--cs-danger) 45%, transparent);
    color: var(--cs-danger);
  }
</style>
