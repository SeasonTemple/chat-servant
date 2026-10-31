/**
 * 量**面板头部**在任意态里的位置、高度、计算字号、过渡与滚动指示线，
 * 可选把 `.cs-sheet` 截到指定目录（@2x）。
 *
 * ## 为什么量它（2026-10-24，用户报「标题部分样式在动」）
 *
 * 用户原话：「开始生成候选后为什么标题部分样式也在动？图1对比图2」
 * ⇒ 要回答的不是「哪里动了」，而是**哪条 flex 规则让整列高度一变就重排**。
 *
 * ## 为什么扩到四态 + 计算字号（2026-10-31，用户报「名称和最新回复没有字号差异」）
 *
 * 原来只量 noprep / drafted / sent 三态。两个缺口：
 * ① `sent` 在 `ui/static/demo/` 下**没有夹具**（只有 noprep / drafting / drafted / failed），
 *    `?state=sent` 回落成 drafted ⇒ 那一栏恒等于 drafted，看着「三态一致」其实是两态。
 * ② 只看矩形量不出「12px 与 15px 看起来一样高」这类问题 ⇒ 计算字号、字族、
 *    `clientHeight`/`scrollHeight`（判有没有被裁）、`transition` 一起打出来。
 *
 * ## 为什么还要量滚动（2026-11-01，用户报「长引文挤掉候选 / 滚动条是深灰的 / 滚动没反馈」）
 *
 * 三条判据都是**行为**，矩形量不到：原生滚动条在不在（`offsetWidth - clientWidth`）、
 * 渐隐深度（`--fade-top` / `--fade-bottom`）、指示线的位置高度与 800ms 后的透明度。
 * ⇒ 传 `--scroll=<0..1>` 会先把两个滚动容器滚到那个比例，**立刻**量一次（=滚动中），
 *    再等 900ms 量一次（=停手后），两次数值一起打出来。
 *
 * 用法：
 *   cd ui && node scripts/probe-header.mjs
 *   cd ui && node scripts/probe-header.mjs --states=many,longdraft --scroll=0.5
 *   cd ui && node scripts/probe-header.mjs --states=longdraft --scroll=0 --out=../target/probe/p2 --tag=longdraft-top
 */
import { chromium } from 'playwright-core';

const args = process.argv.slice(2);
const opt = (name, dflt) => {
  const hit = args.find((a) => a.startsWith(`--${name}=`));
  return hit === undefined ? dflt : hit.slice(name.length + 3);
};

const BASE = 'http://localhost:1420';
const STATES = opt('states', 'noprep,drafting,drafted,failed').split(',');
const SCROLL = opt('scroll', '');
const OUT = opt('out', '');
const TAG = opt('tag', '');
const BRIEF = opt('brief', '') !== '';

const browser = await chromium.launch({ channel: 'chrome' });
const page = await browser.newPage({ viewport: { width: 460, height: 760 }, deviceScaleFactor: 2 });
const INJECT = opt('inject', '');

const measure = () =>
  page.evaluate(() => {
    const q = (s) => document.querySelector(s);
    const rect = (el) => {
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { top: +r.top.toFixed(1), h: +r.height.toFixed(1), bottom: +r.bottom.toFixed(1) };
    };
    const face = (el) => {
      if (!el) return null;
      const cs = getComputedStyle(el);
      return {
        fontSize: cs.fontSize,
        fontFamily: cs.fontFamily.split(',')[0],
        color: cs.color,
        fontWeight: cs.fontWeight,
        lineHeight: cs.lineHeight,
      };
    };
    const scrollState = (el) => {
      if (!el) return null;
      const cs = getComputedStyle(el);
      return {
        clientH: el.clientHeight,
        scrollH: el.scrollHeight,
        scrollTop: +el.scrollTop.toFixed(1),
        clientW: el.clientWidth,
        offsetW: el.offsetWidth,
        barW: el.offsetWidth - el.clientWidth,
        gutter: cs.scrollbarGutter,
        scrollbarWidth: cs.scrollbarWidth,
        fadeTop: cs.getPropertyValue('--fade-top').trim(),
        fadeBottom: cs.getPropertyValue('--fade-bottom').trim(),
        mask: cs.maskImage !== 'none',
        moreBelow: el.getAttribute('data-more'),
        moreAbove: el.getAttribute('data-more-above'),
      };
    };
    const mark = (el) => {
      if (!el) return null;
      const cs = getComputedStyle(el);
      const r = el.getBoundingClientRect();
      return {
        scrolling: el.getAttribute('data-scrolling'),
        opacity: cs.opacity,
        width: cs.width,
        bg: cs.backgroundColor,
        radius: cs.borderRadius,
        transition: cs.transitionProperty + ' ' + cs.transitionDuration,
        h: +r.height.toFixed(1),
        top: +r.top.toFixed(1),
      };
    };
    const body = q('.cs-body-col');
    const draft = q('.cs-draft');
    const draftText = q('.cs-draft-text');
    const quote = q('.cs-where-quote');
    const quoteCs = quote ? getComputedStyle(quote) : null;
    const marks = [...document.querySelectorAll('.cs-scroll-mark')];
    return {
      where: rect(q('.cs-where')),
      whereText: (q('.cs-where')?.textContent ?? '').replace(/\s+/g, ' ').trim().slice(0, 30),
      quote: rect(quote),
      quoteFontSize: quoteCs?.fontSize ?? null,
      quoteLineHeight: quoteCs?.lineHeight ?? null,
      quoteClientH: quote?.clientHeight ?? null,
      quoteScrollH: quote?.scrollHeight ?? null,
      quoteLines:
        quote && quoteCs ? +(quote.clientHeight / parseFloat(quoteCs.lineHeight)).toFixed(2) : null,
      quoteTitleLen: quote?.getAttribute('title')?.length ?? null,
      quoteTitleTail: (quote?.getAttribute('title') ?? '').slice(-20),
      draft: rect(draft),
      draftScroll: scrollState(draft),
      draftTextScrollH: draftText?.scrollHeight ?? null,
      draftTextContent: (draftText?.textContent ?? '').replace(/\s+/g, ' ').trim().slice(0, 40),
      actions: rect(q('.cs-actions')),
      actionsScroll: scrollState(q('.cs-actions')),
      pick: rect(q('.cs-pick')),
      body: rect(body),
      bodyKids: body ? [...body.children].map((c) => c.className.split(' ')[0]) : [],
      sheet: rect(q('.cs-sheet')),
      facePeer: face(q('.cs-peer') ?? q('.cs-peer-unknown')),
      peerPxAttr: q('.cs-sheet')?.getAttribute('data-peer-px') ?? null,
      faceQuote: face(quote),
      faceDraftText: face(draftText),
      transitionDraft: draft ? getComputedStyle(draft).transitionProperty : null,
      transitionDraftText: draftText ? getComputedStyle(draftText).transitionProperty : null,
      transitionDraftTextDur: draftText ? getComputedStyle(draftText).transitionDuration : null,
      draftTextAnim: draftText
        ? getComputedStyle(draftText).animationName +
          ' ' +
          getComputedStyle(draftText).animationDuration
        : null,
      actTextW: q('.cs-act-text')?.getBoundingClientRect().width ?? null,
      actW: q('.cs-act')?.getBoundingClientRect().width ?? null,
      markDraft: mark(marks[0]),
      markActs: mark(marks[1]),
      marks: marks.length,
    };
  });

