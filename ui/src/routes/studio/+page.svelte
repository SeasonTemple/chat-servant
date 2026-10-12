<script lang="ts">
  import { TICK_PATH, TICK_NESTED, RING_MARK, RING_LABEL_MARK, MARK_VIEW } from '$lib/marks';

  /*
   *  两件事一次比完：
   *   A 笔迹的**表面**够不够铅笔（轮廓已经对了，怀疑表面不对）
   *   B **圈还是勾**——上一轮否圈用的是我自己画坏的那版，从没拿
   *     生成版的圈和勾在同一版面 1:1 比过。
   *
   *  A 是**消融实验**，不是三选一：每档只加一个变量，才能知道到底是
   *  哪一步起作用。上一版「P1/P2/P3」其实不是消融——P2 和 P3 的差别
   * 被埋在两三个变量里，结果 6× 下一眼看不出，就误以为「参数不够强」。
   */
  const SHEET_W = 380;
  const SHEET_H = 560;
  const SHEET = '/probe-sheet.svg';
  const RULER = Array.from({ length: 20 }, (_, k) => k);

  const TOUCH = [
    {
      id: 'P1',
      name: '平涂单层',
      claim: '基线。单一 opacity，边缘一刀切：非纸即满色，只有两个密度值。',
      grain: false,
      nest: false,
      stack: false,
    },
    {
      id: 'P2',
      name: '叠笔偏移三层（上一版）',
      claim:
        '同一轮廓偏移三份叠起来。外层最外缘**仍是满 0.42 的硬边**，叠出来只是变粗 + 多几级台阶。',
      grain: false,
      nest: false,
      stack: true,
    },
    {
      id: 'P3',
      name: '嵌套五层（羽化）',
      claim: '同一中心线、宽度剖面逐级收窄。密度 0.22→0.41→0.56→0.69→0.85，才是芯重边虚。',
      grain: false,
      nest: true,
      stack: false,
    },
    {
      id: 'P4',
      name: '平涂 + 噪芯',
      claim: 'P1 只加颗粒。纸纹会让笔尖「跳格」，留下一撮一撮的断续和局部密度差。',
      grain: true,
      nest: false,
      stack: false,
    },
    {
      id: 'P5',
      name: '嵌套 + 噪芯 + 位移',
      claim: 'P3 加 P4，再把轮廓用 feTurbulence 抖 ±1px，边缘不再是数学般光滑。',
      grain: true,
      nest: true,
      stack: false,
    },
  ];

  const TIERS = [
    { key: 'Manual', label: '只填入' },
    { key: 'Confirm', label: '确认后发' },
    { key: 'Auto', label: '自动发送' },
  ];

  type Glyph = 'tick' | 'ring' | 'ring-label';

  const MARKS = [
    {
      id: 'T',
      name: '勾在字上方',
      claim: '标记独立，字对齐。语义是「这一档被批了」。',
      kind: 'tick',
    },
    {
      id: 'R1',
      name: '圈在字上方',
      claim: '同样独立。语义比勾弱一点——圈更像「这里有个东西」。',
      kind: 'ring',
    },
    {
      id: 'R2',
      name: '圈住字',
      claim: '圈把档位名包起来。语义最强，但环要贴着字，笔不能重。',
      kind: 'ring-label',
    },
  ];
</script>

<svelte:head>
  <title>材质台 · 笔触与标记</title>
</svelte:head>

<!--
  三种笔迹画法。**都画在同一个 60×40 取景框里**，所以 A 段之间是严格
  1:1 的——密度差就是形状差，不会被尺寸差掩盖。
