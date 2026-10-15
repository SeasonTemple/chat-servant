<!--
  票 44 的对照页：候选动作行到底该长成什么样。

  为什么要有这一页：票 44 的根因是 §4.2c 定的是**按钮族**几何，
  而候选行动作行承载的是**整句正文**——规范自己承认了这个冲突，
  落地时用 `min-height` 打了补丁，结果既不是 30px 按钮，也不是正文排版。
  「到底哪个更好」是眼睛的判断，纯文字描述里看不出来。

  ⚠ 这一页只做**并排比形**，不做落地。定案之后本路由应当删掉，
     改动直接落在 `PaperPanel.svelte` 的 `.cs-act` 族上。

  ⚠ 下面的几何是从 `PaperPanel.svelte` 的 `.cs-act*` 规则**逐字抄来**的，
     不是凭印象重画。改这里之前请先确认原文没变，否则这页就在骗人。
     token（--cs-ink / --cs-pencil / --cs-pencil-red …）来自全局 `app.css`，
     所以颜色是真的，不是抄来的十六进制。
-->
<script lang="ts">
  /*
   * 样本刻意挑成**会换行两三行**的中性句子。票 44 的全部争议都在
   * 「一行要读的话被塞进一个框里」，若只放单行短句，四栏会看起来一样，
   * 那正是这个取舍在短文本下不显形的原因。
   *
   * ⚠ 不含任何真实聊天内容 —— `ui/static` 可能被部署到公开 URL。
   */
  const CANDS = [
    { tag: '按这个', prob: 100, text: '周三下午三点的会我这边没问题，材料我提前十分钟到就行。' },
    { tag: '备选 2', prob: 46, text: '会我知道了，几点开始、在哪见你回头再跟我说一声。' },
    { tag: '备选 3', prob: 21, text: '收到，我看一下时间。' },
  ];
</script>

