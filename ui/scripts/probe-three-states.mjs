/**
 * 量**三态**（+一态：改选另一条）的面板几何。
 *
 * ## 为什么需要它（2026-10-25，用户六条反馈里的第 ② 条）
 *
 * 用户原话：「对比图2可以发现 候选生成前的布局，候选出现的布局，
 * 选择候选后的布局**依旧不一致**」。
 *
 * ⚠ **「依旧」两个字很重** —— 上一轮（票 92 续 / 票 95）已经改过一次，
 * 当时的结论是「正文位高度 47.5 → 41.9px ⇒ 顶部不再随候选长高」。
 * ⇒ 那一轮量的只是**正文位那一行**，没量**整张纸的骨架**。
 * ⇒ 这一轮量**骨架**：列宽 / 归属行 / 正文位 / 候选组 / 档位行各态的 top 与 height。
 *
 * ## 真机取证（2026-10-25）——这一轮量具为什么必须量「列宽」
 *
 * 真机两张截图（`target/probe/live-check.png` 候选已出、
 * `live-after-click.png` 点过之后）逐像素对照的结果：
 *
 * | | A（点之前） | B（点之后） |
 * |---|---|---|
 * | 纸条外框 x | 74..300（宽 227） | 38..336（宽 299） |
 * | 候选正文墨迹 x | 95..246 | 59..210（左移 36） |
 * | 百分数墨迹 x | 254..278 | 290..315（右移 36） |
 * | 档位名 x | 106..230 | 88..291（外扩 79） |
 * | 纵向 | 11 条原有带 **y 逐行相同** | 同左 |
 *
 * ⇒ **纵向零位移，横向整体重排**，而且是**居中对称**地变宽 72px。
 * ⇒ 根因：`.cs-body-col` 是 `.cs-sheet`（flex 列）的 flex 项，
 *   而它写了 `margin: 0 auto` ⇒ **交叉轴上的 auto 边距会取消 `stretch`**
 *   ⇒ 它的宽度退化成**随内容变化**（`max-width: 316px` 只是上限）。
 *   候选一出现、点一下多一条回执 ⇒ 最宽的子元素变了 ⇒ **整列跟着变宽**
 *   ⇒ 归属行 / 正文位 / 候选组 / 档位行**全部横向重排**。
 * ⇒ 「整个候选组的阴影都变化」是**同一件事**：纸条变宽 ⇒ 投影带变长
 *   （逐像素实测：同像素亮度差 ≤ 0.9 lum ⇒ **影没变浓，只是变长**）。
 *
 * ⇒ 所以这一轮量具的第一判据是**列宽三态恒定**。
 *
 * ## 四态怎么造
 *
 * · 候选给出前 → `?demo=1&state=noprep`（`phase=read`，没有候选）
 * · 候选出现   → `?demo=1&state=drafted`（`phase=drafted`，三条候选）
 * · 选过之后   → `drafted` **点一下第 1 条候选**
 *   （演示态没有 Tauri ⇒ `invoke('dispatch')` 抛错 ⇒ 前端 catch ⇒
 *    `sentNote` 被填上 ⇒ **回执那条状态句真的渲染出来了**，
 *    与真机点候选后的形态同构）
 * · 改选另一条 → 再点**第 2 条** ⇒ 记号应当**跟着选中项走**（反馈 ⑤）
 *
 * ## 输出
 *
 * 一张表：每个态 × 每个部件的 top / height / bottom / left / width；
 * 外加候选组的 `scrollHeight / clientHeight` 与**记号挂在哪一条**。
 *
 * ```bash
 * cd ui && node scripts/probe-three-states.mjs
 * ```
 */
import { chromium } from 'playwright-core';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

/**
 * 截图落在**仓库根**的 `target/probe/`（gitignored）。
 * ⚠ 不用相对路径：脚本从 `ui/` 起跑时，相对路径会写进 `ui/target/probe/`
 * ——而 `.gitignore` 只忽略**根**的 `/target` 与 `/ui/src-tauri/target`
 * ⇒ 那份会变成**未跟踪文件**混进 `git status`。
 */
