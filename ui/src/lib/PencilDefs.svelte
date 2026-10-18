<!--
  红铅笔滤镜 `#pencil` 的**唯一定义处**（2026-10-18 抽出）。

  ## 为什么必须是单独一个组件，而不是「三处各写一份逐字相同的」

  它原来在三个文件里各写了一份：
    - `lib/PaperPanel.svelte`（主面板档位行的勾）
    - `routes/studio/+page.svelte`
    - `routes/studio/actrow/+page.svelte`
  三份**逐字相同**，而 `PaperPanel` 自己的注释就写着：
  「与 `routes/studio/` 里那份**逐字相同**——两处各写一份的话，
  改一边另一边就会悄悄退回单层平涂。」

  ⚠ 注释写了这件事，但**没有任何东西保证它们一致**：
  改 `feFuncA` 的 tableValues 时编译器不会报、门禁不会报、
  界面上也只表现为「另一处忽然变淡」——**三处渐隐程度不一致**
  在单张截图里根本看不出来。
  ⇒ 判据是「`id="pencil"` 在仓库里出现几次」，
  而不是「我记得三份一样」。

  ## 跨 SVG 引用能工作，但有前提

  `filter="url(#pencil)"` 引用的是**同一个文档**里的 id。
  设置屏（`PanelSettings`）要用这枚滤镜画芯片上的圈，
  所以它也得挂一份 —— 以前它没有，而 `url(#id)` 找不到目标时
  SVG 规范是**整个元素不渲染**（不是「没有滤镜效果」），
  症状是「勾/圈整个不见了」，不报错。

  ⚠ 因此本组件在**每个用到红铅笔的界面里都要挂**。
  漏挂的表现是元素消失，不是效果变淡 —— 这也是当初漏掉
  设置屏的那一处没人发现的原因。
-->
<svg class="cs-pencil-defs" width="0" height="0" aria-hidden="true">
  <!--
    石墨颗粒 + 边缘位移。三处旧副本的说明合并在这里：

    ⚠ `color-interpolation-filters="sRGB"` 不能省：SVG 滤镜默认走
    linearRGB，`feComposite operator="in"` 的覆盖率会被当成亮度重新
    映射，叠出来整体偏淡——省掉的结果是「像蒙了一层灰纱」。

    颗粒来自 `feTurbulence` 的 alpha：先把三通道求和映射成 alpha，再用
    `feComponentTransfer` 的 table 拉对比。线性映射出的 alpha 集中在
    0.6~1.0，只是**均匀变淡**，和平涂一个毛病；拉高对比后才出现 0.1
    附近的低谷，也就是笔尖「跳格」的断续。

    位移交给合成器，不产生二值掩码，所以不会出现 `SetWindowRgn` 那种
    硬阶梯。scale 2 ≈ 抖动 ±1px，再多就变成毛边而不是石墨了。
  -->
  <defs>
    <filter
      id="pencil"
      x="-30%"
      y="-30%"
      width="160%"
      height="160%"
      color-interpolation-filters="sRGB"
    >
      <feTurbulence type="fractalNoise" baseFrequency="0.62" numOctaves="4" seed="11" result="n" />
      <feColorMatrix
        in="n"
        type="matrix"
        result="g"
        values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0.72 0.72 0.72 0 -0.30"
      />
      <feComponentTransfer in="g" result="gc">
        <feFuncA type="table" tableValues="0 0.08 0.30 0.62 0.88 1" />
      </feComponentTransfer>
      <feComposite in="SourceGraphic" in2="gc" operator="in" result="t" />
      <feDisplacementMap in="t" in2="n" scale="2" xChannelSelector="R" yChannelSelector="G" />
    </filter>
  </defs>
</svg>

<style>
  /* 只用来挂 filter 定义，不占布局。 */
  .cs-pencil-defs {
    position: absolute;
    width: 0;
    height: 0;
    overflow: hidden;
  }
</style>
