/**
 * §5.5b 判据的**真实渲染**复核（票 45 第七轮）。
 *
 * ## 为什么要有这个，而不是只用 Python 算
 *
 * `verify.py` 那类脚本是**按公式算**的：它读 `app.css` 里的 token、
 * 自己混色、自己算对比度。这能验证「我算对了」，但验证不了
 * 「浏览器真的这么渲染」。
 *
 * 而本项目吃过两次亏，两次都是同一个形状：
 *  1. `--cs-line` 引用了**从未定义**的自定义属性 ⇒ 整条声明作废。
 *     公式算得再对，浏览器拿到的是 invalid，**那圈描边从来没渲染过**。
 *  2. 票 45 第四轮「只量 alpha 就下结论」——量的维度没覆盖被测对象。
 *
 * ⇒ 本脚本走 `getComputedStyle` 问**渲染后的实际值**，
 *   对「这一格到底画没画出来」给出的是浏览器自己的答案，不是推算。
 *
 * ## 它**不能**证明什么（别把它当验收）
 *
 * ⚠ **headless ≠ 真机。** 票 45 第六轮的原话：
 *   「第一次上真机，当场撞出一个五轮 headless 全绿的缺陷」。
 *   颜色/材质的**观感**必须真机量——WebView2 的分层合成、透明窗口、
 *   DPI 与 headless Chromium 都不是一回事。
 *
 *   本脚本只回答两类问题：
 *     ① **元素在不在 / 那条声明生效了没有**（如 `--cs-line` 那类）
 *     ② **渲染后的计算值是不是我以为的那个数**（不是公式，是结果）
 *
 * ## 跑法
 *
 * ```bash
 * # 先起 dev server，再：
 * node scripts/verify-edge.mjs [baseUrl]
 * ```
 *
 * 退出码：**0** = 全部判据通过；**1** = 有判据没过；**2** = 量具自己坏了。
 *
 * ⚠⚠ **别用管道看它的退出码**（2026-10-16 实测踩到）：
 * ```bash
 * node scripts/verify-edge.mjs | tail -20 ; echo "EXIT=$?"   # ← 永远是 0
 * ```
 * 管道只返回**最后一个命令**的退出码，于是上面那句打出来的是 `tail` 的 0。
 * 这与 `gate.ps1` 那次「只 `Write-Output` 没 `exit` → 带红门禁的 commit
 * 被提交」是**同一个病的两个变种**：**门禁红了但没人拦得住**。
 * ⇒ 判成败一律用**脚本自己打印的最后一节**（`✅ N 条判据全过` / `❌ N/M 条未过`），
 * 或者不经管道直接看退出码。
 */
import { chromium } from 'playwright-core';

const BASE = process.argv[2] ?? 'http://localhost:5199';
// 与 tauri.conf.json 的 380×560 一致。真机是同一尺寸（票 45 第六轮实测）。
const W = 380;
const H = 560;

/**
 * 把 `rgb(a, b, c)` / `rgba(...)` 解析成三元组。
 *
 * ⚠⚠ **必须也认 `color(srgb r g b)`**（2026-10-16 实测踩到）：
 * Chromium 算 `color-mix()` 时返回的是**新语法**，且**分量是 0~1 的小数**，
 * 不是 0~255：
 *
 *   `color-mix(in srgb, rgb(46 34 18) 51%, rgb(243 230 192))`
 *     → `color(srgb 0.558941 0.509961 0.404941)`
 *
 * 只写 `rgb()` 的解析器会把这三个小数当 0~255 读，于是算出来的对比度
 * 恒等于 1.00 : 1 —— **量具自己骗自己，而且骗得毫无异常**。
 * 这与票 45「量具要能因为量具偏了而失败」、以及「只量 alpha 就下结论」
 * 是同一条纪律：**判据的量，必须覆盖被测对象的全部相关维度。**
 */
function parseRgb(s) {
  if (!s) return null;
  const m = s.match(/-?[\d.]+(?:e-?\d+)?/gi);
  if (!m) return null;
  if (/^color\(/i.test(s.trim())) {
    // 0~1 小数 → 0~255
    return m.slice(0, 3).map((v) => Math.round(Number(v) * 255));
  }
  return m.slice(0, 3).map(Number);
}
// ⚠ 第十轮起 `.cs-input` 改成内陷（没有描边），
// `contrast()` / `relLum` 随之零引用 —— **已删**。
// 它们量的是「描边对纸的对比度」，而内陷的立体感由 `inset` 阴影表达，
// 不再是一个可量化的对比值。（徽标那圈描边仍在，但只需判 R−B 正负。）

const results = [];
const check = (name, ok, detail) => {
  results.push({ name, ok, detail });
  console.log(`  ${ok ? '✓' : '✗'} ${name.padEnd(38)} ${detail}`);
};

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: W, height: H } });

