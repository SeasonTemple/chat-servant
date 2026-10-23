/**
 * 量**候选行（`.cs-act`）**的渲染：静息 / 悬停 / 两种档位。
 *
 * ## 为什么需要它（2026-10-24）
 *
 * 用户报「为什么有一个候选不是纸片样式」——而**看图**分不清是
 * 「样式真的不同」还是「那一行正被鼠标悬停」：
 * · 悬停 = 「把它拿起来」：上抬 2px + 转 0.5° + 影从 `0 3px 6px / .2`
 *   加深到 `0 8px 16px / .26` ⇒ **看起来更像一片纸**；
 * · `Auto` 档 = 整组换成 `cs-act-red`（勾是红铅笔）+ 组级红字警告行
 *   ⇒ **纸片本身没变**，变的是勾的颜色与那行红字。
 *
 * ⇒ 这个脚本把这两种情况**并排拍出来**，不用猜。
 *
 * ## 跑法
 *
 * ```bash
 * cd ui && node scripts/probe-act-rows.mjs [输出目录]
 * ```
 */
import { chromium } from 'playwright-core';
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';

const OUT = process.argv[2] ?? 'target/probe/panel-type';
const BASE = 'http://localhost:1420';
await mkdir(OUT, { recursive: true });

const browser = await chromium.launch({ channel: 'chrome' });
const page = await browser.newPage({
  viewport: { width: 460, height: 760 },
  deviceScaleFactor: 2,
});

const dump = async (tag) => {
  const info = await page.evaluate(() => {
    const rows = [...document.querySelectorAll('.cs-act')];
    return {
      tier: document.querySelector('.cs-pick-name')?.textContent?.trim() ?? null,
      warn: document.querySelector('.cs-act-warn')?.textContent?.trim() ?? null,
      rows: rows.map((el) => {
        const cs = getComputedStyle(el);
        return {
          cls: el.className.replace(/s-\w+/g, '').trim(),
          transform: cs.transform,
          filter: cs.filter,
          borderImage: cs.borderImageSource.slice(0, 42),
        };
      }),
    };
  });
  console.log(`=== ${tag} ===`);
  console.log(JSON.stringify(info));
};

for (const state of ['drafted', 'auto']) {
  await page.goto(`${BASE}/panel/?demo=1&state=${state}`, { waitUntil: 'networkidle' });
  await page.locator('.cs-sheet').first().waitFor({ timeout: 15000 });
  await page.waitForTimeout(900);
  await dump(`${state} · 静息`);
  const acts = page.locator('.cs-actions');
  if (await acts.count()) {
    await acts.screenshot({ path: join(OUT, `act-${state}-rest.png`) });
  }
  if (await page.locator('.cs-act').count()) {
    await page.locator('.cs-act').first().hover();
    await page.waitForTimeout(500);
    await dump(`${state} · 悬停第一条`);
    if (await acts.count()) {
      await acts.screenshot({ path: join(OUT, `act-${state}-hover.png`) });
    }
  }
}
await browser.close();
console.log('done');