const HERE = path.dirname(fileURLToPath(import.meta.url));
const OUT = path.resolve(HERE, '..', '..', 'target', 'probe');

const BASE = 'http://localhost:1420';

/** 要量的部件。选择器 → 名字。 */
const PARTS = [
  ['.cs-body-col', '内容列'],
  ['.cs-where', '归属行'],
  ['.cs-draft', '正文位'],
  ['.cs-draft-text', '正文位那句话'],
  ['.cs-actions', '候选组容器'],
  ['.cs-act', '候选纸条'],
  ['.cs-act-meta', '右侧一列'],
  ['.cs-act-mark', '记号(勾)'],
  ['.cs-act-prob', '置信度(笔迹)'],
  ['.cs-act-note', '回执'],
  ['.cs-pick', '档位行'],
  ['.cs-sheet', '整张纸'],
];

const browser = await chromium.launch({ channel: 'chrome' });
const page = await browser.newPage({ viewport: { width: 380, height: 560 } });

/** 量一个态的几何。 */
async function measure(label) {
  return page.evaluate(
    ([parts, label]) => {
      const out = { 态: label, 部件: {} };
      for (const [sel, name] of parts) {
        const els = [...document.querySelectorAll(sel)];
        out.部件[name] = els.map((el) => {
          const b = el.getBoundingClientRect();
          return {
            top: +b.top.toFixed(1),
            h: +b.height.toFixed(1),
            bottom: +b.bottom.toFixed(1),
            left: +b.left.toFixed(1),
            w: +b.width.toFixed(1),
            文字: el.textContent.trim().slice(0, 24),
          };
        });
      }
      const acts = document.querySelector('.cs-actions');
      if (acts) {
        const cs = getComputedStyle(acts);
        out.候选组 = {
          scrollH: acts.scrollHeight,
          clientH: acts.clientHeight,
          scrollW: acts.scrollWidth,
          clientW: acts.clientWidth,
          溢出: acts.scrollHeight - acts.clientHeight,
          more: acts.dataset.more,
          渐隐: cs.maskImage !== 'none' || cs.webkitMaskImage !== 'none',
        };
      }
      const sheet = document.querySelector('.cs-sheet');
      if (sheet) {
        out.纸 = { scrollH: sheet.scrollHeight, clientH: sheet.clientHeight };
      }
      // 正文位的**计算样式**：高度为什么三态不同，要看这里。
      const draft = document.querySelector('.cs-draft');
      const p = document.querySelector('.cs-draft-text');
      if (draft && p) {
        const ds = getComputedStyle(draft);
        const ps = getComputedStyle(p);
        out.正文位样式 = {
          容器padding: ds.padding,
          容器高: ds.height,
          容器min高: ds.minHeight,
          段落字号: ps.fontSize,
          段落行高: ps.lineHeight,
          段落高: ps.height,
          段落字: p.textContent.trim(),
        };
      }
      // 记号挂在**哪一条**上（反馈 ⑤：记号必须跟着选中项走）。
      // ⚠ 置信度那一格**不再有文本**（2026-10-25 改成笔迹）⇒ 读它的
      //   `textContent` 只会得到空串 ⇒ 改成读「有没有笔迹」与它的层数。
      const marks = [...document.querySelectorAll('.cs-act')].map((el, i) => ({
        第几条: i,
        有记号: !!el.querySelector('.cs-act-mark'),
        置信度笔迹: el.querySelector('.cs-act-prob')?.querySelectorAll('svg path').length ?? 0,
        lead: el.classList.contains('cs-act-is-lead'),
        文字: el.querySelector('.cs-act-text')?.textContent.trim().slice(0, 12) ?? '',
      }));
      out.每条 = marks;
      return out;
    },
    [PARTS, label],
  );
}

const rows = [];

