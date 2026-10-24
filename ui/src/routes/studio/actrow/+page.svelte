<!--
  票 44：候选行动行到底该长成什么样。

  上一版（2026-10-15 早）的错，**读这一段之前先看**：
  那一版出了「现状 / A 不是按钮 / B 框贴合文字宽 / C 拆成两行」四栏，
  看起来是四个方案，**其实全都在同一个维度里打转**——黑边框去不去、
  侧边竖笔留不留。用户原话：「你就这么喜欢在一个纸布上画黑色边框+
  黑色侧边？」

  病根有两个：
  1. **没读规范就动手。** §4.2b v2 早就否决过「按钮拿现成色块当零件」
     （纯墨底/纯红底是 Web 默认按钮语言），而 §4.2c 定的按钮族恰恰是
     「radius 4 + 13px 600 + 全是 1px 描边」——**规范自己两节打架**。
     票 44 的根因就在这个交叉点上，而我绕着它想了三轮边框。
  2. **没想过这东西在纸上是什么。** 撕边纸 + 手绘笔迹 + 留白分隔，
     这套语言里根本没有「按钮」这个零件。而项目里**已经有**一支真的
     手绘勾（TICK_NESTED，五层嵌套 + 红铅笔滤镜），我却一直用 CSS 画
     3px 硬边竖笔冒充。

  这一版的三栏是**三个真正不同的方向**，不是边框的排列组合。
-->
<script lang="ts">
  import { TICK_NESTED, TICK_VIEW } from '$lib/marks';
  import PencilDefs from '$lib/PencilDefs.svelte';

  /*
   * 样本与上一版相同，刻意挑成**会换行两三行**的中性句子。
   * 单行短句会让三种方案看起来一样——那正是「用短样例比排版」的老毛病。
   *
   * ⚠ 2026-10-24：三条里曾经有**真会话里的原话** ⇒ 整组换成等长合成样本。
   * 硬约束第 3 条：示例代码里不写真实聊天内容（`ui/static` 可能被部署到
   * 公开 URL）；「当时以为是编的」不构成理由。
   * 长度与标点位置逐条对齐旧样本（27 / 24 / 10 字），排版比较不受影响。
   */
  const CANDS = [
    { tag: '按这个', prob: 100, text: '版面那版稿子我改挪到下周五了，麻烦你把行宽行数标一下。' },
    { tag: '备选 2', prob: 46, text: '样张改完了，字号先定、你照新版本再排一次给我看。' },
    { tag: '备选 3', prob: 21, text: '行宽，我看一下排版。' },
  ];

  /*
   * `marks.js` 导出的 `TICK_NESTED` / `CROSS_PATHS` **已经是分层结构**
   * `[{ d, o }]`（`PaperPanel.svelte` 直接当 `MARK_LAYERS` 用）。
   * 别再自己 map 一遍重新算 o —— 那一层的 opacity 是生成器按羽化剖面
   * 定出来的（§3.1/§3.2），覆盖它等于把五层压回单层平涂。
   */
</script>

