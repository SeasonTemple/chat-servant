<script lang="ts">
  import { flashOnce } from './attach';
  import { DUR, EASE, prefersReducedMotion } from './motion';
  import type { CallCounts } from './types';

  interface Props {
    reason: string;
    /** 分段调用次数。四个都要显示，包括 0。 */
    counts?: CallCounts | null;
  }

  const { reason, counts = null }: Props = $props();

  /**
   * 理由变化时闪一下绿色背景。
   *
   * 为什么值得做：这一栏存在的全部意义就是让用户看清「它为什么这么判」。
   * 理由变了却毫无提示，用户会以为面板卡住了。
   *
   * `{@attach flash(reason)}` 里读了 `reason`，attachment 跑在 effect 中，
   * reason 一变就自动重跑一次——不需要 `{#key}` 重建节点，也不需要
   * 「记住上次值」的状态。少一个状态就少一处能写错的地方。
   */
  const flash = (dep: string) =>
    flashOnce(dep, {
      from: { backgroundColor: 'rgba(114,205,144,0.18)' },
      to: { backgroundColor: 'rgba(114,205,144,0)' },
      duration: prefersReducedMotion() ? 0 : DUR.slow,
      ease: EASE.out,
    });

  /**
   * 四个数字，**连 0 一起显示**。
   *
   * 「视觉 0」是默认配置下的正常状态。不显示它，用户会以为程序没统计
   * 视觉调用；显示成一个模糊的「已统计」同样没用——ticket 15 要的就是
   * 「默认多模态配置下永远是 2 + 1 + 1 + 0」这种能一眼核对的形式。
   */
  const summary = $derived(
    counts
      ? `判断 ${counts.judge} · 起草 ${counts.draft} · 排序 ${counts.rank} · 视觉 ${counts.vision}`
      : '',
  );
</script>

<!-- reason 一变就闪一下：判断依据变了必须看得见 -->
<p {@attach flash(reason)} class="cs-reason" title={reason}>{reason}</p>

{#if summary}
  <!-- 这一行不是装饰：它是「静默期零调用」那条硬约束的唯一可视证据。
       静默时四个数应该全是 0，任何非 0 都说明循环在没人说话时还在花钱。 -->
  <p class="cs-counts" title={`本轮各段模型调用次数。${summary}。静默期应全为 0。`}>
    {summary}
  </p>
{/if}

<style>
  .cs-reason {
    margin: 0;
    padding: 0.15rem 0.3rem;
    border-radius: 6px;
    font-size: 0.68rem;
    line-height: 1.45;
    color: var(--cs-ink-2);
    /* 单行 + 省略号：理由通常很长，但不该占据整块面板。
       完整内容在 title 里。 */
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cs-counts {
    margin: 0.1rem 0 0;
    padding-left: 0.3rem;
    font-size: 0.62rem;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.02em;
    color: var(--cs-ink-3);
  }
</style>
