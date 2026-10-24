/**
 * 量**候选的两种材质**，并把「悬停时纸片变糊」这件事**量化**。
 *
 * ## 为什么需要它（2026-10-24）
 *
 * 用户报：「为什么**有一个**候选不是纸片样式？」——**他是对的**。
 * 面板上候选分**两个元素**渲染，材质故意不同：
 *
 * | 谁 | 元素 | 材质 |
 * |---|---|---|
 * | **第一条候选（推荐）** | `.cs-act`（动作区第 1 条） | **撕边纸片** + 一枚手绘勾（`.cs-act-mark`） |
 * | **第 2 条起（备选）** | `.cs-act`（同一个动作区） | **撕边纸片**：`border-image: url('/strip.png')` + 两层影 |
 *
 * ⇒ **2026-10-24 票 92 续**：正文位**不再住候选**。原来住在那儿的
 * `.cs-draft-lead` 是 21px 仿宋 + 撕边 + 右上角一枚勾 + **没有概率列**
 * ⇒ 面板上两种形制 ⇒ 用户原话：「为什么和下面的两个候选样式不同？
 * 百分比置信度没有、勾选的位置奇怪，长度不一致」。
 * ⇒ 现在**全部候选都是 `.cs-act`** ⇒ 形制统一，只靠那枚勾分出推荐那条。
 *
 * ⚠ **量具量的是谁**（本脚本第一版栽在这里）：
 * 上一轮我按 `.cs-act` 取样 ⇒ **只量到备选** ⇒ 得出「两条都是纸片」。
 * ⇒ 本脚本两个都取，且**分别**报。
 *
 * ## 第二个问题：为什么悬停那一行看起来「不是纸」
 *
 * `.cs-act:hover` 给的是 `transform: translateY(-2px) rotate(-0.5deg)`。
 * Chromium 对**带 transform 的元素**会提成独立合成层，
 * 文字与撕边纹理被**重采样** ⇒ 那��行比别的行**糊**。
 * ⇒ 本脚本量**同一条行**在三种悬停写法下的锐度：
 *   · `rotate`（生产现状）· 只上抬（`translateY`）· 反向补偿（文字单独转回来）
 *
 * ## 跑法
 *
 * ```bash
 * cd ui && node scripts/probe-act-states.mjs [输出目录]
 * ```
 */
import { chromium } from 'playwright-core';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';

const OUT = resolve(process.argv[2] ?? 'target/probe/panel-type');
const BASE = 'http://localhost:1420';
await mkdir(OUT, { recursive: true });

const browser = await chromium.launch({ channel: 'chrome' });
const page = await browser.newPage({
  viewport: { width: 460, height: 760 },
  deviceScaleFactor: 2,
});

/** 量一张图的「糊不糊」：边缘强度 + 拉普拉斯方差。 */
const measure = async (file) => {
  const b64 = (await readFile(file)).toString('base64');
  return page.evaluate(async (dataUrl) => {
    const img = new Image();
    img.src = dataUrl;
    await img.decode();
    const c = document.createElement('canvas');
    c.width = img.width;
    c.height = img.height;
    const ctx = c.getContext('2d', { willReadFrequently: true });
    ctx.drawImage(img, 0, 0);
    const d = ctx.getImageData(0, 0, c.width, c.height).data;
    const gray = new Float32Array(c.width * c.height);
    for (let i = 0, j = 0; i < d.length; i += 4, j++) {
      gray[j] = 0.299 * d[i] + 0.587 * d[i + 1] + 0.114 * d[i + 2];
    }
    const grads = [];
    for (let y = 0; y < c.height; y++) {
      for (let x = 0; x < c.width - 1; x++) {
        grads.push(Math.abs(gray[y * c.width + x + 1] - gray[y * c.width + x]));
      }
    }
    grads.sort((a, b) => b - a);
    const top = grads.slice(0, Math.max(1, Math.floor(grads.length * 0.01)));
    const edge = top.reduce((a, b) => a + b, 0) / top.length;
    let sum = 0;
    let sum2 = 0;
    let n = 0;
    for (let y = 1; y < c.height - 1; y++) {
      for (let x = 1; x < c.width - 1; x++) {
        const l =
          4 * gray[y * c.width + x] -
          gray[(y - 1) * c.width + x] -
          gray[(y + 1) * c.width + x] -
          gray[y * c.width + x - 1] -
          gray[y * c.width + x + 1];
        sum += l;
        sum2 += l * l;
        n++;
      }
    }
    const mean = sum / n;
    return {
      w: c.width,
      h: c.height,
      edge: Number(edge.toFixed(2)),
      lapVar: Number((sum2 / n - mean * mean).toFixed(1)),
    };
  }, `data:image/png;base64,${b64}`);
};

