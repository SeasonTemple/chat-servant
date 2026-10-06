<script lang="ts">
  import { flashOnce } from './attach';
  import { DUR, EASE, prefersReducedMotion } from './motion';

  interface Props {
    reason: string;
  }

  const { reason }: Props = $props();

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
</script>

<!-- reason 一变就闪一下：判断依据变了必须看得见 -->
<p {@attach flash(reason)} class="cs-reason" title={reason}>{reason}</p>

<style>
  .cs-reason {
    margin: 0;
    padding: 0.15rem 0.3rem;
    border-radius: 6px;
    font-size: 0.68rem;
    line-height: 1.45;
    color: var(--cs-text-dim);
    /* 单行 + 省略号：理由通常很长，但不该占据整块面板。
       完整内容在 title 里。 */
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
