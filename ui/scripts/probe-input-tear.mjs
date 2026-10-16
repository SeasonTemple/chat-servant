/**
 * `.cs-input` 换**撕边**的可行性实测（第二版，修了第一版的判据）。
 *
 * ## ⚠ 第一版量具的错（留着当反面教材）
 *
 * 第一版判据是「高度变化 > 0」，量出来 +0px，于是打印「❌ 不成立」。
 * 而真相恰恰相反：**`.cs-input` 本来就是 61px，和同屏撕边按钮同高**
 * ⇒ 票 45 记的「撕边上下 16px ⇒ 30px 变 61px」那个代价，
 * **空间在这一屏早就预留好了**，不需要额外腾。
 *
 * ⇒ 判据本身错了：它把「没变高」当成「没生效」，
 * 而真正要问的是「**内容区还装得下文字吗**」。
 * 这与本仓那条同族：**判据的量必须覆盖被测对象的全部相关维度**
 * —— 我只量了 border-box，没量 padding box 与文字位置。
 *
 * ## 这一版量三样
 *
 * 1. `border-image` 有没有真的作用到 `<input>` 上（替换元素，UA 样式有特殊处理）；
 * 2. 注入后 **padding box**（文字实际待的地方）还剩多高；
 * 3. 文字基线有没有被 16px×2 的边框压下去。
 *
 * ## ⚠ 它不回答「好不好看」—— 那是真机的事
 * ```bash
 * cd ui && pnpm dev --port 5199
 * node scripts/probe-input-tear.mjs
 * ```
 */
import { chromium } from 'playwright-core';
import { mkdir } from 'node:fs/promises';

const BASE = process.argv[2] ?? 'http://localhost:5199';
const OUT = process.argv[3] ?? '/workspace/shots';

await mkdir(OUT, { recursive: true });
const browser = await chromium.launch();
const page = await browser.newPage({
  viewport: { width: 380, height: 560 },
  deviceScaleFactor: 4,
});
await page.goto(`${BASE}/?demo=1&state=many&settings=1`, { waitUntil: 'networkidle' });
await page.waitForSelector('.cs-input', { timeout: 15000 });
await page.waitForTimeout(800);

const box = await page.locator('.cs-input').first().boundingBox();
const clip = { x: box.x - 8, y: box.y - 8, width: box.width + 16, height: box.height + 16 };
await page.screenshot({ path: `${OUT}/05-input-01px-edge.png`, clip });

const measure = () =>
  page.evaluate(() => {
    const i = document.querySelector('.cs-input');
    const b = i.getBoundingClientRect();
    const cs = getComputedStyle(i);
    // 用 Range 量文字实际占多高（比 line-height 准）
    const range = document.createRange();
    range.selectNodeContents(i);
    const tb = range.getBoundingClientRect();
    return {
      borderBox: Math.round(b.height),
      // padding box = border-box 减去上下边框
      padBox: Math.round(
        b.height - parseFloat(cs.borderTopWidth) - parseFloat(cs.borderBottomWidth),
      ),
      borderW: cs.borderTopWidth,
      textH: Math.round(tb.height),
      textTop: Math.round(tb.top - b.top),
      textBottom: Math.round(b.bottom - tb.bottom),
      biSource: cs.borderImageSource,
    };
  });

const before = await measure();

// 注入撕边（与 .cs-btn-mini / .cs-act 逐项同源）
await page.evaluate(() => {
  const i = document.querySelector('.cs-input');
  i.style.border = '16px solid transparent';
  i.style.borderImage = "url('/strip.png') 32 fill / 16px / 0 stretch";
  i.style.background = 'transparent';
});
await page.waitForTimeout(500);

const after = await measure();
await page.screenshot({ path: `${OUT}/06-input-teared.png`, clip });

console.log('\n══ `.cs-input` → 撕边纸条（实测）══\n');
const row = (k, b, a) =>
  console.log(`  ${k.padEnd(18)} ${String(b).padStart(6)}  →  ${String(a).padStart(6)}`);
console.log(`  ${''.padEnd(18)} ${'1px 描边'.padStart(6)}  →  ${'撕边 16px'.padStart(6)}`);
console.log('  ' + '─'.repeat(46));
row('border-box 高', before.borderBox, after.borderBox);
row('padding box 高', before.padBox, after.padBox);
row('文字占高', before.textH, after.textH);
row('文字距顶', before.textTop, after.textTop);
row('文字距底', before.textBottom, after.textBottom);
console.log(
  `  ${'border-image'.padEnd(18)} ${'-'.padStart(6)}  →  ${after.biSource.replace('http://localhost:5199', '')}`,
);

const applied = /strip\.png/.test(after.biSource);
const textFits = after.padBox >= after.textH + 8;
console.log(
  `\n  ${applied ? '✅' : '❌'} border-image 作用到了 <input> 上` +
    `（替换元素，UA 样式没有挡掉它）`,
);
console.log(
  `  ${textFits ? '✅' : '❌'} 撕边后文字仍装得下` +
    `（padding box ${after.padBox}px ≥ 文字 ${after.textH}px + 8px 余量）`,
);
console.log(
  `  ${Math.abs(after.borderBox - before.borderBox) < 2 ? '✅' : '⚠'} ` +
    `高度几乎没变（${before.borderBox} → ${after.borderBox}）` +
    `  ⇒ **不需要额外腾空间**，` +
    `这一屏早就按撕边按钮的 61px 留过了`,
);
console.log(`\n  图：${OUT}/05-input-01px-edge.png`);
console.log(`      ${OUT}/06-input-teared.png`);

await browser.close();
