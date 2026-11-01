/**
 * 把「真机那一眼」要看的面板态摆成一张对照板（八格 + 悬停对照）。
 *
 * ## 为什么有这个脚本
 *
 * 一批验收项写的是「字看着顺不顺」「措辞读着自然不自然」——这种东西机器判不了，
 * 但「把判断需要的材料摆齐」能做。原来要开七张票、跑七遍量具、
 * 再自己拼图，现在一条命令出一张 PNG。
 *
 * ## 它拍的是什么
 *
 * `http://localhost:1420/?demo=1&state=<夹具名>`，取 `.cs-sheet`（那张纸，380×560，
 * 与真机 Tauri 窗口同尺寸），`deviceScaleFactor: 2`。每一格的标题写明它对应
 * 哪张票的哪一条未勾项。
 *
 * ## ⚠⚠ 它**不能**证明什么
 *
 * 1. **headless Chromium ≠ 真机 WebView2**：字体族回退与亚像素渲染不同
 *    （同 `shoot-panel.mjs` 的提醒）。
 * 2. **不含微信窗口与托盘**，也没有真实输入——所以「点一下」类的验收项
 *    **不在本板覆盖范围内**。
 * 3. demo 数据是假的（排版、字号、措辞、笔迹是真的）。
 *
 * ## 跑法
 *
 * ```bash
 * cd ui && node scripts/shoot-board.mjs [输出目录]
 * ```
 *
 * 默认输出到 `<仓库>/target/probe/panel-board/（`target/` 已 gitignore）。
 * 需要 dev server 在 `http://localhost:1420`（`cd ui && pnpm dev`）。
 */