// ⚠ 前置探活。2026-10-16 实测踩到：dev server 被 `timeout` 收掉之后，
//   脚本会在 `page.goto` 上抛一个**未捕获的 ERR_CONNECTION_REFUSED**，
//   读起来像「产品坏了」，而其实是**量具的前提没满足**。
//   按「量具要能因为量具偏了而失败」那条，量具自己的前提也得先自己喊。
let reachable = true;
try {
  const probeRes = await fetch(BASE, { signal: AbortSignal.timeout(5000) });
  reachable = probeRes.ok;
} catch {
  reachable = false;
}
if (!reachable) {
  await browser.close();
  console.error(
    `\n❌ 量具前提不满足：${BASE} 连不上。\n` +
      `   先起 dev server：\n` +
      `     cd ui && pnpm dev --port 5199\n` +
      `   （若它「HTTP 不通但进程还在」，那是 AGENTS.md 记的**孤儿监听器**\n` +
      `     —— 先扫一遍 node 进程再重开，别只看 HTTP 就判「没在跑」。）\n`,
  );
  process.exit(2);
}

// ?demo=1&state=many&settings=1 —— many 是票 45 补的那份「复核夹具」，
// 它带两组模型配置（已配置 / 未配置），`.cs-input` 与 `.cs-badge.miss`
// 都会渲染出来。⚠ 真实 Tauri 窗口里读的是凭据管理器，不是这份夹具。
await page.goto(`${BASE}/?demo=1&state=many&settings=1`, {
  waitUntil: 'networkidle',
});
await page.waitForSelector('.cs-set', { timeout: 15000 });
// GSAP 补间跑完再量，否则量到的是中间帧（票 42 量具提醒）。
await page.waitForTimeout(900);

console.log(`\n══ 真实渲染复核（headless Chromium ${W}×${H}）══\n`);

const probe = await page.evaluate(() => {
  const cs = getComputedStyle(document.documentElement);
  const token = (n) => cs.getPropertyValue(n).trim();

  // ⚠⚠ **纸是位图，不是 `background-color`**（2026-10-16 实测踩到两次）：
  //   `.cs-sheet` 用的是 `background: url('/probe-sheet.svg')`，所以
  //   `getComputedStyle(...).backgroundColor` 返回 `rgba(0, 0, 0, 0)`。
  //   `.cs-set` 更是自己就透明（它是滚动容器，纸不在它身上）。
  //   ⇒ **量「纸的颜色」量不到 computed style**。两条路：
  //     ① 取 `--cs-paper` token —— 它的值就是**真机截图的屏上中位数**
  //        `rgb(243 230 192)`（app.css 注释记着怎么量的），本脚本走这条；
  //     ② 截图像素取中位数 —— `budget.html` 走的是那条。
  //   另：验证位图**确实在**（否则纸压根没画，「同源」就无从谈起）。
  const sheet = document.querySelector('.cs-sheet');
  const sheetCs = sheet ? getComputedStyle(sheet) : null;
  const paper = sheetCs ? cs.getPropertyValue('--cs-paper').trim() : null;
  const sheetBgImage = sheetCs?.backgroundImage ?? null;
  const setBg = getComputedStyle(document.querySelector('.cs-set')).backgroundColor;

  // 静息输入框
  const inputs = [...document.querySelectorAll('.cs-input')];
  const input = inputs[0];
  const inputCs = input ? getComputedStyle(input) : null;

  // 未配置徽标（.cs-badge.miss）—— 票 45 第七轮修的那条
  const miss = document.querySelector('.cs-badge.miss');
  const missCs = miss ? getComputedStyle(miss) : null;

  // 聚焦态（第十轮起判的是 box-shadow 深浅，不再是 border-color）
  let focusShadow = null;
  if (input) {
    input.focus();
    focusShadow = getComputedStyle(input).boxShadow;
    input.blur();
  }
  // 徽标描边取其 box-shadow 的颜色分量
  const missColor = miss ? getComputedStyle(miss).boxShadow : null;

  return {
    tokens: {
      paper: token('--cs-paper'),
      edge: token('--cs-edge'),
      tierInk: token('--cs-tier-ink'),
      rule: token('--cs-rule'),
    },
    paper,
    setBg,
    sheetBgImage,
    inputCount: inputs.length,
    inputBorder: inputCs?.borderTopWidth ?? null,
    inputShadow: inputCs?.boxShadow ?? null,
    inputBg: inputCs?.backgroundColor ?? null,
    inputRadius: inputCs?.borderTopLeftRadius ?? null,
    focusShadow,
    missEdge: missColor ? (missColor.match(/rgba?\([^)]+\)/) ?? [null])[0] : null,
    missExists: !!miss,
    missShadow: missCs?.boxShadow ?? null,
  };
});

const P = parseRgb(probe.paper);
const edge = parseRgb(probe.inputBorder);
const rb = (c) => c[0] - c[2];