/** 取一个元素的计算样式（判据：不是「看图」，是「量它」）。 */
const styleOf = (sel, idx = 0) =>
  page.evaluate(
    ([s, i]) => {
      const el = document.querySelectorAll(s)[i];
      if (!el) return null;
      const cs = getComputedStyle(el);
      return {
        cls: el.className.replace(/\bs-[a-z]+(-[a-z]+)?/g, '·').trim(),
        transform: cs.transform,
        filter: cs.filter,
        borderImage: cs.borderImageSource,
        borderWidth: cs.borderWidth,
        background: cs.backgroundColor,
        fontSize: cs.fontSize,
        fontFamily: cs.fontFamily.split(',')[0],
        textColor: getComputedStyle(el.querySelector('.cs-act-text') ?? el).color,
      };
    },
    [sel, idx],
  );

const report = {};

for (const tier of ['noprep', 'drafted', 'auto']) {
  await page.goto(`${BASE}/panel/?demo=1&state=${tier}`, { waitUntil: 'networkidle' });
  await page.locator('.cs-sheet').first().waitFor({ timeout: 15000 });
  await page.waitForTimeout(900);

  const shots = {};
  const shot = async (sel, idx, name) => {
    const el = page.locator(sel).nth(idx);
    if ((await page.locator(sel).count()) <= idx) return;
    const p = join(OUT, name);
    await el.screenshot({ path: p });
    shots[name] = p;
  };

  await page
    .locator('.cs-sheet')
    .first()
    .screenshot({ path: join(OUT, `sheet-${tier}.png`) });

  // ── 形制：推荐那条（动作区第 1 条） vs 备选（第 2 条）────────────────
  // ⚠ 2026-10-24 票 92 续：正文位不再住候选 ⇒「推荐那条」就是 `.cs-act` 的
  //   **第 1 条** ⇒ 这里比的是**第 1 条 vs 第 2 条**，形制必须**逐项相同**
  //   （字号 / 字体 / 宽 / 材质），只允许**墨色与那枚勾**不同。
  await shot('.cs-act', 0, `lead-${tier}.png`);
  await shot('.cs-act', 0, `act-${tier}-0-rest.png`);
  await shot('.cs-act', 1, `act-${tier}-1-rest.png`);

  report[`${tier}/lead`] = await styleOf('.cs-act', 0);
  report[`${tier}/alt`] = await styleOf('.cs-act', 1);

  // ⚠ `noprep`（候选给出前）根本没有候选 ⇒ 下面几拍跳过。
  if ((await page.locator('.cs-act').count()) === 0) {
    report[`${tier}/measure`] = {};
    report[`${tier}/shots`] = shots;
    continue;
  }

  // ── 悬停写法三连：生产 / 只上抬 / 文字反向补偿 ───────────────────
  const row = page.locator('.cs-act').nth(0);
  await row.hover();
  await page.waitForTimeout(500);
  await shot('.cs-act', 0, `act-${tier}-0-hover-rotate.png`);

  await page.evaluate(() => {
    document.querySelectorAll('.cs-act').forEach((el) => {
      el.style.setProperty('transform', 'translateY(-2px)', 'important');
    });
  });
  await page.waitForTimeout(300);
  await shot('.cs-act', 0, `act-${tier}-0-hover-lift.png`);

  await page.evaluate(() => {
    document.querySelectorAll('.cs-act').forEach((el) => {
      el.style.removeProperty('transform');
    });
    const s = document.createElement('style');
    s.id = 'counter-rotate';
    s.textContent =
      '.cs-act:hover .cs-act-text, .cs-act:focus-visible .cs-act-text { transform: rotate(0.5deg); }';
    document.head.appendChild(s);
  });
  await page.mouse.move(0, 0);
  await page.waitForTimeout(200);
  await row.hover();
  await page.waitForTimeout(500);
  await shot('.cs-act', 0, `act-${tier}-0-hover-counter.png`);

  await page.evaluate(() => document.getElementById('counter-rotate')?.remove());
  await page.mouse.move(0, 0);
  await page.waitForTimeout(300);

  // ── 点过（`:disabled`）：影子被收掉 ⇒「一片没有落在纸上的纸」──────
  // ⚠ 真实触发是 `sending`（投递中）；这里直接给元素加 `disabled` 属性，
  //   **渲染走的是生产 CSS 的 `:disabled` 分支**，只是触发方式不同。
  await page.evaluate(() => {
    document.querySelectorAll('.cs-act').forEach((el) => el.setAttribute('disabled', ''));
  });
  await page.waitForTimeout(300);
  await shot('.cs-act', 0, `act-${tier}-0-disabled.png`);

  report[`${tier}/measure`] = {};
  for (const [k, p] of Object.entries(shots)) {
    // ⚠ 2026-10-24 票 92 续：推荐那条**也是纸条**了 ⇒ 它不再被排除。
    report[`${tier}/measure`][k] = await measure(p);
  }
  report[`${tier}/shots`] = shots;
}

