<!--
  窗口控制。三颗记号，**一个重量、一个中心、一个家族**。

  ## 为什么用 lucide 的现成图标，不自己画

  曾经手搓过三轮 SVG path，还配了两张刻度表。**全是白费**：
  `@lucide/svelte` 已经装在依赖里（`+page.svelte` 一直在用
  `Lock` / `RefreshCcwDot` / `ScanSearch` / `Settings2`），里面
  1869 个图标全部经过视觉平衡与无障碍审校。

  看 `x.svelte` 的数据就是答案：

      x        = M18 6  6 18  +  M6 6  12 18   → 对角 12 × 12
      minus    = M5 12 h14                    → 14 长
      ellipsis = cx 12/19/5, cy 12, r 1        → 三点同一条 y

  `cy` 全是 12 → 中心天然对齐；`round` 端点由 `Icon.svelte` 默认
  给；三颗同在一个 viewBox → 缩放必然同步。

  而手搓的那版呢？标了「12 × 12」，画成了 6 × 6，还偏上 2.5px，
  三颗粗细各不相同。**在一个已经审校平衡过的图标体系旁边重画，
  只会更差。**

  ## 一条设计纪律

  **先查库，再谈设计。** 把「光学尺寸平衡」「圆头 vs 尖角」
  「统一基线」当原创发现是危险的——那些是这个图标库**已经解决**
  的问题。文档里写「我决定用 17 × 17 配平」比写「用 lucide 的 x，
  size 14」危险得多：前者把一个可复用的决策固化成了个人品味。
-->
<script lang="ts">
  /**
   * 三颗记号，共用一个 32px 高的带子。
   *
   * - `size` 三颗一样。lucide 的默认描边（1.5 / 24）在这个尺寸下
   *   约合 1.1px，正好。
   * - `strokeWidth` 三颗一样。**必须显式给** —— 不给的话每颗走各自
   *   的默认值，看起来就又不齐了。
   * - `hit` 统一 28 × 32。**比记号本身大**，鼠标不用瞄。
   *   400px 的面板上三个这样的键占 88px，还剩 312px 给内容。
   */
  import { X, Minus, Ellipsis } from '@lucide/svelte/icons';

  interface Props {
    /**
     * 「关闭时」当前值。
     *
     * `exit`（默认）→ 只给 ×。没有托盘时「收起」没有落点，
     *   **一个离屏 + alwaysOnTop + skipTaskbar 的程序就是用户看不见的
     *   僵尸**——它还在替人说话。所以没有落点的动作不提供。
     * `tray` → 加一颗 −，面板离屏、程序继续跑。
     */
    closeBehavior: 'exit' | 'tray';
    onquit: () => void;
    onhide?: () => void;
    onmore?: () => void;
  }

  const { closeBehavior, onquit, onhide, onmore }: Props = $props();

  const SIZE = 14;
  const SW = 1.75;
</script>

<!--
  ⚠ 这三个键**必须在拖拽区之外**。
  `+page.svelte` 的头部整块是 `data-tauri-drag-region`，而按键自身
  会终止冒泡——否则点 × 会变成「拖动窗口」。
-->
<div class="cs-winctl" data-no-drag>
  {#if onmore}
    <button class="cs-win" type="button" title="识别画布与设置" aria-label="更多" onclick={onmore}>
      <Ellipsis {SIZE} {SW} strokeWidth={SW} />
    </button>
  {/if}

  {#if closeBehavior === 'tray' && onhide}
    <button
      class="cs-win"
      type="button"
      title="收进托盘 · 程序继续运行"
      aria-label="收进托盘"
      onclick={onhide}
    >
      <Minus {SIZE} {SW} strokeWidth={SW} />
    </button>
  {/if}

  <button
    class="cs-win cs-win-quit"
    type="button"
    title={closeBehavior === 'tray' ? '退出 · 停了就是停了' : '退出'}
    aria-label="退出"
    onclick={onquit}
  >
    <X {SIZE} {SW} strokeWidth={SW} />
  </button>
</div>

<style>
  /* 三颗键的**唯一差别**是悬停底色。形态、尺寸、间距、圆角全部相同。
   *
   * 早先给危险键开过更大的盘子（46 × 48），那是个错误：在专门区分
   * 「主要动作」的界面里，最容易误点的键长成了主要动作的样子。
   * 红色已经把话说完了，不需要额外权重。 */
  .cs-winctl {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: none;
  }

  .cs-win {
    display: grid;
    place-items: center;
    width: 28px;
    height: 32px;
    border-radius: var(--cs-radius-sm);
    color: var(--cs-mark);
    transition:
      background var(--cs-dur-fast) var(--cs-ease),
      color var(--cs-dur-fast) var(--cs-ease);
  }

  .cs-win:hover {
    /* 墨的 7% —— 是「这一格亮了」，不是「这是一颗按钮」 */
    background: rgb(20 26 31 / 7%);
    color: var(--cs-ink);
  }

  .cs-win:active {
    background: rgb(20 26 31 / 12%);
  }

  /* 退出的红**只在悬停时出现**。
   *
   * 发送是「字已经出去了」，退出是「字不会再出去了」——两件都不可逆，
   * 同一个红。看到红就是「有东西出去过，或者要出去」。
   *
   * 静止时是铅笔灰：一个替你说话的程序，它的退出键不该一直在喊。 */
  .cs-win-quit:hover {
    background: var(--cs-live);
    color: #fff;
  }

  .cs-win-quit:active {
    background: var(--cs-live-dim);
  }
</style>