-->
{#snippet flat()}
  <path d={TICK_PATH} fill-opacity="0.86" />
{/snippet}

{#snippet stacked()}
  <g transform="translate(0.35 0.5)"><path d={TICK_PATH} fill-opacity="0.42" /></g>
  <g transform="translate(-0.42 0.22)"><path d={TICK_PATH} fill-opacity="0.42" /></g>
  <g transform="translate(0.16 -0.45)"><path d={TICK_PATH} fill-opacity="0.42" /></g>
{/snippet}

{#snippet nested()}
  {#each TICK_NESTED as layer (layer.d)}
    <path d={layer.d} fill-opacity={layer.o} />
  {/each}
{/snippet}

{#snippet glyph(kind: Glyph)}
  {#if kind === 'tick'}
    {#each TICK_NESTED as layer (layer.d)}
      <path d={layer.d} fill-opacity={layer.o} />
    {/each}
  {:else if kind === 'ring'}
    {#each RING_MARK as layer (layer.d)}
      <path d={layer.d} fill-opacity={layer.o} />
    {/each}
  {:else}
    {#each RING_LABEL_MARK as layer (layer.d)}
      <path d={layer.d} fill-opacity={layer.o} />
    {/each}
  {/if}
{/snippet}

<div class="desk">
  <figure class="slot">
    <figcaption>纸 · 1:1 · 380 × 560</figcaption>
    <div class="sheet" style="width:{SHEET_W}px;height:{SHEET_H}px"></div>
  </figure>

  <div class="rack">
    <h1>A · 轮廓对了，表面还不对</h1>
    <p class="lede">
      笔迹的<b>收放方向和横纵比</b>都已照照片校准（1.48 : 1），但看着仍不像铅笔。 差的是<b>表面</b
      >。下面是五档消融，每档只加一个变量——上一版把变量 混在一起，结果 6×
      下看不出差别，还误判成「参数不够强」。
    </p>

    {#each TOUCH as t (t.id)}
      <section class="cand">
        <header>
          <b>{t.id}</b><span class="cname">{t.name}</span>
          <span class="claim">{t.claim}</span>
        </header>
        <div class="crop" style="width:{SHEET_W}px">
          <div
            class="band"
            style="width:{SHEET_W}px;height:{MARK_VIEW.h + 16}px;
                   background-image:url({SHEET});
                   background-size:{SHEET_W}px {SHEET_H}px;
                   background-position:0 0"
          >
            <svg
              class="mark"
              viewBox="0 0 {MARK_VIEW.w} {MARK_VIEW.h}"
              width={MARK_VIEW.w}
              height={MARK_VIEW.h}
              filter={t.grain ? 'url(#pencil)' : undefined}
            >
              {#if t.nest}{@render nested()}{:else if t.stack}{@render stacked()}{:else}{@render flat()}{/if}
            </svg>
          </div>
        </div>
      </section>
    {/each}

    <h1 class="gap">B · 圈还是勾</h1>
    <p class="lede">
      上一轮否圈，用的是<b>我自己画坏的那版</b>（2.75 : 1 的扁椭圆，读作糊斑）。
      生成版的圈从没和勾在同一版面 1:1 比过——这次补上。三行都是真实排版语境： 撕边纸 + 三个档位名 +
      标记落在当前档，<b>纹理统一用 P5</b>， 所以这一段比的是<b>形</b>，不是质感。
    </p>

    {#each MARKS as m (m.id)}
      <section class="cand">
        <header>
          <b>{m.id}</b><span class="cname">{m.name}</span>
          <span class="claim">{m.claim}</span>
        </header>
        <div class="crop" style="width:{SHEET_W}px">
          <div
            class="band"
            style="width:{SHEET_W}px;height:74px;
                   background-image:url({SHEET});
                   background-size:{SHEET_W}px {SHEET_H}px;
                   background-position:0 0"
          >
            <div class="strip">
              {#each TIERS as t, i (t.key)}
                <span class="cell" data-on={i === 0}>
                  <!--
                    ⚠ 每格**必须**留一个 40px 的 markslot，哪怕里面是空的。
                    不留的话，没有标记的两格名字会上浮、跟有标记的那格错开，
                    三行对不齐——而「三行对齐」正是圈字环能读的前提。
                  -->
                  <span class="markslot">
                    {#if i === 0 && m.kind === 'tick'}
                      <svg
                        class="mark"
                        viewBox="0 0 {MARK_VIEW.w} {MARK_VIEW.h}"
                        width={MARK_VIEW.w}
                        height={MARK_VIEW.h}
                        filter="url(#pencil)"
                      >
                        {@render glyph('tick')}
                      </svg>
                    {:else if i === 0 && m.kind === 'ring'}
                      <svg
                        class="mark"
                        viewBox="0 0 {MARK_VIEW.w} {MARK_VIEW.h}"
                        width={MARK_VIEW.w}
                        height={MARK_VIEW.h}
                        filter="url(#pencil)"
                      >
                        {@render glyph('ring')}
                      </svg>
                    {/if}
                  </span>
                  <span class="name">{t.label}</span>
                  {#if i === 0 && m.kind === 'ring-label'}
                    <svg
                      class="mark encircle"
                      viewBox="0 0 {MARK_VIEW.w} {MARK_VIEW.h}"
                      width={MARK_VIEW.w}
                      height={MARK_VIEW.h}
                      filter="url(#pencil)"
                    >
                      {@render glyph('ring-label')}
                    </svg>
                  {/if}
                </span>
              {/each}
            </div>
          </div>
        </div>
        <div class="ruler" aria-hidden="true">
          {#each RULER as i (i)}<i class:long={i % 5 === 0}></i>{/each}
        </div>
      </section>
    {/each}
  </div>

  <svg class="defs" width="0" height="0" aria-hidden="true">
    <!--
      ⚠ `color-interpolation-filters="sRGB"` 不能省。SVG 滤镜默认走
      linearRGB，`feComposite operator="in"` 的覆盖率会被当成亮度重新映射，
      叠出来整体偏淡——省掉这一行的结果是「颗粒像蒙了一层灰纱」。

      颗粒来自 `feTurbulence` 的 alpha：先把三通道求和映射成 alpha，
      再用 `feComponentTransfer` 的 table 拉对比——线性映射出的 alpha
      集中在 0.6~1.0 之间，只是**均匀变淡**，和 P1 一个毛病。
      拉高对比后才出现 0.1 附近的低谷，也就是笔尖「跳格」的断续。

      位移交给合成器，不产生二值掩码，所以不会有 SetWindowRgn 那种
      硬阶梯。scale 2.0 = 约 ±1px；再多就变成毛边而不是石墨了。
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
        <feTurbulence
          type="fractalNoise"
          baseFrequency="0.62"
          numOctaves="4"
          seed="11"
          result="n"
        />
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
</div>

<style>
  .desk {
    display: flex;
    gap: 56px;
    align-items: flex-start;
    padding: 40px 48px 80px;
    background: #6b7278;
    min-height: 100vh;
    font-family: ui-sans-serif, system-ui, sans-serif;
    color: #f2f4f5;
  }
  .slot {
    margin: 0;
  }
  .slot figcaption {
    font-size: 12px;
    letter-spacing: 0.04em;
    opacity: 0.7;
    margin-bottom: 8px;
  }
  .sheet {
    background-image: url('/probe-sheet.svg');
    background-size: 380px 560px;
    box-shadow: 0 18px 40px rgb(0 0 0 / 0.45);
  }
  .defs {
    position: absolute;
  }
  .rack {
    max-width: 400px;
  }
  .rack h1 {
    font-size: 18px;
    margin: 0 0 6px;
    font-weight: 600;
  }
  .rack h1.gap {
    margin-top: 40px;
  }
  .lede {
    font-size: 13px;
    line-height: 1.65;
    opacity: 0.82;
    margin: 0 0 28px;
  }
  .lede b {
    color: #ffd9a0;
    font-weight: 600;
  }
  .cand {
    margin-bottom: 26px;
  }
  .cand header {
    margin-bottom: 8px;
  }
  .cand b {
    font-size: 15px;
    color: #ffd9a0;
    margin-right: 6px;
  }
  .cname {
    font-size: 14px;
    font-weight: 600;
  }
  .claim {
    font-size: 12px;
    opacity: 0.72;
    display: block;
    margin-top: 2px;
  }
  .crop {
    outline: 1px solid rgb(255 255 255 / 0.22);
    outline-offset: 0;
  }
  .band {
    position: relative;
    overflow: hidden;
  }

  /*  ⚠ 绝对定位。`.band` 不建立 BFC（无 overflow / padding / border），
   *  子元素的 margin 会与父元素塌陷，把整块纸背景一起推下去。 */
  .mark {
    position: absolute;
    top: 8px;
    left: 100px;
  }

  /*  只有 fill，不设 fill-opacity——每一层的透明度是**笔迹数据的一部分**，
   *  由生成器算出来（嵌套的 0.22/0.24/0.26/0.30/0.36），CSS 一旦兜底
   *  就会把它们全覆盖掉。 */
  .mark path {
    fill: rgb(168 74 56);
  }

  /* ── B 段的整条档位排版 ── */
  .strip {
    position: absolute;
    top: 6px;
    left: 32px;
    right: 32px;
    display: flex;
  }
  .cell {
    position: relative;
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
  }

  /*  固定 40px，即使空着。少了它三行字就对不齐。 */
  .markslot {
    display: block;
    position: relative;
    width: 60px;
    height: 40px;
  }

  .strip .mark {
    position: absolute;
    left: 50%;
    top: 0;
    margin-left: -30px;
  }

  /*  圈字环：环心在取景框 y=17，字心在 y=48，所以往下挪 31px。
   *  它是**画在字外面**的，不占 markslot。 */
  .strip .mark.encircle {
    top: 31px;
  }

  .name {
    display: block;
    width: 100%;
    height: 16px;
    line-height: 16px;
    text-align: center;
    white-space: nowrap;
    font-size: 11px;
    letter-spacing: 0.02em;
    color: var(--cs-pencil);
    opacity: 0.62;
  }
  .cell[data-on='true'] .name {
    opacity: 1;
  }

  .ruler {
    display: flex;
    margin-top: 6px;
    width: 380px;
  }
  .ruler i {
    width: 1px;
    height: 3px;
    background: rgb(255 255 255 / 0.3);
  }
  .ruler i.long {
    height: 7px;
    background: rgb(255 255 255 / 0.55);
  }
</style>
