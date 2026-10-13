<script lang="ts">
  import { page } from '$app/state';
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

  /*
   *  隔离开关：`?only=b` 只留 B 段。
   *
   *  为什么需要：整页是 min-height:100vh 的普通流，B 段在页面下方，
   *  截图按视口高度截 → 圈还是勾永远只看得见第一张卡的一半，
   *  而「三行对齐」恰恰是判断圈字环能不能读的前提。看到半张就下结论，
   *  就是上一轮用自己画坏的圈否掉圈的原因。
   */
  const only = $derived(page.url.searchParams.get('only'));

  /*
   *  C 段隔离开关：`?mark=overlay`。
   *
   *  B 段的两种排布（标记悬在字上方 / 圈套住字）都在**比形**，
   *  但 2026-10-13 用户手绘的参考否掉了它们共用的前提：
   *
   *    标记应当**盖在字上**（盖章 / 批注），不是**悬在字上方**（图标）。
   *
   *  两者不是同一种东西：图标式标记要留净空才读得出「这一格有标记」，
   *  批注式标记恰恰靠**交叠**表达「这一格被批了」——压住字才是重点，
   *  躲开字反而变成一个跟内容无关的浮标。
   *
   *  ⚠ 顺带记一条反直觉的：盖住字**不会**读成删除线。删除线是**一条横线
   *  穿过字中部**，圈是**包围**，拓扑不同。2026-10-13 我把「环与字交叠」
   *  当成 R2 的缺陷判掉，那是拿形状交叠去推语义的错判。
   */
  const MARK_MODE = $derived(page.url.searchParams.get('mark') ?? 'above');
  const INK_MODE = $derived(page.url.searchParams.get('ink') ?? 'light');

  /* ── C 段（票 22）的正文样本与三档字号 ──
   *
   *  样本刻意选成**会换行三次以上**的中性句子：只排两行的话，三档字号的
   *  行长差别会被纸张下沿吃掉一半，看不出来。
   *  ⚠ 不含任何真实聊天内容——`ui/static` 可能被部署到公开 URL。
   */
  const BODY_SAMPLE =
    '周三下午三点的会我这边没问题，材料我提前十分钟到就行。' +
    '如果需要我把上周那版数据也一起带上，你回头跟我说一声就好。';

  const TYPE_SCALES = [
    // lines / perLine 由两条独立路径交叉验证后回填：
    //   ① 内容宽 348px ÷（字号 + 字距）得每行字数；
    //   ② 1:1 截图里数行数、并逐行数字数与样本总字数（57 字）对账。
    // 两条一致才填。lines / perLine **不参与高度计算**（纸带自适应）。
    // 曾经在写死纸带高度时先填了估算值 3/3/4，结果三档全裁掉末行
    // ——估算值当量具输入，量具只会把自己的假设再吐一遍。
    { px: 19, perLine: 17, lines: 4, claim: '实测 17 字/行 · 4 行' },
    { px: 21, perLine: 16, lines: 4, claim: '实测 16 字/行 · 4 行' },
    { px: 23, perLine: 14, lines: 5, claim: '实测 14 字/行 · 5 行' },
  ];

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