// ── 态 1：候选给出前 ────────────────────────────────────────────────
await page.goto(`${BASE}/panel/?demo=1&state=noprep`, { waitUntil: 'networkidle' });
await page.locator('.cs-sheet').first().waitFor({ timeout: 15000 });
await page.waitForTimeout(700);
rows.push(await measure('候选给出前(noprep)'));
await page
  .locator('.cs-sheet')
  .first()
  .screenshot({ path: path.join(OUT, 'state-before.png') });

// ── 态 2：候选出现 ─────────────────────────────────────────────────
await page.goto(`${BASE}/panel/?demo=1&state=drafted`, { waitUntil: 'networkidle' });
await page.locator('.cs-sheet').first().waitFor({ timeout: 15000 });
await page.waitForTimeout(700);
rows.push(await measure('候选出现(drafted)'));
await page
  .locator('.cs-sheet')
  .first()
  .screenshot({ path: path.join(OUT, 'state-drafted.png') });

// ── 态 3：选过之后（点第 1 条候选 ⇒ 回执出现） ───────────────────────
await page.locator('.cs-act').first().click();
await page.waitForTimeout(900);
rows.push(await measure('选过之后(点了第1条)'));
await page
  .locator('.cs-sheet')
  .first()
  .screenshot({ path: path.join(OUT, 'state-after-click.png') });

// ── 态 4：改选第 2 条（记号必须跟过去，反馈 ⑤） ───────────────────────
await page.locator('.cs-act').nth(1).click();
await page.waitForTimeout(900);
rows.push(await measure('改选第2条'));
await page
  .locator('.cs-sheet')
  .first()
  .screenshot({ path: path.join(OUT, 'state-after-click2.png') });

// ── 打印表 ─────────────────────────────────────────────────────────
console.log('=== 三态几何 ===');
for (const r of rows) {
  console.log(`\n【${r.态}】`);
  for (const [name, list] of Object.entries(r.部件)) {
    if (!list.length) {
      console.log(`  ${name.padEnd(12)} （不存在）`);
      continue;
    }
    list.forEach((b, i) => {
      console.log(
        `  ${name.padEnd(12)} #${i}  top=${String(b.top).padStart(7)}  h=${String(b.h).padStart(6)}  bottom=${String(b.bottom).padStart(7)}  left=${String(b.left).padStart(6)}  w=${String(b.w).padStart(6)}  ${b.文字}`,
      );
    });
  }
  if (r.候选组) {
    const c = r.候选组;
    console.log(
      `  候选组容器   scrollH=${c.scrollH} clientH=${c.clientH} 溢出=${c.溢出} more=${c.more} 渐隐=${c.渐隐}`,
    );
  }
  if (r.纸) console.log(`  整张纸       scrollH=${r.纸.scrollH} clientH=${r.纸.clientH}`);
  if (r.正文位样式) {
    const s = r.正文位样式;
    console.log(
      `  正文位样式   段落字号=${s.段落字号} 行高=${s.段落行高} 段落高=${s.段落高} 容器padding=${s.容器padding} 字=「${s.段落字}」`,
    );
  }
  console.log('  每条候选：');
  for (const m of r.每条) {
    console.log(
      `    #${m.第几条}  记号=${m.有记号 ? '有' : '无'}  置信度笔迹=${m.置信度笔迹 ? m.置信度笔迹 + '层' : '无'}  墨色=${m.lead ? '是' : '否'}  「${m.文字}」`,
    );
  }
}

// ── 判据：三态的骨架必须一致 ────────────────────────────────────────
const key = (r, name, i = 0) => {
  const b = r.部件[name]?.[i];
  return b ? `${b.left}/${b.w}` : '—';
};
console.log('\n=== 判据：列宽与骨架三态一致 ===');
const base = rows[0];
for (const r of rows) {
  const same = key(r, '内容列') === key(base, '内容列');
  console.log(
    `  ${r.态.padEnd(22)} 列=${key(r, '内容列')}  归属行=${key(r, '归属行')}  正文位=${key(r, '正文位')}  候选组=${key(r, '候选组容器')}  档位行=${key(r, '档位行')}  ${same ? '✔ 与首态同宽' : '✘ 列宽变了'}`,
  );
}

await browser.close();
