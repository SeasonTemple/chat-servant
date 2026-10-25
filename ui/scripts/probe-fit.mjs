/**
 * 量「**三条候选放不放得下**」——排版回归量具（票 92 续，2026-10-25）。
 *
 * ## 它量什么
 *
 * 真机反馈：「怎么还有滚动条了？这么大的面板放不下三个高度适合的纸条？」
 * ⇒ 这条量具把「放得下」变成**机器判据**：
 *
 * 1. 用**真机长度**的合成句（2 行 / 1 行 / 1 行）灌进候选区
 *    ⇒ ⚠ 合成句，**不是**真会话内容（真帧与真句都不进仓库）。
 * 2. 断言**候选组不溢出**（`scrollHeight <= clientHeight`）⇒ 没有滚动条。
 *    ⚠ **怎么读这个数**：`clientHeight` 是**它被分到的空间**，
 *    `scrollHeight` 是**内容高**。
 *    · 内容 ≤ 空间 ⇒ 两者相等（`clientHeight == scrollHeight`）⇒ 没被压 ✔
 *    · 内容 > 空间 ⇒ `clientHeight < scrollHeight` ⇒ **被压** ⇒ 溢出 ⇒ 出滚动条。
 *    ⚠ 所以判据是「**容器不能小于内容**」，
 *    不是「溢出 ≤ 0」（那在没被压时恒为 0，是同义反复）。
 * 3. 点一条候选 ⇒ 断言**回执区不出现**
 *    ⇒ 夹具拒投是**开发态**信息（`FixtureSkipped`），不该在面板上摆红字；
 *    而那段红字排在候选组**之后**却比候选组高 ⇒ 它把候选组压小
 *    ⇒ 逼出滚动条（真机实测：点过候选后容器 167 / 内容 187 ⇒ 溢出 20px）。
 *
 * ## 为什么用 **mock 的 Tauri invoke**
 *
 * 浏览器里没有 Tauri ⇒ `invoke('dispatch')` 直接抛错 ⇒ 前端走 `catch`
 * ⇒ 回执变成「投递失败：TypeError…」（那是 `Failed`，**必须显示**）。
 * ⇒ 要验「`FixtureSkipped` 不显示」，得**把 invoke 换成返回它**：
 * `page.addInitScript` 注入 `window.__TAURI_INTERNALS__.invoke`。
 * ⚠ 注入的是**一个假的后端**，所以它只证明**前端这条分支**，
 *   不证明 Rust 那边给的就是这个 outcome（那条由
 *   `commands::fixture_reply_tests` 守着）。
 *
 * ## 跑法
 *
 * ```bash
 * cd ui && node scripts/probe-fit.mjs
 * ```
 *
 * ⚠ 需要 dev server 在 `http://localhost:1420`（`pnpm dev`）。
 * ⚠ 退出码非 0 = 有判据没过。
 */
import { chromium } from 'playwright-core';

const BASE = 'http://localhost:1420';

/**
 * 真机长度的**合成**句（自造，不是真会话内容）。
 * ⚠ 长度是照着真机那三条量的：2 行 / 1 行 / 1 行。
 */
const SYNTHETIC = [
  '额度还是按原来的规则走，剩下那部分照常抵扣就行',
  '积分的计算方式没变，还是原来那一套',
  '暂时不用改，下个月再说这件事',
];

/** 注入假的 Tauri invoke：只答 `dispatch`，答成夹具拒投。 */
const MOCK_INVOKE = `
window.__MOCK_OUTCOME__ = 'FixtureSkipped';
window.__TAURI_INTERNALS__ = {
  invoke: async (cmd) => {
    if (cmd === 'dispatch') {
      const outcome = window.__MOCK_OUTCOME__;
      if (outcome === 'FixtureSkipped') {
        return {
          outcome: 'FixtureSkipped',
          detail: '当前跑在**冻结的夹具帧**上（CHATSERVANT_FIXTURE），不投递。',
          next_hint: null,
        };
      }
      // 真投递成功：一行回执（「已填入输入框」那种）
      return { outcome: 'Filled', detail: '已填入输入框。', next_hint: null };
    }
    throw new Error('mock: 未知命令 ' + cmd);
  },
};
`;