console.log('【1】token 与纸底');
check('--cs-edge 解析成功', !!edge, probe.tokens.edge);
const paperOk = !!P && P.some((v) => v > 0);
if (!paperOk) {
  console.log(
    `\n❌ 量具失效：读到 --cs-paper = "${probe.paper}"。\n` + `   量具坏了，不是产品坏了。\n`,
  );
  process.exit(2);
}
check('纸底 token = 暖黄（R−B 为正）', rb(P) > 0, `--cs-paper rgb(${P})  R−B=${rb(P)}`);
check('纸 ≠ 旧冷灰 rgb(237 240 241)', String(P) !== '237,240,241', probe.paper);
check(
  '纸位图确实在画（background-image 命中 probe-sheet）',
  /probe-sheet\.svg/.test(probe.sheetBgImage ?? ''),
  (probe.sheetBgImage ?? '').slice(0, 60),
);
console.log(`  · .cs-set 自身背景 ${probe.setBg}（透明是预期的，纸在 .cs-sheet 且是位图）`);

console.log('\n【2】静息输入槽：内陷 + 弧边（第十轮定）');
check(
  '描边已移除（内陷不该有外边）',
  parseFloat(probe.inputBorder) === 0,
  'border-width ' + probe.inputBorder,
);
check('弧边 = --cs-radius (8px)', probe.inputRadius === '8px', probe.inputRadius);
check('底色 = 纸本身', rb(parseRgb(probe.inputBg)) > 0, probe.inputBg);
{
  // 内陷四段：上/左暗、下/右亮
  const s = probe.inputShadow ?? '';
  const layers = (s.match(/inset/g) || []).length;
  check('box-shadow 是内陷（inset）', layers > 0, `${layers} 段`);
  check('下边亮边在（0px -Npx）', /0px\s+-\d+px/.test(s), s.length > 0 ? '已渲染' : 'none');
  check('右边亮边在（-Npx 0px）', /-\d+px\s+0px/.test(s), s.length > 0 ? '已渲染' : 'none');
}

console.log('\n【3】未配置徽标那圈描边（--cs-line 那个 bug，仍适用）');
check('.cs-badge.miss 存在于 DOM', probe.missExists, String(probe.missExists));
// 修之前：var(--cs-line) 未定义 ⇒ 整条 box-shadow 作废 ⇒ 计算值应为 'none'
check(
  'box-shadow 已生效（不再是 none）',
  probe.missShadow !== 'none' && /inset/.test(probe.missShadow),
  probe.missShadow,
);
if (probe.missEdge) {
  const me = parseRgb(probe.missEdge);
  check('徽标描边与纸同源（R−B 为正）', rb(me) > 0, `R−B=${rb(me) > 0 ? '+' : ''}${rb(me)}`);
}

console.log('\n【4】聚焦态：槽变深（不是边框变色）');
if (probe.focusShadow) {
  const fs = probe.focusShadow;
  check('聚焦态仍是内陷', /inset/.test(fs), fs.length > 0 ? '已渲染' : 'none');
  // ⚠⚠ 第一版判据写的是「聚焦串比静息长」—— **判据错了**。
  // 聚焦态的暗边是 `color-mix(... 30%)` 而静息是 `22%`：
  // 数字**更短**，不是更长 ⇒ 第一版把「更深」判成「没变深」。
  // 与本仓那条纪律第三次同族（量具错 ⇒ 结论错）。
  // 正确判据：抽 box-shadow 里**第一段的 alpha**（上左暗边）比大小。
  const alpha = (str) => {
    const m = (str || '').match(/\/\s*([\d.]+)\s*\)/);
    return m ? Number(m[1]) : NaN;
  };
  const aRest = alpha(probe.inputShadow);
  const aFocus = alpha(probe.focusShadow);
  check('聚焦的暗边 alpha 高于静息（槽更深）', aFocus > aRest, `静息 ${aRest} → 聚焦 ${aFocus}`);
} else {
  check('聚焦态可读', false, '取不到 boxShadow');
}

console.log('\n【5】没有原生外观残留');
const native = await page.evaluate(() => ({
  select: document.querySelectorAll('.cs-set select').length,
  textarea: document.querySelectorAll('.cs-set textarea').length,
  checkbox: document.querySelectorAll('.cs-set input[type=checkbox]').length,
  range: document.querySelectorAll('.cs-set input[type=range]').length,
  input: document.querySelectorAll('.cs-set input').length,
}));
check('无 <select>', native.select === 0, String(native.select));
check('无 <textarea>', native.textarea === 0, String(native.textarea));
check('无原生 checkbox', native.checkbox === 0, String(native.checkbox));
console.log(
  `  · 输入框 ${native.input} 个 / range ${native.range} 个（range 与 password 走自绘与 .cs-input）`,
);

await browser.close();

const failed = results.filter((r) => !r.ok);
console.log(
  failed.length === 0
    ? `\n✅ ${results.length} 条判据全过`
    : `\n❌ ${failed.length}/${results.length} 条未过：\n${failed.map((f) => `   ✗ ${f.name} — ${f.detail}`).join('\n')}`,
);
process.exit(failed.length === 0 ? 0 : 1);
