<script lang="ts">
  /*
   *  新面板。**从零建的，不是旧面板改的。**
   *
   *  旧面板（`(panel)/`）已冻结：它的旧正文排版、旧按钮、旧窗口控制
   *  都被否定了，在那个容器里做任何新设计判断都会被污染。
   *
   *  这里目前**只有两样东西**：
   *    · 一张撕边的纸（真实素材，撕口即顶边）
   *    · 顶边的档位区
   *
   *  ⚠ 下半部分**故意是空的**。正文怎么排还没有定稿，硬塞一个布局
   *  就是把旧面板那套又抄一遍——那正是被否掉的东西。空着是诚实的。
   *
   *  ── 档位区为什么长这样 ────────────────────────────────────
   *
   *  三道短记号是**中性**的，不编码任何东西。抄写员把三档写在纸上，
   *  当前那一档被红铅笔勾上。所以：
   *    · 位置  = 档位集合（配合铅笔小字，不靠猜）
   *    · 勾    = 当前在哪
   *  「勾」还是「圈」**尚未定案**，材质台 `routes/studio/` 里 T / R1 / R2
   *  三行同版面 1:1 对着比。
   *  ⚠ 早先否圈的理由站不住：那一版圈是 2.75 : 1 的扁椭圆、读作糊斑，
   *  是我自己画坏的。唯一还站得住的是**形**上的一条——纸面最强的形状
   *  语言已经是撕口，再加一条闭合轮廓等于两个边界打架。这条要拿生成版
   *  的圈（0.94 : 1、三道叠画）重验，不能拿坏样本定案。
   *
   *  笔迹几何来自 `$lib/marks`（由 `ui/scripts/pencilgen.py`
   *  按「中心线 + 宽度剖面」生成），不是手写 path——手写过一次收放
   *  方向反了。理由与量测都记在那边。
   */
  import { TICK_NESTED, RING_LABEL_MARK, MARK_VIEW } from '$lib/marks';
  import { markShape } from '$lib/mark-shape.svelte';
  import { onMount } from 'svelte';

  /** 档位从低到高。Rust 侧持有档位值，这里只按顺序摆位置。 */
  const TIERS = [
    { key: 'Manual', label: '只填入' },
    { key: 'Confirm', label: '确认后发' },
  ] as const;

  type TierKey = (typeof TIERS)[number]['key'];

  /**
   *  `?tier=` 只是**探针口**，方便把三档各截一张 1× 图。
   *  它不读后端、不缓存任何状态——档位真值仍在 Rust 侧。
   */
  const fromUrl =
    typeof location !== 'undefined' ? (new URLSearchParams(location.search).get('tier') ?? '') : '';
  let tier = $state<TierKey>(
    TIERS.some((t) => t.key === fromUrl) ? (fromUrl as TierKey) : 'Manual',
  );

  onMount(() => {
    /*
     * 标记形状落在 Rust 侧的 prefs，这里只读镜像。
     *
     * ⚠ `?shape=` 是**探针口**，与上面 `?tier=` 同一性质：方便把两支笔
     * 各截一张 1× 图对着比。它在演示/静态页里只改内存镜像
     * （`markShape.set` 见 `HAS_TAURI` 分支），**不落盘**，
     * 所以拿它做实验不会把真机设置改成圈。
     */
    const probe = new URLSearchParams(location.search).get('shape');
    if (probe === 'tick' || probe === 'ring') {
      void markShape.set(probe);
    } else {
      void markShape.load();
    }
  });

  /*
   * 勾还是圈 —— **用户的选择，不是设计定论**（2026-10-13）。
   *
   * 两支笔在 `marks.js` 里本来就都生成好了，切换只是换一组 path；
   * 笔性也一致：都是「中心线 + 宽度剖面」算出来的轮廓，都落在同一个
   * 60×40 取景框里。语义分工：勾 = 「这一档被批了」，圈字环 = 「这一档
   * 被圈中了」，形状差异在 1:1 下比粗细差异更容易读出来。
   *
   * ⚠ 只有两支笔。两支的**笔心在取景框里不在同一个位置**，所以摆放
   * 不能共用一个偏移——见下面的 CSS，两者各自有实测出来的 top。
   */
  const MARK_LAYERS = $derived(markShape.current === 'ring' ? RING_LABEL_MARK : TICK_NESTED);
