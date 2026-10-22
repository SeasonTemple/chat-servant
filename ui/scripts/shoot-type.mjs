/**
 * 出「字号阶梯」的样本图（票 35）。
 *
 * ## 它拍的是什么
 *
 * 工作台的 E 段（`/studio?only=ladder`）：**同一条中性正文**，
 * 按多个字号各排一张 sheet。纸底默认**真实纸材位图**（票 35 的口径），
 * `?paper=solid` 出纯色对照。
 *
 * ## 为什么一次只出一档
 *
 * 票 24 的教训：一个 sheet 混排多种字号时，行切分会把两种字号的墨
 * 并进同一个掩码 ⇒ 量出来的不是那个字号的东西。
 * ⇒ 这里**每档单独开一个 page**（新 URL = 新挂载，参数立刻生效）。
 *
 * ⚠ 票 35 记过：在 app 的 webview 里，探针**在页面挂载时读一次**，
 * 改 URL 参数不重启就不生效 ⇒ 当时「每换一个组合就得重启一次应用」。
 * **本脚本用无头 Chromium，一次进程内换 URL 即换组合** ⇒ 那条限制不成立。
 *
 * ## ⚠⚠ 它**不能**证明什么（读图之前先看这段）
 *
 * 1. **headless ≠ 真机 WebView2。** 票 45 第六轮：「第一次上真机，当场撞出
 *    一个五轮 headless 全绿的缺陷」。字体栅格化、DPI、合成路径都不是一回事。
 *    ⇒ 这些图的效力上限是「**在 Chromium 的栅格化下**这条判据成不成立」。
 * 2. **1× 出图**（`deviceScaleFactor: 1`）——`ui/scripts/strokeval.py` 的
 *    `--font-px` 用的就是这个口径（CSS px = 图上 px）。
 *    3× 出图的话字号要乘 3，量具的窗宽就全错了。
 *
 * ## 字体静默回落（app.css 里记着的坑）
 *
 * `?draft=lxgw` 在没装字体的机器上会静默回落成楷体，图看着「只是字体没生效」，
 * 不比对就会误判成「这个字体效果不行」。
 * ⇒ 本脚本**每次都报出实际生效的字体**：用 canvas `measureText` 比宽度，
 * 与一个不存在的字体族同宽 ⇒ 就是回落了。
 *
 * ## 跑法
 *
 * ```bash
 * cd ui && node scripts/shoot-type.mjs <输出目录> [--font=fangsong|song|kai]
 *                                       [--weight=400] [--paper=texture|solid]
 *                                       [--sizes=21,17,14,12,11]
 * ```
 */
import { chromium } from 'playwright-core';
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';

const args = process.argv.slice(2);
const OUT = args.find((a) => !a.startsWith('--')) ?? 'target/probe/type';
const opt = (name, dflt) => {
  const hit = args.find((a) => a.startsWith(`--${name}=`));
  return hit ? hit.slice(name.length + 3) : dflt;
};

const FONT = opt('font', 'fangsong');
const WEIGHT = opt('weight', '400');
const PAPER = opt('paper', 'texture');
const SIZES = opt('sizes', '21,17,14,12,11')
  .split(',')
  .map((s) => Number(s.trim()))
  .filter(Boolean);

const W = 380;
const H = 560;
const BASE = opt('base', 'http://localhost:1420');
// ⚠ **1× 是量具口径**（strokeval 的 --font-px 用 CSS px = 图上 px）。
// ⚠ **3× 只给人眼看细节**——量具吃 3× 时字号要乘 3，窗宽就全错了。
const SCALE = Number(opt('scale', '1'));

await mkdir(OUT, { recursive: true });

// ⚠ **用系统 Chrome，不下 playwright 的浏览器**（2026-10-22 实测）：
// 本机 `ms-playwright` 里只有 `chromium-1208`，而 playwright-core 1.64 要 1248
// ⇒ `chromium.launch()` 直接报「Executable doesn't exist」。
// 而 `npx playwright install` 会把浏览器下到 **C 盘**（%LOCALAPPDATA%\ms-playwright）
// ⇒ 违反本仓「产物留项目盘」的规矩 ⇒ **不这么做**。
// 改用系统已装的 Chrome（Blink + DirectWrite，与 WebView2 同代栅格化）。
const CHANNEL = opt('channel', 'chrome');
const browser = await chromium.launch({ channel: CHANNEL });
console.log(`浏览器：系统 ${CHANNEL}（不是 playwright 自带的）`);

/**
 * 量出「实际生效的字体」：**渲染成像素再比**。
 *
 * ⚠⚠ **不能用 measureText 的宽度判**（2026-10-22 实测踩到）：
 * 中文是**全宽**字，所有 CJK 字体的字宽都一样（1em）⇒
 * 「仿宋 / 宋体 / 楷体 / 一个不存在的字体族」量出来的宽度**完全相同**
 * ⇒ 宽度法**必然**把每个字体都误报成「回落了」。
 * ⇒ 改用**像素**：同一段字分别用目标字体与一个不存在的字体族画进 canvas，
 * 比 dataURL。字形不同 ⇒ 像素不同 ⇒ 这才判得出回落没回落。
 */
