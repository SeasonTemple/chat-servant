/**
 * 出设置屏对比图：**修之前** vs **修之后**（票 45 第七轮）。
 *
 * ## 它拍的是什么
 *
 * `pnpm build` 的**真实产物**（不是 dev server）里的
 * `?demo=1&state=many&settings=1` —— 380×560，与真机 Tauri 窗口同尺寸
 * （票 45 第六轮 `shotpanel.ps1` 实测 `rect 246,215 380x560`）。
 *
 * `state=many` 是票 45 补的那份「复核夹具」，它带两组模型配置：
 * 一组「已配置」（徽标 + 三颗按钮）、一组「未配置」（直接给输入框）
 * ⇒ **`.cs-input` 与 `.cs-badge.miss` 都会渲染**。
 *
 * ## ⚠⚠ 它**不能**证明什么（读图之前先看这段）
 *
 * 1. **headless ≠ 真机。** 票 45 第六轮：「第一次上真机，当场撞出
 *    一个五轮 headless 全绿的缺陷」。WebView2 的分层合成、透明窗口、
 *    DPI 缩放与 headless Chromium 都不是一回事。
 *    ⇒ 这些图的效力上限是「**那条声明生效了没有**」与
 *      「**渲染后的颜色是不是我以为的那个数**」。
 * 2. **demo ≠ 真实运行时。** 真实 Tauri 窗口读的是 Windows 凭据管理器，
 *    而这里是静态夹具；键名、路由、徽标状态都是摆出来的。
 * 3. **票 45 量具提醒**：`shoturl.ps1` 默认 `-Scale 2` 时像素不是 CSS px，
 *    且会截到 GSAP 补间的中间帧 —— 所以本脚本固定 **1×**，
 *    并在截图前等补间跑完。
 *
 * ## 跑法
 *
 * ```bash
 * cd ui && pnpm build                     # 必须先 build（截的是产物）
 * node scripts/shoot-compare.mjs <输出目录>
 * ```
 */
import { chromium } from 'playwright-core';
import { createServer } from 'node:http';
import { readFile, mkdir } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { extname, join, normalize } from 'node:path';

const OUT = process.argv[2] ?? '/tmp/edge-shots';
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

// 静态伺服：SPA 路由一律回落 index.html
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
const PORT = server.address().port;
const BASE = `http://127.0.0.1:${PORT}`;

const browser = await chromium.launch();

/** 截一屏。deviceScaleFactor=1 ⇒ 1 图�� px = 1 CSS px（票 45 量具提醒）。 */
async function shoot(name, note) {
  const page = await browser.newPage({
    viewport: { width: W, height: H },
    deviceScaleFactor: 1,
  });
  await page.goto(`${BASE}/?demo=1&state=many&settings=1`, { waitUntil: 'networkidle' });
  await page.waitForSelector('.cs-set', { timeout: 15000 });
  // 等 GSAP 补间跑完，否则量到/截到中间帧
  await page.waitForTimeout(1200);

  // 顺带把实测值打出来，图和数一起看
  const facts = await page.evaluate(() => {
    const inp = document.querySelector('.cs-input');
    const miss = document.querySelector('.cs-badge.miss');
    return {
      inputBorder: inp ? getComputedStyle(inp).borderTopColor : '（无输入框）',
      missShadow: miss ? getComputedStyle(miss).boxShadow : '（无 .cs-badge.miss）',
    };
  });

  const file = join(OUT, `${name}.png`);
  await page.screenshot({ path: file });
  await page.close();
  console.log(`\n── ${name} ────────────────────────────────`);
  console.log(`  ${file}`);
  if (note) console.log(`  ${note}`);
  console.log(`  .cs-input 描边  ${facts.inputBorder}`);
  console.log(`  .cs-badge.miss  ${facts.missShadow}`);
  return file;
}

console.log(`静态伺服 ${BASE}（截 build/ 产物，${W}×${H}，1×）`);
await shoot('01-after', '修后：描边 = --cs-edge（暖墨混纸 51%）');

// 静息 / 聚焦 两态特写：内陷是常态形体，聚焦是「槽变深」
{
  const p2 = await browser.newPage({ viewport: { width: W, height: H }, deviceScaleFactor: 4 });
  await p2.goto(`${BASE}/?demo=1&state=many&settings=1`, { waitUntil: 'networkidle' });
  await p2.waitForSelector('.cs-input', { timeout: 15000 });
  await p2.waitForTimeout(1000);
  await p2.evaluate(() => {
    const s = document.querySelector('.cs-set');
    const i = document.querySelector('.cs-input');
    s.scrollTop = Math.max(
      0,
      i.getBoundingClientRect().top -
        s.getBoundingClientRect().top -
        s.getBoundingClientRect().height * 0.18,
    );
  });
  await p2.waitForTimeout(500);
  const b2 = await p2.locator('.cs-input').first().boundingBox();
  const c2 = { x: b2.x - 24, y: b2.y - 16, width: b2.width + 48, height: b2.height + 40 };
  await p2.screenshot({ path: join(OUT, '09-input-inset-rest.png'), clip: c2 });
  await p2.locator('.cs-input').first().focus();
  await p2.waitForTimeout(350);
  await p2.screenshot({ path: join(OUT, '10-input-inset-focus.png'), clip: c2 });
  await p2.close();
  console.log(`\n── 内陷两态 ────────────────────────────────`);
  console.log(`  ${join(OUT, '09-input-inset-rest.png')}  (静息)`);
  console.log(`  ${join(OUT, '10-input-inset-focus.png')}  (聚焦：槽更深)`);
}

await browser.close();
server.close();
console.log(`\n✅ 图已出到 ${OUT}`);
