/**
 * 量「面板那张纸到底多大，跟视口有没有关系」—— 冷启动包里那条例外的证据。
 *
 * `docs/HANDOFF.md` §「输出尺寸证明不了布局尺寸」原本把这条判据挂在一个
 * `ui/scripts/_cardbox.py` 上，而那个文件**从来没有进过仓库**（`git log --all --
 * --diff-filter=D` 与文件名搜索都零命中）⇒ 引用是死的。这个脚本是它该指的东西。
 *
 * 判据：`.cs-sheet` 的 `getBoundingClientRect()`（**CSS px**，不受
 * `deviceScaleFactor` 影响）在两个差得很远的视口下**相同**，且等于
 * `tauri.conf.json` 的 380×560。证明「面板被取景框钉死 ⇒ 浏览器量具可用」。
 *
 * ⚠ 它量的是**布局矩形**，不是截图尺寸——后者才是量不了布局的那个数。
 *
 * 跑法：`node ui/scripts/probe-sheet-fit.mjs`（dev server 需在 1420 上）。
 */
import { chromium } from 'playwright-core';

const BASE = process.argv[2] ?? 'http://localhost:1420';
const VIEWPORTS = [
  { width: 520, height: 760 },
  { width: 1200, height: 900 },
];

const browser = await chromium.launch({ channel: 'chrome' });
const rows = [];
for (const viewport of VIEWPORTS) {
  const page = await browser.newPage({ viewport, deviceScaleFactor: 1 });
  await page.goto(`${BASE}/?demo=1&state=drafted`, { waitUntil: 'networkidle' });
  const sheet = page.locator('.cs-sheet').first();
  await sheet.waitFor({ timeout: 15000 });
  const box = await page.evaluate(() => {
    const r = document.querySelector('.cs-sheet').getBoundingClientRect();
    return { w: Math.round(r.width * 10) / 10, h: Math.round(r.height * 10) / 10 };
  });
  rows.push({ viewport: `${viewport.width}x${viewport.height}`, ...box });
  await page.close();
}
await browser.close();

for (const r of rows) console.log(`${r.viewport}\tsheet ${r.w} x ${r.h}`);
const same = rows.every((r) => r.w === rows[0].w && r.h === rows[0].h);
console.log(same ? 'SHEET_FIT: SAME (视口无关)' : 'SHEET_FIT: DIFFERS (视口影响了面板)');
