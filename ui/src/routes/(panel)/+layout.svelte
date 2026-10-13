<script lang="ts">
  //  ⚠ `app.css` 由**根** `+layout.svelte` 引入，这里不再引一次：
  //  重复引入会把 token 定义算两遍，而 `/studio` 那张设计画布要的是
  //  干净的一遍。
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { HAS_TAURI, IS_DEMO as DEMO } from '$lib/runtime';

  let { children } = $props();

  /**
   * 演示模式／静态部署下**连不上后端**。
   *
   * ⚠ 不摆成一句「加载中」。既没有 IPC 也没有演示数据时，界面必须
   * **明说连不上** —— 一个永远空着的面板比一个说清楚的错误危险得多：
   * 用户会以为程序坏了，或者更糟，以为它正在替自己回消息。
   */
  let offline = $state(false);

  onMount(() => {
    // 只监听，不轮询。Rust 那边什么时候 emit，前端就什么时候更新。
    let dispose: (() => void) | null = null;
    let cancelled = false;

    void (async () => {
      // ⚠ **整个链路包在 try 里。**
      //
      // 演示模式里没有 `window.__TAURI_INTERNALS__`，`listen` 读它的
      // `.transformCallback` 会拿到 undefined，于是**每次打开演示页都产生
      // 一个 unhandledrejection**：
      //   TypeError: Cannot read properties of undefined (reading 'transformCallback')
      //
      // 判据走 `$lib/runtime` 的 `IS_DEMO`，**不是**在这里再判一次
      // `typeof window.__TAURI_INTERNALS__`——那个表达式在打包产物里会被
      // 静态消除，换个构建模式就失效，而 URL 参数是显式的、不会自己变。
      if (DEMO) return;
      if (!HAS_TAURI) {
        // 正式部署的静态页面：既没 IPC 也没演示数据。**说清楚。**
        offline = true;
        return;
      }

      try {
        // 「静默期零调用」不在主面板上证明，它在调试窗口里。
        // 这里保留 `telemetry` 的接收，是为了让遥测通道有订阅者——
        // Rust 侧的 emit 有接收方，才算一条真的链路。
        const un = await listen<{ frames: number }>('telemetry', () => {});
        if (cancelled) {
          un();
          return;
        }
        dispose = un;
      } catch (e) {
        offline = true;
        console.error('连不上 Rust 的推送：', e);
      }
    })();

    return () => {
      cancelled = true;
      dispose?.();
    };
  });
</script>

{#if DEMO}
  <!--
    ⚠ **演示模式必须按真实面板宽度渲染，否则它验不出任何东西。**

    真实面板是 **380px**（`tauri.conf.json`），还能缩到 340px。早先演示页
    直接铺满浏览器，于是：
      · 头部那一排在 770px 下看着松松垮垮，一到 380px 就挤成一团——
        而**没人发现**，因为评审时看的就是那个宽版本；
      · 「紧凑模式」那套判据（档位名隐藏、衬线降 17px）触发不了，
        ResizeObserver 量到的永远是「宽」，验证等于没做。

    也就是说：一个用来验收窄面板的工具，本身跑在宽面板上。
    这里把它钉到 380px 并居中，让演示页第一次真的能当尺子用。
  -->
  <div class="cs-demo-frame">
    {@render children?.()}
  </div>
{:else}
  {@render children?.()}
{/if}

{#if offline}
  <div class="cs-error" role="alert">
    这个页面**连不上后台**，显示的不是实时数据。
    <br />
    要看界面设计请打开 <code>?demo=1</code>；要真的用，请在 Tauri 窗口里打开。
  </div>
{/if}

<!--
  ⚠ 静默期指示**不在主面板上**，它只在调试窗口里。

  它原来是 `position: fixed` 挂在右上角、还带着一个裸数字，结果和头部
  的窗口控制叠在一起。「静默期零调用」是硬约束，但它该在**调试视图**里
  被证明，不该在主面板上占一个常驻位置。
  ⚠ 末句原来写「主面板顶边那条 3px 才是档位」——已作废：档位在 `6f10f9f`
  挪到了**纸底**，现在是一条红铅笔的勾（盖章式）。改这个注释是因为它
  会让下一个人以为顶边还有东西，实际去那儿找。
-->

<style>
  /* 演示模式的取景框。尺寸 = `tauri.conf.json` 里的 380 × 560。
   * 外圈那一圈不是装饰：它标出**这就是面板的边界**，
   * 免得看的人把外面的浏览器留白也算进布局里。 */
  .cs-demo-frame {
    width: 380px;
    height: 560px;
    max-height: 100%;
    margin: 0 auto;
    outline: 1px solid rgb(20 26 31 / 12%);
  }

  .cs-error {
    position: fixed;
    inset: auto 0 0 0;
    padding: 0.6rem 0.85rem;
    /* 红色 = 不可逆 / 出事了。这里确实是出事了，一致的。 */
    background: var(--cs-live);
    color: #fff;
    font-size: 0.75rem;
    line-height: 1.5;
  }

  .cs-error code {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    background: rgb(255 255 255 / 22%);
    padding: 0 3px;
    border-radius: 3px;
  }
</style>