</script>

<svelte:head>
  <title>纸面板</title>
</svelte:head>

<main class="cs-sheet" data-tier={tier}>
  <!--
    档位区。三个 slot 各 1/3，勾只落在当前那一道上，
    其余两道 opacity:0 —— 用 CSS 控显隐而不是移动元素，
    升档/回落时勾是「淡出 + 淡入」而不是「跳过去」。
  -->
  <div class="cs-tier" role="radiogroup" aria-label="发送档位">
    {#each TIERS as t, i (t.key)}
      <button
        type="button"
        class="cs-slot"
        data-idx={i}
        aria-checked={tier === t.key}
        role="radio"
        tabindex={tier === t.key ? 0 : -1}
        onclick={() => (tier = t.key)}
      >
        <span class="cs-slot-mark">
          <!--
            参考纸上没有线：三行字，谁被勾了谁就是当前档。
            所以这里**不画占位的短记号**——空着就是空着，
            勾的位置本身就是编码。早先加了一道中性短记号，
            结果勾的底角垂到标签下面 11px，读着像戳进文字。
          -->
          <span class="cs-markbox">
            <svg
              class="cs-tick"
              data-shape={markShape.current}
              viewBox="0 0 {MARK_VIEW.w} {MARK_VIEW.h}"
              width={MARK_VIEW.w}
              height={MARK_VIEW.h}
              aria-hidden="true"
              filter="url(#pencil)"
            >
              <!--
                五层嵌套，不是单层平涂。单层不管给多少 opacity，边缘都是
                一刀切的硬边，所以它读成矢量而不是红铅笔——材质台 P1 与 P5
                同框实测：落在中间密度带的像素 23.0% → 70.5%，总墨量反而少
                17%。密度从芯往外逐级下降，才有「芯重边虚」。
              -->
              {#each MARK_LAYERS as layer (layer.d)}
                <path d={layer.d} fill-opacity={layer.o} />
              {/each}
            </svg>
          </span>
          <span class="cs-slot-name">{t.label}</span>
        </span>
      </button>
    {/each}
  </div>

  <!--
    正文区。**故意空着**——见文件头。空着比塞一个没定稿的布局诚实。
  -->
</main>

<svg class="cs-defs" width="0" height="0" aria-hidden="true">
  <!--
    石墨颗粒 + 边缘位移。与 `routes/studio/` 里那份**逐字相同**——
    两处各写一份的话，改一边另一边就会悄悄退回单层平涂。

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
  /*  只用来挂 filter 定义，不占布局。 */
  .cs-defs {
    position: absolute;
  }

  .cs-sheet {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    /* 纸由 SVG 自己画，边缘交给合成器抗锯齿。
     * 不用 SetWindowRgn：它是二值掩码，撕口会变成硬阶梯。 */
    background: url('/probe-sheet.svg') center / 100% 100% no-repeat;
    /* 撕口已含在 SVG 里，padding 只负责把内容推离撕口。
       撕口实测 7~11 CSS px（随 x 变，对 alpha 阈值不敏感），
       量法见 ui/scripts/sheetprobe.ps1。原注释写「17.5px」，对不上。 */
    padding: 40px 32px 30px;
    color: var(--cs-ink);
    overflow: hidden;
  }

  /* ── 档位区 ─────────────────────────────────────────────── */
  .cs-tier {
    flex: none;
    display: flex;
    gap: 0;
  }

  .cs-slot {
    flex: 1;
    display: flex;
    justify-content: center;
    /* 按钮化之后默认的 UA 样式必须全部抹掉，否则会多出一圈边框，
       而边框在撕边纸上等于第二个边界。 */
    appearance: none;
    margin: 0;
    padding: 0;
    border: 0;
    background: none;
    font: inherit;
    color: inherit;
    cursor: pointer;
  }

  .cs-slot-mark {
    display: flex;
    flex-direction: column;
    align-items: center;
  }

  /*  取景框只负责给标记一个固定高度，好让三行字对齐。它**不画任何东西**。 */
  .cs-markbox {
    position: relative;
    display: block;
    width: 60px;
    height: 40px;
  }

  /*  标记。红铅笔，蜡质，`fill-opacity: 0.86` 让纸的颗粒透上来。
   *  ⚠ 实心不透明会读成印刷或马克笔，照片里没有一根是那样。
   *
   *  **盖章式：笔盖在字上，不是悬在字上方。** 用户 2026-10-13 用手绘参考
   *  点破了这一点——标记靠**交叠**表达「这一档被批了」，躲开字反而变成
   *  一个跟内容无关的浮标。旧版 `top: 2px` 是「图标式」排布。
   *
   *  ⚠ 盖住字**不会**读成删除线：删除线是一条横线穿过字中部，圈是包围，
   *  拓扑不同。2026-10-13 我把「标记与字交叠」当成缺陷判掉过，那是拿形状
   *  交叠去推语义的错判。
   *
   *  两个 top 是实测值（材质台 B 段，`?mark=overlay`），不是估的：
   *  笔心 = `marks.js` 里 path 的 bbox 中心，字心 = 40 + 2 + 11/2 ≈ 47.5
   *      TICK_NESTED     心 (28.0, 20.8) → top ≈ 27
   *      RING_LABEL_MARK 心 (30.1, 17.1) → top ≈ 30
   *  **两支笔的笔心在取景框里位置不同，所以不能共用一个偏移。**
   *
   *  水平同理：`left:50% + margin-left:-30px` 对准的是**取景框**中心，
   *  要对准的是**笔心**。勾的笔心 x=28（框心 30）天生偏左 2px，但实测
   *  （截图数出「入」露在勾的右臂外）要右移 9px 才盖得住三个字。
   *  圈字环不动——它是横长环，左右本该盖满字，实测 -30 已经对。
   */
  .cs-tick {
    position: absolute;
    left: 50%;
    top: 27px;
    margin-left: -21px;
    opacity: 0;
    transition: opacity var(--cs-dur) var(--cs-ease);
  }

  .cs-tick[data-shape='ring'] {
    top: 30px;
    margin-left: -30px;
  }

  /*  只有 fill，不设 fill-opacity——每层的透明度是**笔迹数据的一部分**，
    由生成器算出来。CSS 一旦兜底就会把嵌套的五级全覆盖掉，
    羽化立刻塌回单层平涂。 */
  .cs-tick path {
    fill: var(--cs-pencil-red);
  }

  .cs-slot-name {
    margin-top: 2px;
    font-size: 11px;
    line-height: 1;
    letter-spacing: 0.02em;
    color: var(--cs-pencil);
    /* 未选中的两档退半档，让勾是唯一的高对比处。
     * 但 0.62 混的是透明度，落在纸上的实际对比度仍要够 ——
     * `--cs-pencil` 是 5.10 : 1，半透明后仍高于 AA 正文 4.5。 */
    opacity: 0.62;
    transition: opacity var(--cs-dur) var(--cs-ease);
  }

  .cs-slot[aria-checked='true'] .cs-tick {
    opacity: 1;
  }
  .cs-slot[aria-checked='true'] .cs-slot-name {
    opacity: 1;
  }

  /*  Confirm：会把话发出去。笔从红铅笔换成**警报红**——这是红铅笔
   *  唯一让位给 `--cs-live` 的地方。
   *
   *  ⚠ 2026-10-13：`Auto` 取消后，`Confirm` 成为唯一不可逆的档，
   *  「红＝不可逆」这条规则从它身上继续成立。
   *
   *  ⚠ 这里**只能改 fill，不能写 fill-opacity**。CSS 声明会盖掉
   *  每层 path 上的 `fill-opacity` 属性，五层嵌套立刻塌成单层平涂，
   *  不可逆的档就成了唯一一支不像铅笔的。要改浓度就改生成器里的
   *  `NEST` 剖面，让五级一起动。 */
  .cs-sheet[data-tier='Confirm'] .cs-tick path {
    fill: var(--cs-live);
  }

  /*  聚焦环。撕边纸上不能靠描边，得用一圈**纸色的负空间**把
   *  底下的笔画顶开，否则键盘用户看不到焦点在哪。 */
  .cs-slot:focus-visible {
    outline: none;
  }
  .cs-slot:focus-visible .cs-slot-mark {
    outline: 2px solid var(--cs-tier-ink);
    outline-offset: 3px;
  }
</style>
