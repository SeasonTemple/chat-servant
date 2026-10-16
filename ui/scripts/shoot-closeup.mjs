/**
 * 设置屏两处边界的**局部特写**（票 45 第七轮）。
 *
 * ## 为什么还要专门拍这个
 *
 * 全屏对比图出来之后，**肉眼差别很小** —— 而「看不出差别」正是票 45
 * 反复警告的那句话：「**出图看不出问题**不等于真机没问题」。
 * ⇒ 局部 4× 放大是**为了让人眼能分辨**，
 *   但它同时把另一个问题暴露出来：**这处修复本来就不是「一眼看出来」的**。
 *
 * ## 拍的是哪两处
 *
 * 1. `.cs-badge.miss`（"未配置"徽标）—— 修之前 `box-shadow` 计算值是
 *    **`none`**（`var(--cs-line)` 从未定义 ⇒ 整条声明作废），
 *    所以它**从来没有过那圈描边**。这一处是「有 vs 无」，不是深浅。
 * 2. `.cs-input`（"粘贴 key" 输入槽）—— 修之前 1.14 : 1 且冷
 *    （R−B = −10），修之后 3.04 : 1 且同源（R−B = +40）。
 *
 * ## ⚠ deviceScaleFactor ≠ 1
 *
 * 票 45 量具提醒：`-Scale 2` 时**像素不是 CSS px**。
 * 本脚本 4× 只为**看**、不用于量 —— 每张图下方都打出实测 CSS 值。
 *
 * ## ⚠ 仍然不是验收
 *
 * headless ≠ WebView2（分层合成 / 透明窗口 / DPI）。
 * 真机验收仍欠着。
 *
 * ```bash
 * cd ui && pnpm build && node scripts/shoot-closeup.mjs <输出目录>
 * ```
 */
import { chromium } from 'playwright-core';
import { createServer } from 'node:http';
import { readFile, mkdir } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { extname, join, normalize } from 'node:path';

const OUT = process.argv[2] ?? '/workspace/shots';
const ZOOM = 4;
const W = 380;
const H = 560;
const ROOT = new URL('../build/', import.meta.url).pathname;

if (!existsSync(join(ROOT, 'index.html'))) {
  console.error('❌ 没有 build/ 产物。先跑：cd ui && pnpm build');
  process.exit(2);
}
await mkdir(OUT, { recursive: true });

const MIME = {
  '.html': 'text/html',
  '.js': 'text/javascript',
  '.css': 'text/css',
  '.json': 'application/json',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
};
const server = createServer(async (req, res) => {
  const url = (req.url ?? '/').split('?')[0];
  let f = join(ROOT, normalize(url).replace(/^(\.\.[/\\])+/, ''));
  if (!existsSync(f) || f.endsWith('/')) f = join(ROOT, 'index.html');
  try {
    const body = await readFile(f);
    res.writeHead(200, { 'content-type': MIME[extname(f)] ?? 'application/octet-stream' });
    res.end(body);
  } catch {
    res.writeHead(404).end('nope');
  }
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const BASE = `http://127.0.0.1:${server.address().port}`;

const browser = await chromium.launch();
const page = await browser.newPage({
  viewport: { width: W, height: H },
  deviceScaleFactor: ZOOM, // 只为看，不用于量
});
await page.goto(`${BASE}/?demo=1&state=many&settings=1`, { waitUntil: 'networkidle' });
await page.waitForSelector('.cs-set', { timeout: 15000 });
await page.waitForTimeout(1200);

/** 给元素外扩 pad px 后截特写。 */
async function closeup(sel, name, note) {
  const box = await page.locator(sel).first().boundingBox();
  if (!box) {
    console.log(`  ⚠ ${name}：找不到 ${sel}`);
    return;
  }
  const pad = 10;
  const clip = {
    x: Math.max(0, box.x - pad),
    y: Math.max(0, box.y - pad),
    width: box.width + pad * 2,
    height: box.height + pad * 2,
  };
  const file = join(OUT, `${name}.png`);
  await page.screenshot({ path: file, clip });
  console.log(
    `  ${file}  (裁切 ${Math.round(clip.width)}×${Math.round(clip.height)} CSS px @${ZOOM}×` +
      ` ⇒ 成品 ${Math.round(clip.width * ZOOM)}×${Math.round(clip.height * ZOOM)} px)`,
  );
  if (note) console.log(`    ${note}`);
}

const facts = await page.evaluate(() => {
  const inp = document.querySelector('.cs-input');
  const miss = document.querySelector('.cs-badge.miss');
  return {
    border: inp ? getComputedStyle(inp).borderTopColor : '—',
    radius: inp ? getComputedStyle(inp).borderTopLeftRadius : '—',
    bg: inp ? getComputedStyle(inp).backgroundColor : '—',
    miss: miss ? getComputedStyle(miss).boxShadow : '—',
  };
});

console.log(`\n特写 ${ZOOM}×（只看，不量）｜实测 CSS 值：`);
console.log(`  .cs-input   border ${facts.border}  radius ${facts.radius}  bg ${facts.bg}`);
console.log(`  .cs-badge.miss  box-shadow ${facts.miss}`);
console.log('');

await closeup('.cs-badge.miss', '03-closeup-badge', '修前 = none（整条声明作废，从未渲染）');
await closeup(
  '.cs-input',
  '04-closeup-input',
  '修前 rgb(211 218 221) 1.14:1 冷 → 修后 3.04:1 R−B +40',
);

await browser.close();
server.close();
console.log(`\n✅ 特写到 ${OUT}`);