const pngSize = async (file) => {
  const { readFileSync } = await import('node:fs');
  const b = readFileSync(file);
  return { w: b.readUInt32BE(16), h: b.readUInt32BE(20) };
};

for (const state of STATES) {
  await page.goto(`${BASE}/panel/?demo=1&state=${state}`, { waitUntil: 'networkidle' });
  await page.locator('.cs-sheet').first().waitFor({ timeout: 15000 });
  await page.waitForTimeout(700);

  if (SCROLL !== '') {
    await page.evaluate((frac) => {
      for (const sel of ['.cs-actions', '.cs-draft']) {
        const el = document.querySelector(sel);
        if (!el) continue;
        el.scrollTop = (el.scrollHeight - el.clientHeight) * frac;
        el.dispatchEvent(new Event('scroll'));
      }
    }, Number(SCROLL));
    await page.waitForTimeout(120);
  }

  if (INJECT !== '') {
    await page.evaluate((n) => {
      const q = document.querySelector('.cs-where-quote');
      if (q) q.textContent = '合成的长句用来量行数。'.repeat(Math.ceil(n / 11)).slice(0, n);
    }, Number(INJECT));
    await page.waitForTimeout(80);
    const mi = await measure();
    console.log(
      'INJECTED ' +
        JSON.stringify({
          quote: mi.quote,
          quoteClientH: mi.quoteClientH,
          quoteScrollH: mi.quoteScrollH,
          quoteLines: mi.quoteLines,
          quoteLineHeight: mi.quoteLineHeight,
          whereH: mi.where.h,
          draftTop: mi.draft.top,
          quoteTitleLen: mi.quoteTitleLen,
        }),
    );
  }

  const m0 = await measure();
  console.log(`=== ${state}${SCROLL !== '' ? ` @scroll=${SCROLL}` : ''} ===`);
  console.log(BRIEF ? JSON.stringify(m0) : JSON.stringify(m0, null, 1));

  if (SCROLL !== '' && OUT !== '') {
    await page.waitForTimeout(900);
    console.log(`--- ${state} 停手 900ms 后 ---`);
    const after = await measure();
    console.log(
      BRIEF
        ? JSON.stringify({ markDraft: after.markDraft, markActs: after.markActs })
        : JSON.stringify({ markDraft: after.markDraft, markActs: after.markActs }, null, 1),
    );
  }

  if (OUT !== '') {
    const { mkdirSync } = await import('node:fs');
    mkdirSync(OUT, { recursive: true });
    // 截图前必须让指示线**重新亮起来**：上面那 900ms 是量「停手淡出」用的，
    // 它带着 screenshot 一起等 ⇒ 拍到的永远是已经归零的那一帧。
    // 2026-11-01 父代理据此判「线看不见」，有一半是这条时序造成的假象。
    if (SCROLL !== '') {
      await page.evaluate((frac) => {
        for (const sel of ['.cs-actions', '.cs-draft']) {
          const el = document.querySelector(sel);
          if (!el) continue;
          el.scrollTop = (el.scrollHeight - el.clientHeight) * frac;
          el.dispatchEvent(new Event('scroll'));
        }
      }, Number(SCROLL));
      await page.waitForTimeout(200);
    }
    const suffix = SCROLL !== '' ? `-scroll${SCROLL}` : '';
    const file = `${OUT}/panel-${TAG || state}-${state}${suffix}@2x.png`;
    await page.locator('.cs-sheet').first().screenshot({ path: file });
    console.log('SHOT ' + file + ' ' + JSON.stringify(await pngSize(file)));
  }
}
await browser.close();
