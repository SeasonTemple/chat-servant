/** 跑 `ui/static/budget.html` 量具并把判定读出来（票 88 用）。 */
import { chromium } from 'playwright-core';

const browser = await chromium.launch({ channel: 'chrome' });
const page = await browser.newPage({ viewport: { width: 900, height: 900 } });
await page.goto('http://localhost:1420/budget.html', { waitUntil: 'networkidle' });
await page.waitForFunction(() => document.title === 'READY', { timeout: 60000 });
const verdict = await page.locator('#verdict').innerText();
const raw = await page.locator('#out').innerText();
console.log('=== 判定 ===');
console.log(verdict);
const data = JSON.parse(raw);
for (const [k, s] of Object.entries(data.states)) {
  if (s.error) {
    console.log(`${k}: 载入失败 ${s.error}`);
    continue;
  }
  const t = s.pick_clipped;
  console.log(
    `${k.padEnd(11)} 档位格 slotBottom=${t?.slotBottom} 纸底=${t?.paperBottom} ` +
      `⇒ clipped=${t?.clipped} over=${t?.over} 草稿scrollH=${s.draft?.scrollH} ` +
      `候选scrollH=${s.actsScroll?.scrollH ?? s.actions?.scrollH ?? '-'}`,
  );
}
await browser.close();
