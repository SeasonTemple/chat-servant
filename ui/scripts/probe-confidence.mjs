/**
 * 量**置信度笔迹**（2026-10-25，票 92 §十五）。
 *
 * ## 它量什么
 *
 * 面板那一格**不再显示百分数**，改画一道**笔迹轻重**表示置信程度
 * （高 / 中 / 低三档）。⇒ 「三档在真实像素下分得清、低档没废」，
 * 这件事必须变成**机器判据**，不能靠「看着还行」。
 *
 * 三条判据：
 *
 * 1. **三档的墨迹厚度严格递减**（真实像素，量的是**行数**）。
 *    ⚠ 为什么量「行数」而不是宽度：笔划近水平 ⇒ 它的厚度就是**占了几行**。
 *    ⚠ 判据是**实测**的，不是估的——上一轮第一版低档只有 0.6px，
 *    淡到等于没有 ⇒ **那一档是失效的**，而肉眼看整页图根本看不出来。
 * 2. **低档不能低于 1px**（真实像素）。
 *    ⚠ 这条是第 1 条的补充：「递减」允许低档是 0。
 * 3. **那一格里是笔迹，不是百分数**；且**三条纸条的正文宽度一致**
 *    （槽宽恒定 ⇒ 折行一致 ⇒ 纸条高度一致）。
 *
 * ## 跑法
 *
 * ```bash
 * cd ui && node scripts/probe-confidence.mjs
 * ```
 *
 * ⚠ 需要 dev server 在 `http://localhost:1420`（`pnpm dev`）。
 * ⚠ 退出码非 0 = 有判据没过。
 */
import { chromium } from 'playwright-core';
import { CONF_META } from '../src/lib/marks.js';

const BASE = 'http://localhost:1420';

/** 槽的取景框 → 实际像素的缩放。26px 宽 / 56 单位。 */
const SCALE = 26 / 56;

