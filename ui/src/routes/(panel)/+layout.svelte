<script lang="ts">
  //  ⚠ `app.css` 由**根** `+layout.svelte` 引入，这里不再引一次：
  //  重复引入会把 token 定义算两遍，而 `/studio` 那张设计画布要的是
  //  干净的一遍。
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
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
    // ⚠ **这里不订阅任何事件，也不轮询。**
    //
    // 原来这里 `listen('telemetry', () => {})`，注释写着「为了让遥测通道
    // 有订阅者，Rust 侧的 emit 有接收方，才算一条真的链路」（票 28）。
    // **那个 emit 从来不存在** —— 这条链路从头到尾没有过，而注释把它
    // 说成是「补完最后一块」，读的人只会以为差一步。
    //
    // 它唯一真实的作用是：`listen` 在没有 IPC 时会抛，用那个抛错点亮
    // 下面的离线横幅。**功能要留，理由要换掉**：横幅该由一次真实的
    // IPC 调用来证明，而不是由「成功订阅了一条没人发的事件」来证明。
    // 后者证明的是「订阅成功了」，不是「连得上 Rust」。
    if (DEMO) return;
    if (!HAS_TAURI) {
      // 正式部署的静态页面：既没 IPC 也没演示数据。**说清楚。**
      offline = true;
      return;
    }

    // `latest_readout` 只读缓存里最近一轮快照，命令自身的注释就写着
    // 「不轮询——用户手动触发」。这里调一次纯粹是当探针用，载荷丢掉。
    void invoke('latest_readout').catch((e) => {
      offline = true;
      console.error('连不上 Rust：', e);
    });
  });
</script>

{#if DEMO}
  <!--
    ⚠ **演示模式必须按真实面板宽度渲染，否则它验不出任何东西。**

    真实面板是 **380px**（`tauri.conf.json`），还能缩到 340px。早先演示页
    直接铺满浏览器，于是：
      · 头部那一排在 770px 下看着松松垮垮，一到 380px 就挤成一团——
        而**没人发现**，因为评审时看的就是那个宽版本；
      · 「紧凑模式」那套判据触发不了，验证等于没做。

    ⚠ 那句括号里的原文是「档位名隐藏、衬线降 17px」，**已于 2026-10-14
    作废**：那块代码随旧面板一起删了（`5031ab3`），紧凑分支全仓无引用
    （票 27）。留着会让下一个人去找一套**已经不存在**的判据。
    这里保留「窄面板要有窄面板的验收」这半句——它现在指向的是
    纸面板在 `minWidth` 340 下会不会挤，而不是某个已被删掉的开关。

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