<div class="cs-ar">
  <!-- ── 现状（票 44 的问题本体） ───────────────────────────────── -->
  <section class="cs-ar-col">
    <h2>现状 · 通栏描边框</h2>
    <p class="cs-ar-why">
      通栏四边闭合的框，里面躺着一句<strong>要读</strong>的话。 它读起来是一个控件，不是一句候选项。
    </p>
    <div class="cs-ar-sheet">
      <div class="cs-ar-stack">
        {#each CANDS as c, i (c.tag)}
          <button class="cs-act {i === 0 ? 'cs-act-lead' : 'cs-act-alt'}">
            <span class="cs-act-tag">{c.tag}</span>
            <span class="cs-act-text">{c.text}</span>
            <span class="cs-act-prob">{c.prob}%</span>
          </button>
        {/each}
      </div>
    </div>
  </section>

  <!-- ── A：不是按钮 ────────────────────────────────────────────── -->
  <section class="cs-ar-col">
    <h2>A · 不是按钮</h2>
    <p class="cs-ar-why">
      取消整框，只留左侧竖笔，正文按 §4.2b 排。 第一职责是<strong>被读</strong>，第二才是被点。
    </p>
    <div class="cs-ar-sheet">
      <div class="cs-ar-stack">
        {#each CANDS as c, i (c.tag)}
          <button class="cs-act-a {i === 0 ? 'cs-act-lead' : 'cs-act-alt'}">
            <span class="cs-act-a-body">
              <span class="cs-act-a-text">{c.text}</span>
              <span class="cs-act-a-meta">
                <span class="cs-act-tag">{c.tag}</span>
                <span class="cs-act-prob">{c.prob}%</span>
              </span>
            </span>
          </button>
        {/each}
      </div>
    </div>
  </section>

  <!-- ── B：是按钮，但形状换 ────────────────────────────────────── -->
  <section class="cs-ar-col">
    <h2>B · 框改成贴合文字宽</h2>
    <p class="cs-ar-why">
      保留框，但用 <code>width: fit-content</code> 而非通栏。 ⚠ 句子的长度决定了它**几乎必然还是满宽**——这一栏要判断的正是
      「B 到底有没有把「控件」这个读感去掉」。
    </p>
    <div class="cs-ar-sheet">
      <div class="cs-ar-stack">
        {#each CANDS as c, i (c.tag)}
          <button class="cs-act-b {i === 0 ? 'cs-act-lead' : 'cs-act-alt'}">
            <span class="cs-act-tag">{c.tag}</span>
            <span class="cs-act-text">{c.text}</span>
            <span class="cs-act-prob">{c.prob}%</span>
          </button>
        {/each}
      </div>
    </div>
  </section>

  <!-- ── C：拆成两行 ────────────────────────────────────────────── -->
  <section class="cs-ar-col">
    <h2>C · 拆成两行</h2>
    <p class="cs-ar-why">
      上面一行正文（可读），下面一行才是可点的动作。
      「读」与「点」的边界被重新划出来，代价是高度翻倍。
    </p>
    <div class="cs-ar-sheet">
      <div class="cs-ar-stack">
        {#each CANDS as c, i (c.tag)}
          <div class="cs-act-c" class:is-lead={i === 0}>
            <p class="cs-act-c-text">{c.text}</p>
            <button class="cs-act-c-btn">
              <span class="cs-act-tag">{c.tag}</span>
              <span class="cs-act-prob">{c.prob}%</span>
            </button>
          </div>
        {/each}
      </div>
    </div>
  </section>
</div>

<style>
  /*
   * ⚠ 下面 `.cs-act*` 到 `.cs-act-prob` 这一段是从 `PaperPanel.svelte`
   *   **逐字抄来**的现状几何。抄的时候不要顺手"优化"——
   *   这一页的价值全在于它与现状一致，改了就失去对照意义。
   */
  .cs-act,
  .cs-act-b {
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

  .cs-act-alt {
    background: transparent;
  }

  .cs-act-lead {
    background: transparent;
    color: var(--cs-ink);
  }

  .cs-act-lead::before {
    content: '';
    position: absolute;
    left: 1px;
    top: 0;
    bottom: 0;
    width: 3px;
    background: var(--cs-ink);
  }

  .cs-act-lead:hover::before {
    width: 5px;
  }

  .cs-act-tag {
    font-size: 13px;
    color: inherit;
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

  /* B 的唯一增量：贴合文字宽而不是通栏。 */
  .cs-act-b {
    width: fit-content;
    max-width: 100%;
  }

  /* ── A ──────────────────────────────────────────────────────── */
  /*
   * A 与现状的差别只有一条：**去掉框**。竖笔保留（它是红批的既有机制，
   * 三变体靠颜色分工，去掉它就等于拆掉了「墨色竖笔 = 划重点」这条约定）。
   * 正文改成左对齐、放开行高约束 —— 它现在是一段要读的字，不是一个标签。
   */
  .cs-act-a {
    position: relative;
    display: block;
    width: 100%;
    appearance: none;
    margin: 0;
    /* ⚠ 左内边距 15px 而非 20px：竖笔占 1~4px，20px 会让笔与文字之间
     *   空出 16px，笔看着像飘在行外面而不是这行的一部分。 */
    padding: 7px 16px 7px 15px;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: var(--cs-ink);
    font: inherit;
    cursor: pointer;
    text-align: left;
  }

  /* 行与行之间用一道细线分隔，替代原来那圈框。 */
  .cs-act-a + .cs-act-a {
    border-top: 1px solid var(--cs-rule);
  }

  .cs-act-a-body {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }

  /*
   * ⚠ 正文**全走墨色**，不走现状里 `.cs-act-text` 的铅笔灰。
   *   铅笔灰是「机器认出来的」这层语义（§4.2b），而这一栏的第一职责
   *   是**被读**——把要读的字压成灰色，读得清的代价太大。
   *
   * ⚠ 主次因此不能靠颜色，改为**字重 + 竖笔**两个通道。
   *   这也暴露了 A 的真实代价：「墨线」那一档的载体就是那圈框，
   *   去掉框之后它**无处安放**，三变体实际只剩「竖笔有无 + 竖笔颜色」。
   *   票 44 的验收项要验的正是这一点 ⇒ 这条代价必须让人看见，不藏。
   */
  .cs-act-a-text {
    flex: 1 1 auto;
    font-size: 13px;
    font-weight: 400;
    line-height: 1.6;
    color: var(--cs-ink);
  }

  .cs-act-lead .cs-act-a-text {
    font-weight: 600;
  }

  .cs-act-a-meta {
    flex: none;
    display: flex;
    align-items: baseline;
    gap: 8px;
  }

  /* ── C ──────────────────────────────────────────────────────── */
  /*
   * ⚠ C 的容器**不能**挂 `cs-act-lead`。那条规则带着一个 `::before`
   *   绝对定位竖笔，而 `.cs-act-c` 并不 `position: relative`，
   *   于是那道笔会一路往上找到更外层的定位祖先，**画到容器外面去**——
   *   在这张对照图里表现为页面左缘一道贯穿的黑条，而 C 自己毫不知情。
   *
   *   C 的「不可逆」表达在**按钮**上（红描边），不在这只容器上，
   *   所以用 `is-lead` 这个本页私有的类，不复用现状那套。
   */
  .cs-act-c {
    position: relative;
    padding: 8px 0 10px;
  }

  .cs-act-c + .cs-act-c {
    border-top: 1px solid var(--cs-rule);
  }

  .cs-act-c-text {
    margin: 0 0 7px;
    font-size: 13px;
    line-height: 1.6;
    color: var(--cs-ink);
  }

  /* C 的动作行**回到按钮族几何**：短标签、等高、1px 墨描边，
     也就是 §4.2c 原本要为它准备的那一族。 */
  .cs-act-c-btn {
    position: relative;
    display: inline-flex;
    align-items: baseline;
    gap: 8px;
    appearance: none;
    margin: 0;
    height: 30px;
    padding: 0 16px;
    border-radius: 4px;
    border: 1px solid var(--cs-ink);
    background: transparent;
    color: var(--cs-ink);
    font: 600 13px/1.5 inherit;
    cursor: pointer;
  }

  .cs-act-c.is-lead .cs-act-c-btn {
    border-color: var(--cs-pencil-red);
    color: var(--cs-pencil-red);
  }

  /* ── 对照页自身的版式（与被对照物无关） ─────────────────────── */
  .cs-ar {
    display: grid;
    grid-template-columns: repeat(4, 380px);
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
    line-height: 1.5;
    color: var(--cs-pencil);
  }

  /*
   * 每栏就是**一整块真面板**：380 × 560，与 `tauri.conf.json` 里的窗口
   * 尺寸和这张纸的 SVG 原始尺寸三者一致。
   *
   * ⚠ 不要用 `background-size: 100% auto`：那样纸只按栏宽铺一次，
   *   栏高与纸高不等时下半截就露白，四栏于是长得不一样高 ——
   *   而"哪个方案更好"恰恰要靠**等大并排**来判断。
   */
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
    gap: 6px;
  }
</style>