<div class="desk" data-mark={MARK_MODE} data-ink={INK_MODE}>
  <figure class="slot">
    <figcaption>纸 · 1:1 · 380 × 560</figcaption>
    <div class="sheet" style="width:{SHEET_W}px;height:{SHEET_H}px"></div>
  </figure>

  <div class="rack">
    {#if only !== 'b' && only !== 'type' && MARK_MODE === 'above'}
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
    {/if}

    {#if only !== 'type'}
      <h1 class="gap">B · 圈还是勾</h1>
      <p class="lede">
        {#if MARK_MODE === 'overlay'}
          <b>这一档比的不是形，是位置。</b>
          三行都改成 <b>盖在字上</b>（标记 z-index 在字之上，红笔连续可见），对应你 2026-10-13
          手绘的那张参考。 纹理仍是 P5，所以差异全部来自空间关系：勾压着字起笔、圈把字整个圈进去。
          顺带否掉我上一轮的判据——盖住字<b>不会</b
          >读成删除线，删除线是一条横线穿过字中部，圈是包围，拓扑不同。
        {:else}
          上一轮否圈，用的是<b>我自己画坏的那版</b>（2.75 : 1 的扁椭圆，读作糊斑）。
          生成版的圈从没和勾在同一版面 1:1 比过——这次补上。三行都是真实排版语境： 撕边纸 +
          三个档位名 + 标记落在当前档，<b>纹理统一用 P5</b>， 所以这一段比的是<b>形</b>，不是质感。
        {/if}
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
                          class="mark tick"
                          viewBox="0 0 {MARK_VIEW.w} {MARK_VIEW.h}"
                          width={MARK_VIEW.w}
                          height={MARK_VIEW.h}
                          filter="url(#pencil)"
                        >
                          {@render glyph('tick')}
                        </svg>
                      {:else if i === 0 && m.kind === 'ring'}
                        <svg
                          class="mark ring"
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
    {/if}

    {#if only === 'type'}
      <!--
      C 段 · 正文字号（票 22）。

      三档 19 / 21 / 23px，**同一段正文、同一条 380px 纸面**。
      只变字号这一���变量，行高 1.75、字距 0.02em 三档锁死——否则比的是
      「行高也一起变」的复合效果，就退回上一轮 P1/P2/P3 那种
      「变量混在一起、6× 下看不出差别」的毛病了。

      正文取中性的起草回复，**不含任何真实聊天内容**（演示页可能部署到
      公开 URL，见 AGENTS.md Security 一节）。

      标注文案只报**量出来的东西**：每行实际字数、共几行。不写「更好读」
      这种观感结论——那是人拍板的事，量具只负责给数。
    -->
      <h1 class="gap">C · 正文字号 19 / 21 / 23</h1>
      <p class="lede">
        票 22 只剩字号没拍，<b>字体已经定了：楷体</b>（实测 21px 细笔画占比 38.1%，比宋体的 31.5%
        字面率更接近手写）。 这三行只变字号，行高与字距锁死。 看的不是「哪个好看」，是<b
          >行长与密度</b
        >：380px 宽的纸面上，19px 每行多塞 几个字、23px
        在第几字断行——这两件事直接决定正文骨架的栏宽能不能定。
      </p>

      {#each TYPE_SCALES as t (t.px)}
        <section class="cand">
          <header>
            <b>{t.px}px</b><span class="cname">楷体 · 行高 1.75 · 字距 0.02em</span>
            <span class="claim">{t.claim}</span>
          </header>
          <div class="crop" style="width:{SHEET_W}px">
            <div
              class="band grow"
              style="background-image:url({SHEET});
                   background-size:{SHEET_W}px {SHEET_H}px;
                   background-position:0 0"
            >
              <div class="bodytype" style="font-size:{t.px}px">{BODY_SAMPLE}</div>
            </div>
          </div>
          <div class="ruler" aria-hidden="true">
            {#each RULER as i (i)}<i class:long={i % 5 === 0}></i>{/each}
          </div>
        </section>
      {/each}
    {/if}
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

      <!--
        `?ink=heavy` 用的加深 pass：RGB 压暗 + alpha 抬浓。
        刻意不走 fill-opacity —— 那是**笔迹数据**（.mark path 的 fill 会被
        CSS 兜底覆盖，见下面那条注释），把它塞进 CSS 就不再是「同一支笔的
        重按」，而是「另一支笔」。滤镜留在合成阶段，改的只是落墨深浅。
      -->
      <filter
        id="ink-deepen"
        x="-30%"
        y="-30%"
        width="160%"
        height="160%"
        color-interpolation-filters="sRGB"
      >
        <feColorMatrix
          type="matrix"
          values="0.72 0 0 0 0.10  0 0.72 0 0 0.06  0 0 0.72 0 0.04  0 0 0 1.35 0"
        />
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
  /* `?only=b` 时 A 段被摘掉，这段顶距就变成了孤儿空白。 */
  .rack h1.gap:first-child {
    margin-top: 0;
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

  /* ── `?mark=overlay`：标记盖在字上，而不是悬在字上方 ──
   *
   *  markslot 保留 40px 的占位（它撑着三行对齐，也决定字落在 y=40..56，
   *  **字心 = 48**）。只改标记自己的 top，让**笔心**落到字心上。
   *
   *  ⚠ 这三个 top 是 2026-10-13 从 `ui/src/lib/marks.js` 的 path 坐标
   *  **量出来的**，不是估的（笔心 = 笔迹 bbox 中心）：
   *      TICK_NESTED     bbox x 14.0~42.0  y 5.2~36.4  → 心 (28.0, 20.8)
   *      RING_MARK       bbox x 15.5~48.2  y 3.6~37.1  → 心 (31.9, 20.4)
   *      RING_LABEL_MARK bbox x 6.3~53.9   y 10.9~23.3 → 心 (30.1, 17.1)
   *  top = 48 − 笔心 y。
   *
   *  旧版 `.strip .mark { top: 0 }` 让勾的笔心落在 y=20，也就是**偏上
   *  28px**——「标记在字上方」不是设计，是没人量过摆放。marks.js 的注释
   *  （圈字环「隔着空隙去包住字，太轻圈不住」）写明它本来就是要盖住字的。
   */
  .desk[data-mark='overlay'] .strip .mark {
    top: 27px;
  }
  .desk[data-mark='overlay'] .strip .mark.ring {
    top: 28px;
  }
  .desk[data-mark='overlay'] .strip .mark.encircle {
    top: 31px;
  }

  /*  水平方向：`left:50% + margin-left:-30px` 把**取景框**中心对准格心，
   *  但要对准的是**笔心**。两者不等，于是各支笔的笔心天然带着自身偏移：
   *      TICK   笔心 x 28.0（框心 30）→ 偏左 2
   *      RING   笔心 x 31.9（框心 30）→ 偏右 2
   *  实测 5× 裁图后补齐：勾再右移 9，独立圈左移 4；圈字环不动（它本来就是
   *  横长环，左右本该盖满字，实测已对）。
   */
  .desk[data-mark='overlay'] .strip .mark.tick {
    margin-left: -21px;
  }
  .desk[data-mark='overlay'] .strip .mark.ring {
    margin-left: -34px;
  }
  /*  笔在字之上：两个都要 position，markslot 是 static 的，
   *  z-index 给了也不生效——这个坑跟面板那套 ACL 少一条权限一个性质。 */
  .desk[data-mark='overlay'] .strip .mark {
    z-index: 2;
  }
  .desk[data-mark='overlay'] .name {
    position: relative;
    z-index: 1;
  }

  /*  `?ink=heavy`：叠一层加深的 pass。
   *  P5 是五层 0.22→0.93 的嵌套，单独看是**淡痕**——那是 A 段验证过
   *  的笔迹形态，但盖章要的是重墨。加深走 feColorMatrix 的线性偏暗，
   *  而不是改 fill-opacity：后者会被 `.mark path` 的 fill 兜底吃掉。
   *
   *  **不缩放**：摆正之后笔迹原尺寸就对上了字——勾 28px 宽约等于 2 倍
   *  字高，圈字环 48px 约 1.45 倍字宽，与用户 2026-10-13 手绘参考同量级。
   *  之前试过的 scale(1.55) 是在掩盖 28px 的错位，撤掉。
   */
  .desk[data-ink='heavy'] .strip .mark {
    filter: url(#pencil) url(#ink-deepen);
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

  /*  `grow`：**不写死高度**，让纸带跟着内容长。
   *  写死 height 的版本是按「我估的行数」算的，结果三档全裁了末行
   *  （19/21 实际 4 行、23 实际 5 行，而我标的是 3/3/4）。
   *  高度一旦由估算值推导，标注的数字和看到的东西就会各说各话——
   *  量具不能自己喂自己数据。
   */
  .band.grow {
    height: auto;
    padding: 14px 0 18px;
  }

  /* ── C 段：正文字号（票 22） ──
   *
   *  行高 1.75 与字距 0.02em 在三档之间**锁死**：字号是唯一变量。
   *  手写体比印刷体需要更松的行距——1.75 是楷体的常规行距下限，
   *  再紧字与字的连带就糊成一片（票 24 要量的就是这类下限）。
   */
  .bodytype {
    padding: 0 16px;
    font-family: KaiTi, STKaiti, '楷体', serif;
    line-height: 1.75;
    letter-spacing: 0.02em;
    color: var(--cs-pencil);
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