const lines = [];
let failed = 0;
function check(ok, name, detail) {
  lines.push(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? '  — ' + detail : ''}`);
  if (!ok) failed += 1;
}

// ── 判据 0：几何（离线，从 marks.js 直接算）─────────────────────────────
// ⚠ 这一段**不依赖浏览器** ⇒ 它是最稳的一层：笔形数据本身对不对。
// ⚠ **量的是生成器给的笔宽定义值**（`CONF_META`），**不是**从 path 的 bbox
//   反推——笔划近水平，bbox 高度被**中心线的起伏**主导
//   （起伏 4.2 单位 ≫ 笔宽 3.6）⇒ 反推出来的是「起伏 + 笔宽」，
//   三档只差 20% ⇒ 那样的判据几乎恒真，等于没有守卫。
const geo = {};
for (const [tier, meta] of Object.entries(CONF_META)) {
  geo[tier] = meta.stroke * SCALE; // 实际像素
}
check(
  geo.high > geo.mid && geo.mid > geo.low,
  '三档笔重严格递减（几何）',
  `高 ${geo.high.toFixed(2)}px / 中 ${geo.mid.toFixed(2)}px / 低 ${geo.low.toFixed(2)}px`,
);
check(geo.low >= 0.9, '低档不是废档（几何 ≥0.9px）', `低 ${geo.low.toFixed(2)}px`);
check(
  geo.high / geo.low >= 1.6,
  '高低两档拉得开（几何比值 ≥1.6）',
  `比值 ${(geo.high / geo.low).toFixed(2)}`,
);

// ── 判据 1~3：真实渲染 ────────────────────────────────────────────────
const browser = await chromium.launch({ channel: 'chrome' });
const page = await browser.newPage({ viewport: { width: 380, height: 560 }, deviceScaleFactor: 2 });
await page.goto(`${BASE}/panel/?demo=1&state=drafted`, { waitUntil: 'networkidle' });
await page.locator('.cs-sheet').first().waitFor({ timeout: 15000 });
await page.waitForTimeout(900);

// 判据 3a：那一格是笔迹，不是百分数
const slotInfo = await page.evaluate(() => {
  const slots = [...document.querySelectorAll('.cs-act-prob')];
  return slots.map((el) => ({
    svg: el.querySelectorAll('svg').length,
    paths: el.querySelectorAll('svg path').length,
    text: (el.textContent ?? '').trim(),
    w: +el.getBoundingClientRect().width.toFixed(1),
    h: +el.getBoundingClientRect().height.toFixed(1),
  }));
});
check(slotInfo.length >= 2, '面板上有置信度槽', `${slotInfo.length} 个`);
check(
  slotInfo.every((s) => s.svg === 1 && s.paths >= 3),
  '每一格画的是笔迹（三道叠画）',
  slotInfo.map((s) => `svg=${s.svg} path=${s.paths}`).join(' · '),
);
check(
  slotInfo.every((s) => s.text === ''),
  '那一格不再有百分数文本',
  slotInfo.map((s) => `「${s.text}」`).join(' · '),
);
check(
  slotInfo.every((s) => s.w === slotInfo[0].w),
  '槽宽恒定（各条正文宽度一致）',
  slotInfo.map((s) => `${s.w}px`).join(' · '),
);

// 判据 3b：三条候选的正文宽度一致 ⇒ 折行一致 ⇒ 纸条高度一致
const textWidths = await page.evaluate(() =>
  [...document.querySelectorAll('.cs-act-text')].map(
    (el) => +el.getBoundingClientRect().width.toFixed(1),
  ),
);
check(new Set(textWidths).size === 1, '三条候选正文宽度一致', textWidths.join(' / '));

// 判据 1、2：真实像素下的墨迹厚度
// ⚠ 量法：把槽截成 PNG，**交给浏览器自己解码**（本机没有 PIL），
//   再逐行数「这一行有几个暗像素」⇒ 笔划近水平 ⇒ 厚度 = 墨迹行数。
// ⚠ 为什么不在 Node 里解码 PNG：那要引入一个依赖；浏览器本来就有解码器。
const slots = page.locator('.cs-act-prob');
const slotCount = await slots.count();
const measured = [];
for (let i = 0; i < slotCount; i++) {
  const b64 = (await slots.nth(i).screenshot()).toString('base64');
  const r = await page.evaluate(async (data) => {
    const img = new Image();
    img.src = 'data:image/png;base64,' + data;
    await img.decode();
    const c = document.createElement('canvas');
    c.width = img.width;
    c.height = img.height;
    const ctx = c.getContext('2d');
    ctx.drawImage(img, 0, 0);
    const d = ctx.getImageData(0, 0, c.width, c.height).data;
    const lum = (i) => 0.2126 * d[i] + 0.7152 * d[i + 1] + 0.0722 * d[i + 2];
    // 逐行：暗像素数 ≥ 3 的行算「有墨迹」。
    const rows = [];
    for (let y = 0; y < c.height; y++) {
      let n = 0;
      for (let x = 0; x < c.width; x++) {
        const p = (y * c.width + x) * 4;
        if (lum(p) < 200) n++;
      }
      rows.push(n);
    }
    // 最长连续段 = 笔迹厚度（行）
    let best = 0;
    let cur = 0;
    for (const n of rows) {
      cur = n >= 3 ? cur + 1 : 0;
      best = Math.max(best, cur);
    }
    // 顺手报一下亮度分布，阈值要按实测调，不能拍脑袋。
    const hist = new Array(10).fill(0);
    for (let p = 0; p < d.length; p += 4) hist[Math.min(9, Math.floor(lum(p) / 26))]++;
    return { thick: best, w: c.width, h: c.height, hist, rows };
  }, b64);
  measured.push(r);
  console.log(
    `  槽 ${i}：${r.w}×${r.h} 设备像素 ⇒ 墨迹 ${r.thick} 行（≈ ${(r.thick / 2).toFixed(1)} CSS px）`,
  );
}
check(
  measured.length >= 2,
  '能同时量到两档（面板上首选不显示置信度 ⇒ 最多两档）',
  `${measured.length} 档`,
);
if (measured.length >= 2) {
  const [a, b] = measured;
  check(a.thick > b.thick, '两档在真实像素下分得开（高档更粗）', `${a.thick} 行 vs ${b.thick} 行`);
  check(b.thick >= 1, '低档不是废档（真实像素下至少一行墨迹）', `${b.thick} 行`);
}

await browser.close();
console.log(lines.join('\n'));
console.log(
  failed === 0 ? `\nALL GREEN（${lines.length} 条判据）` : `\nFAILED ${failed}/${lines.length}`,
);
process.exit(failed === 0 ? 0 : 1);
