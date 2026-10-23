<script lang="ts">
  import { page } from '$app/state';
  import { TICK_PATH, TICK_NESTED, RING_MARK, RING_LABEL_MARK, MARK_VIEW } from '$lib/marks';
  import PencilDefs from '$lib/PencilDefs.svelte';

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
   *  顺带记一条反直觉的：盖住字**不会**读成删除线。删除线是**一条横线
   *  穿过字中部**，圈是**包围**，拓扑不同。2026-10-13 我把「环与字交叠」
   *  当成 R2 的缺陷判掉，那是拿形状交叠去推语义的错判。
   */
  const MARK_MODE = $derived(page.url.searchParams.get('mark') ?? 'above');
  const INK_MODE = $derived(page.url.searchParams.get('ink') ?? 'light');

  /* ── C 段（票 22）的正文样本与三档字号 ──
   *
   *  样本刻意选成**会换行三次以上**的中性句子：只排两行的话，三档字号的
   *  行长差别会被纸张下沿吃掉一半，看不出来。
   *  不含任何真实聊天内容——`ui/static` 可能被部署到公开 URL。
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
    { px: 19, perLine: 16, lines: 4, claim: '实测 16 字/行 · 4 行 · 细笔画 48.4%' },
    { px: 21, perLine: 14, lines: 5, claim: '实测 14 字/行 · 5 行 · 细笔画 38.1%' },
    { px: 23, perLine: 13, lines: 5, claim: '实测 13 字/行 · 5 行 · 细笔画 25.2%' },
  ];

  /* ── D 段（票 24）的面板小字：每档单独成行 ──
   *
   *  这里量的是**面板里真正在用的小字**，不是正文（正文在 C 段）。
   *  两个待测元素与它们在 `PaperPanel.svelte` 里的样式逐条对应：
   *
   *  | 台上的类        | 真出处                       | 现在 |
   *  |-----------------|------------------------------|------|
   *  | `.small-name`   | `.cs-pick-name`（档位名）     | 11px |
   *  | `.small-where`  | `.cs-where`（归属行）        | 12px |
   *
   *  **改这两个类时必须同步改这里**，否则量的是一个不存在的界面。
   *  下面每条声明后面都注了出处行号。
   *
   *  ## 为什么必须「每档单独成行」
   *
   *  票 24 首轮数据作废的原因：字号档的文本行切分把汉字行误并进了等宽行——
   *  一个 sheet 里混排多种行，行切分按「比纸暗」判定，等宽行的墨也进了掩码，
   *  切出来的行于是横跨两种行。⇒ **一个 sheet 只放一个字号**，
   *  量具的掩码才只可能落在一行文字上。
   */
  // ⚠ 2026-10-23 加了 **13**：票 24 收尾时人眼说「12px 开始费劲看不清晰」
  // ⇒ 「下限抬到 13px 够不够」是当时**唯一没验过的那一档**。
  // 量具那边不受影响（它一次只出一档，自己传 `?small=N`）。
  const SMALL_SCALES = [10.5, 11, 12, 13];

  /** 归属行样本。假内容，与 `static/demo/*.json` 同一规矩。 */
  const SMALL_WHERE_SAMPLE = '示例联系人';
  const SMALL_QUOTE_SAMPLE = '设成 30s 是不是太短了';
  /** 档位名样本：两个都是面板里真实出现的档位名。 */
  const SMALL_NAME_SAMPLE = '只填入';

  /* 只出**一个**字号（`?small=11`）。默认出全部三档。
   *
   * 这是为票 24 的量具准备的，不是给人看的：一次只出一档，
   *   量具的掩码就不可能同时吃到两种字号——票 24 首轮就是这么作废的。
   */
  const SMALL_ONE = $derived.by(() => {
    const v = page.url.searchParams.get('small');
    if (v === null) return null;
    const n = Number(v);
    return SMALL_SCALES.includes(n) ? n : null;
  });
  const SMALL_VISIBLE = $derived(
    SMALL_ONE === null ? SMALL_SCALES : SMALL_SCALES.filter((p) => p === SMALL_ONE),
  );

  /* 纸底用实色还是纸纹位图（`?paper=solid`）。
   *
   * **默认纯色，这是量具的选择，不是偷懒**：
   * 纸纹位图里有暗折痕，它比墨阈值还暗，会把一行的包围盒撑宽
   * （实测「示例联系人」那一行被撑到 229 CSS px，真值约 60，
   * 字面率从 27% 掉到 9%，而表看起来完全正常）。
   * 折痕与 11px 的竖笔同形、密度也同形，按形状与密度都剔不掉。
   *
   * ⇒ 量**字**的三个量（字面率 / 细笔画 / 墨高比）必须在干净底上量。
   * 纸纹只会让细笔画**更容易发虚**，方向单向——所以纯色底上量出的
   * 细笔画占比是**乐观下界**，真机只会更差。这条与票 22 正文那套
   * （仿宋 75.9% 就是在纯色底上量的）同一条纪律。
   * `?paper=texture` 保留，用于肉眼看纸纹上到底糊成什么样。
   */
  const PAPER_SOLID = $derived(page.url.searchParams.get('paper') === 'solid');
  const PAPER_CSS = $derived(PAPER_SOLID ? 'rgb(243 230 192)' : `url(${SHEET})`);

  /* 字重探针（`?weight=600`）。默认 **600**。
   *
   ***默认值必须跟着 `PaperPanel.svelte` 走**：2026-10-14 票 24 已把
   * `.cs-where-quote` 与 `.cs-pick-name` 定为 **600**（11~12px 下常规字重的
   * ≤1px 细笔画是 75~79%，600 是 15~18%）。这里默认值不同步的话，
   * 量具就在量一个**不存在的界面**——而它照样报出一张正常的表。
   *
   * 想看 400 的数据要显式写 `?weight=400`；**500 在本机等于 400**
   *（雅黑没有 500 字重，浏览器渲成 400，两档逐字节相同）。
   *
   * 为什么需要这个开关：首轮数据量出来之后冒出一个**反直觉**的结果——
   * 同一档字号下，`.cs-peer`（本来就是 600）的 ≤1px 细笔画只有 6~14%，
   * 而常规字重的引文与档位名是 71~82%。⇒ 断笔的杠杆很可能是
   * **字重**而不是字号。这条只能量，不能推。
   */
  const SMALL_WEIGHT = $derived.by(() => {
    const v = page.url.searchParams.get('weight');
    const n = v === null ? 600 : Number(v);
    return [300, 400, 500, 600, 700].includes(n) ? n : 600;
  });

  /* ── E 段（票 35）的字号阶梯 ──
   *
   *  票 35 要的是**同一条正文在多个字号下的并排**（默认 21/17/14/12/11），
   *  用来给「小字到底糊不糊」补一个**阳性对照**——
   *  量具能给出数，但「读得出 / 明显糊」是人的判断，两边缺一不可。
   *
   *  ⚠ **每档单独一张 sheet**（票 24 的教训）：一个 sheet 混排多种字号，
   *  行切分会把两种字号的墨并进同一个掩码 ⇒ 量出来的不是那个字号的东西。
   *
   *  ⚠ **字体可配，但「静默回落」是已知陷阱**（`app.css` 里记着）：
   *  `?draft=lxgw` 在没装字体的机器上会静默回落成楷体，
   *  图看着「只是字体没生效」，不比对就会误判成「这个字体效果不行」。
   *  ⇒ 本段只负责把**想要的**字体写进去；
   *  出图脚本必须**同时报出实际生效的字体**（`document.fonts.check`）。
   */
  const LADDER_SIZES = [21, 17, 14, 12, 11];

  const LADDER_FONTS = {
    fangsong: { name: '仿宋', stack: "FangSong, STFangsong, '仿宋', serif" },
    song: { name: '宋体', stack: "SimSun, '宋体', serif" },
    kai: { name: '楷体', stack: "KaiTi, STKaiti, '楷体', serif" },
  };

  const LADDER_VISIBLE = $derived.by(() => {
    const v = page.url.searchParams.get('sizes');
    if (v === null) return LADDER_SIZES;
    const list = v
      .split(',')
      .map((s) => Number(s.trim()))
      .filter((n) => Number.isFinite(n) && n >= 8 && n <= 48);
    return list.length ? list : LADDER_SIZES;
  });

  const LADDER_FONT = $derived.by(() => {
    const v = page.url.searchParams.get('font');
    // ⚠ `searchParams.get` 的类型是 `string | null` ⇒ **null 不能当索引**
    // （svelte-check 会报 `Type 'null' cannot be used as an index type`）。
    if (v !== null && v in LADDER_FONTS) {
      return LADDER_FONTS[v as keyof typeof LADDER_FONTS];
    }
    return LADDER_FONTS.fangsong;
  });

  const LADDER_WEIGHT = $derived.by(() => {
    const v = page.url.searchParams.get('weight');
    const n = v === null ? 400 : Number(v);
    return [300, 400, 500, 600, 700].includes(n) ? n : 400;
  });

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