import { chromium } from 'playwright-core';
import { mkdir, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const OUT = process.argv[2] ? resolve(process.argv[2]) : join(ROOT, 'target/probe/panel-board');
const BASE = process.env.CS_BASE ?? 'http://localhost:1420';
const SHOTS = join(OUT, 'shots');

/**
 * 八格：每格 = 一个夹具 + 它在哪张票里承担什么判断。
 * ⚠ 加夹具就加一行，不要把每格写死在模板字符串里。
 */
const CELLS = [
  {
    state: 'drafted',
    title: '① drafted',
    subs: ['票 97 层级：名字 / 引文 / 状态句 / 候选 / 档位', '票 92 候选措辞 · 票 94 最优选那枚勾'],
  },
  { state: 'auto', title: '② auto（自动发送档）', subs: ['票 92 措辞：档位与提示句换了'] },
  { state: 'drafting', title: '③ drafting（起草中）', subs: ['票 97 层级：状态句是否压得住'] },
  {
    state: 'many',
    title: '④ many（候选很多）',
    subs: ['票 44/88：候选区吃掉余量、档位行还在不在纸底'],
  },
  {
    state: 'longquote',
    title: '⑤ longquote（对方最后一句 200 字）',
    subs: ['票 92：引文夹到 3 行后读着还自然吗'],
  },
  {
    state: 'longdraft',
    title: '⑥ longdraft（长草稿）',
    subs: ['票 88：长草稿把档位行顶出纸了吗', '票 91：正文下缘是化掉的，不是齐腰切断'],
  },
  {
    state: 'auto-longdraft',
    title: '⑦ auto-longdraft（自动档 + 长草稿）',
    subs: ['票 91：同上，但档位在自动档'],
  },
];

const HOVER_PAIR = [
  ['静息', join(ROOT, 'target/probe/issue90-after/act-drafted-0-rest.png')],
  ['悬停', join(ROOT, 'target/probe/issue90-after/act-drafted-0-hover-lift.png')],
];

await mkdir(SHOTS, { recursive: true });

// ⚠ 用系统 Chrome：playwright 自带浏览器版本对不上，且 `npx playwright install`
// 会往 C 盘下（违反「产物留项目盘」）。
const browser = await chromium.launch({ channel: 'chrome' });
const page = await browser.newPage({ viewport: { width: 460, height: 700 }, deviceScaleFactor: 2 });

const files = [];
for (const cell of CELLS) {
  await page.goto(`${BASE}/?demo=1&state=${cell.state}`, { waitUntil: 'networkidle' });
  const sheet = page.locator('.cs-sheet').first();
  await sheet.waitFor({ timeout: 15000 });
  // ⚠ 等补间跑完：截到补间中间帧 = 量到半个状态（同 shoot-panel.mjs）。
  await page.waitForTimeout(1200);
  const file = join(SHOTS, `${cell.state}@2x.png`);
  await sheet.screenshot({ path: file });
  files.push(file);
  console.log(`出图 ${cell.state} -> ${file}`);
}
await browser.close();

const hoverOk = HOVER_PAIR.every(([, p]) => existsSync(p));
const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
const cellHtml = CELLS.map(
  (cell) => `  <figure>
    <figcaption>${esc(cell.title)}<span>${cell.subs.map(esc).join('<br>')}</span></figcaption>
    <img src="shots/${cell.state}@2x.png">
  </figure>`,
).join('\n');

const hoverHtml = hoverOk
  ? `  <figure>
    <figcaption>⑧ 悬停对照（票 90）<span>左＝静息，右＝悬停（只剩上抬 + 影）<br>两图放大看候选条边缘糊不糊</span></figcaption>
    <div class="hover">
      <img src="${pathToFileURL(HOVER_PAIR[0][1]).href}">
      <img src="${pathToFileURL(HOVER_PAIR[1][1]).href}">
    </div>
  </figure>`
  : `  <figure>
    <figcaption>⑧ 悬停对照（票 90）<span>⚠ 缺图：先跑 <code>node scripts/probe-act-states.mjs</code></span></figcaption>
  </figure>`;

const html = `<!doctype html><meta charset="utf-8">
<style>
  body{margin:0;padding:28px 32px 40px;background:#e8e2d2;font-family:'Microsoft YaHei UI','Microsoft YaHei',system-ui,sans-serif;color:#2b2620}
  h1{font-size:26px;margin:0 0 6px}
  p.lead{margin:0 0 22px;font-size:15px;line-height:1.7;color:#4a4238}
  .grid{display:grid;grid-template-columns:repeat(3,1fr);gap:26px 22px}
  figure{margin:0}
  figcaption{font-size:14px;font-weight:600;margin:0 0 8px;line-height:1.5}
  figcaption span{display:block;font-weight:400;color:#5c5348;font-size:13px}
  img{display:block;width:100%;height:auto;border-radius:4px}
  .hover{display:grid;grid-template-columns:1fr 1fr;gap:14px}
  .hover img{width:100%}
  .warn{background:#f7efd8;border-left:4px solid #b8860b;padding:10px 14px;font-size:13px;line-height:1.7;margin-top:20px}
  .note{margin-top:26px;font-size:13px;line-height:1.7;color:#5c5348}
  code{font-family:ui-monospace,Consolas,monospace;font-size:12.5px;background:#dcd5c2;padding:1px 4px;border-radius:3px}
</style>
<h1>面板人眼复核板</h1>
<p class="lead">全部为 <b>demo 夹具 + headless Chrome 2×</b>，不是真机 WebView2——<b>排版、字号、措辞、笔迹是真的</b>（同一套 CSS 与组件），只是数据是假的。每一格都标了它对应哪张票的哪一条。</p>
<div class="grid">
${cellHtml}
${hoverHtml}
</div>
<div class="warn"><b>它不能证明什么</b>：headless Chromium ≠ 真机 WebView2 的字体族回退与亚像素渲染；也不含微信窗口与托盘，所以「点一下」类的验收项不在本板覆盖范围内。</div>
<div class="note">每张图都是 CSS 尺寸的 2 倍像素，放大看细节不会糊。本页与图片由 <code>ui/scripts/shoot-board.mjs</code> 生成（输出在 <code>target/</code>下，不入库）。</div>
`;

const htmlPath = join(OUT, 'board.html');
await writeFile(htmlPath, html, 'utf8');

const shot = await chromium.launch({ channel: 'chrome' });
const shotPage = await shot.newPage({
  viewport: { width: 1600, height: 1200 },
  deviceScaleFactor: 1,
});
await shotPage.goto(pathToFileURL(htmlPath).href, { waitUntil: 'load' });
await shotPage.waitForTimeout(400);
const board = join(OUT, 'board.png');
await shotPage.screenshot({ path: board, fullPage: true });
const dim = await shotPage.evaluate(() => ({
  w: document.documentElement.scrollWidth,
  h: document.documentElement.scrollHeight,
}));
await shot.close();

console.log(`\n对照板 ${board}  (${dim.w}×${dim.h})`);
console.log(`八格夹具：${CELLS.map((c) => c.state).join(' / ')}`);
if (!hoverOk) console.log('⚠ 悬停对照缺图：先跑 node scripts/probe-act-states.mjs');
