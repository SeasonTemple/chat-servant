/**
 * 出**主面板**的整屏图，并把「布局被小字吃掉多少」逐项量出来。
 *
 * ## 它拍的是什么
 *
 * `http://localhost:1420/?demo=1&state=drafted` —— 真实路由
 * （`src/routes/(panel)/+page.svelte`），**演示夹具**的内容（假内容，
 * 不含任何真机聊天）。
 *
 * 拍的是 `.cs-sheet`（那张纸，380×560，与真机 Tauri 窗口同尺寸）。
 *
 * ## 为什么它要**量**，不只拍
 *
 * 小字放大不是免费的：内容栏只有 316px 宽，候选区本来就装不下 3 条
 * （票 44 有算式），而档位行上方那 40px 是给「档位名 + 撕口留白」留的。
 * ⇒ 光看图说「好像还行」不算数 ⇒ 这里把
 * **归属行高度 / 候选区高度 / 档位行到纸底的余白** 三个数打出来。
 *
 * ## ⚠⚠ 它**不能**证明什么
 *
 * 1. **headless Chromium ≠ 真机 WebView2**（票 45 第六轮）。
 * 2. **demo ≠ 真实运行时**（键名/路由/徽标是摆出来的）——
 *    但**排版是真排版**（同一套 CSS、同一份组件）。
 *
 * ## 跑法
 *
 * ```bash
 * cd ui && node scripts/shoot-panel.mjs <输出目录> [--tag=before] [--scale=1]
 * ```
 */
import { chromium } from 'playwright-core';
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';

const args = process.argv.slice(2);
const OUT = args.find((a) => !a.startsWith('--')) ?? 'target/probe/panel';
const opt = (name, dflt) => {
  const hit = args.find((a) => a.startsWith(`--${name}=`));
  return hit ? hit.slice(name.length + 3) : dflt;
};
const TAG = opt('tag', 'panel');
const SCALE = Number(opt('scale', '1'));
const STATE = opt('state', 'drafted');
const BASE = opt('base', 'http://localhost:1420');

await mkdir(OUT, { recursive: true });

// ⚠ 用系统 Chrome（同 shoot-type.mjs：playwright 自带的浏览器版本对不上，
// 且 `npx playwright install` 会往 C 盘下 ⇒ 违反「产物留项目盘」）。
const browser = await chromium.launch({ channel: opt('channel', 'chrome') });
const page = await browser.newPage({
  viewport: { width: 460, height: 700 },
  deviceScaleFactor: SCALE,
});

const url = `${BASE}/?demo=1&state=${STATE}`;
await page.goto(url, { waitUntil: 'networkidle' });
const sheet = page.locator('.cs-sheet').first();
await sheet.waitFor({ timeout: 15000 });
// ⚠ 等补间跑完（票 45 量具提醒：截到补间中间帧 = 量到半个状态）。
await page.waitForTimeout(1200);

/**
 * 布局的三个数：**归属行多高 / 候选区还剩多高 / 档位行离纸底多高**。
 */
