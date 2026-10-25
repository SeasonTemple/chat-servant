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

const lines = [];
let failed = 0;
function check(ok, name, detail) {
  lines.push(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? '  — ' + detail : ''}`);
  if (!ok) failed += 1;
}

/** 量候选组的容纳情况。 */
async function fit() {
  return page.evaluate((texts) => {
    const acts = document.querySelectorAll('.cs-act-text');
    acts.forEach((el, i) => {
      if (texts[i]) el.textContent = texts[i];
    });
    const box = document.querySelector('.cs-actions');
    const cs = box ? getComputedStyle(box) : null;
    const note = document.querySelector('.cs-act-note');
    const pick = document.querySelector('.cs-pick');
    const draft = document.querySelector('.cs-draft');
    return {
      clientH: box ? box.clientHeight : null,
      scrollH: box ? box.scrollHeight : null,
      溢出: box ? box.scrollHeight - box.clientHeight : null,
      回执在: !!note,
      回执高: note ? +note.getBoundingClientRect().height.toFixed(1) : 0,
      档位行top: pick ? +pick.getBoundingClientRect().top.toFixed(1) : null,
      正文位高: draft ? +draft.getBoundingClientRect().height.toFixed(1) : null,
      gutter: cs ? cs.scrollbarGutter : null,
    };
  }, SYNTHETIC);
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
check(before.档位行top === 408.5, '档位行位置不动', `top=${before.档位行top}（期望 408.5）`);
check(before.正文位高 === 47.5, '正文位高度不动', `h=${before.正文位高}（期望 47.5）`);

// 点第 1 条 ⇒ mock 返回 FixtureSkipped ⇒ 前端不该生成回执
await page.locator('.cs-act').first().click();
await page.waitForTimeout(900);

const after = await fit();
check(!after.回执在, '夹具拒投不在面板上摆回执', `回执存在=${after.回执在}`);
check(
  after.溢出 <= 0,
  '点过候选后仍放得下（无滚动条）',
  `容器 ${after.clientH} / 内容 ${after.scrollH} ⇒ 溢出 ${after.溢出}`,
);
check(
  after.档位行top === 408.5,
  '点过候选后档位行位置不动',
  `top=${after.档位行top}（期望 408.5）`,
);

// ── 真投递那一档：回执**会**出现（一行）⇒ 它必须挤不掉候选 ──────
await page.evaluate(() => {
  window.__MOCK_OUTCOME__ = 'Filled';
});
await page.locator('.cs-act').first().click();
await page.waitForTimeout(900);

const sent = await fit();
check(sent.回执在, '真投递的回执照样显示（不许被一起吞掉）', `回执高=${sent.回执高}`);
check(
  sent.溢出 <= 0,
  '有回执时三条仍放得下（无滚动条）',
  `容器 ${sent.clientH} / 内容 ${sent.scrollH} ⇒ 溢出 ${sent.溢出}（回执高 ${sent.回执高}）`,
);
check(sent.档位行top === 408.5, '有回执时档位行位置不动', `top=${sent.档位行top}（期望 408.5）`);
check(sent.gutter !== '15', '滚动条槽宽已被接管', `gutter=${sent.gutter}`);

console.log(lines.join('\n'));
console.log(
  failed === 0 ? `\nALL GREEN（${lines.length} 条判据）` : `\nFAILED ${failed}/${lines.length}`,
);
await browser.close();
process.exit(failed === 0 ? 0 : 1);
