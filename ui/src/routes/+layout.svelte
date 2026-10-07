<script lang="ts">
  import '../app.css';
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';

  let { children } = $props();

  /** Rust 侧推过来的帧计数——用来证明「不是轮询」。 */
  let lastError = $state<string | null>(null);

  onMount(() => {
    // 只监听，不轮询。Rust 那边什么时候 emit，前端就什么时候更新。
    let dispose: (() => void) | null = null;
    let cancelled = false;

    void (async () => {
      // 「静默期零调用」不在主面板上证明，它在调试窗口里。
      // 这里保留 `frames` 的接收，是为了让遥测通道有订阅者——
      // Rust 侧的 emit 有接收方，才算一条真的链路。
      const un = await listen<{ frames: number }>('telemetry', () => {});
      if (cancelled) {
        un();
        return;
      }
      dispose = un;
    })();

    return () => {
      cancelled = true;
      dispose?.();
    };
  });
</script>

{@render children?.()}

{#if lastError}
  <div class="cs-error" role="alert">{lastError}</div>
{/if}

<!-- ⚠ 静默期指示**从主面板撤掉了**，只留在调试窗口。
     它原来是 `position: fixed` 挂在右上角、还带着一个裸数字，
     结果和标题栏的图标按钮叠在一起，截图上能直接看到那个 "0"。
     「静默期零调用」是硬约束，但它该在**调试视图**里被证明，
     不该在主面板上占一个常驻的、还和按钮打架的位置。
     主面板右上角那个黄铜状态点（会话名旁边）承担「活着没有」这件事。 -->

<style>
  .cs-error {
    position: fixed;
    inset: auto 0 0 0;
    padding: 0.5rem 0.75rem;
    background: var(--cs-alarm);
    color: rgb(19 18 17);
    font-size: 0.75rem;
  }
</style>