console.log(JSON.stringify(report, null, 2));

// ── 拼对照板 ────────────────────────────────────────────────────────
// ⚠ 板子里的图是**相对路径**，所以 HTML 必须与 PNG 同目录。
const fig = (label, file, note) => `
  <figure>
    <img src="${file.split(/[\\/]/).pop()}" />
    <figcaption><b>${label}</b><span>${note}</span></figcaption>
  </figure>`;

// ⚠ 键名里有 `·`，手抄进模板字符串容易抄成别的码位 ⇒ **只用变量拿**。
const D = report['drafted/measure'];
const DS = report['drafted/shots'];
const AS = report['auto/shots'];
const row = (k) => (D[k] ? `边缘强度 ${D[k].edge} / 锐度方差 ${D[k].lapVar}` : '');

const board = `<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8" />
<style>
  body { margin:0; padding:26px 28px; background:#efe7d8; color:#2b2118;
         font-family:"Microsoft YaHei","PingFang SC",sans-serif; }
  h1 { font-size:22px; margin:0 0 4px; }
  .lead { font-size:13.5px; margin:0 0 18px; color:#6b5a48; line-height:1.75; max-width:1180px; }
  h2 { font-size:16px; margin:20px 0 10px; padding-left:9px; border-left:4px solid #b8543a; }
  .row { display:flex; gap:13px; align-items:flex-start; flex-wrap:wrap; }
  figure { margin:0; background:#f7f1e4; border:1px solid #d8cbb4; border-radius:6px;
           padding:9px; width:330px; }
  figure img { width:100%; display:block; border-radius:3px; background:#efe7d8; }
  figcaption { margin-top:7px; font-size:12.5px; line-height:1.6; }
  figcaption b { font-weight:700; }
  figcaption > b:first-of-type { display:block; font-size:13.5px; margin-bottom:2px; }
  figcaption span { color:#6b5a48; }
  code { background:#e6dcc8; padding:0 4px; border-radius:3px; font-size:12px; }
  .big { width:430px; }
</style></head><body>
<h1>候选**全是纸条** —— 推荐那条只多一枚勾</h1>
<p class="lead">
<b>用户看得没错。</b>面板上的候选不是一个列表渲染的：原来**第一条候选占住正文位**
（<code>.cs-draft-lead</code>，21px 仿宋、<b>无概率列</b>、勾浮在右上角、横向还更窄），
下面第 2 条起才是动作区里的 <code>.cs-act</code> 撕边纸片（14px，带
<code>border-image: url('/strip.png')</code> + 两层影）。
⇒ 用户原话：「为什么和下面的两个候选样式不同？百分比置信度没有、勾选的位置奇怪，长度不一致」。
<br /><b>2026-10-24 票 92 续：正文位不再渲染候选</b> ⇒ 动作区列**全部**候选
⇒ 每一条都是同一种纸条（14px 雅黑、同宽、概率列在右），推荐那条只多**一枚手绘勾**。
</p>

<h2>① 同一张纸的三种内容：候选给出前 / 候选出来时 / Auto 档</h2>
<div class="row">
${fig('候选给出前（noprep）', join(OUT, 'sheet-noprep.png'), '正文位是一句<b>占位灰字</b>，下面<b>没有纸片组</b>')}
${fig('候选出来时（drafted）', join(OUT, 'sheet-drafted.png'), '正文位是<b>一句状态句</b>，下面<b>三张纸条</b>')}
${fig('Auto 档（auto）', join(OUT, 'sheet-auto.png'), '结构相同 + 红字警告 + 红铅笔勾')}
</div>

<h2>② 推荐那条 vs 备选（同一时刻、同一档）</h2>
<div class="row">
${fig('推荐那条（动作区第 1 条）', DS['act-drafted-0-rest.png'], '<code>.cs-act</code> + <code>cs-act-lead/cs-act-is-lead</code> ⇒ 与其它行<b>同一种纸条</b>，只多一枚勾')}
${fig('备选（动作区第 2 条）', DS['act-drafted-1-rest.png'], '<code>.cs-act</code>：撕边位图 + 影 <code>0 3px 6px/.2</code> ⇒ 纸片')}
</div>
</div>

<h2>③ 悬停的那一条为什么会「糊」——三种写法实测</h2>
<div class="row">
${fig('生产现状：+2px / 转 0.5°', DS['act-drafted-0-hover-rotate.png'], `影加到 <code>0 8px 16px/.26</code>；旋转 ⇒ 合成层重采样<br>${row('act-drafted-0-hover-rotate.png')}`)}
${fig('只上抬，不转', DS['act-drafted-0-hover-lift.png'], `整数位移 ⇒ 像素对齐，不重采样<br>${row('act-drafted-0-hover-lift.png')}`)}
${fig('转，但文字反向转回来', DS['act-drafted-0-hover-counter.png'], `纸是斜的，字是正的<br>${row('act-drafted-0-hover-counter.png')}`)}
${fig('Auto 档·生产现状', AS['act-auto-0-hover-rotate.png'], `Auto 档同一条现象<br>边缘强度 ${report['auto/measure']['act-auto-0-hover-rotate.png'].edge} / 锐度方差 ${report['auto/measure']['act-auto-0-hover-rotate.png'].lapVar}`)}
</div>

<h2>④ 唯一真的「不像纸片」的那种状态</h2>
<div class="row">
${fig('点过 / 投递中（:disabled）', DS['act-drafted-0-disabled.png'], `影被收掉 ⇒「一片没有落在纸上的纸」<br>${row('act-drafted-0-disabled.png')}`)}
</div>
</body></html>`;

const htmlPath = join(OUT, 'act-states-board.html');
await writeFile(htmlPath, board, 'utf8');
const boardPage = await browser.newPage({
  viewport: { width: 1200, height: 900 },
  deviceScaleFactor: 2,
});
await boardPage.goto(`file:///${htmlPath.replace(/\\/g, '/')}`, { waitUntil: 'networkidle' });
await boardPage.waitForTimeout(700);
await boardPage.screenshot({ path: join(OUT, 'act-states-compare.png'), fullPage: true });
await boardPage.close();

await browser.close();
console.log('done');
