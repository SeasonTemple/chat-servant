/**
 * 量**面板头部**在三态里的位置与高度：`.cs-where`（归属行）、`.cs-body-col`、`.cs-pick`（档位行）。
 *
 * ## 为什么量它（2026-10-24，用户报「标题部分样式在动」）
 *
 * 用户原话：「开始生成候选后为什么标题部分样式也在动？图1对比图2」
 * ⇒ 要回答的不是「哪里动了」，而是**哪条 flex 规则让整列高度一变就重排**。
 *
 * 用法：cd ui && node scripts/probe-header.mjs
 */
import { chromium } from 'playwright-core';

const BASE = 'http://localhost:1420';
const browser = await chromium.launch({ channel: 'chrome' });
const page = await browser.newPage({ viewport: { width: 460, height: 760 }, deviceScaleFactor: 2 });

for (const state of ['noprep', 'drafted', 'sent']) {
  await page.goto(`${BASE}/panel/?demo=1&state=${state}`, { waitUntil: 'networkidle' });
  await page.locator('.cs-sheet').first().waitFor({ timeout: 15000 });
  await page.waitForTimeout(700);
  const m = await page.evaluate(() => {
    const q = (s) => document.querySelector(s);
    const rect = (el) => {
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { top: +r.top.toFixed(1), h: +r.height.toFixed(1), bottom: +r.bottom.toFixed(1) };
    };
    const body = q('.cs-body-col');
    return {
      where: rect(q('.cs-where')),
      whereLines: q('.cs-where')?.textContent?.trim().split('\n').length ?? 0,
      whereText: (q('.cs-where')?.textContent ?? '').replace(/\s+/g, ' ').trim().slice(0, 40),
      draft: rect(q('.cs-draft')),
      actions: rect(q('.cs-actions')),
      pick: rect(q('.cs-pick')),
      body: rect(body),
      bodyKids: body ? [...body.children].map((c) => c.className.split(' ')[0]) : [],
      sheet: rect(q('.cs-sheet')),
    };
  });
  console.log(`=== ${state} ===`);
  console.log(JSON.stringify(m, null, 1));
}
await browser.close();