const browser = await chromium.launch({ channel: 'chrome' });
const page = await browser.newPage({ viewport: { width: 380, height: 560 } });
await page.addInitScript(MOCK_INVOKE);

/* ⚠⚠⚠ **合成短候选必须从「数据源」灌进去，不能改 DOM**（2026-10-25 23:14 实测）。
 *
 * ## 为什么
 * 原先是在量具里 `el.textContent = 合成句`。
 * ⇒ 点一下候选 ⇒ 组件重渲染 ⇒ **DOM 被还原成夹具自己的长句**
 * ⇒ 「点过候选后」那一档量的其实是**夹具的长句**，
 *   而「未点」那一档量的是**合成短句** ⇒ 两档不是同一个东西，
 *   差异会被误当成「点一下就变形」。
 *   （实测症状：第一条纸的字盒高 42 → 44px。）
 *
 * ## 怎么灌
 * 演示夹具是 `fetch('/demo/<state>.json')` 取的 ⇒ **在网络层拦它**，
 * 把 `candidates[].text` 换成合成短句再放行 ⇒
 * 组件拿到的从头到尾就是短句 ⇒ 重渲染也还原成短句。
 */
await page.route('**/demo/*.json', async (route) => {
  const res = await route.fetch();
  const json = await res.json();
  if (Array.isArray(json.candidates)) {
    json.candidates = json.candidates.map((c, i) =>
      SYNTHETIC[i] ? { ...c, text: SYNTHETIC[i] } : c,
    );
  }
  await route.fulfill({ json, status: 200, contentType: 'application/json' });
});

