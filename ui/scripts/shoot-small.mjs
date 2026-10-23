/**
 * 出「面板小字」的对照图（票 24 收尾，2026-10-23）。
 *
 * ## 它拍的是什么
 *
 * 工作台的 D 段（`/studio?only=small&small=<px>`）：**归属行**（`.cs-where`
 * 的样子：名字 + 引文）与**档位名**（`.cs-pick-name` 的样子），
 * 照抄 `PaperPanel.svelte` 的字体、色值、行高，只变**字号**一个变量。
 * 纸底默认**真实纸材位图**，`?paper=solid` 出纯色对照。
 *
 * ## 为什么一次只出一档
 *
 * 票 24 的教训：一个 sheet 混排多种字号时，行切分会把两种字号的墨
 * 并进同一个掩码 ⇒ 量出来的不是那个字号的东西。
 * ⇒ 这里**每档单独开一个 page**（新 URL = 新挂载，参数立刻生效）。
 *
 * ## ⚠⚠ 它**不能**证明什么（读图之前先看这段）
 *
 * 1. **headless Chromium ≠ 真机 WebView2。** 票 45 第六轮：「第一次上真机，
 *    当场撞出一个五轮 headless 全绿的缺陷」。字体栅格化、DPI、合成路径
 *    都不是一回事。⇒ 这些图的效力上限是「**在 Chromium 的栅格化下**
 *    读不读得出」。
 * 2. **1× 出图**（`deviceScaleFactor: 1`）——CSS px = 图上 px，
 *    与 `strokeval.py --font-px` 同一口径。3× 只给人眼看笔画细节。
 * 3. ⚠ **纸纹底会让细笔画更容易发虚**（方向单向）⇒ 纸纹上的观感
 *    **比真机更乐观**还是更悲观，要以真机为准；这张图的作用是
 *    **让人眼先过一遍**（票 24 要求的正是这一步），不是代替真机。
 *
 * ## 跑法
 *
 * ```bash
 * cd ui && node scripts/shoot-small.mjs <输出目录> [--sizes=11,12,13]
 *                                         [--weight=600] [--paper=texture]
 *                                         [--scale=1]
 * ```
 */
import { chromium } from 'playwright-core';
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';

const args = process.argv.slice(2);
const OUT = args.find((a) => !a.startsWith('--')) ?? 'target/probe/small';
const opt = (name, dflt) => {
  const hit = args.find((a) => a.startsWith(`--${name}=`));
  return hit ? hit.slice(name.length + 3) : dflt;
};

const SIZES = opt('sizes', '11,12,13')
  .split(',')
  .map((s) => Number(s.trim()))
  .filter(Boolean);
const WEIGHT = opt('weight', '600');
const PAPER = opt('paper', 'texture');
const BASE = opt('base', 'http://localhost:1420');
const SCALE = Number(opt('scale', '1'));

await mkdir(OUT, { recursive: true });

// ⚠ **用系统 Chrome，不下 playwright 的浏览器**（2026-10-22 实测）：
// 本机 `ms-playwright` 里只有 `chromium-1208`，而 playwright-core 1.64 要 1248
// ⇒ `chromium.launch()` 直接报「Executable doesn't exist」。
// 而 `npx playwright install` 会把浏览器下到 **C 盘** ⇒ 违反本仓
// 「产物留项目盘」的规矩。改用系统已装的 Chrome（Blink + DirectWrite，
// 与 WebView2 同代栅格化）。
const browser = await chromium.launch({ channel: opt('channel', 'chrome') });
console.log(`浏览器：系统 ${opt('channel', 'chrome')}（不是 playwright 自带的）`);

const page = await browser.newPage({
  viewport: { width: 460, height: 900 },
  deviceScaleFactor: SCALE,
});

/**
 * 报出**实际生效**的字体与字号。
 *
 * ⚠ 不信 URL 参数：参数只决定「请求什么」，**渲染成什么**要问浏览器。
 */
async function facts() {
  return await page.evaluate(() => {
    const pick = (sel) => {
      const el = document.querySelector(sel);
      if (!el) return null;
      const cs = getComputedStyle(el);
      const r = el.getBoundingClientRect();
      return {
        fontSize: cs.fontSize,
        fontFamily: cs.fontFamily,
        fontWeight: cs.fontWeight,
        color: cs.color,
        letterSpacing: cs.letterSpacing,
        w: Math.round(r.width * 10) / 10,
        h: Math.round(r.height * 10) / 10,
        text: (el.textContent ?? '').trim(),
      };
    };
    return {
      where: pick('.small-where'),
      peer: pick('.small-peer'),
      quote: pick('.small-quote'),
      name: pick('.small-name'),
    };
  });
}

console.log(`\n出图目录 ${OUT}`);
console.log(`参数 sizes=${SIZES.join(',')} weight=${WEIGHT} paper=${PAPER} scale=${SCALE}`);
console.log(`伺服 ${BASE}（dev server），deviceScaleFactor=${SCALE}\n`);

for (const px of SIZES) {
  const url = `${BASE}/studio?only=small&small=${px}&paper=${PAPER}&weight=${WEIGHT}`;
  await page.goto(url, { waitUntil: 'networkidle' });
  const crop = page.locator('.crop').first();
  await crop.waitFor({ timeout: 15000 });
  await page.waitForTimeout(400);
  const file = join(OUT, `small-${WEIGHT}-${px}${SCALE === 1 ? '' : `@${SCALE}x`}.png`);
  await crop.screenshot({ path: file });

  const f = await facts();
  console.log(`  ${px}px  ->  ${file}`);
  if (f.where) {
    console.log(
      `        归属行 ${f.where.fontSize}/${f.where.fontWeight} ` +
        `family=${f.where.fontFamily} color=${f.where.color} ` +
        `ls=${f.where.letterSpacing}  box=${f.where.w}×${f.where.h}`,
    );
    console.log(`        名字   ${f.peer?.fontSize}/${f.peer?.fontWeight}  「${f.peer?.text}」`);
    console.log(`        引文   ${f.quote?.fontSize}/${f.quote?.fontWeight}  「${f.quote?.text}」`);
    console.log(`        档位名 ${f.name?.fontSize}/${f.name?.fontWeight}  「${f.name?.text}」`);
  }
}

// 整页一张（各档依次排列），给人看的
await page.goto(`${BASE}/studio?only=small&paper=${PAPER}&weight=${WEIGHT}`, {
  waitUntil: 'networkidle',
});
await page.locator('.crop').first().waitFor({ timeout: 15000 });
await page.waitForTimeout(600);
const sheet = join(OUT, `small-ladder-${WEIGHT}-${PAPER}${SCALE === 1 ? '' : `@${SCALE}x`}.png`);
await page.screenshot({ path: sheet, fullPage: true });
console.log(`\n  整页（各档依次） -> ${sheet}`);

await browser.close();
console.log(`\n✅ 出图完成`);
