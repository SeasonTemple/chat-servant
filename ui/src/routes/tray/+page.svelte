<script lang="ts">
  import { IS_DEMO } from '$lib/runtime';

  const items = [
    { id: 'show', label: '显示面板' },
    { id: 'relock', label: '锁定会话' },
    { id: 'settings', label: '打开设置' },
    { id: 'debug', label: '识别画布' },
  ] as const;

  type ItemId = (typeof items)[number]['id'] | 'quit';

  async function dismiss() {
    if (IS_DEMO) return;
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      await getCurrentWindow().close();
    } catch (e) {
      console.error('收起托盘菜单失败：', e);
    }
  }

  async function pick(id: ItemId) {
    if (IS_DEMO) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      if (id === 'quit') {
        await invoke('quit_app');
        return;
      }
      await dismiss();
      if (id === 'show') {
        await invoke('show_panel');
      } else if (id === 'relock') {
        await invoke('relock_session');
      } else if (id === 'settings') {
        await invoke('open_settings');
      } else if (id === 'debug') {
        await invoke('open_debug');
      }
    } catch (e) {
      console.error(`执行托盘菜单项 ${id} 失败：`, e);
    }
  }
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key === 'Escape') {
      dismiss();
    }
  }}
/>

<div class="tray-frame">
  <nav class="tray-sheet" aria-label="言侍托盘菜单">
    {#each items as item (item.id)}
      <button type="button" class="tray-item" onclick={() => pick(item.id)}>
        {item.label}
      </button>
    {/each}
    <div class="tray-sep" role="separator"></div>
    <button type="button" class="tray-item quit" onclick={() => pick('quit')}>退出程序</button>
  </nav>
</div>

<style>
  .tray-frame {
    width: 134px;
    height: 204px;
    padding: 6px 8px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    user-select: none;
  }

  .tray-sheet {
    position: relative;
    isolation: isolate;
    width: 118px;
    border: 14px solid transparent;
    border-image: url('/probe-sheet.png') 76 68 76 68 fill / 14px 12px 14px 12px / 0 stretch;
    filter: drop-shadow(0 4px 8px rgb(12 16 20 / 0.36));
    padding: 2px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .tray-item {
    position: relative;
    isolation: isolate;
    width: 100%;
    padding: 5px 6px;
    font:
      700 14px/1.35 'FangSong',
      'STFangsong',
      '仿宋',
      serif;
    color: var(--cs-ink);
    text-align: center;
    letter-spacing: 0.06em;
    white-space: nowrap;
    cursor: pointer;
    transition: transform 45ms cubic-bezier(0.22, 1, 0.36, 1);
  }

  .tray-item::before {
    content: '';
    position: absolute;
    inset: -6px -4px;
    z-index: -1;
    pointer-events: none;
    opacity: 0;
    border: 8px solid transparent;
    border-image: url('/strip.png') 32 fill / 8px / 0 stretch;
    filter: drop-shadow(0 2px 5px rgb(20 26 31 / 0.22));
    transform-origin: 15% 30%;
    transition:
      opacity 45ms ease-out,
      transform 45ms cubic-bezier(0.22, 1, 0.36, 1),
      filter 45ms ease-out;
  }

  .tray-item:hover,
  .tray-item:focus-visible {
    z-index: 2;
    transform: translateY(-1px);
    outline: none;
  }

  .tray-item:hover::before,
  .tray-item:focus-visible::before {
    opacity: 1;
  }

  .tray-item:active {
    z-index: 2;
    transform: translateY(1px);
    transition-duration: 25ms;
  }

  .tray-item:active::before {
    opacity: 1;
    transform: perspective(360px) translate3d(0.5px, 0.8px, 0) rotateX(-1.6deg) rotateY(2deg)
      rotateZ(0.4deg) scale(0.99, 0.98);
    filter: drop-shadow(0 1px 2px rgb(20 26 31 / 0.14));
    transition-duration: 25ms;
  }
  .tray-sep {
    border-top: 1px dashed rgb(46 34 18 / 0.26);
    margin: 3px 4px;
  }

  .tray-item.quit {
    color: var(--cs-pencil-red);
  }
</style>