const lines = [];
let failed = 0;
function check(ok, name, detail) {
  lines.push(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? '  — ' + detail : ''}`);
  if (!ok) failed += 1;
}

/**
 * ⚠⚠⚠ **量布局之前必须先把鼠标挪开**（2026-10-25 23:17 实测）。
 *
 * `.cs-act:hover` 有 `transform: translateY(-2px) rotate(-0.5deg)`
 * ⇒ 点击之后鼠标还悬停在第一条纸上 ⇒ 它的 `getBoundingClientRect()`
 *   **已经不是布局位置**了：
 *   · 旋转 0.5° 把 225×42 的纸的包围盒撑成 225.4×44（实测）
 *   · 再叠上 `translateY(-2px)` ⇒ 首纸上沿比布局位置高 **3.4px**
 * ⇒ 那会被误判成「纸溢出候选区、被裁」，而布局其实分毫不差。
 * ⇒ 所以点完候选要先撤悬停 + 等过渡走完再量。
 */
async function unhover() {
  await page.mouse.move(2, 2);
  await page.waitForTimeout(450);
}

/** 量候选组的容纳情况。 */
async function fit() {
  return page.evaluate(() => {
    const box = document.querySelector('.cs-actions');
    const cs = box ? getComputedStyle(box) : null;
    const note = document.querySelector('.cs-act-note');
    const pick = document.querySelector('.cs-pick');
    const draft = document.querySelector('.cs-draft');
    const name = document.querySelector('.cs-pick-name');
    const markbox = document.querySelector('.cs-markbox');
    const svg = document.querySelector('.cs-tick');
    /* ⚠ 墨迹范围 = 所有 path 的 **user-space bbox**，按 svg 的
     * viewBox→CSS 比例换算到屏幕坐标。
     * ⚠ **不是**量 svg 的盒子：盒子里有大片空白画布，量它会把
     * 「笔有多大」说成「盒子有多大」（同一个错：量偏差要问相对谁）。 */
    let ink = null;
    if (svg) {
      const b = svg.getBoundingClientRect();
      const vb = svg.viewBox.baseVal;
      let ux0 = Infinity;
      let ux1 = -Infinity;
      let uy0 = Infinity;
      let uy1 = -Infinity;
      for (const p of svg.querySelectorAll('path')) {
        const pb = p.getBBox();
        ux0 = Math.min(ux0, pb.x);
        ux1 = Math.max(ux1, pb.x + pb.width);
        uy0 = Math.min(uy0, pb.y);
        uy1 = Math.max(uy1, pb.y + pb.height);
      }
      const sy = b.height / vb.height;
      ink = {
        top: +(b.top + (uy0 - vb.y) * sy).toFixed(1),
        bottom: +(b.top + (uy1 - vb.y) * sy).toFixed(1),
      };
    }
    return {
      clientH: box ? box.clientHeight : null,
      scrollH: box ? box.scrollHeight : null,
      溢出: box ? box.scrollHeight - box.clientHeight : null,
      回执在: !!note,
      回执高: note ? +note.getBoundingClientRect().height.toFixed(1) : 0,
      档位行top: pick ? +pick.getBoundingClientRect().top.toFixed(1) : null,
      /* ⚠⚠ **「档位行不动」量的是「档位名」，不是 `.cs-pick` 的盒子上沿。**
       * 盒子上沿**含那块空的取景框**（2026-10-25 从 54px 收到 10.8px）
       * ⇒ 盒子矮了 ⇒ 上沿下移 ⇒ 而**用户看到的那行字一像素没动**
       * （档位行是贴着纸底的，框变矮时框的上沿往下走，名字不动）。
       * ⇒ 拿盒子上沿当判据 = 拿一个**没人看得见的边**当判据
       * ⇒ 框一改，判据就红，而界面其实没坏。 */
      档位名top: name ? +name.getBoundingClientRect().top.toFixed(1) : null,
      档位名bottom: name ? +name.getBoundingClientRect().bottom.toFixed(1) : null,
      取景框高: markbox ? +markbox.getBoundingClientRect().height.toFixed(1) : null,
      候选区高: box ? +box.getBoundingClientRect().height.toFixed(1) : null,
      候选区底: box ? +box.getBoundingClientRect().bottom.toFixed(1) : null,
      墨迹top: ink ? ink.top : null,
      墨迹bottom: ink ? ink.bottom : null,
      正文位高: draft ? +draft.getBoundingClientRect().height.toFixed(1) : null,
      gutter: cs ? cs.scrollbarGutter : null,
      /* ⚠⚠ **纸条自己长高，还是「空纸带」**（2026-10-25 22:49）。
       *
       * ## 为什么必须量「间隙」
       * 候选**短**的时候（真机就是 1~2 行），候选区里会多出几十像素；
       * 旧做法（`justify-content: space-between`）把这几十像素**摊成
       * 纸与纸之间的空纸带** ⇒ 画面读成「三条小纸条浮在纸上，中间挖了两个洞」
       * ⇒ 主人原话：「**你觉得这样叫摊开吗？**」
       *
       * ⚠⚠ **而 demo 夹具看不见这个坑**：demo 的候选很长（4/3/3 行），
       *   内容几乎正好填满候选区 ⇒ `space-between` 根本没有余量可摊
       *   ⇒ **间隙是 −16（撕边互相填进去）** ⇒ 看着一点问题都没有。
       * ⇒ 所以这里的判据必须用**短候选**（`SYNTHETIC` 就是照真机行数造的）。
       */
      纸条间隙: (() => {
        const rects = [...document.querySelectorAll('.cs-act')].map((a) =>
          a.getBoundingClientRect(),
        );
        const g = [];
        for (let i = 1; i < rects.length; i++) {
          g.push(+(rects[i].top - rects[i - 1].bottom).toFixed(1));
        }
        return g;
      })(),
      /* ⚠⚠⚠ **「铺满」要钉看得见的边缘，不是钉算术**（2026-10-25 23:16 订正）。
       *
       * ## 原先那条判据错在哪
       * 原式是 `sum(纸高) − 32 == 候选区高`。
       * ⇒ 那个 −32 是「相邻两片各退 8px ⇒ 互相叠 16px × 2 处」的换算，
       *   **只对 n = 3 恰好成立**（`16×(n−1)` 与 `16×n−16` 在 n=3 时相等），
       *   候选条数一变就错 ⇒ **判据绑死了一个恰好值**。
       * ⇒ 而且它量的是**算出来的版面**，不是**用户看到的那摞纸的上下边**。
       *
       * ## 现在的判据
       * 第一条纸的**上沿**与候选区的**上沿**齐、最后一条纸的**下沿**与候选区的
       * **下沿**齐 ⇒ 这摞纸**正好铺满**候选区。
       * ⚠ 纸有 `margin: -8px 0`（撕边要溢出布局框）⇒ 所以「齐」指的是
       *   **纸的边框盒**与**候选区的边框盒**齐，而不是内容盒齐
       *   —— 实测：未点时 162 == 162、437.7 == 437.7。
       * ⇒ 差值取绝对值：往上顶出（被裁）或往下留洞（没铺满）都会红。
       */
      铺满差: (() => {
        const rects = [...document.querySelectorAll('.cs-act')].map((a) =>
          a.getBoundingClientRect(),
        );
        if (!rects.length || !box) return null;
        const b = box.getBoundingClientRect();
        return {
          首: +(rects[0].top - b.top).toFixed(1),
          末: +(rects[rects.length - 1].bottom - b.bottom).toFixed(1),
        };
      })(),
      字心偏差: [...document.querySelectorAll('.cs-act')].map((a) => {
        const r = a.getBoundingClientRect();
        const t = a.querySelector('.cs-act-text').getBoundingClientRect();
        return +((t.top + t.bottom) / 2 - (r.top + r.bottom) / 2).toFixed(1);
      }),
    };
  });
}

/**
 * 「纸自己长高」那三条判据。
 *
 * ⚠ 三条要**一起**看：只钉「间隙 ≤ 0」不够（把纸条全挤在一起也能过，
 * 那就变成另一种难看了）；只钉「铺满」也不够（余量摊在最后一条下面也能过）。
 */
function checkTiling(m, prefix) {
  const gaps = m.纸条间隙 || [];
  check(
    gaps.length > 0 && gaps.every((g) => g <= 0),
    `${prefix}纸条之间没有空纸带`,
    `间隙 ${gaps.join(' / ')}（≤0 = 撕边互相填进去；>0 就是挖了洞）`,
  );
  const diff =
    m.铺满差 !== null && m.铺满差 !== undefined
      ? {
          首: m.铺满差.首,
          末: m.铺满差.末,
          max: Math.max(Math.abs(m.铺满差.首), Math.abs(m.铺满差.末)),
        }
      : null;
  check(
    diff !== null && diff.max <= 1.5,
    `${prefix}纸自己长高填满候选区`,
    `首纸上沿−候选区上沿 ${diff?.首} / 末纸下沿−候选区下沿 ${diff?.末}（上限 1.5；都齐 = 铺满）`,
  );
  const off = m.字心偏差 || [];
  check(
    off.length > 0 && off.every((d) => Math.abs(d) <= 2),
    `${prefix}字在长出来的那段纸里居中`,
    `偏差 ${off.join(' / ')}px（上限 2）`,
  );
}

/**
 * 档位行那三条判据（位置不动 / 取景框不空 / 空白不这么高）。
 *
 * ⚠ 为什么它们要**一起**看：收掉空取景框是**位置换空间**——
 * 框矮 43px ⇒ 候选区高 43px ⇒ 三条纸摊得更开 ⇒ 底下那块空白从
 * 70.7px 掉到 27.5px。⇒ 少了任何一条，剩下的那条就可能悄悄假绿。
 *
 * @param extra 候选区被**别的东西**占掉的高度（回执那档）——
 *   ⚠ 回执（`.cs-act-note`）就排在候选区与档位行**之间**
 *   ⇒ 它一出现，候选区就矮一截 ⇒ 候选底往上走 ⇒ 空白变大。
 *   ⇒ 那部分**不是空白**，判据里要扣掉，否则回执那一档必红。
 */
function checkTierRow(m, prefix, extra = 0) {
  check(
    m.档位名top === 465.2 && m.档位名bottom === 480,
    `${prefix}档位名位置不动`,
    `top=${m.档位名top} bottom=${m.档位名bottom}（期望 465.2 / 480）`,
  );
  check(
    m.墨迹top === 458.5 && m.墨迹bottom === 483.9,
    `${prefix}标记墨迹位置不动`,
    `top=${m.墨迹top} bottom=${m.墨迹bottom}（期望 458.5 / 483.9）`,
  );
  check(
    m.取景框高 !== null && m.取景框高 <= 16,
    `${prefix}档位行上方的取景框不再空着`,
    `框高=${m.取景框高}px（上限 16）`,
  );
  const band =
    m.候选区底 !== null && m.档位名top !== null ? +(m.档位名top - m.候选区底).toFixed(1) : null;
  const net = band !== null ? +(band - extra).toFixed(1) : null;
  check(
    net !== null && net <= 32,
    `${prefix}最后一条候选到档位名的空白不这么高`,
    `空白=${band}px${extra ? ` − 回执占的 ${extra}px` : ''} ⇒ 净 ${net}px（上限 32；原来 70.7）`,
  );
}

await page.goto(`${BASE}/panel/?demo=1&state=drafted`, { waitUntil: 'networkidle' });
await page.locator('.cs-sheet').first().waitFor({ timeout: 15000 });
await page.waitForTimeout(700);

const before = await fit();
check(
  before.溢出 <= 0,
  '三条候选放得下（未点候选）',
  `容器 ${before.clientH} / 内容 ${before.scrollH} ⇒ 溢出 ${before.溢出}`,
);
checkTierRow(before, '');
checkTiling(before, '');
check(before.正文位高 === 47.5, '正文位高度不动', `h=${before.正文位高}（期望 47.5）`);

// 点第 1 条 ⇒ mock 返回 FixtureSkipped ⇒ 前端不该生成回执
await page.locator('.cs-act').first().click();
await page.waitForTimeout(900);
await unhover(); // ⚠ 撤掉悬停的 transform，否则量到的不是布局位置（见 unhover 的注释）

const after = await fit();
check(!after.回执在, '夹具拒投不在面板上摆回执', `回执存在=${after.回执在}`);
check(
  after.溢出 <= 0,
  '点过候选后仍放得下（无滚动条）',
  `容器 ${after.clientH} / 内容 ${after.scrollH} ⇒ 溢出 ${after.溢出}`,
);
checkTierRow(after, '点过候选后');
checkTiling(after, '点过候选后');

// ── 真投递那一档：回执**会**出现（一行）⇒ 它必须挤不掉候选 ──────
await page.evaluate(() => {
  window.__MOCK_OUTCOME__ = 'Filled';
});
await page.locator('.cs-act').first().click();
await page.waitForTimeout(900);
await unhover(); // ⚠ 同上：撤掉悬停再量

const sent = await fit();
check(sent.回执在, '真投递的回执照样显示（不许被一起吞掉）', `回执高=${sent.回执高}`);
check(
  sent.溢出 <= 0,
  '有回执时三条仍放得下（无滚动条）',
  `容器 ${sent.clientH} / 内容 ${sent.scrollH} ⇒ 溢出 ${sent.溢出}（回执高 ${sent.回执高}）`,
);
checkTierRow(sent, '有回执时', +(before.候选区高 - sent.候选区高).toFixed(1));
checkTiling(sent, '有回执时');
check(sent.gutter !== '15', '滚动条槽宽已被接管', `gutter=${sent.gutter}`);

console.log(lines.join('\n'));
console.log(
  failed === 0 ? `\nALL GREEN（${lines.length} 条判据）` : `\nFAILED ${failed}/${lines.length}`,
);
await browser.close();
process.exit(failed === 0 ? 0 : 1);