async function effectiveFonts() {
  return await page.evaluate(() => {
    const S = '周三Ag';
    const render = (family) => {
      const c = document.createElement('canvas');
      c.width = 320;
      c.height = 64;
      const ctx = c.getContext('2d');
      ctx.fillStyle = '#fff';
      ctx.fillRect(0, 0, c.width, c.height);
      ctx.fillStyle = '#000';
      ctx.font = `40px ${family}`;
      ctx.textBaseline = 'top';
      ctx.fillText(S, 4, 4);
      return c.toDataURL();
    };
    const base = render('__no_such_family_zzz__');
    const named = {
      fangsong: ["FangSong, STFangsong, '仿宋', serif"],
      song: ["SimSun, '宋体', serif"],
      kai: ["KaiTi, STKaiti, '楷体', serif"],
    };
    const out = { __base: base.length };
    for (const [k, stacks] of Object.entries(named)) {
      const d = render(stacks[0]);
      out[k] = { same: d === base, px: d.length };
    }
    return out;
  });
}

const page = await browser.newPage({
  viewport: { width: W, height: H },
  deviceScaleFactor: SCALE,
});

const want = FONT === 'song' ? 'SimSun' : FONT === 'kai' ? 'KaiTi' : 'FangSong';
const check = await effectiveFonts();
console.log(`字体自检（**像素**比对：与「不存在的字体族」画出来一样 ⇒ 静默回落）:`);
console.log(`  __base（不存在的字体族）  ${check.__base} 字节`);
for (const [k, v] of Object.entries(check)) {
  if (k === '__base') continue;
  console.log(
    `  ${k.padEnd(10)} ${v.same ? '⚠ 与 fallback 逐像素相同 ⇒ 回落了' : '✓ 与 fallback 不同 ⇒ 生效'}`,
  );
}
if (check[want] && check[want].same) {
  console.log(`\n⚠⚠ 要的字体（${want}）在本机**没有生效**（静默回落）——`);
  console.log(`   出了图也不能当这一档字体的证据。先装字体或换一族。`);
}

console.log(`\n出图目录 ${OUT}`);
console.log(`参数 font=${FONT} weight=${WEIGHT} paper=${PAPER} sizes=${SIZES.join(',')}`);
console.log(`伺服 ${BASE}（dev server），viewport ${W}×${H}，deviceScaleFactor=1\n`);

for (const px of SIZES) {
  const url = `${BASE}/studio?only=ladder&sizes=${px}&font=${FONT}&weight=${WEIGHT}&paper=${PAPER}`;
  await page.goto(url, { waitUntil: 'networkidle' });
  const crop = page.locator('.crop[data-ladder]').first();
  await crop.waitFor({ timeout: 15000 });
  await page.waitForTimeout(400);
  const file = join(OUT, `type-${FONT}-${WEIGHT}-${px}${SCALE === 1 ? '' : `@${SCALE}x`}.png`);
  await crop.screenshot({ path: file });

  // 顺手报出「这张图里到底有什么」：行数、每行字数（量的输入，得是真的）
  const facts = await page.evaluate(() => {
    const b = document.querySelector('.bodytype');
    if (!b) return null;
    const r = b.getBoundingClientRect();
    const cs = getComputedStyle(b);
    return {
      fontSize: cs.fontSize,
      fontFamily: cs.fontFamily,
      fontWeight: cs.fontWeight,
      lineHeight: cs.lineHeight,
      width: Math.round(r.width * 100) / 100,
      height: Math.round(r.height * 100) / 100,
      text: b.textContent ?? '',
    };
  });
  console.log(`  ${px}px  ->  ${file}`);
  if (facts) {
    console.log(
      `        font=${facts.fontSize}/${facts.fontWeight}  family=${facts.fontFamily}  ` +
        `line-height=${facts.lineHeight}  box=${facts.width}×${facts.height}`,
    );
  }
}

// 整页一张（五档依次排列），给人看的
await page.goto(
  `${BASE}/studio?only=ladder&sizes=${SIZES.join(',')}&font=${FONT}&weight=${WEIGHT}&paper=${PAPER}`,
  { waitUntil: 'networkidle' },
);
await page.locator('.crop[data-ladder]').first().waitFor({ timeout: 15000 });
await page.waitForTimeout(600);
const sheet = join(
  OUT,
  `type-ladder-${FONT}-${WEIGHT}-${PAPER}${SCALE === 1 ? '' : `@${SCALE}x`}.png`,
);
await page.screenshot({ path: sheet, fullPage: true });
console.log(`\n  整页（五档依次） -> ${sheet}`);

await browser.close();
console.log(`\n✅ 出图完成`);
