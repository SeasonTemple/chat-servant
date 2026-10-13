<!--
  `/panel` —— **旧路由，保留是为了不让外部引用集体失效**。

  设计稿、`?demo=1&state=` 演示夹具、`scripts/shoturl.ps1` 出图，
  全都按 `/panel` 写。删掉旧面板时新面板从 `panel/` 上移到 `(panel)/`，
  这个路由本来会 404 —— 于是让它继续渲染同一个组件。

  ⚠ **薄壳，不是第二份面板。** 两个路由渲染的是同一个
  `$lib/PaperPanel.svelte`：面板只有一份实现。
  哪天要合并成一个路由，删掉这个文件即可。

  ⚠ 它必须**留在 `(panel)` 组内**而不是提到 `routes/panel/`：
  组里的 `+layout.svelte` 才是演示模式那个 380px 取景框
  （「演示模式必须按真实面板宽度渲染，否则它验不出任何东西」）。
  挪出去，`?demo=1` 就会铺满浏览器，评审时看的是宽版本，
  而真机是 380px —— 于是窄面板上的问题一个都验不出来。
-->
<script lang="ts">
  import PaperPanel from '$lib/PaperPanel.svelte';
</script>

<PaperPanel />
