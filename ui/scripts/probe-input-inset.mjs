/**
 * `.cs-input` 的**内陷**方案候选（票 45 第九轮）。
 *
 * ## 形态的由来
 *
 * 用户 2026-10-16 21:3x 原话：
 * > 「按钮是纸片、候选项是纸片，那么输入框你觉得应该是怎么设计？
 * >  **不应该是类似从纸面抠出来的内陷的区域**」
 *
 * ⇒ 这否掉了上一轮「也给它换撕边」的建议，而且理由是**语义**的：
 * **撕边 = 可以从纸上拿起来 = 可点击**。按钮与候选项确实是纸片，因为它们
 * **能被拿起来按**；而输入框不是零件，它是「你往里写字的地方」，
 * 在真实的纸面上对应的是**被压凹 / 挖出来的一道槽**。
 * ⇒ 浮在纸上的东西（撕边或描边）都在说「我可以拿起来」，
 * 对一个输入槽是**错误**的信号。
 *
 * ## 本脚本做两件不能猜的事
 *
 * 1. **`box-shadow: inset` 能不能作用在 `<input>` 上**（替换元素，
 *    UA 样式表有特殊处理；本仓吃过两次「CSS 写了不等于渲染了」的亏）。
 * 2. **三个候选各自长什么样** —— 逐个截特写，并打出实测计算值。
 *
 * ## 候选怎么定的
 *
 * 真实压痕的物理：**凹进去的上/左侧受光少（暗），
 * 凸起挡光的那一侧（���/右侧）反光（亮）**。
 * ⇒ 经典 emboss 用「上左内暗 + 下右内亮」两段 inset 表达。
 * ⚠ 颜色全部**从暖墨 / 纸混出来**，不用冷灰 —— 同源那条仍然成立。
 *
 * ## ⚠ 它不回答「好不好看」—— 那是真机的事
 * ```bash
 * cd ui && pnpm dev --port 5199
 * node scripts/probe-input-inset.mjs
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

/**
 * ⚠⚠ **先把输入框滚到纸内安全区，再裁切。**
 *
 * 第一版直接按 boundingBox 裁，结果**裁到了纸底之外** ——
 * 底部那道灰色是**纸的撕边**，而输入框的**下边/右边落在纸外**，
 * 于是「下右亮边」这条压痕线索**根本没拍到**，
 * 三个候选看起来一模一样（A 与 B 的差别全在看不见的那半边）。
 *
 * 这就是票 45 记的那条：`.cs-set` 铺到 540 而**纸底在 512**，
 * 「元素底 ≠ 纸底」。**判据的参照物必须是被测对象，不是它的容器。**
 *
 * 做法：量输入框相对 `.cs-sheet`（纸）的位置，若贴近纸底就滚动 `.cs-set`，
 * 让它落在纸的中部。
 */
/**
 * ⚠⚠⚠ **纸底是 512，不是 560。**（票 45 原文：「纸底在 y=512，而元素底在 560。
 * 下面 48px 里根本没有纸」）
 *
 * 第一版拿 `.cs-sheet` 的 `boundingBox`（元素盒 560）当「纸」——
 * **那正是票 45 点名说错的那个参照物**。于是裁切区跨进了
 * 512 以下那段**透明**（headless 下看穿成灰白），
 * 看起来像「输入框下面压着一道灰边」，其实是**桌面**。
 *
 * ⇒ 纸底用常量 512（`probe-sheet.svg` 实测），不用元素盒。
 */
const PAPER_BOTTOM = 512;

const rel = await page.evaluate(() => {
  const set = document.querySelector('.cs-set');
  const inp = document.querySelector('.cs-input');
  const ib = inp.getBoundingClientRect();
  const sb = set.getBoundingClientRect();
  return { offsetTop: ib.top - sb.top, setH: sb.height };
});

await page.evaluate(
  (y) => {
    document.querySelector('.cs-set').scrollTop = y;
  },
  Math.max(0, rel.offsetTop - rel.setH * 0.18),
);
await page.waitForTimeout(600);

const sheetBox = await page.locator('.cs-sheet').first().boundingBox();
const box = await page.locator('.cs-input').first().boundingBox();
// 多留上下文：左右各 24、上 16、下 24（都在纸内）
const clip = { x: box.x - 24, y: box.y - 16, width: box.width + 48, height: box.height + 40 };
const paperBottomScreen = sheetBox.y + PAPER_BOTTOM;
const inside = box.y + box.height < paperBottomScreen;
console.log(
  `\n  纸底（实测常量）y=${Math.round(paperBottomScreen)} ` +
    `输入框 y ${Math.round(box.y)}..${Math.round(box.y + box.height)} ` +
    `${inside ? '✅ 完整落在纸内' : '⚠ 越过了纸底'}`,
);

/**
 * 三个候选。都**去掉 border**（内陷的东西不该有「外」的边）、
 * 去掉圆角（`--cs-radius` 注释：面板是纸，直角；圆角只给字条与可点按钮）。
 */