const facts = await page.evaluate(() => {
  const box = (sel) => {
    const el = document.querySelector(sel);
    if (!el) return null;
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    return {
      top: Math.round(r.top * 10) / 10,
      bottom: Math.round(r.bottom * 10) / 10,
      h: Math.round(r.height * 10) / 10,
      fontSize: cs.fontSize,
      fontWeight: cs.fontWeight,
      fontFamily: cs.fontFamily.split(',')[0],
    };
  };
  const sheetEl = document.querySelector('.cs-sheet');
  const sr = sheetEl?.getBoundingClientRect();
  const pick = box('.cs-pick');
  const where = box('.cs-where');
  const actions = box('.cs-actions');
  const name = box('.cs-pick-name');
  const quote = box('.cs-where-quote');
  const draft = box('.cs-draft-text');
  // ⚠ **溢出量**：静默裁掉的东西只能在这里看见——
  //   `clientHeight` 是看得见的高度，`scrollHeight` 是内容真正要的高度。
  //   两者不等 ⇒ **有内容被裁掉了**（`.cs-sheet` 的 overflow: hidden 会让它无声消失）。
  const over = (sel) => {
    const el = document.querySelector(sel);
    if (!el) return null;
    const cs = getComputedStyle(el);
    const lh = parseFloat(cs.lineHeight) || 0;
    return {
      client: el.clientHeight,
      scroll: el.scrollHeight,
      overflow: el.scrollHeight - el.clientHeight,
      lineHeight: lh,
      lines: lh > 0 ? Math.round((el.scrollHeight / lh) * 10) / 10 : null,
      visibleLines: lh > 0 ? Math.round((el.clientHeight / lh) * 10) / 10 : null,
    };
  };
  return {
    sheet: sr ? { w: Math.round(sr.width), h: Math.round(sr.height) } : null,
    where,
    quote,
    name,
    draft,
    actions,
    pick,
    draftOverflow: over('.cs-draft-text'),
    actionsOverflow: over('.cs-actions'),
    // 档位行到纸底的余白（撕口那一侧的净空）
    pickToSheetBottom: pick && sr ? Math.round((sr.bottom - pick.bottom) * 10) / 10 : null,
    // 候选区离档位行顶的净空
    actionsH: actions?.h ?? null,
    // **块间距**：纸高固定 ⇒ 谁长高了，吃掉的是**留白**，不是别人。
    gapWhereToDraft: where && draft ? Math.round((draft.top - where.bottom) * 10) / 10 : null,
    gapDraftToActions: draft && actions ? Math.round((actions.top - draft.bottom) * 10) / 10 : null,
    gapActionsToPick: actions && pick ? Math.round((pick.top - actions.bottom) * 10) / 10 : null,
    // 候选行数（每条候选的高度）
    actRows: Array.from(document.querySelectorAll('.cs-act-text')).map((el) => {
      const r = el.getBoundingClientRect();
      return Math.round(r.height * 10) / 10;
    }),
  };
});

const file = join(OUT, `panel-${TAG}-${STATE}${SCALE === 1 ? '' : `@${SCALE}x`}.png`);
await sheet.screenshot({ path: file });

console.log(`\n面板图 -> ${file}`);
console.log(`纸 ${facts.sheet?.w}×${facts.sheet?.h}`);
console.log(
  `归属行 .cs-where   ${facts.where?.fontSize}/${facts.where?.fontWeight} ` +
    `h=${facts.where?.h}  (引文 ${facts.quote?.fontSize} 字体 ${facts.quote?.fontFamily})`,
);
console.log(
  `档位名 .cs-pick-name ${facts.name?.fontSize}/${facts.name?.fontWeight} 底=${facts.name?.bottom}`,
);
console.log(
  `候选区 .cs-actions  h=${facts.actionsH}   档位行底=${facts.pick?.bottom}   ` +
    `档位行→纸底余白=${facts.pickToSheetBottom}`,
);
console.log(
  `块间距  归属行→草稿=${facts.gapWhereToDraft}  草稿→候选区=${facts.gapDraftToActions}  ` +
    `候选区→档位行=${facts.gapActionsToPick}`,
);
console.log(`候选行高 ${JSON.stringify(facts.actRows)}`);
console.log(
  `草稿正文 .cs-draft-text  h=${facts.draft?.h}  font=${facts.draft?.fontSize}  ` +
    `底=${facts.draft?.bottom}`,
);
console.log(
  `  溢出 client=${facts.draftOverflow?.client} scroll=${facts.draftOverflow?.scroll} ` +
    `⇒ 裁掉 ${facts.draftOverflow?.overflow}px（可见 ${facts.draftOverflow?.visibleLines} 行 / ` +
    `共 ${facts.draftOverflow?.lines} 行）`,
);
console.log(
  `  候选区溢出 client=${facts.actionsOverflow?.client} scroll=${facts.actionsOverflow?.scroll} ` +
    `⇒ 折叠 ${facts.actionsOverflow?.overflow}px`,
);

await browser.close();
console.log(`\n✅ 出图完成`);