<div class="cs-ar">
  <!-- ── 现状 ──────────────────────────────────────────────────── -->
  <section class="cs-ar-col">
    <h2>现状 · 一排描边按钮</h2>
    <p class="cs-ar-why">
      通栏四边闭合的框，里面躺一句<strong>要读</strong>的话。左边一道 3px 硬边竖笔。 竖笔是 CSS
      画的直边，它在纸上像一道划痕，而项目里真正的笔是手绘的。
    </p>
    <div class="cs-ar-sheet">
      <div class="cs-ar-stack">
        {#each CANDS as c, i (c.tag)}
          <button class="cs-act {i === 0 ? 'is-lead' : ''}">
            <span class="cs-act-tag">{c.tag}</span>
            <span class="cs-act-text">{c.text}</span>
            <span class="cs-act-prob">{c.prob}%</span>
          </button>
        {/each}
      </div>
    </div>
    <p class="cs-ar-note">⛔ 被否：这是 Web 控件语言落进纸面。§4.2b v2 早已否过「色块当零件」。</p>
  </section>

  <!-- ── 方向一：草稿三行 + 手绘记号 ────────────────────────────── -->
  <section class="cs-ar-col">
    <h2>一 · 草稿三行 + 手绘记号</h2>
    <p class="cs-ar-why">
      候选不是控件，是<strong>助手写在纸上的三行草稿</strong>。零边框、零竖笔、零色块； 往下推开，靠<strong
        >留白</strong
      >分隔（§5.5：<code>--cs-rule</code> 在暖黄纸上等于隐形）。 推荐由一枚<strong>真手绘勾</strong
      >承担——项目里早就有这一支。
    </p>
    <div class="cs-ar-sheet">
      <div class="cs-ar-stack">
        {#each CANDS as c, i (c.tag)}
          <div class="cs-draft">
            <span class="cs-draft-box">
              {#if i === 0}
                <!--
                  `TICK_VIEW` 是 56×34，那是**档位选择器**那一行的尺寸 ——
                  它独占一整行。而在 380px 的内容栏里 56px 占 15%，
                  一支勾读成「图标」而不是「批注」。
                  ⇒ 缩到 26px 宽：够认出手绘的收放，又退回到批注的分量。
                  viewBox 不变，所以轮廓数据一个字节没动。
                -->
                <svg
                  class="cs-pencil-mark"
                  viewBox="0 0 {TICK_VIEW.w} {TICK_VIEW.h}"
                  width="26"
                  height="16"
                  aria-hidden="true"
                  filter="url(#pencil)"
                >
                  {#each TICK_NESTED as l (l.d)}<path d={l.d} fill-opacity={l.o} />{/each}
                </svg>
              {/if}
            </span>
            <p class="cs-draft-body">{c.text}</p>
            <span class="cs-draft-prob">{c.prob}%</span>
          </div>
        {/each}
      </div>
    </div>
    <p class="cs-ar-note">
      ✓ 分工成立且更清楚：<strong>墨线档没有载体了</strong>（去框的代价）， 变体收敛成「有无手绘勾 /
      勾什么颜色」两轴。
    </p>
  </section>

  <!-- ── 方向三：裁剪纸条 + 抬起动效 ─────────────────────────────── -->
  <section class="cs-ar-col">
    <h2>三 · 裁剪纸条</h2>
    <p class="cs-ar-why">
      每条候选是<strong>从这张纸上裁下来的一条</strong>。边是刀口不是描边，
      厚度来自纸芯密度与那一线投影。中间那块可以拿起来 ——
      <strong>图里第一条就是抬起状态</strong>，动效不是承诺，是画出来的。
    </p>
    <div class="cs-ar-sheet">
      <div class="cs-ar-stack">
        {#each CANDS as c, i (c.tag)}
          <div class="cs-strip" class:is-lifted={i === 0}>
            <span class="cs-draft-box">
              {#if i === 0}
                <svg
                  class="cs-pencil-mark"
                  viewBox="0 0 {TICK_VIEW.w} {TICK_VIEW.h}"
                  width="26"
                  height="16"
                  aria-hidden="true"
                  filter="url(#pencil)"
                >
                  {#each TICK_NESTED as l (l.d)}<path d={l.d} fill-opacity={l.o} />{/each}
                </svg>
              {/if}
            </span>
            <p class="cs-draft-body">{c.text}</p>
            <span class="cs-draft-prob">{c.prob}%</span>
          </div>
        {/each}
      </div>
    </div>
    <p class="cs-ar-note">
      ✓ 一条候选 = 一块纸，<strong>高度随内容长</strong>，不必给 1/2/3 行各做一张图
      （九宫格中间可拉伸）。选中时可整条<strong>撕走</strong>。
    </p>
  </section>
</div>

<!--
  红铅笔滤镜，与 `PaperPanel.svelte` 的 `#pencil` 逐字一致。
  少了它那支笔就是矢量而不是红铅笔（§3.2：五层嵌套 + 羽化是一套，
  单层平涂边缘一刀切）——所以这里连滤镜一起搬，不能只搬 path。
-->
<PencilDefs />

<style>
  /* ── 现状栏：从 PaperPanel.svelte 逐字抄来，只作对照，不改 ────── */
  .cs-act {
    position: relative;
    display: flex;
    align-items: baseline;
    gap: 8px;
    appearance: none;
    margin: 0;
    min-height: 30px;
    padding: 0 16px;
    border-radius: 4px;
    border: 1px solid var(--cs-ink);
    background: transparent;
    color: var(--cs-ink);
    font: 600 13px/1.5 inherit;
    cursor: pointer;
    text-align: left;
  }

  .cs-act.is-lead::before {
    content: '';
    position: absolute;
    left: 1px;
    top: 0;
    bottom: 0;
    width: 3px;
    background: var(--cs-ink);
  }

  .cs-act-tag {
    font-size: 13px;
    white-space: nowrap;
  }

  .cs-act-text {
    font-size: 12.5px;
    font-weight: 400;
    line-height: 1.5;
    color: var(--cs-pencil);
  }

  .cs-act-prob {
    font-size: 12px;
    color: var(--cs-ink);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
    margin-left: auto;
  }

  /* ── 方向一：草稿行 ───────────────────────────────────────────
   *
   * 判据来自规范，不是审美偏好：
   * · §5.5 `--cs-rule` 在暖黄纸上等于隐形 ⇒ **不用任何线**，
   *   分隔全靠 `padding` 给的留白。
   * · §4.2b 「按钮不能拿现成色块当零件」⇒ 零背景色、零边框。
   * · §4.2b 「机器认出来的」走铅笔灰 ⇒ 正文铅笔灰，可读性 5.10:1 达标。
   * · §4.2b 纸面板的手感由**手绘记号**承担 ⇒ 推荐用真的手绘勾。
   */
  .cs-draft {
    display: flex;
    align-items: baseline;
    gap: 8px;
    /* 留白就是分隔本身。上下各 9px，两行之间自然断开。 */
    padding: 9px 0;
  }

  /* 勾的槽位宽度**固定**，让三条的文字左边缘对齐 ——
   * 勾的出现与消失不该让文字横向跳动。 */
  .cs-draft-box {
    flex: none;
    width: 26px;
    align-self: flex-start;
    /* 勾落在**首行的 x 高度**上，不是浮在行顶 —— 批注压在句子上，
     * 才像「这一句被批了」，而不是「这一行前面有个图标」。 */
    margin-top: 3px;
  }

  .cs-pencil-mark {
    display: block;
    /* 勾是墨，不是红：红只给不可逆（§5.3）。 */
    color: var(--cs-ink);
    fill: currentColor;
  }

  .cs-draft-body {
    flex: 1 1 auto;
    margin: 0;
    /* 与正文同族：仿宋（§4.2b 定案）。候选比正文小两号，但仍同一支笔。 */
    font-family: FangSong, STFangsong, '仿宋', serif;
    font-size: 15px;
    line-height: 1.55;
    color: var(--cs-pencil);
  }

  .cs-draft-prob {
    flex: none;
    align-self: flex-start;
    margin-top: 2px;
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 11px;
    color: var(--cs-pencil);
    opacity: 0.75;
  }

  /* ── 方向三：裁剪纸条 ───────────────────────────────────────────
   *
   * **不是 border。** 这里没有「描边」这个动作——纸条的四条边是**刀口**，
   *   裁开的那一刻露出的是纸芯的密度，加上投在下面那张纸上的那一线影。
   *   一条 1px 黑框读成「控件的外框」，一条纸口读成「一片纸的边」。
   *
   * 九宫格：`slice 40 / 20px` 是 @2x 的换算（源 40 源像素 → 20 CSS px）。
   * `fill` 让中间那块纸芯铺满，`stretch` 让它随内容长度横向拉伸，
   * 四条边保持不动 ⇒ 1 行 / 2 行 / 3 行都是**同一块纸**。
   *
   * 为什么纵向也能 stretch 而不出问题：上下两条边的高度由 `border-width`
   * 定死（20px，不拉伸），被纵向拉的是中间那块**均匀纸芯**——
   * 纸纹是细颗粒，纵向拉不出可见的条纹（横向拉才会，理由见
   * `scripts/stripgen.py` 的说明）。
   */
  .cs-strip {
    position: relative;
    border-style: solid;
    /* 14px 不是随手取的：20px 时每条纸条上下各占 20px，
     *   三条叠起来**超出面板 560px**，第三条被裁掉——而「候选看不全」
     *   恰恰是这个方案要解决的事。14px 让三条加上间隙正好落进面板。
     *   这个数依赖 `stripgen.py` 的 slice=40（@2x），
     *   改了那边记得回来看这里。 */
    border-width: 13px;
    border-image: url('/strip.png') 40 fill / 13px / 0 stretch;
    /* 内边距要**大于** border-width 才不压到刀口上。 */
    padding: 1px 11px 1px 7px;
    display: flex;
    align-items: baseline;
    gap: 8px;
    /* 常态：一条安静躺着的纸。只有极浅的一线影说明它压在另一张纸上。 */
    box-shadow: 0 1px 2px rgb(24 18 10 / 0.14);
    transition:
      transform var(--cs-dur) var(--cs-ease),
      box-shadow var(--cs-dur) var(--cs-ease);
  }

  /* 抬起：拿起来的那一下。位移只有 2px、旋转 0.5°——
   * 纸张很轻，抬得太高读成「浮起来的气泡」，而它是一张纸，
   * 抬起的作用只是**让手知道它可以被拿走**。 */
  .cs-strip.is-lifted,
  .cs-strip:hover {
    transform: translateY(-2px) rotate(-0.5deg);
    box-shadow: 0 6px 12px rgb(24 18 10 / 0.26);
  }

  /* ── 方向三的正文与方向一共用一套排版 ───────────────────────── */
  /* ── 对照页自身的版式（与被对照物无关） ─────────────────────── */
  .cs-ar {
    display: grid;
    grid-template-columns: repeat(3, 380px);
    gap: 26px;
    align-items: start;
    padding: 20px;
  }

  .cs-ar-col h2 {
    margin: 0 0 4px;
    font-size: 14px;
    font-weight: 700;
  }

  .cs-ar-why {
    margin: 0 0 12px;
    font-size: 11.5px;
    line-height: 1.55;
    color: var(--cs-pencil);
  }

  .cs-ar-note {
    margin: 10px 0 0;
    font-size: 11px;
    line-height: 1.5;
    color: var(--cs-ink);
  }

  .cs-ar-sheet {
    width: 380px;
    height: 560px;
    overflow: hidden;
    border: 1px solid rgb(20 26 31 / 12%);
    background-color: var(--cs-paper);
    background-image: url('/probe-sheet.svg');
    background-size: 380px 560px;
    background-position: top left;
    background-repeat: no-repeat;
    padding: 16px 14px;
  }

  .cs-ar-stack {
    display: flex;
    flex-direction: column;
    /* 方向三的纸条之间要比方向一/现状松——它们是**摊在桌上的几张纸**，
     *   挨太紧会读成一块被分成几段的板子。 */
    gap: 9px;
  }

  .cs-ar-col:last-child .cs-ar-stack {
    gap: 9px;
  }
</style>