const CANDIDATES = [
  {
    name: 'A 单侧浅压痕',
    css: `
      border: none;
      border-radius: 0;
      box-shadow:
        inset 0 1px 2px color-mix(in srgb, var(--cs-tier-ink) 20%, transparent),
        inset 1px 0 2px color-mix(in srgb, var(--cs-tier-ink) 12%, transparent);
    `,
  },
  {
    name: 'B 双向压痕（上左暗 / 下右亮）',
    css: `
      border: none;
      border-radius: 0;
      box-shadow:
        inset 0 1px 3px color-mix(in srgb, var(--cs-tier-ink) 26%, transparent),
        inset 1px 0 2px color-mix(in srgb, var(--cs-tier-ink) 14%, transparent),
        inset 0 -1px 2px color-mix(in srgb, #fff 60%, transparent),
        inset -1px 0 2px color-mix(in srgb, #fff 45%, transparent);
    `,
  },
  {
    name: 'C 压痕 + 槽底略实',
    css: `
      border: none;
      border-radius: 0;
      background: color-mix(in srgb, var(--cs-tier-ink) 4%, var(--cs-paper));
      box-shadow:
        inset 0 1px 3px color-mix(in srgb, var(--cs-tier-ink) 30%, transparent),
        inset 1px 0 2px color-mix(in srgb, var(--cs-tier-ink) 16%, transparent),
        inset 0 -1px 2px color-mix(in srgb, #fff 55%, transparent);
    `,
  },
  {
    // ⚠ 修正 B 的错：B 用 `color-mix(#fff 60%, transparent)` = **纯白半透明**，
    // 落在暖黄纸上必然显冷（破同源）。真实压痕的亮边是「**纸被照亮**」，
    // 不是「上面加了一层白」⇒ 改成**与纸混**的提亮版。
    name: 'D B的结构 + 亮边改「纸提亮」（同源）',
    css: `
      border: none;
      border-radius: 0;
      box-shadow:
        inset 0 2px 4px color-mix(in srgb, var(--cs-tier-ink) 22%, transparent),
        inset 2px 0 3px color-mix(in srgb, var(--cs-tier-ink) 11%, transparent),
        inset 0 -2px 3px color-mix(in srgb, #fff 42%, var(--cs-paper)),
        inset -2px 0 2px color-mix(in srgb, #fff 30%, var(--cs-paper));
    `,
  },
];

console.log('\n══ `.cs-input` 内陷方案候选 ══\n');

/** 注入用**内联 + important**。
 * ⚠⚠ 第一版用 `<style>` 标签注入，被 Svelte 的 scoped 样式
 * （`svelte-xxx` 类，特异性更高）整体压掉 ⇒ 三个候选全被报成
 * 「❌ 不成立」，而真相是**一个都没注进去**
 * （证据：`radius` 仍是 5px、`border` 仍是 1px、那条 `box-shadow: none`
 * 是 `.cs-input` **原本就有**的 —— 票 42 清 UA 阴影加的）。
 *
 * ⇒ 与票 45 那条纪律第三次同族：**量具错，结论就错**。
 * 所以下面加了**前置判据**：先确认注入真的生效（`radius` 必须变 0），
 * 再看 `inset` 渲没渲染。两者分开报，不再混成一个 ❌。
 */
async function inject(css) {
  await page.evaluate((c) => {
    const el = document.querySelector('.cs-input');
    el.style.cssText = '';
    // 逐条加 !important，绕开 scoped 样式与既有内联
    for (const decl of c.split(';').filter((x) => x.trim())) {
      const [prop, val] = decl.split(':');
      el.style.setProperty(prop.trim(), val.trim(), 'important');
    }
  }, css);
  await page.waitForTimeout(400);
}

for (const [i, c] of CANDIDATES.entries()) {
  await inject(c.css);

  const got = await page.evaluate(() => {
    const cs = getComputedStyle(document.querySelector('.cs-input'));
    return {
      shadow: cs.boxShadow,
      radius: cs.borderTopLeftRadius,
      border: cs.borderTopWidth,
      bg: cs.backgroundColor,
    };
  });

  // 4 段是否都在：按 `, ` 切开数（颜色里的逗号不切，用 inset 出现次数判断更稳）
  const layers = (got.shadow.match(/inset/g) || []).length;
  const hasDown = /0px\s+-\d+px/.test(got.shadow); // 下边（y 偏移为负）
  const hasRight = /-\d+px\s+0px/.test(got.shadow); // 右边（x 偏移为负）
  const LBL = ['a', 'b', 'c', 'd'][i];
  const file = `${OUT}/07-inset-${LBL}.png`;
  await page.screenshot({ path: file, clip });

  // 前置判据：注入到底进没进去
  const injected = got.radius === '0px' && got.border === '0px';

  console.log(`  ${injected ? '✅ 注入生效' : '❌ 注入未生效'}  ${c.name}`);
  console.log(`     radius ${got.radius}  border ${got.border}  bg ${got.bg}`);
  console.log(
    `     inset 段数 ${layers} 下边亮边 ${hasDown ? '✅' : '—'} 右边亮边 ${hasRight ? '✅' : '—'}`,
  );
  console.log(`     ${got.shadow}`);
  console.log(`     → ${file}`);
  await page.evaluate(() => {
    document.querySelector('.cs-input').style.cssText = '';
  });
  console.log('');
}

await browser.close();
console.log('  ⚠ headless 只能证明「内陷的线索渲染出来了」，**不能定观感**。');
console.log('    票 45 第六轮：五轮 headless 全绿，真机当场撞出缺陷。');
