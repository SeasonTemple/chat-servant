<script lang="ts">
  /*
   *  新面板。**从零建的，不是旧面板改的。**
   *
   *  旧面板（`(panel)/`）已冻结：它的旧正文排版、旧按钮、旧窗口控制
   *  都被否定了，在那个容器里做任何新设计判断都会被污染。
   *
   *  这里已经有四样东西：
   *    · 一张撕边的纸（真实素材，撕口即顶边）
   *    · 顶边的档位区
   *    · 归属行（这句是替谁写的 + 对方最后一句原文）
   *    · 正文行（楷体 21px / 行高 1.5，四态：已写 / 在写 / 失败 / 空）
   *
   *  ⚠ **动作行故意还没有。** 它要碰发送路径（硬约束第 5 条），不能跟排版
   *  一起顺手做掉。宁可少一行，不要一个没验过的发送控件。
   *
   *  ── 档位区为什么长这样 ────────────────────────────────────
   *
   *  三道短记号是**中性**的，不编码任何东西。抄写员把三档写在纸上，
   *  当前那一档被红铅笔勾上。所以：
   *    · 位置  = 档位集合（配合铅笔小字，不靠猜）
   *    · 勾    = 当前在哪
   *  「勾」还是「圈」**已定案**（设计规范 §10）：做成用户外观偏好，
   *  两支笔各配三档幅面，`?shape=` 与 `?size=` 两个探针分开选。
   *
   *  笔迹几何来自 `$lib/marks`（由 `ui/scripts/pencilgen.py`
   *  按「中心线 + 宽度剖面」生成），不是手写 path——手写过一次收放
   *  方向反了。理由与量测都记在那边。
   */
  import {
    TICK_NESTED,
    TICK_NESTED_MID,
    TICK_NESTED_BIG,
    RING_LABEL_MARK,
    RING_LABEL_MARK_BIG,
    RING_LABEL_MARK_HUGE,
    MARK_VIEW,
    RING_VIEW,
    TICK_VIEW,
  } from '$lib/marks';
  import { markShape } from '$lib/mark-shape.svelte';
  import type { ReadoutPayload, Message, Candidate } from '$lib/types';
  import { IS_DEMO } from '$lib/runtime';
  import { onMount } from 'svelte';

  /**
   * 标记的**幅面档**。跟「勾还是圈」正交：勾也有大小，圈也有大小。
   * `?shape=tick|ring` 选笔，`?size=base|mid|big` 选幅面，都是探针不落盘。
   *
   * 默认 `big` 是量出来的，不是拍的（`target/probe/mark-size-compare.png`）：
   *   圈  base 52.10×16.40 → 扁椭圆上沿压在字头上，读成「框」；big 74.20×28.60
   *      才读成「盖了一个章」💭 判据是圈高/字高：<1.5 读成边框，>2.5 才是印章。
   *   勾  base 29.10×30.80 相对放大后的圈显得单薄，且**偏右 6px**——
   *      旧值 `margin-left:-21px` 是观感试出来的，笔心其实落在槽心右侧 6px。
   *      现全部改为按笔心反算，与圈同一套公式。
   * 要回退就把默认值改回 `base`。
   */
  type MarkSize = 'base' | 'mid' | 'big';
  let markSize = $state<MarkSize>('big');

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

  /* ── 正文数据 ──────────────────────────────────────────────────
   *
   *  Rust 是唯一事实来源：阶段由 `Phase` 的判别字段决定，界面**不自己从
   *  candidates 的长度反推**（那是旧面板犯过的错——同一个判断在三处各写
   *  一遍、其中两处还写得不一样）。
   *
   *  `?demo=1&state=<名字>` 读 `ui/static/demo/<名字>.json`，让正文排版
   *  能在浏览器里全速迭代，不必每次都去点微信。演示数据不含真实聊天内容。
   */
  let readout = $state<ReadoutPayload | null>(null);
  let peer = $state('');

  const lastFromThem = $derived(
    readout?.messages.filter((m) => m.speaker === 'Them').at(-1) ?? null,
  );
  const drafted = $derived(readout?.phase === 'drafted' ? (readout.candidates[0] ?? null) : null);

  onMount(() => {
    /*
     * 标记形状落在 Rust 侧的 prefs，这里只读镜像。
     *
     * ⚠ `?shape=` / `?size=` 都是**探针口**，与上面 `?tier=` 同一性质：方便把
     * 各档各截一张 1× 图对着比。它们在演示/静态页里只改内存镜像
     * （`markShape.set` 见 `HAS_TAURI` 分支），**不落盘**，
     * 所以拿它做实验不会把真机设置改掉。
     */
    const q = new URLSearchParams(location.search);
    const shapeProbe = q.get('shape');
    const sizeProbe = q.get('size');
    const sizes: MarkSize[] = ['base', 'mid', 'big'];

    if (sizeProbe !== null && sizes.includes(sizeProbe as MarkSize)) {
      markSize = sizeProbe as MarkSize;
    }
    if (shapeProbe === 'tick' || shapeProbe === 'ring') {
      void markShape.set(shapeProbe);
    } else if (markSize !== 'big') {
      // 只给了幅面没给形状时，至少保证渲染的是「圈」，否则会以为探针没生效
      void markShape.set('ring');
    } else {
      void markShape.load();
    }

    // 演示数据在真实 Tauri 里永远不装载。理由见 `loadDemo` 的注释。
    if (!IS_DEMO) return;
    void loadDemo();
  });

  /**
   * 演示数据。`?demo=1&state=<名字>` → `ui/static/demo/<名字>.json`。
   *
   * ⚠ **尊重数据里写的 `phase`，别自己从 candidates 推。** 旧面板就是从
   *   `candidates.length === 0` 反推阶段的，于是「静默 / 出错 / 刚启动」
   *   三种完全不同的状态在同一个条件下坍成一种——设计评审看不见的状态，
   *   等于没设计它。
   */
  async function loadDemo() {
    const state = new URLSearchParams(location.search).get('state') ?? 'drafted';
    const r = (await fetch(`/demo/${state}.json`)
      .then((x) => x.json())
      .catch(() => fetch('/demo/drafted.json').then((x) => x.json()))) as {
      phase?: string;
      peer?: string;
      messages?: Message[];
      candidates?: Candidate[];
      error?: string | null;
      status_hint?: string | null;
    };
    const msgs = r.messages ?? [];
    const cands = r.candidates ?? [];
    const hint = r.status_hint ?? null;
    readout =
      r.phase === 'drafting'
        ? { phase: 'drafting', messages: msgs, status_hint: hint }
        : r.phase === 'failed'
          ? { phase: 'failed', messages: msgs, error: r.error ?? '未知原因' }
          : r.phase === 'drafted'
            ? { phase: 'drafted', messages: msgs, candidates: cands }
            : { phase: 'read', messages: msgs };
    peer = r.peer ?? '';
  }

  /*
   * 勾还是圈 —— **用户的选择，不是设计定论**（2026-10-13）。
   *
   * 两支笔在 `marks.js` 里本来就都生成好了，切换只是换一组 path；
   * 笔性也一致：都是「中心线 + 宽度剖面」算出来的轮廓。语义分工：
   * 勾 = 「这一档被批了」，圈字环 = 「这一档被圈中了」，
   * 形状差异在 1:1 下比粗细差异更容易读出来。
   *
   * ⚠ 取景框**不是两支笔共用的**。根 `<svg>` 的 overflow 是 hidden，
   * 放大的档一旦画到 60×40 之外就被静默裁掉，表现为「放大了但还是没变大」——
   * 那是量具自己骗自己。所以放大的档各自配一个大 VIEW，见 `ACTIVE_VIEW`。
   */
  const MARK_LAYERS = $derived(
    markShape.current !== 'ring'
      ? markSize === 'mid'
        ? TICK_NESTED_MID
        : markSize === 'big'
          ? TICK_NESTED_BIG
          : TICK_NESTED
      : markSize === 'mid'
        ? RING_LABEL_MARK_BIG
        : markSize === 'big'
          ? RING_LABEL_MARK_HUGE
          : RING_LABEL_MARK,
  );

  /** 放大档放不进 60×40，得换框；base 两支笔共用 MARK_VIEW。 */
  const ACTIVE_VIEW = $derived(
    markSize === 'big'
      ? markShape.current === 'ring'
        ? RING_VIEW
        : TICK_VIEW
      : markSize === 'mid'
        ? markShape.current === 'ring'
          ? MARK_VIEW
          : TICK_VIEW
        : MARK_VIEW,
  );
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
              data-ring={markSize}
              viewBox="0 0 {ACTIVE_VIEW.w} {ACTIVE_VIEW.h}"
              width={ACTIVE_VIEW.w}
              height={ACTIVE_VIEW.h}
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
    正文区。四行固定骨架（§4.2b）：档位 → 归属 → 正文 → 动作。
    动作行**故意还没有**——它要碰发送路径（硬约束第 5 条），不能跟排版
    一起顺手做掉。宁可少一行，不要一个没验过的发送控件。
  -->
  <div class="cs-body-col">
    <!--
      归属行。回答「这句话是替谁写的」。
      ⚠ 证据**必须显示**：这是机器认出来的对话，不是用户自己翻的。
      认错了还照发，比不写更糟。
    -->
    <p class="cs-where">
      <span class="cs-peer">{peer || '—'}</span>
      {#if lastFromThem}
        <span class="cs-where-quote">{lastFromThem.text}</span>
      {/if}
    </p>

    <!--
      正文行。**只有墨色**：`--cs-mark` 在真实纸底上只有 3.00 : 1，掉出 AA，
      正文小字用它就是不合规（app.css 的注释记着这条是怎么翻的）。
    -->
    <div class="cs-draft">
      {#if drafted}
        <p class="cs-draft-text">{drafted.text}</p>
      {:else if readout?.phase === 'drafting'}
        <!--
          这句是 Rust 的 `Phase::status_hint()`，**不在这里写死**。
          判据 12：空状态必须给明确的下一步——「正在写……」只说了现状，
          Rust 那句连着说了「通常几秒」，用户才知道该等。夹具没带这个字段时
          才退回短句，那只是演示数据的缺口，不是口径。
        -->
        <p class="cs-draft-text cs-draft-pending">
          {readout.status_hint ?? '正在写……'}
        </p>
      {:else if readout?.phase === 'failed'}
        <p class="cs-draft-text cs-draft-failed">没写成：{readout.error}</p>
      {:else}
        <p class="cs-draft-text cs-draft-idle">对方说完话，这里才落笔。</p>
      {/if}
    </div>
  </div>
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
    /*  ⚠ 必须留：标记**盖在标签上**（§10 盖章式），所以笔的下垂段必然
     *  伸到槽底之外。按 CSS 几何推：槽高 = markbox 40 + margin 2 + 11px 标签
     *  = 53，而最大的标记（勾 big，取景框 72×58，top 17）底边在 17+58 = 75
     *  ——伸到槽底**以下 22px**。早先这里没间距，归属行直接被压中，
     *  实测截图（380×560 取景框）里红笔横穿「示例联系人」。
     *  面板下半还有大片空白，28px 代价很低。
     */
    margin-bottom: 28px;
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
   *  摆放偏移**全部按笔心反算**，不靠观感试：
   *      top         = 字心y - 1.1 - 笔心y
   *      margin-left = -笔心x
   *  字心y = markbox 40 + 标签 margin 2 + 字号 11 ÷ 2 = 47.5；-1.1 是现状圈
   *  对齐字心时本来就带着的那点偏上。笔心 = `marks.js` 里 path 的 bbox 中心。
   *
   *  ⚠ 这套公式替换掉过一个**试出来的**值：勾原来是 `margin-left:-21px`，
   *    注释写着「要右移 9px 才盖得住三个字」，但公式算出该是 -27——
   *    旧值把笔心推到了槽心**右侧 6px**。标一个东西却没对准它。
   *    6× 真像素上看得见：右上那截挑脚伸出「入」外面。
   *
   *  交叉验证：公式代回状态圈算出 (-30.7, 30.0)，与已上线的 -30 / 30 吻合。
   */
  .cs-tick {
    position: absolute;
    left: 50%;
    top: 26px;
    margin-left: -27px;
    opacity: 0;
    transition: opacity var(--cs-dur) var(--cs-ease);
  }

  .cs-tick[data-shape='ring'] {
    top: 30px;
    margin-left: -31px;
  }

  /*  放大档换了取景框，笔心坐标随之改变，偏移**重算**而不是在旧值上加减。
   *  实测（解析 marks.js 的 path 坐标，非估算）：mid 笔心 (27.70, 17.50)、
   *  big 笔心 (27.75, 17.50)——两档同框同高，偏移相同。代回公式
   *  top = 46.4 - 笔心y = 29，margin-left = -笔心x = -28。 */
  .cs-tick[data-shape='tick'][data-ring='mid'],
  .cs-tick[data-shape='tick'][data-ring='big'] {
    top: 29px;
    margin-left: -28px;
  }

  .cs-tick[data-shape='ring'][data-ring='big'] {
    top: 21px;
    margin-left: -43px;
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

  /* ── 归属行 + 正文行 ──────────────────────────────────────────
   *
   *  内容栏 316px。写成 `max-width` 而不是 `width`，316 才 grep 得到——
   * 排版的第一变量是栏宽，栏宽错了后面量出的每个数字都错。
   */
  .cs-body-col {
    max-width: 316px;
    margin: 0 auto;
    padding: 0 0 24px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }

  .cs-where {
    margin: 0;
    font-size: 12px;
    line-height: 1.45;
    color: var(--cs-pencil);
  }

  .cs-peer {
    /* 机器认出来的名字。**铅笔灰**，跟正文那支墨笔分开——
       墨 = 你写下的，铅笔 = 机器认出来的（app.css 的分工）。 */
    font-weight: 600;
    color: var(--cs-pencil-2);
  }

  .cs-where-quote {
    display: block;
    margin-top: 3px;
    color: var(--cs-pencil);
  }

  /*  正文。楷体 21px / 行高 1.5（§4.2a 定案）。
   *  ⚠ 21px 是定案值，换之前先看 §4.2a 那条取舍：19px 是唯一保持 4 行的
   *    档，但 48.4% 的笔画 ≤1px 会断笔——**断笔 = 读不出来**。
   */
  .cs-draft-text {
    margin: 0;
    font-family: KaiTi, STKaiti, '楷体', serif;
    font-size: 21px;
    line-height: 1.5;
    color: var(--cs-ink);
    letter-spacing: 0.01em;
    /* 正文是**要选中复制**的（用户可能想改），与面板整体 user-select:none
       不同——那是给档位控件设的。 */
    user-select: text;
  }

  .cs-draft-pending,
  .cs-draft-idle {
    /* 还没落笔的时候没有「墨」可说。用铅笔灰，且字号不缩——缩了会让
       真落笔时正文往下跳一格。 */
    color: var(--cs-pencil);
  }

  .cs-draft-failed {
    /* 红只给不可逆（app.css 文件头）。出错不算不可逆，所以**不用红**，
       沿用铅笔灰 + 一句人话。 */
    color: var(--cs-pencil-2);
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
