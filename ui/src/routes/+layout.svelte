<script lang="ts">
  /*
   *  根 layout：**只有 reset 与 token**。
   *
   *  这里刻意不放任何产品 chrome。
   *
   *  早先根 layout 直接挂着「连不上后台」的红色告警和演示取景框，
   *  结果连 `/studio` 设计画布也被那条红条压住了底边。而设计画布的
   *  全部意义就是**屏上除了被决策的元素什么都没有**——旧 chrome 漏进来
   *  一条，我的判断就被污染一次。
   *
   *  所以 chrome 全部下沉到 `(panel)/+layout.svelte`，那是旧面板的。
   */
  import '../app.css';

  let { children } = $props();
</script>

{@render children?.()}
