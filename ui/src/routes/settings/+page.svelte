<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import TierSwitch from '$lib/TierSwitch.svelte';
  import type { Tier } from '$lib/types';

  let tier = $state<Tier>('Manual');
  let closeHint = $state<string | null>(null);

  async function close() {
    // 关闭不是销毁：窗口留着但隐藏，下次打开不用重建 WebView。
    // `getCurrentWindow` 只在 Tauri 环境下可用，浏览器里退化成提示。
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      await getCurrentWindow().close();
    } catch {
      closeHint = '这个页面在浏览器里预览，关不掉窗口。';
    }
  }

  async function openMain() {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      await getCurrentWindow().hide();
    } catch {
      /* 浏览器预览下忽略 */
    }
    await invoke('open_settings').catch(() => {
      closeHint = '找不到 settings 窗口。';
    });
  }
</script>

<main class="cs-settings">
  <header class="cs-head">
    <h1>设置</h1>
    <button class="cs-close" onclick={close} title="关闭（窗口保留，下次直接开）">×</button>
  </header>

  <section class="cs-row">
    <label class="cs-label" for="tier">发送档位</label>
    <div id="tier"><TierSwitch bind:tier /></div>
  </section>

  <p class="cs-note">
    档位只在**你按了之后**才生效。会话结束时一律回落到「只填入」， 程序自己不会升档。
  </p>

  {#if closeHint}
    <p class="cs-hint" role="status">{closeHint}</p>
  {/if}

  <footer class="cs-foot">
    <button class="cs-btn" onclick={openMain}>回到副驾</button>
  </footer>
</main>

<style>
  .cs-settings {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    height: 100%;
    padding: 1rem 1.1rem;
    background: var(--cs-surface);
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
  }

  .cs-hint {
    margin: 0;
    font-size: 0.7rem;
    color: var(--cs-danger);
  }

  .cs-foot {
    margin-top: auto;
    display: flex;
    justify-content: flex-end;
  }

  .cs-btn {
    padding: 0.35rem 0.8rem;
    font-size: 0.72rem;
    border-radius: 8px;
    border: 1px solid var(--cs-border-strong);
    background: transparent;
  }

  .cs-btn:hover {
    background: rgb(255 255 255 / 6%);
  }
</style>