<div class="desk" data-mark={MARK_MODE} data-ink={INK_MODE}>
  <!--
    **D 段（票 24）把参照纸摘掉**，否则量具会框错块。
    量具按「暖色连通块」定位要量的那块纸，而这页左侧常驻着一张
    380×560 的纸样参照——它也是暖色的、面积还更大。
    初版量到的就是这张**空白参照纸**：0 行字面墨、墨高÷字号 47.6，
    而那张表**看起来完全正常**。
    ⇒ 渲染时只有一个纸块，量具就不必猜。
  -->
  {#if only !== 'small' && only !== 'ladder'}
    <figure class="slot">
      <figcaption>纸 · 1:1 · 380 × 560</figcaption>
      <div class="sheet" style="width:{SHEET_W}px;height:{SHEET_H}px"></div>
    </figure>
  {/if}

  <div class="rack">
    {#if only !== 'b' && only !== 'type' && only !== 'small' && only !== 'ladder' && MARK_MODE === 'above'}
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

    {#if only !== 'type' && only !== 'small' && only !== 'ladder'}
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
                    每格**必须**留一个 40px 的 markslot，哪怕里面是空的。
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
      只变字号这一个变量，行高 1.75、字距 0.02em 三档锁死——否则比的是
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
        >：316px 正文栏里，19px 每行多塞几个字、21px 会不会多出一行——
        这直接决定正文骨架的高度预算够不够。
      </p>

      {#each TYPE_SCALES as t (t.px)}
        <section class="cand">
          <header>
            <b>{t.px}px</b><span class="cname">楷体 · 316px 栏 · 行高 1.5</span>
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

    {#if only === 'ladder'}
      <!--
      E 段 · 字号阶梯（票 35）。

      问的是「同一条正文缩到多小还读得出」。票 35 连否三个判据
      （≤1px 占比 / 碎片率 / 墨块实心度）——它们只量几何与拓扑，
      而 17px 这个尺度上的可读性不由它们决定。

      **每档单独一张 sheet**：混排会让行切分把两种字号的墨并进同一个掩码
      （票 24 首轮就是这么作废的）。

      样本是中性起草回复，**不含任何真实聊天内容**（演示页可能部署到公开 URL）。
      -->
      <h1 class="gap">E · 字号阶梯</h1>
      <p class="lede">
        同一条正文，按 <b>{LADDER_VISIBLE.join(' / ')}px</b> 排开，字体与字重锁死（
        {LADDER_FONT.name} · {LADDER_WEIGHT}）。 看的不是「哪个好看」，是<b>哪一档还读得出</b>——
        量具给数，<b>读不读得出要人看</b>。
      </p>

      {#each LADDER_VISIBLE as px (px)}
        <section class="cand">
          <header>
            <b>{px}px</b>
            <span class="cname"
              >{LADDER_FONT.name} · 316px 栏 · 行高 1.5 · 字重 {LADDER_WEIGHT}</span
            >
            <span class="claim">量具：谷底亮度 / 碎片率 / 细笔画墨块</span>
          </header>
          <div class="crop" data-ladder style="width:{SHEET_W}px">
            <div
              class="band grow"
              style="background:{PAPER_CSS};
                   background-size:{SHEET_W}px {SHEET_H}px;
                   background-position:0 0"
            >
              <div
                class="bodytype"
                style="font-size:{px}px;font-weight:{LADDER_WEIGHT}"
                style:font-family={LADDER_FONT.stack}
              >
                {BODY_SAMPLE}
              </div>
            </div>
          </div>
          <div class="ruler" aria-hidden="true">
            {#each RULER as i (i)}<i class:long={i % 5 === 0}></i>{/each}
          </div>
        </section>
      {/each}
    {/if}

    {#if only === 'small'}
      <!--
      D 段 · 面板小字（票 24）。

      问的是「面板上最小的字到底能不能读」，不是「正文多大」。
      三个候选 **10.5 / 11 / 12px**，**每个字号单独一张 sheet**——
      混排会让行切分把两种字号的墨并进同一个掩码（票 24 首轮就是这么作废的）。

      排版照抄 `PaperPanel.svelte` 的 `.cs-pick-name` / `.cs-where`，
      **含色值**：`--cs-pencil` = `rgb(76 90 99)`（在真实纸底上 5.10 : 1）。
      纸底用**真实纸材位图**，不是纯色——票 22 的仿宋风险就是这么来的：
      纯色纸上 75.9% 的 ≤1px 细笔画，落到有纤维的纸纹上只会更容易断。

      标注文案只报量出来的东西，不写观感结论。
    -->
      <h1 class="gap">D · 面板小字 10.5 / 11 / 12 / 13px</h1>
      <p class="lede">
        票 24 要回答的是<b>当前用的 11px 与 12px 站不站得住</b>， 以及要不要放 10.5、要不要抬到
        13。四档<b>每档单独成行</b>，字体、色值、行高、纸材 全部照抄 <code>PaperPanel.svelte</code> 的对应规则，只变字号一个变量。
      </p>

      {#each SMALL_VISIBLE as px (px)}
        <section class="cand">
          <header>
            <b>{px}px</b><span class="cname">雅黑 · 纸材实底 · 铅笔灰 rgb(76 90 99)</span>
            <span class="claim">量具：字面率 / ≤1px 细笔画 / 墨迹高÷字号</span>
          </header>
          <div class="crop" style="width:{SHEET_W}px">
            <div
              class="band grow"
              style="background:{PAPER_CSS};
                   background-size:{SHEET_W}px {SHEET_H}px;
                   background-position:0 0"
            >
              <div class="small-where" style="font-size:{px}px">
                <span class="small-peer" style="font-weight:{SMALL_WEIGHT}"
                  >{SMALL_WHERE_SAMPLE}</span
                >
                <span class="small-quote" style="font-weight:{SMALL_WEIGHT}"
                  >{SMALL_QUOTE_SAMPLE}</span
                >
              </div>
              <div class="small-name" style="font-size:{px}px;font-weight:{SMALL_WEIGHT}">
                {SMALL_NAME_SAMPLE}
              </div>
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
      `color-interpolation-filters="sRGB"` 不能省。SVG 滤镜默认走
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
      <PencilDefs />

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

  /*  绝对定位。`.band` 不建立 BFC（无 overflow / padding / border），
   *  子元素的 margin 会与父元素塌陷，把整块纸背景一起推下去。 */
  .mark {
    position: absolute;
    top: 8px;
    left: 100px;
  }

  /*  只有 fill，不设 fill-opacity——每一层的透明度是**笔迹数据的一部分**，
   *  由生成器算出来（嵌套的 0.22/0.24/0.26/0.30/0.36），CSS 一旦兜底
   *  就会把它们全覆盖掉。
   *
   *  ⚠ 2026-10-18：色值从字面量 `rgb(168 74 56)` 改成 `var(--cs-pencil-red)`。
   *   这一页**画在纸上**（`.mark` 绝对定位在带 `PAPER_CSS` 的 `.band` 里），
   *   与主面板档位行**同一张底**，而根 `+layout.svelte` 已经 import 了
   *   `app.css` ⇒ token 在这一页是可用的，写字面量没有理由。
   *   ⚠ 改法不是「换个写法」而是「少一处副本」：`--cs-pencil-red` 一改，
   *   这里跟着改；而字面量那一份只会在有人改 token 时**悄悄过期**——
   *   排版台于是开始展示一个产品里已经不存在的颜色。
   *   （这一页的其他深色（`#6b7278` 等）是**它自己那套对照台的底**，
   *   不属于产品色板，不动。） */
  .mark path {
    fill: var(--cs-pencil-red);
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
   *  这三个 top 是 2026-10-13 从 `ui/src/lib/marks.js` 的 path 坐标
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
   *  **栏宽与行高都取自 docs/设计规范.md，不是自己定的**：
   *      §4.2「重测环境：纯色纸底 rgb(247 233 194)，**内容栏 316px**」
   *      §4.3「**正文 19px 行高 1.5**」
   *  第一版这里错用了 380px 纸面减 padding = 348px、行高 1.75，于是报出
   *  「17/16/14 字每行」——比真实值每行多 2~3 字。**栏宽是排版的第一变量，
   *  它错了，后面量出来的每一个数字都是错的。**
   *
   *  316px 是纸面 380 里的正文栏，所以左右各留 32px；
   *  写成 max-width 而不是 padding，是为了让「316」这个数在代码里可见，
   *  改动时能被 grep 到——padding 算出来的那两个 32 不容易被发现是设计值。
   *
   *  行高 1.5：手写体笔画细，1.5 已是最松裕的下限，再紧字与字连带就糊。
   */
  .bodytype {
    max-width: 316px;
    margin: 0 auto;
    font-family: KaiTi, STKaiti, '楷体', serif;
    line-height: 1.5;
    letter-spacing: 0.02em;
    color: var(--cs-pencil);
  }

  /* ── D 段：面板小字（票 24） ──
   *
   * **这里的每一条都要跟着 `PaperPanel.svelte` 走**，出处注在行尾。
   *   两边不同步的话，量的是一个不存在的界面——而量具全绿。
   *
   * 为什么照抄行高与字距：判据量的是**断笔**（≤1px 的笔画占比），
   * 而断笔既由字号决定、也由栅格化时的行距决定。只变字号一个变量，
   * 三个候选之间才可比。
   */
  .small-where {
    /* `.cs-where`：PaperPanel 的 line-height 1.45 + --cs-pencil */
    max-width: 316px;
    margin: 0 auto;
    line-height: 1.45;
    color: var(--cs-pencil);
  }
  .small-peer {
    /* 这里**不能**照抄 `.cs-peer` 的 `font-weight: 600`（2026-10-14 修正）。
     *
     *  `.cs-peer` 自己写的是 600，但它的默认呈现是 `data-peer-look='tracked'`
     *  （PaperPanel.svelte:221 的 `peerLook` 初值），而 `tracked` 那条规则把
     *  字重**改回 400**、改用 0.18em 字距扛层级。所以真实界面上名字是
     *  **400 + 疏排**，不是 600。
     *
     *  照抄 600 的后果是量具把「名字 600 + 引文 600」量了一遍——那是
     *  **一个界面上不存在的组合**，得出来的「三行同粗、层级靠颜色」结论
     *  也就跟着失效。量具抄规则要抄**最终值**，不是抄被覆盖的那一行。
     *  同类教训：`ui/scripts/analysetype.py` 头号坑（量具量的是别的东西）。
     *
     * 字重走 `SMALL_WEIGHT` 是因为「名字要不要也加粗」与「引文要不要加粗」
     * 是**两个独立决策**却共用一个杠杆；这里先让三行共用，好让一屏里
     * 直接比出「全 600」和「全 400」两个整档。要单独调名字再加参数。 */
    letter-spacing: 0.18em;
    color: var(--cs-pencil-2);
  }
  .small-quote {
    /* `.cs-where-quote`：display block + margin-top 3px + --cs-pencil */
    display: block;
    margin-top: 3px;
    color: var(--cs-pencil);
  }
  .small-name {
    /* `.cs-pick-name`：line-height 1 + letter-spacing .02em + --cs-pencil。
     * 上下留白是为了让行切分（按「比纸暗」判定）切得开——票 24 首轮
     * 切错行就是因为两行墨挨得太近。 */
    max-width: 316px;
    margin: 26px auto 0;
    line-height: 1;
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
