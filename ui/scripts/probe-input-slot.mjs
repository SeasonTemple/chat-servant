/**
 * `.cs-input` 的**内陷 + 撕边外形**候选（票 45 第十轮）。
 *
 * ## 形态的由来
 *
 * 用户 2026-10-16 原话（**两轮，第二轮把我读反的那句纠正回来**）：
 * > 「就是要内陷啊？但也不应边框这么周正吧？」
 *
 * ⇒ 两件事同时定下来：
 *  1. **内陷**（不是撕边纸片）—— 内陷对，因为撕边说「可以拿起来」，
 *     而输入槽不是可拿起的零件，它是你往里写字的地方。
 *  2. **形状不周正** —— 而「周正的矩形」正是 Web 控件的标志。
 *     这一屏所有真零件的边都是**撕出来的**（`strip.png`），
 *     一个方方正正的框放在它们中间，就是唯一的异类。
 *
 * ⇒ 组合：**外形 = 撕边位图**（不周正、有毛边、纸上味），
 *   **立体 = inset 阴影**（上左暗 / 下右亮 = 压凹的物理线索）。
 *
 * ## 这一版要验的技术点
 *
 * `border-image` 与 `box-shadow: inset` **能不能同时生效**：
 * 撕边占的是 border 区，inset 阴影画在 border 内侧 —— 两者不该打架，
 * 但**必须实测**（本仓吃过两次「CSS 写了不等于渲染了」的亏）。
 * 另外要处理 `fill`：**洞底该略深**，而 `fill` 会在 background 之上
 * 画一层纸芯把底色盖住 ⇒ 候选分「带 fill」与「不带 fill」两路。
 *
 * ## ⚠ headless 只能证明「线索渲染出来了」，定不了观感
 * ```bash
 * cd ui && pnpm dev --port 5199
 * node scripts/probe-input-slot.mjs
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

// ⚠ 纸底是 **512**（probe-sheet.svg 实测），不是 `.cs-sheet` 元素盒的 560
// —— 票 45：纸底 512，元素底 560，下面 48px 根本没有纸。
const PAPER_BOTTOM = 512;
const sheetBox = await page.locator('.cs-sheet').first().boundingBox();
const rel = await page.evaluate(() => {
  const set = document.querySelector('.cs-set');
  const inp = document.querySelector('.cs-input');
  return {
    offsetTop: inp.getBoundingClientRect().top - set.getBoundingClientRect().top,
    setH: set.getBoundingClientRect().height,
  };
});
await page.evaluate(
  (y) => {
    document.querySelector('.cs-set').scrollTop = y;
  },
  Math.max(0, rel.offsetTop - rel.setH * 0.18),
);
await page.waitForTimeout(600);

const box = await page.locator('.cs-input').first().boundingBox();
const clip = { x: box.x - 24, y: box.y - 16, width: box.width + 48, height: box.height + 40 };
console.log(
  `\n  纸底 y=${Math.round(sheetBox.y + PAPER_BOTTOM)} 输入框 y ${Math.round(box.y)}..` +
    `${Math.round(box.y + box.height)} ${box.y + box.height < sheetBox.y + PAPER_BOTTOM ? '✅ 纸内' : '⚠ 越界'}\n`,
);

/** 凹陷暗/亮两段，全部从暖墨与纸混出来（不用纯白，见上一轮的 B 教训）。 */
const DARK = (a) => `color-mix(in srgb, var(--cs-tier-ink) ${a}%, transparent)`;
const LIT = (a) => `color-mix(in srgb, #fff ${a}%, var(--cs-paper))`;

const CANDIDATES = [
  {
    id: 'e',
    name: 'E 撕边外形 + 内陷（不带 fill，洞底＝纸）',
    css: `
      border: 12px solid transparent;
      border-image: url('/strip.png') 32 / 12px / 0 stretch;
      border-radius: 0;
      background: var(--cs-paper);
      box-shadow:
        inset 0 2px 5px ${DARK(26)},
        inset 2px 0 3px ${DARK(13)},
        inset 0 -2px 3px ${LIT(40)},
        inset -2px 0 2px ${LIT(28)};
    `,
  },
  {
    id: 'f',
    name: 'F 撕边 + 洞底压实（暖墨混纸，更像真的凹进去）',
    css: `
      border: 12px solid transparent;
      border-image: url('/strip.png') 32 / 12px / 0 stretch;
      border-radius: 0;
      background: color-mix(in srgb, var(--cs-tier-ink) 7%, var(--cs-paper));
      box-shadow:
        inset 0 3px 7px ${DARK(30)},
        inset 3px 0 4px ${DARK(15)},
        inset 0 -2px 3px ${LIT(46)},
        inset -2px 0 2px ${LIT(30)};
    `,
  },
  {
    id: 'g',
    name: 'G 撕边 + 内陷 + 聚焦反馈位（更深的槽）',
    css: `
      border: 12px solid transparent;
      border-image: url('/strip.png') 32 / 12px / 0 stretch;
      border-radius: 0;
      background: color-mix(in srgb, var(--cs-tier-ink) 11%, var(--cs-paper));
      box-shadow:
        inset 0 4px 10px ${DARK(34)},
        inset 4px 0 5px ${DARK(18)},
        inset 0 -3px 4px ${LIT(52)},
        inset -3px 0 2px ${LIT(34)};
    `,
  },
];

console.log('══ `.cs-input` 内陷 + 撕边外形 ══\n');

for (const c of CANDIDATES) {
  await page.evaluate((css) => {
    const el = document.querySelector('.cs-input');
    el.style.cssText = '';
    for (const d of css.split(';').filter((x) => x.trim())) {
      const i = d.indexOf(':');
      el.style.setProperty(d.slice(0, i).trim(), d.slice(i + 1).trim(), 'important');
    }
  }, c.css);
  await page.waitForTimeout(450);

  const got = await page.evaluate(() => {
    const cs = getComputedStyle(document.querySelector('.cs-input'));
    return {
      bi: cs.borderImageSource,
      biSlice: cs.borderImageSlice,
      biFill: cs.borderImageRepeat,
      shadow: cs.boxShadow,
      borderW: cs.borderTopWidth,
      radius: cs.borderTopLeftRadius,
      bg: cs.backgroundColor,
    };
  });

  const file = `${OUT}/08-slot-${c.id}.png`;
  await page.screenshot({ path: file, clip });

  const injected = got.borderW !== '0px' && /strip\.png/.test(got.bi);
  const insetOk = /inset/.test(got.shadow);
  const bothOk = injected && insetOk;

  console.log(`  ${bothOk ? '✅' : injected ? '⚠' : '❌'} ${c.name}`);
  console.log(
    `     border-image ${got.bi.replace(/https?:\/\/[^/]+/, '')}  width ${got.borderW}  radius ${got.radius}`,
  );
  console.log(
    `     ${insetOk ? '✅' : '❌'} inset 段数 ${(got.shadow.match(/inset/g) || []).length} bg ${got.bg}`,
  );
  console.log(`     → ${file}`);
  await page.evaluate(() => {
    document.querySelector('.cs-input').style.cssText = '';
  });
  console.log('');
}

await browser.close();
console.log('  ⚠ headless 只能证明「线索渲染出来了」，**定不了观感**（票 45 第六轮）。');
