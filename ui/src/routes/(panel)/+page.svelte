<!--
  主窗口的面板。`tauri.conf.json` 的 `url` 是 `/`，而 `(panel)` 是**路由组**、
  不产生路径段，所以这里渲染的就是 `main` 窗口看到的东西。

  ⚠ **2026-10-13：这一行指向过被判为废弃的旧面板。** 旧面板 1553 行，
  正文排版、按钮、窗口控制都被设计规范否掉了，而用户打开应用看到的
  一直是它 —— 一张准备删的纸。票 25 换掉的就是这件事。

  这里只有一行，真正的东西在 `$lib/PaperPanel.svelte`。
  拆成薄壳是因为 `/panel` 那个旧路由也要渲染同一个面板
  （见 `panel/+page.svelte` 那条注释）。
-->
<script lang="ts">
  import PaperPanel from '$lib/PaperPanel.svelte';
</script>

<PaperPanel />
