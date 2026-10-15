<script lang="ts">
  /*
   *  纸面板。**从零建的，不是旧面板改的。**
   *
   *  旧面板（`(panel)/+page.svelte`，1553 行）已整体删除：它的旧正文排版、
   *  旧按钮、旧窗口控制都被否定了，在那个容器里做任何新设计判断都会被污染。
   *  它的三个组件（`PanelSettings` / `SegSlider` / `TierSwitch`）
   *  **没有跟着删**——它们不是旧排版的一部分，而是承载已交付功能的零件，
   *  见下面「旧组件的去留」那条。
   *  ⚠ 第四个 `WinCtl` 已于 2026-10-14 删除：它在 5031ab3 换手绘窗口键时
   *    就成了零引用的死代码，却还 import lucide 的 `X / Minus / Ellipsis`
   *    ——**旧面板的视觉语言借死代码继续住在仓库里**，截图搜不到、
   *    跑起来也不加载，但读代码的人会以为那是当前形态。
   *
   *  这里已经有五样东西：
   *    · 一张撕边的纸（真实素材，撕口即顶边）
   *    · 纸底的档位记号（**真值在 Rust**，见下面「档位为什么不能只存一份」）
   *    · 归属行（这句是替谁写的 + 对方最后一句原文）
   *    · 正文行（仿宋 21px / 行高 1.5，四态：已写 / 在写 / 失败 / 空）
   *    · 窗口控制 + 设置入口（右上角；缺了它们面板关不掉、设置进不去）
   *
   *  ⚠ **动作行故意还没有。** 它要碰发送路径（硬约束第 5 条），不能跟排版
   *  一起顺手做掉。宁可少一行，不要一个没验过的发送控件。→ 票 23
   *
   *  ── 这个组件为什么住在 `lib/` 而不是 `routes/` ────────────────────
   *
   *  `/` 与 `/panel` **两个路由渲染的是同一个面板**。所以组件只有一份，
   *  两个薄壳各 import 它。
   *
   *  起因：删掉旧面板后新面板从 `(panel)/panel/` 上移到 `(panel)/`，
   * 于是 `/panel` 这个路由会 404。而它是**外部引用过的**——设计稿、
   * 演示夹具、`?demo=1&state=` 全按 `/panel` 写。与其让旧链接集体失效，
   * 不如让两个路由指向同一份实现：既保住链接，又没有第二份面板。
   *
   *  ── 档位为什么不能只存一份 ──────────────────────────────────────
   *
   *  2026-10-13 实测：这一版原先把档位写成 `$state` + `?tier=` 探针口，
   *  **一个 `invoke` 都没有**——点档位只改内存里的副本，Rust 侧的档位
   *  从来没动过。注释写着「档位真值仍在 Rust 侧」，可代码里那份副本
   *  就是真值的唯一来源：改档之后发送走的还是 Rust 侧**旧**那一档，
   *  界面显示的却已经是新档。
   *
   *  这条破的是硬约束第 5 条（「档位、会话名这类值由 Rust 持有，
   *  界面只显示，不自己存第二份」）。`?tier=` 作为**演示探针口**保留，
   *  但只在脱离 Tauri 时起作用；真 Tauri 下真值一律走 `get_tier` / `set_tier`。
   */
  import {
    TICK_NESTED,
    TICK_NESTED_MID,
    TICK_NESTED_BIG,
    RING_LABEL_MARK,
    RING_LABEL_MARK_BIG,
    RING_LABEL_MARK_HUGE,
    MARK_VIEW,
    RING_VIEW,
    TICK_VIEW,
  } from '$lib/marks';
  import { markShape } from '$lib/mark-shape.svelte';
  import PanelSettings from '$lib/PanelSettings.svelte';
  import type { ReadoutPayload, Message, Candidate } from '$lib/types';
  import { IS_DEMO, HAS_TAURI } from '$lib/runtime';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';

  /**
   * 标记的**幅面档**。跟「勾还是圈」正交：勾也有大小，圈也有大小。
   * `?shape=tick|ring` 选笔，`?size=base|mid|big` 选幅面，都是探针不落盘。
   *
   * 默认 `big` 是量出来的，不是拍的（`target/probe/mark-size-compare.png`）：
   *   圈  base 52.10×16.40 → 扁椭圆上沿压在字头上，读成「框」；big 74.20×28.60
   *      才读成「盖了一个章」💭 判据是圈高/字高：<1.5 读成边框，>2.5 才是印章。
   *   勾  base 29.10×30.80 相对放大后的圈显得单薄，且**偏右 6px**——
   *      旧值 `margin-left:-21px` 是观感试出来的，笔心其实落在槽心右侧 6px。
   *      现全部改为按笔心反算，与圈同一套公式。
   * 要回退就把默认值改回 `base`。
   */
  type MarkSize = 'base' | 'mid' | 'big';
  /**
   *  ⚠ 默认 `big`。2026-10-13 有一轮把它改成 `base`，理由是「档位收进行内后
   *  取景框只剩 24×16，`big` 会被静默裁掉」——**那个框是同一轮自己造的**，
   *  于是变成「用我自己制造的限制去论证推翻定案」。已恢复 `big`。
   *
   *  判据：档位不再挤进别人的行，取景框按定案恢复 60×40（见 `.cs-markbox`），
   *  `big`（`TICK_VIEW` 56×34 / `RING_VIEW` 84×52）放得下且**笔尖不裁**。
   */
  let markSize = $state<MarkSize>('big');

  /** 档位从低到高。Rust 侧持有档位值，这里只按顺序摆位置。 */
  const TIERS = [
    { key: 'Manual', label: '只填入' },
    { key: 'Auto', label: '自动发送' },
  ] as const;

  type TierKey = (typeof TIERS)[number]['key'];

  /**
   *  档位。**这是 Rust 那份的镜像，不是第二份真值**（硬约束第 5 条）。
   *
   *  初值分两种情况，差别就是安全观感：
   *    · 脱离 Tauri（演示）→ 认 `?tier=`，那是**探针口**，
   *      本来就没有「真档位」可言。
   *    · 真 Tauri → **一律从最低档起步**，哪怕 URL 写了 `?tier=Auto`。
   *      因为 `get_tier` 是异步的，回包之前界面已经画出来了；
   *      那一瞬若显示高档，用户看到的是「程序自己升到确认档」，
   *      而硬约束第 5 条要求默认档最低。真值回来后立刻被覆盖，
   *      但那一下已经被人看见了。
   */
  const fromUrl =
    typeof location !== 'undefined' ? (new URLSearchParams(location.search).get('tier') ?? '') : '';
  const probeTier = TIERS.some((t) => t.key === fromUrl) ? (fromUrl as TierKey) : null;
  let tier = $state<TierKey>(HAS_TAURI ? 'Manual' : (probeTier ?? 'Manual'));

  /** 「关闭时」当前值。Rust 侧持有，同样只做镜像。 */
  let closeBehavior = $state<'exit' | 'tray'>('exit');

  /** 设置主区开没开。**它与正文区互斥**，理由见模板里那条。 */
  let settingsOpen = $state(false);

  /**
   *  点档位。**先问 Rust，再画勾。**
   *
   *  脱离 Tauri 时（演示页）没有 IPC 可问，探针口自己生效——那种情况下
   *  本来就没有「真档位」可言，不构成第二份真值。
   */
  async function chooseTier(key: TierKey) {
    if (!HAS_TAURI) {
      tier = key;
      return;
    }
    try {
      const real = await invoke<string>('set_tier', { tier: key });
      // ⚠ **采信返回值，不乐观更新。** Rust 侧可能不认这次升档
      // （`parse_tier` 认不出来一律落到 `Manual`），会话结束时也会自动回落。
      // 界面先动的话，用户点了「自动发送」、勾已经挪过去了，
      // 而程序发出去的仍按旧档走——屏幕和事实不一致的那一秒最要命。
      if (TIERS.some((t) => t.key === real)) {
        tier = real as TierKey;
      }
    } catch (e) {
      // 失败时**回读真值**，不留在乐观值上：Rust 可能已经回落到 Manual
      // （会话结束自动回落就是这条路径），勾还停在用户点的那一档，
      // 屏幕就会说谎。
      //
      // ⚠ 这里与 `PanelSettings.changeTier` 是**同一个判断的两处实现**，
      //   连「失败怎么办」都写成一样的——早先一处回读、一处只打日志，
      //   于是改档失败的表现取决于你在哪块面板上点的。
      console.error('改档没生效，回读真值：', e);
      try {
        const real = await invoke<string>('get_tier');
        if (TIERS.some((t) => t.key === real)) tier = real as TierKey;
      } catch {
        // 连真值都读不到：停在最低档。方向与 Rust 侧 `parse_tier` 一致，
        // 不因为读不到就假定用户想升档。
        tier = 'Manual';
      }
    }
  }

  /* ── 正文数据 ──────────────────────────────────────────────────
   *
   *  Rust 是唯一事实来源：阶段由 `Phase` 的判别字段决定，界面**不自己从
   *  candidates 的长度反推**（那是旧面板犯过的错——同一个判断在三处各写
   *  一遍、其中两处还写得不一样）。
   *
   *  `?demo=1&state=<名字>` 读 `ui/static/demo/<名字>.json`，让正文排版
   *  能在浏览器里全速迭代，不必每次都去点微信。演示数据不含真实聊天内容。
   */
  let readout = $state<ReadoutPayload | null>(null);
  let peer = $state('');

  const lastFromThem = $derived(
    readout?.messages.filter((m) => m.speaker === 'Them').at(-1) ?? null,
  );
  const drafted = $derived(readout?.phase === 'drafted' ? (readout.candidates[0] ?? null) : null);

  /**
   *  动作行要选**任意一条**候选，而 `drafted` 只给了第一条
   *  （正文行渲染的就是它）。所以这里另取一份完整的。
   *
   *  ⚠ 排序由 Rust 侧保证（`watch.rs` 里 `sorted()` 按 probability 降序后
   *   才存进 `candidates`），**前端不再排一次**——排两次就会在两边
   *   得出不同的「第一条」，而用户点的是屏幕上那条。
   */
  const allCandidates = $derived(readout?.phase === 'drafted' ? readout.candidates : []);

  /**
   *  投递回执。**发出去会发生什么，必须让用户看见**——
   *  发出去的撤不回来，而面板是唯一能告诉他「你刚发了什么」的地方。
   */
  let sentNote = $state<{ ok: boolean; text: string } | null>(null);
  let sending = $state(false);

  /**
   *  投递算不算「成了」——**只有真没成才是失败**。
   *
   *  ⚠ 2026-10-15 之前这里写的是 `r.outcome === 'Sent'`，那在两档之下
   *  **都是错的**，而且错的方向是把成功显示成红色失败：
   *  · `Manual` 档成功返回 `Filled`（填进输入框、没发）；
   *  · `Auto` 档成功返回 `PossiblySent`（按了回车但拿不到更高证据，
   *    而「发送后输入框会被清空」让 OCR 那一刻已经读不到它了——
   *    详见 `inject::fill_and_send` 的文档）。
   *  ⇒ 两种成功态都不叫 `Sent`，而 `Sent` 这个档位在真实链路上
   *  **永远不会被观察到**。于是每一次成功投递界面都在报警。
   *
   *  红只留给「确实没成」：`Failed` 与 `NotSent`。
   *  `PossiblySent`/`TimedOut` 是**不确定**不是失败，走中性墨色。
   */
  const DISPATCH_FAILED = new Set(['Failed', 'NotSent']);

  /**
   *  动作按钮用哪个变体（见 `docs/设计规范.md` §4.2c）。
   *
   *  ## 不可逆优先于「推荐」
   *
   *  两个维度本来是正交的——「这是首选吗」和「按下去可逆吗」——
   *  但三档变体是**平级**的，不是二维。于是只能有一个上位：
   *  `Auto` 档下按任何候选都是**不可逆**的，所以整组走**红批**，
   *  此时「首选」不再单独用墨色竖笔区分。
   *
   *  ## 为什么这条不能只做在 CSS 里
   *
   *  它取决于**档位**，而档位是 Rust 侧持有的安全状态。把它写成
   *  纯 CSS 钩子（前端自己判断「这一档危险」）等于在前端复制第二份
   *  安全判断，而那正是硬约束第 5 条不让人做的事。
   *  这里只把 Rust 已经发过来的 `tier` **翻译成视觉**，
   *  发不发仍然只在 `commands::dispatch` 里判一次。
   */
  function actVariant(i: number): 'cs-act-red' | 'cs-act-lead' | 'cs-act-alt' {
    if (tier === 'Auto') return 'cs-act-red';
    return i === 0 ? 'cs-act-lead' : 'cs-act-alt';
  }

  /**
   *  点候选 → 调 `dispatch`。
   *
   *  ## 档位决定发不发，而判断在 Rust 侧
   *
   *  这里**只负责把 id 与动作送过去**，绝不自己判断「该不该发」。
   *  理由是硬约束第 5 条：那条判断一旦在前端，就等于交给一个
   *  可以随手改的变量。`Manual` 档只填入、`Auto` 档填入并发送，
   *  由 `commands::dispatch` + `sends_for_real` 裁决，并有穷举测试守它。
   *
   *  ⚠ 动作这一支取 `Action::Confirm`（字符串 `"confirm"`）：它是
   *  **「按这个动作去投递」**，不是「我已经确认过了」。
   *  档位才是「允不允许发出去」那一位。两者别混。
   */
  async function dispatchCandidate(candidateId: string) {
    if (sending) return;
    sending = true;
    try {
      const r = await invoke<{ outcome: string; detail: string; next_hint: string | null }>(
        'dispatch',
        { candidateId, action: 'confirm' },
      );
      sentNote = { ok: !DISPATCH_FAILED.has(r.outcome), text: r.detail };
    } catch (e) {
      sentNote = { ok: false, text: `投递失败：${e}` };
    } finally {
      sending = false;
    }
  }

  /**
   *  正文**字体**探针（`?draft=`）。与 `?tier=` / `?shape=` / `?size=` 同一性质，
   *  **只改这一次的渲染，不落盘**。
   *
   *  ⚠ 为什么要有这个口：2026-10-13 出五张字体对照图时，用的是另写的一页
   *  mock。**那页排版坏了**——纸只画了元素中间一小块，正文与按键都跑到纸外，
   *  图是假的。教训：凡是「要判断纸面上的东西」，量具必须跑在真面板上，
   *  mock 一旦排版出错，量具就在骗自己，而且骗得毫无征兆。
   *
   *  ⚠ 本机**没有任何手写字体**（实测 `InstalledFontCollection`：楷体 / 仿宋 /
   *  宋体 / 黑体 / 微软雅黑 / 等线 / 新宋体；华文行楷、华文新魏、隶书、
   *  霞鹜文楷都没有装）。所以「手写」在本项目里只能是**别的手段**，
   *  见 `ui/scripts` 与规范 §4.2。
   */
  const DRAFT_FONTS = {
    kai: "KaiTi, STKaiti, '楷体', serif",
    fang: "FangSong, STFangsong, '仿宋', serif",
    song: "SimSun, '宋体', NSimSun, serif",
    yahei: "'Microsoft YaHei', '微软雅黑', sans-serif",
    hei: "SimHei, '黑体', sans-serif",
  } as const;
  type DraftFontKey = keyof typeof DRAFT_FONTS;
  let draftFont = $state<DraftFontKey | null>(null);

  /** 归属行字号探针（`?peersz=`），数字直接是 CSS px。 */
  let peerPx = $state<number>(15);

  /**
   *  归属行**名字**的呈现探针（`?peer=`）。
   *
   *  ⚠ 起因：15px 一放开，原来的「黑体 600 加粗」立刻变成一块黑。
   *    粗细是这个层级唯一的手段时，字号一大它就崩——所以要给别的手段。
   *  候选（都保持 15px 与同样的灰阶）：
   *    w500 / w400  降粗细，用字号与灰阶扛层级
   *    ink         降到 400，靠**颜色**（--cs-ink）扛层级，不用粗细
   *    tracked     400 + 拉开字距，靠**疏排**扛层级
   *    kai / fang  换成楷体/仿宋，名字与正文同一族
   *  ⚠ 默认仍是 `bold`（= 原来的 600），探针口不替默认改口径。
   */
  type PeerLook = 'bold' | 'w500' | 'w400' | 'ink' | 'tracked' | 'kai' | 'fang';
  /** 默认 `tracked`：用户判「黑体 600 加粗在 15px 上是一块黑」。五种里
   *  只有它既保住「铅笔灰＝机器认出来的」这层语义，又用**疏排**而不是
   *  粗细扛住了层级。`ink` 是第二选择（更平，但名字变墨色＝分不清
   *  「机器认的」和「你写的」）。要改就改这一个默认值。 */
  let peerLook = $state<PeerLook>('tracked');

  /**
   *  档位整格放大系数。**1.35 是量出来的**：用户判「1.0 不够、1.5 太抢」，
   *  落在两者之间。1.35 下取景框 81×54、档位名 14.85px、整格 71.6px，
   *  红勾与正文(21px)的比回到 1.41 —— 正文仍然明显主导，但档位不再
   *  像「补了个控件」。
   *
   *  ⚠ 1.35 仍**不是**最终形态。真正要对齐的是生成器的幅面档
   *    （`pencilgen.py` 出 base/mid/big 三档），CSS 系数只是把已有的
   *    `big` 整体拉大；再往上就该加第四档几何，而不是继续乘系数。
   */
  let tierK = $state<number>(1.35);

  onMount(() => {
    /*
     * 标记形状落在 Rust 侧的 prefs，这里只读镜像。
     *
     * ⚠ `?shape=` / `?size=` 都是**探针口**，与上面 `?tier=` 同一性质：方便把
     * 各档各截一张 1× 图对着比。它们在演示/静态页里只改内存镜像
     * （`markShape.set` 见 `HAS_TAURI` 分支），**不落盘**，
     * 所以拿它做实验不会把真机设置改掉。
     */
    const q = new URLSearchParams(location.search);
    const shapeProbe = q.get('shape');
    const sizeProbe = q.get('size');
    const draftProbe = q.get('draft');
    const peerProbe = q.get('peersz');
    const sizes: MarkSize[] = ['base', 'mid', 'big'];

    if (draftProbe !== null && draftProbe in DRAFT_FONTS) {
      draftFont = draftProbe as DraftFontKey;
    }
    if (peerProbe !== null && /^\d{1,2}$/.test(peerProbe)) {
      peerPx = Number(peerProbe);
    }
    const tierProbe = q.get('tiersz');
    if (tierProbe !== null && /^\d(\.\d+)?$/.test(tierProbe)) {
      const k = Number(tierProbe);
      // 只认 1.0~2.0：再大就该改生成器的幅面档，而不是拿 CSS 硬拉。
      if (k >= 1 && k <= 2) tierK = k;
    }
    const peerLookProbe = q.get('peer');
    const looks: PeerLook[] = ['bold', 'w500', 'w400', 'ink', 'tracked', 'kai', 'fang'];
    if (peerLookProbe !== null && looks.includes(peerLookProbe as PeerLook)) {
      peerLook = peerLookProbe as PeerLook;
    }

    // 设置页探针。设置屏是**同一张纸的另一个面**（`data-settings='true'`
    // 整张换掉纸底，见下面那条 CSS），所以出图必须能单独把它打开，
    // 否则每次评审都得手点齿轮。
    if (q.get('settings') === '1') settingsOpen = true;

    if (sizeProbe !== null && sizes.includes(sizeProbe as MarkSize)) {
      markSize = sizeProbe as MarkSize;
    }
    if (shapeProbe === 'tick' || shapeProbe === 'ring') {
      void markShape.set(shapeProbe);
    } else if (sizeProbe !== null && sizes.includes(sizeProbe as MarkSize)) {
      /* 只给了幅面没给形状时，至少保证渲染的是「勾」，否则会以为探针没生效。

       * ⚠ 判据是「**URL 上给了 `size=`**」，**不是** `markSize !== 'big'`。
       *   那个旧判据把「幅面当前是不是 big」当成了「用户有没有要求探针」——
       *   而 `markSize` 的默认值一改（这一版从 big 回到 base），它就恒为真，
       *   于是**每一次正常进入面板都会硬写一次形状**，把 Rust 里存的
       *   用户偏好覆盖掉。症状是：明明在设置里选了圈或勾，进面板就被改回。
       *   探针口只能由探针口触发，不能由内部默认值触发。 */
      void markShape.set('tick');
    } else {
      void markShape.load();
    }

    // 演示数据在真实 Tauri 里永远不装载。理由见 `loadDemo` 的注释。
    if (IS_DEMO) {
      void loadDemo();
      return;
    }

    /*
     * 真 Tauri：档位与「关闭时」都问 Rust，推送听 Rust 侧那条 observe 循环。
     *
     * ⚠ **这里原先一个 `listen` 都没有**（2026-10-13 实测）：面板只在
     * `IS_DEMO` 下读夹具 JSON，真实窗口里 `readout` 推过来也没人接，
     * 正文区永远停在「对方说完话，这里才落笔」。Rust 侧那条 emit 是活的
     * （`commands.rs` 观察循环），断的是这一端。
     *
     * ⚠ 静态部署的静态页（既没 `?demo` 也没 Tauri）落在下面那条
     * `HAS_TAURI` 判据之外：那里既没有 IPC 也没有夹具，`offline` 由
     * `(panel)/+layout.svelte` 那条横幅明说，不在这里假装在等。
     */
    let dispose: (() => void) | null = null;
    let cancelled = false;

    void (async () => {
      if (HAS_TAURI) {
        try {
          const [t, cb] = await Promise.all([
            invoke<string>('get_tier'),
            invoke<'exit' | 'tray'>('get_close_behavior'),
          ]);
          if (TIERS.some((x) => x.key === t)) tier = t as TierKey;
          closeBehavior = cb;
        } catch (e) {
          // 读不到就停在最低档 + 退出档。**解析失败一律落到最低**，
          // 与 Rust 侧 `parse_tier` 的方向一致：不因为读不到就假定用户
          // 想升档，那是要用户自己的动作才该发生的事。
          console.error('读档位/关闭行为失败，按最低档显示：', e);
        }
      }

      try {
        const un = await listen<ReadoutPayload>('readout', (e) => {
          readout = e.payload;
          // 会话名闸关着时 `peer` 也可能是空串——那正是该让用户看见
          // 「没认出来」的时候，归属行据此显示占位而不是上一轮的名字。
          if (typeof e.payload.peer === 'string') peer = e.payload.peer;
        });
        if (cancelled) {
          un();
          return;
        }
        dispose = un;
      } catch (e) {
        // 监听装不上 = 这一轮之后界面再也不会更新。空着的面板比一句
        // 报错危险，所以必须留痕，别让它安静地待着。
        console.error('连不上 Rust 的 readout 推送：', e);
      }

      /*
       * 托盘「打开设置」。Rust 侧 `show_panel` 之后 emit 这条（先把面板
       * 亮出来，再让用户看到设置区——反过来的话用户会看到一块纸，
       * 以为自己还没点生效）。
       *
       * ⚠ 这条**旧面板在听，删掉旧面板时很容易漏掉**：漏了的表现是
       * 托盘菜单点得动、面板也弹出来了，但设置区没打开——一个不报错的
       * 半截功能。
       */
      try {
        const unMenu = await listen('menu:settings', () => {
          settingsOpen = true;
        });
        if (cancelled) {
          unMenu();
          return;
        }
        const prev = dispose;
        dispose = () => {
          unMenu();
          prev?.();
        };
      } catch (e) {
        console.error('连不上托盘的「打开设置」：', e);
      }

      /*
       * 托盘「重新锁定当前会话」。
       *
       * ⚠⚠ **这条旧面板在听，删旧面板时连它一起没了**（`5031ab3`），
       *   而新的纸面板一直没补上——于是托盘那个菜单项点了**什么都不会发生**：
       *   Rust 侧 `app.emit(MENU_RELOCK, ())` 发出去没人收，
       *   `relock_session` 命令前端一次都没调用过
       *   （`rg 'menu:relock|relock_session' ui/src` 零命中）。
       *   票 28 的表把它记成「活的」，那条记录是**假的**。
       *
       *   它要紧的原因是 `commands.rs` 那句：用户在错的会话上启动之后，
       *   「这条命令是**唯一的修正手段**」——硬约束第 1 条（只对锁定
       *   会话动手）的补救入口，整个产品的兜底。
       *
       * ⚠ 与 `menu:settings` 同样的半截功能症状：**不报错、不弹窗、
       *   菜单点得动**。所以新加的 `every_emitted_event_has_a_frontend_listener`
       *   会盯住这一条——补上了监听就别想再悄悄删回去。
       */
      try {
        const unRelock = await listen('menu:relock', () => {
          // 失败要留痕：Rust 那句是「观察循环还没起来」，用户点了
          // 却什么都没发生时，那是唯一能说的话。
          invoke('relock_session').catch((e) => console.error('重新锁定失败：', e));
        });
        if (cancelled) {
          unRelock();
          return;
        }
        const prev = dispose;
        dispose = () => {
          unRelock();
          prev?.();
        };
      } catch (e) {
        console.error('连不上托盘的「重新锁定」：', e);
      }
    })();

    return () => {
      cancelled = true;
      dispose?.();
    };
  });

  /**
   * 演示数据。`?demo=1&state=<名字>` → `ui/static/demo/<名字>.json`。
   *
   * ⚠ **尊重数据里写的 `phase`，别自己从 candidates 推。** 旧面板就是从
   *   `candidates.length === 0` 反推阶段的，于是「静默 / 出错 / 刚启动」
   *   三种完全不同的状态在同一个条件下坍成一种——设计评审看不见的状态，
   *   等于没设计它。
   */
  async function loadDemo() {
    const state = new URLSearchParams(location.search).get('state') ?? 'drafted';
    const r = (await fetch(`/demo/${state}.json`)
      .then((x) => x.json())
      .catch(() => fetch('/demo/drafted.json').then((x) => x.json()))) as {
      phase?: string;
      peer?: string;
      tier?: string;
      reason?: string;
      messages?: Message[];
      candidates?: Candidate[];
      error?: string | null;
      status_hint?: string | null;
    };
    const msgs = r.messages ?? [];
    const cands = r.candidates ?? [];
    const hint = r.status_hint ?? null;
    // ⚠ **`tier` 原来根本没读**（2026-10-14 实测）。于是高档夹具（原
    // `confirm.json`，2026-10-15 已改名 `auto.json`）与 `drafted.json`
    // 渲染出来**逐字节相同**——两个夹具的唯一差别就是
    // `tier` 与 `reason`，而后者一个都没接。演示页于是只演得出四态，
    // 文档里写的「七态」有一半是假的。
    //
    // 边界要写清楚：这里**没有第二份真值**。脱离 Tauri 时压根没有 Rust
    // （`HAS_TAURI` 为 false），`chooseTier` 也是直接改本地 state 的——
    // 同一个「演示下本地说了算」的约定，不是新增的一套。
    if (!HAS_TAURI && r.tier && TIERS.some((t) => t.key === r.tier)) {
      tier = r.tier as TierKey;
    }
    // ⚠⚠ `status_hint` **四支都要带**（2026-10-14 真机实测后补的）。
    //   原来只有 `drafting` 那支带它，`read` 那一支写成 `{ phase, messages }`
    //   就完了 —— 而 `status_hint` 在真实 `Readout` 里是**与 phase 正交**的：
    //   Rust 侧闸一关就会改用它（`GATE_CLOSED_HINT`），哪怕 phase 还是 `read`
    //   甚至 `drafting`。⇒ 夹具装载器若按 phase 挑字段，就等于在夹具里
    //   **把「闸关着」那一支裁掉了**，那条路径在 `?demo` 下永远验不到。
    readout =
      r.phase === 'drafting'
        ? { phase: 'drafting', messages: msgs, status_hint: hint }
        : r.phase === 'failed'
          ? {
              phase: 'failed',
              messages: msgs,
              status_hint: hint,
              error: r.error ?? '未知原因',
            }
          : r.phase === 'drafted'
            ? { phase: 'drafted', messages: msgs, status_hint: hint, candidates: cands }
            : { phase: 'read', messages: msgs, status_hint: hint };
    peer = r.peer ?? '';
  }

  /*
   * 勾还是圈 —— **用户的选择，不是设计定论**（2026-10-13）。
   *
   * 两支笔在 `marks.js` 里本来就都生成好了，切换只是换一组 path；
   * 笔性也一致：都是「中心线 + 宽度剖面」算出来的轮廓。语义分工：
   * 勾 = 「这一档被批了」，圈字环 = 「这一档被圈中了」，
   * 形状差异在 1:1 下比粗细差异更容易读出来。
   *
   * ⚠ 取景框**不是两支笔共用的**。根 `<svg>` 的 overflow 是 hidden，
   * 放大的档一旦画到 60×40 之外就被静默裁掉，表现为「放大了但还是没变大」——
   * 那是量具自己骗自己。所以放大的档各自配一个大 VIEW，见 `ACTIVE_VIEW`。
   */
  const MARK_LAYERS = $derived(
    markShape.current !== 'ring'
      ? markSize === 'mid'
        ? TICK_NESTED_MID
        : markSize === 'big'
          ? TICK_NESTED_BIG
          : TICK_NESTED
      : markSize === 'mid'
        ? RING_LABEL_MARK_BIG
        : markSize === 'big'
          ? RING_LABEL_MARK_HUGE
          : RING_LABEL_MARK,
  );

  /** 放大档放不进 60×40，得换框；base 两支笔共用 MARK_VIEW。 */
  const ACTIVE_VIEW = $derived(
    markSize === 'big'
      ? markShape.current === 'ring'
        ? RING_VIEW
        : TICK_VIEW
      : markSize === 'mid'
        ? markShape.current === 'ring'
          ? MARK_VIEW
          : TICK_VIEW
        : MARK_VIEW,
  );
</script>

<svelte:head>
  <title>纸面板</title>
</svelte:head>

<main
  class="cs-sheet"
  data-tier={tier}
  data-settings={settingsOpen}
  data-draft-font={draftFont ?? ''}
  data-peer-px={peerPx ?? ''}
  data-tiersz={String(tierK)}
  style:--tk={tierK}
  style:--peer-probe={peerPx ?? ''}
  data-peer-look={peerLook}
>
  <!--
    ⚠ **撕口那条上沿是拖拽带**（`data-tauri-drag-region`）。

    纸面板没有「头部」这一行——四行骨架里也没有（设计规范 §4.2b）——
    而 `tauri.conf.json` 的主窗口是 `decorations: false`，没有抓手就
    推不动这个窗。给它整张纸做拖拽区也不行：正文要能选中复制，
    拖拽带会吃掉正文上的每一次按下。

    所以拖拽带 = **撕口那一行**。理由不止是「空着」：
    抓纸上缘拖动正好是这张纸的物理动作，撕口本来就是这条边上唯一的
    起伏。`padding-top` 的 40px 正是它让出来的。

    ⚠ 控件一律 `data-no-drag`（现在挂在 `.cs-cpanel` 上，`WinCtl` 已删）——否则点 × 会变成
    「拖动窗口」，而且**界面上什么都不会报错**。
  -->
  <div class="cs-topbar" data-tauri-drag-region>
    <div class="cs-cpanel" data-no-drag>
      <!--
        ⚠ **记号是自己画的，不是图标库里的。**

        2026-10-13 实测踩到：这一版原先直接复用旧面板的 `WinCtl` + lucide 的
        `Settings`，理由是「零改动复用」。那是错的——旧面板的记号语言
        （lucide Web 默认图标）正是设计规范判为废弃的那一套，搬进纸面就成了
        「贴在手撕纸上的标签」。

        **三项同时对齐**，少一项就还是不对：
          · 外接尺寸 × 端点跨度 11.2 / 减号长 10.4 / 齿轮外径 13
          · 描边粗细 同为 1.55px ——⚠ `stroke-width` 是 **viewBox 单位**，
            SVG 一放大描边就同比变粗。早先按「把 × 的框调大来对齐外接尺寸」
            修过一轮，结果 × 的描边比齿轮粗 36%：外接对齐了，描边没对齐。
          · 笔性     全部套 `url(#pencil)`，与档位标记同一支笔
            ⚠ **正文不套**，2026-10-14 实测否掉，见下面 CSS 那段。
        齿轮第一版偏散、读起来像太阳，把圆环从 r4 加到 4.6、齿尖收到 6.2 才压住。
      -->
      <button
        class="cs-glyph"
        class:on={settingsOpen}
        type="button"
        data-no-drag
        onclick={() => (settingsOpen = !settingsOpen)}
        title="设置"
        aria-label="设置"
      >
        <svg
          width="20"
          height="20"
          viewBox="0 0 20 20"
          fill="none"
          stroke="currentColor"
          stroke-width="1.55"
          stroke-linecap="round"
          aria-hidden="true"
          filter="url(#pencil)"
        >
          <circle cx="10" cy="10" r="4.6" />
          <path d="M10 4.6V2.2" /><path d="M10 15.4v2.4" />
          <path d="M4.6 10H2.2" /><path d="M15.4 10h2.4" />
          <path d="m6.25 6.25-1.7-1.7" /><path d="m13.75 13.75 1.7 1.7" />
          <path d="m13.75 6.25 1.7-1.7" /><path d="m6.25 13.75-1.7 1.7" />
        </svg>
      </button>

      {#if closeBehavior === 'tray'}
        <button
          class="cs-glyph"
          type="button"
          data-no-drag
          title="收进托盘 · 程序继续运行"
          aria-label="收进托盘"
          onclick={() => void invoke('hide_panel').catch(() => {})}
        >
          <svg
            width="20"
            height="20"
            viewBox="0 0 20 20"
            fill="none"
            stroke="currentColor"
            stroke-width="1.55"
            stroke-linecap="round"
            aria-hidden="true"
            filter="url(#pencil)"
          >
            <path d="M4.8 10.3c3.4-.7 6.9-.8 10.4-.2" />
          </svg>
        </button>
      {/if}

      <button
        class="cs-glyph cs-glyph-quit"
        type="button"
        data-no-drag
        title={closeBehavior === 'tray' ? '退出 · 停了就是停了' : '退出'}
        aria-label="退出"
        onclick={() => void invoke('quit_app').catch(() => {})}
      >
        <svg
          width="20"
          height="20"
          viewBox="0 0 20 20"
          fill="none"
          stroke="currentColor"
          stroke-width="1.55"
          stroke-linecap="round"
          aria-hidden="true"
          filter="url(#pencil)"
        >
          <path d="M4.4 4.6c3.3 3.6 8 8.2 11.2 11.1" />
          <path d="M15.6 4.4C12.2 8.1 7.6 12.4 4.5 15.5" />
        </svg>
      </button>
    </div>
  </div>

  <!--
    ⚠ **档位不在顶部**（2026-10-13 用户否决）。
    这一行原先是档位区，现在让给正文区，档位降级到归属行右侧。

    原先它独占一整行：撕口 40px + 槽 53px + 下边距 28px ≈ **121px，占面板
    21%**，而正文只有 4 行。更糟的是红铅笔勾**盖在标签上**，「只填入」三个字
    被笔画压成一团。

    那是把**低频设置放进了黄金位**——档位一年未必改一次，却让纸的上半部分
    一直在喊「看这里」。现在它挪到正文下方，槽高 53 → 24px，
    标记取景框 60×40 → 34×22（所以默认幅面回到 `base`：`big` 那套偏移是按
    60×40 算的，塞进小框里只会糊成一团）。位置的来龙去脉见档位行上那段注释。
  -->

  <!--
    正文区。四行固定骨架（§4.2b）：归属 → 正文 → 档位 → 动作。
    ⚠ 骨架原本第一行是档位，现已按实测挪到最后一段；动作行**故意还没有**
    ——它要碰发送路径（硬约束第 5 条），不能跟排版一起顺手做掉。
    宁可少一行，不要一个没验过的发送控件。
  -->
  <!--
    ⚠ **设置是一块互斥的主区，不是叠在主区上面的一层。**

    早先的 `open_settings` 是**另一个窗口**，于是设置和主区可以同时占着
    屏幕。现在它是同一块面板的另一种显示 —— 两个都显示会让「我现在看到的
    到底是不是正在替我说话的那一句」变得没法回答。

    所以这里用 `{#if}` 换，不是用 `position: absolute` 盖：
    盖住的话底层那份内容还在滚，滚轮会滚到看不见的那一层去。

    **撕口那条拖拽带与窗口控制留在外面**：窗口的出口不能跟着内容消失，
    而设置里也要改档，顶边那道撕口是这张纸的边界，同样得一直在。

    ⚠ 设置态**整张换掉纸底**（`.cs-sheet[data-settings='true']`，见下面那段），
    这不是漏改：设计规范 §1「纸面是配置界面唯一的敌人」，材质切换本身
    就是「你现在不在抄写，在配置」这句提示。`PanelSettings` 用的仍是
    纸面板那套 `--cs-*` 配色 token，只是不铺那张撕口纸。
  -->
  {#if settingsOpen}
    <!--
      `tier` / `closeBehavior` 是 `$bindable` 双向绑：设置里改档，
      **纸底**那道勾立刻跟着变。**不留第二份。**
    -->
    <PanelSettings bind:tier bind:closeBehavior />
  {:else}
    <div class="cs-body-col">
      <!--
        归属行。**只回答一件事**：这句话是替谁写的。

        ⚠ 这里原先在右边塞了档位。2026-10-13 实测截图自查发现那样做会
        把归属文字挤成两行（「示例联系人」被折断）——316px 的内容栏里，
        两枚记号加两个标签要占掉 134px，剩下 170px 装不下「名字 + 引文」。
        **挤断别人的行来给自己腾位置是净负收益**，所以档位移出去了，
        这一行拿回全部宽度。

        ⚠ 证据**必须显示**：这是机器认出来的对话，不是用户自己翻的。
           认错了还照发，比不写更糟。
      -->
      <p class="cs-where">
        <!--
          认不出会话名时**说人话**（票 30 验收项）。
          原来这里写死 `{peer || '—'}`：一个破折号。它没说清「没认出来」，
          而这恰恰是用户必须知道的那件事——面板正拿着一句起草好的话，
          却不知道自己替谁在写。票 30 真机取证里 `peer` 恒为空串，
          于是这块就一直是个破折号。
          ⚠ 红**不用于此**：`--cs-pencil-red` 只给不可逆（app.css 的分工）。
          「认不出」是可恢复的提醒，不是警报，所以走墨色加重。
        -->
        {#if peer}
          <span class="cs-peer">{peer}</span>
        {:else}
          <span class="cs-peer-unknown">没认出这是谁的对话</span>
        {/if}
        {#if lastFromThem}
          <span class="cs-where-quote">{lastFromThem.text}</span>
        {/if}
      </p>

      <!--
      正文行。**只有墨色**：`--cs-mark` 在真实纸底上只有 3.00 : 1，掉出 AA，
      正文小字用它就是不合规（app.css 的注释记着这条是怎么翻的）。
    -->
      <div class="cs-draft">
        {#if drafted}
          <p class="cs-draft-text">{drafted.text}</p>
        {:else if readout?.phase === 'drafting'}
          <!--
          这句是 Rust 的 `Phase::status_hint()`，**不在这里写死**。
          判据 12：空状态必须给明确的下一步——「正在写……」只说了现状，
          Rust 那句连着说了「通常几秒」，用户才知道该等。夹具没带这个字段时
          才退回短句，那只是演示数据的缺口，不是口径。
        -->
          <p class="cs-draft-text cs-draft-pending">
            {readout.status_hint ?? '正在写……'}
          </p>
        {:else if readout?.phase === 'failed'}
          <p class="cs-draft-text cs-draft-failed">没写成：{readout.error}</p>
        {:else if readout?.status_hint}
          <!--
            ⚠ 这一支守的是「闸关着」那条路径（2026-10-14 真机实测出来的假话）：
            用户在微信里切到另一个会话，闸关上、面板被清空，正文却显示
            「对方说完话，这里才落笔。」——**而那个会话对方明明说过话**。
            界面分不出「对方真没说话」与「已停泊、没在读这个会话」，
            两者的 `messages` 都是空数组。

            ⇒ 判据放在 Rust 那边（`watch.rs` 的
            `a_closed_gate_replaces_the_waiting_line_with_an_honest_one`），
            文案也是 Rust 出的 `status_hint`；**这里只渲染，不自己写死**。
            放在 `lastFromThem` 那一支**之前**：闸一关就压过「对方说完了」。

            ⚠⚠ `readout?` 的问号**不能省**：`readout` 初值是 `null`，
            而这一支排在 `readout?.phase === 'failed'` **之后**，
            前几支都用了可选链。2026-10-14 首次写这里时漏了它，
            `pnpm build` 与 `svelte-autofixer` **全过**，
            而真浏览器里整个面板崩成白屏：
            `TypeError: Cannot read properties of null (reading 'status_hint')`。
            ⇒ 判据工具全绿 ≠ 界面能渲染；这一支必须用可选链。
          -->
          <p class="cs-draft-text cs-draft-idle">{readout.status_hint}</p>
        {:else if lastFromThem}
          <!--
            ⚠ 这一支原先不存在，于是 `noprep` 那一态在说假话（2026-10-14 截图自查）：
            归属行刚显示对方最后一句「行，不急」，正文却写着「对方说完话，这里才落笔」——
            **对方已经说完了**，这句话与上一行直接打架。
            票 30 的真机状态正是这一支：会话名认不出来 ⇒ 没有候选，
            而面板把「没起草」说成「还没轮到」，用户会一直等一个不会来的落笔。
            ⇒ 只陈述事实，不猜原因（为什么没起草由归属行那句「没认出这是谁的对话」说）。
          -->
          <p class="cs-draft-text cs-draft-idle">对方说完了，这轮没起草。</p>
        {:else}
          <p class="cs-draft-text cs-draft-idle">对方说完话，这里才落笔。</p>
        {/if}
      </div>

      <!--
        动作行。**2026-10-15 落地**——此前 `dispatch` 是
        `contract.rs` 的 `KNOWN_UNREACHED` 之一：命令注册了、前端不调，
        于是「两档发送」在界面上**没有任何可点的对象**，
        票 20 的实机验收项严格说无从验起。

        ## 位置：正文之下、档位之上

        它是**候选那一组的一部分**，不是一块独立的设置。
        正文写的是第一条候选，按钮就排在这一组下面——
        「看这句 → 按它」中间不插别的东西。
        档位行在动作行下面：档位是**动作参数**，改档位是为了改变
        上面那几个按钮按下去会发生什么，所以它在动作下面读起来才顺。

        ## 按钮上不写「发送」二字

        档位已经在下面明写了「只填入 / 自动发送」，同一个词再写一遍
        就是把**参数**当成**动作**——而这里的动作永远是「按这个候选」，
        发不发由档位决定。⚠ 尤其在「自动发送」档下，按钮若写着「发送」
        会让人以为这里还有一个独立的确认步骤，而那里没有。
      -->
      {#if allCandidates.length > 0}
        <div class="cs-actions" data-no-drag>
          <!--
            不可逆前置说明 —— 规范 §4.2b 定它在**按钮上方**。

            ⚠ 只在 `Auto` 档出现。`Manual` 档按下去只是填进输入框，
            在那里写「按下去就发出去了」是**假的**——
            而一个在可逆路径上喊不可逆的提示，会让人对真不可逆的那次
            也当噪音划过去。

            ⚠ 这段话是**当前档位**的如实描述，不是新的判断。
            「该不该发」仍然只在 `commands::dispatch` 里判一次。
          -->
          {#if tier === 'Auto'}
            <p class="cs-act-warn">「自动发送」档：按下去就发出去了，收不回来。</p>
          {/if}

          {#each allCandidates as c, i (c.id)}
            <!--
              票 44：**候选是「从这张纸上撕下来的一片」，不是按钮。**

              原来是 1px 描边 + radius 4 的按钮（`.cs-act`），而这张纸的其余
              部分是撕边纸 + 手绘笔迹 + 留白分隔。规范 §4.2b v2 早就否过
              「按钮拿现成色块当零件」，§4.2c 定的按钮族恰恰是 Web 按钮语言
              ——**规范自己两节打架**，这才是根因；绕着它画了三轮边框是白费。

              ## 三变体怎么活下来的（票 44 验收项要求「分工仍然成立」）

              旧机制是**页边一道 3px 竖笔**：墨色 = 划重点，红色 = 撤不回来。
              撕边之后页边没有了，通道换成**手绘记号**——项目里本来就有这一支
              （`TICK_NESTED` 五层嵌套 + 红铅笔滤镜，档位行每天在用）：
                  备选  = 无记号          （可逆、不是推荐）
                  首选  = 墨色手绘勾      （划重点）
                  自动发送档 = 红铅笔勾    （撤不回来）
              两个维度仍共用一个通道、靠颜色分工，语义与旧版逐条对应。

              ⚠ **`Auto` 档下只有首选带勾，备选没有**（2026-10-16 实测后改，
                原来写成「整组都是红勾」是**过时的**，那版把面板挤爆了 20px）。
                理由与实测数在下面 `{#if i === 0}` 那段注释里。
                「不可逆优先于推荐」这条上位规则仍然成立，只是它现在由
                **组级通道**承担：红字警告行 + 纸底档位开关上的大红铅笔勾。
            -->
            <button
              type="button"
              class={['cs-act', actVariant(i)]}
              disabled={sending}
              onclick={() => void dispatchCandidate(c.id)}
              aria-label="{i === 0 ? '按这个候选' : `备选 ${i + 1}`}：{c.text}"
            >
              <!--
                ⚠ **记号只给首选**（2026-10-16 实测后从「`Auto` 档三条全带」改的）。
                不可逆在 `Auto` 档下由**组级**通道说，不靠每条各画一枚：
                  ① 组上方那行红字「按下去就发出去了，收不回来」（§4.2b 定的位置）
                  ② 纸底的档位开关本身：`自动发送` 上那枚大红铅笔勾
                两条都是红铅笔、都在说同一件事，而**都不占候选行的横向空间**。
                ⇒ 组已经说过了，就不必每条再说一遍。

                ⚠⚠ **反过来，每条都带勾会挤爆面板**（实测，不是推演）：
                  记号 20px + 间隙 6px = 26px，把备选正文从 217px 压到 191px，
                  而那行字 ~400px —— 191 装不下两行（400/191 = 2.09），
                  于是**每条备选多折一行**（12.5px × 1.5 = 18.75px）。
                  量 `.cs-pick` 的 rect：
                    基线 `Auto` 档        顶边 y=453，档位格底 525（余 5px）
                    三条全带勾的 `Auto` 档  顶边 y=478，档位格底 550 ⇒ **溢出 20px**
                  而 `.cs-sheet` 有 `overflow: hidden` ⇒ **档位行被静默裁掉**。
                  ⚠ 这正是最不能发生的地方：`Auto` 是不可逆的那一档，
                  而被裁掉的恰好是**告诉用户当前在哪一档**的那一行。
                ⚠ 所以记号尺寸与本条 `{#if}` 是**一对**，别单独动其中一个。
              -->
              {#if i === 0}
                <!--
                  ⚠ `TICK_VIEW` 是 56×34，那是**档位选择器**那一行的尺寸——
                  它独占一整行 72px 高。而这里纸条总高才 ~30px，
                  原尺寸放进去会盖住整句话。
                  ⇒ 缩到 **20×12**：够认出手绘的收放，又退回到批注的分量。
                  viewBox 不变，轮廓数据一个字节没动。

                  ⚠⚠ **20 而不是 26**：26px 的勾会让首选那行变高，
                  而面板在 `Auto` 档只剩 5px 余量（见上面那段量测）。
                  这类「宽度/高度差一点点就翻行或溢出」的耦合最阴险：
                  **改记号尺寸必须重新量 `.cs-pick` 的 rect，两个档都要量。**

                  ⚠ 红铅笔滤镜是**笔的一部分**，不是效果。少了 `url(#pencil)`
                  它就是矢量而不是铅笔（§3.2：五层嵌套 + 羽化是一套，
                  单层平涂边缘一刀切）。
                -->
                <span class="cs-act-mark">
                  <svg
                    class="cs-pencil-mark"
                    viewBox="0 0 {TICK_VIEW.w} {TICK_VIEW.h}"
                    width="20"
                    height="12"
                    aria-hidden="true"
                    filter="url(#pencil)"
                  >
                    {#each TICK_NESTED as l (l.d)}<path d={l.d} fill-opacity={l.o} />{/each}
                  </svg>
                </span>
              {/if}
              <!--
                ⚠⚠ **「按这个 / 备选 N」标签整条删掉了**（2026-10-16，用户直接指出
                  「开头有什么必要加『备选X』的字」）。

                **它为什么该删 —— 三个理由，从强到弱：**

                ① **它在抢位置，而位置是这里最稀缺的东西。**
                  量具实测：`.cs-act-tag` 占 **37.9px**，加上 `gap: 6px` = 44px。
                  备选正文宽 221.4px，那行字 ~400px ⇒ 221 装不下，**每条备选多折一行**
                  （12.5px × 1.5 = 18.75px/行）。删掉标签后正文宽到 ~265px。
                  而腾出横向空间**直接等于腾出纵向空间**——这正是撕边要的 67px
                  的主要来源（见 `.cs-act` 里那段算式）。

                ② **它是被别的东西说过的第二遍。**
                  「首选 vs 备选」这件事，**首选那一行已经用手绘勾说了**；
                  而 `aria-label` 里也已经有「按这个候选 / 备选 N：正文」。
                  屏幕上那一行字是对同一件事的第三次陈述。

                ③ **它在替用户做编号，而这个编号没有用。**
                  用户挑的是「哪句话能直接发」，不是「第几个」。
                  三条候选各写一个 1/2/3，反而暗示「顺序有讲究」。

                ⚠ **无障碍没有因此变差**：`aria-label` 保留原有的
                  「按这个候选：…」/「备选 2：…」，屏幕阅读器读到的信息
                  与视觉上写着的一字不差。删的是**视觉冗余**，不是**语义**。
              -->
              <!--
                ⚠⚠ **首选也显示正文**（2026-10-16 改，原先 `{#if i > 0}`）。
                删掉「按这个 / 备选 N」标签之后，首选那行只剩一枚勾和概率，
                整条**中空**——而那正是「一条空白横条」，
                读起来比有字的更像个没做完的控件。
                现在的分工：
                  首选 = 手绘勾 + **正文**（重复了上方 `.cs-draft` 那一句）
                  备选 = 无勾 + 正文
                ⇒ 三条形态一致（都有正文），差别只剩**一枚勾**，
                  这正是「首选」在纸上语言里唯一需要的表达。

                ⚠⚠ **代价是首选那行会多折 1~2 行**，而纵向已经见底（见
                `.cs-actions` 的间隙那段）。这条已量过：首选正文 ~140px 宽栏里
                要 3 行。若后面必须再压，**压这里的行高，不要压撕边**。
              -->
              <span class="cs-act-text">{c.text}</span>
              <!-- ⚠ 只有一条候选时不显示概率：预设起草器恒给 100%，
                那是「没有第二个选择」的算术结果，不是置信度。 -->
              {#if allCandidates.length > 1}
                <span class="cs-act-prob">{Math.round(c.probability * 100)}%</span>
              {/if}
            </button>
          {/each}
          {#if sentNote}
            <!--
              ⚠ 回执**必须显示**。发出去的消息撤不回来，而这是唯一能让
              用户知道「刚才那下到底发生了什么」的地方。
              成功走墨色、失败走红（红只给不可逆，app.css 的分工）。
            -->
            <p class="cs-act-note" class:cs-act-note-bad={!sentNote.ok}>{sentNote.text}</p>
          {/if}
        </div>
      {/if}

      <!--
        档位行 —— **纸的底部**，紧贴动作行。

        位置的三次变化，每一步都是被顶出来的，不是设计出来的：
          ① 原来独占顶部一整行（撕口 40 + 槽 53 + 下边距 28 ≈ 121px，
             占面板 21%），且红铅笔勾**盖在标签上**——把低频设置放进了
             黄金位，纸的上半部分一直在喊「看这里」。
          ② 改到归属行右侧（2026-10-13 用户否决顶部后的第一反应），
             结果挤断了归属文字。位置对了、行是错的。
          ③ 落到正文下方。档位是**动作参数**，不是身份信息，语义上就属于
             动作那一侧；纸底那一片空白也是它本来该待的地方。
        底部的坏处要认：面板高的时候它离正文远，扫视要往下走一趟。
        这与「它一年未必改一次」是一回事——远处的低频项可以接受。
      -->
      <div class="cs-pick" data-no-drag role="radiogroup" aria-label="发送档位">
        {#each TIERS as t, i (t.key)}
          <button
            type="button"
            class="cs-pick-slot"
            data-idx={i}
            aria-checked={tier === t.key}
            role="radio"
            tabindex={tier === t.key ? 0 : -1}
            onclick={() => void chooseTier(t.key)}
          >
            <span class="cs-slot-mark">
              <!--
                参考纸上没有线：三行字，谁被勾了谁就是当前档。
                所以这里**不画占位的短记号**——空着就是空着，
                勾的位置本身就是编码。
              -->
              <span class="cs-markbox">
                <svg
                  class="cs-tick"
                  data-shape={markShape.current}
                  data-ring={markSize}
                  viewBox="0 0 {ACTIVE_VIEW.w} {ACTIVE_VIEW.h}"
                  width={ACTIVE_VIEW.w}
                  height={ACTIVE_VIEW.h}
                  aria-hidden="true"
                  filter="url(#pencil)"
                >
                  <!--
                  五层嵌套，不是单层平涂。单层不管给多少 opacity，边缘都是
                  一刀切的硬边，所以它读成矢量而不是红铅笔——材质台 P1 与 P5
                  同框实测：落在中间密度带的像素 23.0% → 70.5%，总墨量反而少
                  17%。密度从芯往外逐级下降，才有「芯重边虚」。
                -->
                  {#each MARK_LAYERS as layer (layer.d)}
                    <path d={layer.d} fill-opacity={layer.o} />
                  {/each}
                </svg>
              </span>
              <span class="cs-pick-name">{t.label}</span>
            </span>
          </button>
        {/each}
      </div>
    </div>
  {/if}
</main>

<svg class="cs-defs" width="0" height="0" aria-hidden="true">
  <!--
    石墨颗粒 + 边缘位移。与 `routes/studio/` 里那份**逐字相同**——
    两处各写一份的话，改一边另一边就会悄悄退回单层平涂。

    ⚠ `color-interpolation-filters="sRGB"` 不能省：SVG 滤镜默认走
    linearRGB，`feComposite operator="in"` 的覆盖率会被当成亮度重新
    映射，叠出来整体偏淡——省掉的结果是「像蒙了一层灰纱」。

    颗粒来自 `feTurbulence` 的 alpha：先把三通道求和映射成 alpha，再用
    `feComponentTransfer` 的 table 拉对比。线性映射出的 alpha 集中在
    0.6~1.0，只是**均匀变淡**，和平涂一个毛病；拉高对比后才出现 0.1
    附近的低谷，也就是笔尖「跳格」的断续。

    位移交给合成器，不产生二值掩码，所以不会出现 `SetWindowRgn` 那种
    硬阶梯。scale 2 ≈ 抖动 ±1px，再多就变成毛边而不是石墨了。
  -->
  <defs>
    <filter
      id="pencil"
      x="-30%"
      y="-30%"
      width="160%"
      height="160%"
      color-interpolation-filters="sRGB"
    >
      <feTurbulence type="fractalNoise" baseFrequency="0.62" numOctaves="4" seed="11" result="n" />
      <feColorMatrix
        in="n"
        type="matrix"
        result="g"
        values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0.72 0.72 0.72 0 -0.30"
      />
      <feComponentTransfer in="g" result="gc">
        <feFuncA type="table" tableValues="0 0.08 0.30 0.62 0.88 1" />
      </feComponentTransfer>
      <feComposite in="SourceGraphic" in2="gc" operator="in" result="t" />
      <feDisplacementMap in="t" in2="n" scale="2" xChannelSelector="R" yChannelSelector="G" />
    </filter>
  </defs>
</svg>

<style>
  /*  只用来挂 filter 定义，不占布局。 */
  .cs-defs {
    position: absolute;
  }

  .cs-sheet {
    /* topbar / 拖拽带的定位父级。没有这一条它会去找 body，
     *  而 body 的尺寸跟面板无关 —— 窗口控制就会飘到屏幕别处。 */
    position: relative;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    /* 纸由 SVG 自己画，边缘交给合成器抗锯齿。
     * 不用 SetWindowRgn：它是二值掩码，撕口会变成硬阶梯。 */
    background: url('/probe-sheet.svg') center / 100% 100% no-repeat;
    /* 撕口已含在 SVG 里，padding 只负责把内容推离撕口。
       撕口实测 12~27 源 px（6~13.5 CSS px，随 x 变，对 alpha 阈值不敏感），
       量法见 ui/scripts/_tearprobe.py 与 ui/scripts/sheetprobe.ps1。
       原注释写「17.5px」，对不上。
       ⚠⚠ **下边 30 → 20**（票 44，2026-10-16）。这是真撕边候选行的落脚点：
       *   量具实测 `auto` 档档位格底 554.1，而内容底 = 560 − padding-bottom
       *   原来 30 ⇒ 内容底 530，**超 24px**。
       *   ⇒ 收 10px。**取值依据**：纸底破口最深实测 13.5 CSS px，
       *     20px 里留了 6.5px 余量，而档位格自己还有 `padding-bottom: 8px`。
       *   ⚠ 上边 40 **不动**：它是 topbar（窗口键那条带）的高度，与撕边无关。
       * ⚠ 改这里会影响**所有**档的落脚点，改完必须重量具。 */
    padding: 40px 32px 20px;
    color: var(--cs-ink);
    overflow: hidden;
  }

  /*  设置态换掉纸底，**整张换掉**——不是叠一层白。
   *
   *  设计规范 §1：「纸面是配置界面唯一的敌人」——设置页用中性白，
   *  刻意不参与「纸 + 墨 + 红批注」那套叙事。而半透明的中间层在
   *  Tauri 的透明窗口里**整层不绘制**（app.css 记着这条实测），
   *  叠出来是「一层没画」，不是「一层淡」。
   *
   *  撕口随之消失、方角出现，这个跳变是有意的：材质切换本身就是
   *  「你现在不在抄写，在配置」这句提示，比再写一行字省事。
   *  底色取 `--cs-paper`，它就是 token 里标着「设置页底」的那一支。
   */
  .cs-sheet[data-settings='true'] {
    background: var(--cs-paper);
  }

  /* ── 高对比度：**撤掉纸纹**，面板整体退成系统底色 ───────────────
   *
   *  2026-10-14 实测（本机「高对比度 #1」深色）。问题不是配色，
   *  是**纸纹活下来了而文字底被刷掉了**：
   *
   *    纸是 `background-image` 的**位图**，forced-colors 不接管位图；
   *    而规范要求 UA 把每个元素的 `background-color` 强制成 `Canvas`。
   *    两条叠在一起，就成了**一块块深色矩形浮在奶油色纸上**——
   *    归属行、正文、两枚档位名各一个方框，图上看着像贴坏了的便签。
   *
   *  ⇒ 这不是「配色没调好」，是纸材质与 HC 互斥。
   *    HC 的诉求是最大对比度，纸纹在这里是装饰；
   *    撤掉它，面板就是一个干净的 `Canvas` + `CanvasText`，
   *    那些方框也就与底色同色、不再显形。
   *
   *  ⚠ 这条**不要**用 `forced-color-adjust: none` 去保纸。
   *    那是把用户的无障碍设置顶掉——正是 HC 要解决的那件事。
   */
  @media (forced-colors: active) {
    .cs-sheet {
      background-image: none;
    }
  }

  /* ── 拖拽带 + 窗口控制 ─────────────────────────────────────── */
  /*  高度必须等于 `padding-top` 的 40px：那条正是撕口让出来的，
   *  压低一点会盖住撕口的锯齿，压高一点就把正文区吃掉了。
   *
   *  ⚠⚠ **垂直位置是对着实测纸边定的，不是看着定的**（2026-10-13）。
   *  原来的 `align-items: center` 把 28×32 的盘子放在 y=4..36，字形 svg
   *  落在 y=10..30。而纸的顶边内缩是**随 x 变**的，实测（面板 CSS 空间，
   *  量法见 `ui/scripts/sheetprobe.ps1` 的「右上角」一段）恰好在按键所在
   *  的那一段最深：
   *      x=308 → 6.5    x=316 → 6.0    x=324 → 10.5   x=332 → 13.5
   *      x=340 → 13.0   x=348 → 11.5   x=356 → 10.0   x=372 → **25.0**
   *  `×` 的字形横跨 338~358，那一段顶边内缩 10.0~13.0 ——**笔画的顶部
   *  2~3px 骑在破口上**。而且 y=8 那一整行右边内缩 **59px**，右上角是全纸
   *  最脆的位置，三颗键正好停在那儿。这不是「看起来有点出格」，是量出来的。
   *
   *  ⇒ 改成 `align-items: flex-end`：盘子落 y=8..40，字形 svg y=14..34，
   *    整颗键离开顶边 14px，比 `×` 那一段最深内缩（13.0）多 1px 余量。
   *    盘子底边正好压在 40px 撕口带的下沿，归属行从 y=41 起，互不重叠。
   *
   *  ⚠ 量纸时注意**两套坐标空间**：`probe-sheet.svg` 声明的是 348 × 512，
   *    而 `.cs-sheet` 把它按 `100% 100%` 拉到 380 × 560，**撕口一起放大
   *    1.09 倍**。CSS 布局关心的是后者，所以这里引的数必须来自面板空间，
   *    不是 SVG 固有空间——两者的数差 1.09 倍。
   */
  .cs-topbar {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 40px;
    display: flex;
    align-items: flex-end;
    justify-content: flex-end;
    /* ⚠ 26px 不是拍的是对齐**实纸右边**：纸的右边内缩在按键那一段实测
     *  6~9px，原来的 18px 扣掉内缩只剩 9~12px 余量，看上去就贴边了。 */
    padding-right: 26px;
  }

  .cs-cpanel {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: none;
  }

  /*  三颗记号（设置 / 收起 / 退出）**同形**：同样的盘子、圆角、悬停底色。
   *  面板里于是只有一套记号的形态——旧面板那条纪律（「两颗键不能管同一件
   *  事」）的另一半是：一颗键不能有两种形状。
   *  ⚠ 记号本身已换成手绘 path，盘子尺寸照抄旧 `WinCtl`（已删）的 28×32
   *    是为了让点击区与旧面板一致，不是为了「复用组件」。 */
  .cs-glyph {
    display: grid;
    place-items: center;
    width: 28px;
    height: 32px;
    border-radius: var(--cs-radius-sm);
    /* ⚠ 2026-10-13 从 `--cs-mark` 换成 `--cs-pencil`。出图自查时这三颗
     *    在撕口上读不出来：`--cs-mark` 在纸底只有 3.00 : 1（正文用它不合规
     *    的那条理由，这里同样成立），而窗口控制是**界面骨架**，不是弱化的
     *    装饰。5.10 : 1 之后齿轮的齿和 × 的两笔都读得出来。 */
    color: var(--cs-pencil);
    transition:
      background var(--cs-dur-fast) var(--cs-ease),
      color var(--cs-dur-fast) var(--cs-ease);
  }

  .cs-glyph:hover {
    background: rgb(20 26 31 / 7%);
    color: var(--cs-ink);
  }

  /*  退出的红**只在悬停时出现**。
   *
   *  发送是「字已经出去了」，退出是「字不会再出去了」——两件都不可逆，
   *  同一个红。看到红就是「有东西出去过，或者要出去」。
   *
   *  静止时是墨色：一个替你说话的程序，它的退出键不该一直在喊。 */
  .cs-glyph-quit:hover {
    background: var(--cs-live);
    color: #fff;
  }

  /*  开着的时候是「关」而不是「再开一次」。
   *  旧面板在设置态把这颗换成一颗 `←`；这里保留记号加高亮，
   *  因为纸面板**没有头部可退**——退出设置靠再点一次。 */
  .cs-glyph.on {
    background: rgb(20 26 31 / 12%);
    color: var(--cs-ink);
  }

  /* ── 档位（不在顶部：与归属行同行的两枚小记号）─────────────
   *
   *  槽高 53 → 24px，取景框 60×40 → 34×22。
   *  ⚠ 下面那套「按笔心反算 top / margin-left」的公式**整条作废**了：
   *    它的前提是标记**盖在字上**，而现在标记在标签**左边**、独占一行。
   *    公式里那行「字心y = markbox 40 + 标签 2 + 字号 11 ÷ 2」在
   *    新布局里根本不存在——留着它就是在给一个没有对象的问题写答案。
   *    摆放方式改成：标记自带取景框，直接顶格对齐。
   */
  /*  档位自己就是正文列的最后一个 flex 项——归属行那套
   *  `space-between` 已经连同它的容器一起去掉了（挤断归属文字）。 */
  /* ── 动作行 ───────────────────────────────────────────────────────
   *
   * 位置：正文之下、档位之上。它是候选那一组的一部分——
   * 「看这句 → 按它」中间不插别的东西。
   *
   * ⚠⚠ **这一段的下面两条推翻了 2026-10-15 的结论**（票 44，2026-10-16）。
   *   2026-10-15 那版写的是「这里推翻了『按钮不描边框』，必须描边」，
   *   理由是 §4.2c 定了三个变体共用同一套几何、全是 1px 描边，
   *   而「框与字仍是墨色」里红只落在页边一道竖笔上。
   *
   *   **现在描边整个去掉了**，改成**从这张纸上撕下来的一片纸**：
   *     ① §4.2c 与 §4.2b 正面打架才是根因——前者定的是**短标签按钮族**
   *        （原话：「等宽等高是给 S5 那组单行短标签的」），
   *        而候选行承载的是**整句正文**。绕着 1px 边框调了三轮，
   *        调的是那条打架的规则，不是实现。
   *     ② 「共用几何」的前提在纸片上换了个满足方式：三片纸共用
   *        **同一张撕边位图**（`strip.png`），比共用一套边框参数更紧。
   *     ③ 「禁用换线型」在纸片上不成立（`border-image` 一生效，
   *        `border-style` 就画不出来）⇒ 改成收掉投影，
   *        读成「一片没有落在纸上的纸」，见 `.cs-act:disabled`。
   *   竖笔那条通道本身**没有被推翻**，它换了个载体：从**页边一道 3px 竖笔**
   *   变成**纸上的一枚手绘勾**，语义逐条对应。
   */
  .cs-actions {
    flex: none;
    display: flex;
    flex-direction: column;
    /* ⚠⚠ **6 → 10 → 6 → 0**（票 44，2026-10-16 真撕边后归零）。
     *
     *   为什么能归零，而 2026-10-16 上午那版不敢：
     *   那一版的边是**画出来的 1px 线**——线之外什么都没有，
     *   所以必须靠 gap 把相邻两条分开，否则三条粘成一块。
     *   而现在这三条边是**面板自己的撕口**：
     *     撕口外侧本来就是**完全透明**的（纸没到那儿，实测 alpha 0），
     *     内侧 6~13 CSS px 是纸芯的渐变。
     *   ⇒ **透明部分天然就是间隙。** 再叠一层 CSS gap 等于在两条纸
     *     之间又留一道白，那道白与撕口内侧的留白叠起来，
     *     读出来是「两片纸之间空了一块」，而不是「从一整张上撕下来的」。
     *
     *   ⚠ 这是「真撕边」相对「画一条线」的**第二个**收益，
     *     第一个是深度与 alpha 过渡（见 `.cs-act` 上面那段实测数据）。
     */
    gap: 0;
    /* ⚠ padding 也归零，理由同上：`10/12` 是给「1px 描边按钮」留的，
     *   而撕边那 16px 里已经有纸芯的白，再加 22px 就是三重留白。
     *   ⚠ 归零后**相邻两条纸的撕口会贴在一起**，那正是「从同一张上
     *   连续撕下来」的读法：两片之间只有一道撕开的缝，没有额外的沟。
     *   撕口最深处实测 27 源 px，slice 取 32 ⇒ 留 5 源 px 余量，
     *   相邻两片**不会**互相侵入对方的纸芯。 */
    padding: 0;
  }

  /* 不可逆前置说明（规范 §4.2b：它在按钮上方）。
   * ⚠ 它在 forced-colors 下是**唯一**还能说出「这一组按下去不可逆」
   *   的通道——高对比度接管背景色之后，红批那道竖笔会退化成墨线。 */
  .cs-act-warn {
    margin: 0 0 2px;
    font-size: 12px;
    line-height: 1.4;
    /* 红只给不可逆，而这一句就是不可逆本身。 */
    color: var(--cs-pencil-red);
  }

  /* ── 候选行 = 一片从这张纸上撕下来的纸（票 44）────────────────────
   *
   * ⚠⚠ **撕边必须是面板自己的那一种，不能是描边。**
   *   2026-10-16 第一版把撕口画成「一条 1px 深的密度线」，
   *   用户当场否掉：「边缘得是和面板的那种一致啊？你这样和黑框有什么区别？」
   *
   *   「黑框」不是修辞，是那条线的实际读数。`ui/scripts/_tearprobe.py`
   *   实测了面板自己的撕边（落在 `target/probe/tearprobe.txt`）：
   *
   *     内缩深度   12~27 源 px（6~13.5 CSS px），众数 17~25
   *     alpha 剖面 0 → 3 → 53~111 → 203~248 → 254   ← **两级过渡**
   *     外侧       完全透明（纸没到那儿）
   *
   *   而 1px 那条线：深度 0.5 CSS px、alpha 一刀切到满、纸直接顶到边。
   *   ⇒ 真撕口的可辨识度**几乎全部来自那 6~13 CSS px 的深度与两级过渡**。
   *   压成 1px 之后剩下的就是「一圈均匀的深线」——那正是黑框的构造方式。
   *
   *   所以 `strip.png` 由 `ui/scripts/stripgen.py` **从面板位图裁出真边**，
   *   不合成一个像素。一致性不靠调参数维持，靠同源。
   *
   * 九宫格：`slice 32 fill / 16px / 0 stretch`，与 `stripgen.py` 的
   * `SLICE = 32` 逐字绑定（源图 @2x，32 源 px = 16 CSS px）。
   * `fill` 让中间那块纸芯铺满，`stretch` 让它随内容长度横向拉伸，
   * 四条撕边保持不动 ⇒ 1 行 / 2 行 / 3 行都是**同一块纸**。
   *
   * 为什么纵向 stretch 也不出问题：上下两条边的高度由 `border-width` 定死
   * （16px，不拉伸），被纵向拉的是中间那块**均匀纸芯**；纸纹是细颗粒，
   *   纵向拉不出可见的条纹（横向拉才会，理由见 `stripgen.py`）。
   *
   * ⚠⚠ **slice 必须 ≥ 撕边最深处的内缩**（实测 27 源 px），否则渐变会被
   *   九宫格截断，接缝处露出**半透明的硬边**——那比黑框更糟，
   *   因为它看起来像一道折痕。`stripgen.py` 的 `build()` 有断言守这条。
   *
   * ⚠⚠⚠ **上一轮这段注释记的「余量只有 4px」是错的，而且量错了对象。**
   *
   *   它把 `.cs-sheet` 当成「上下 padding 40+30 ⇒ 内容框底在 y=530」，
   *   而 `.cs-sheet` 实际是 **0 → 560 的整张纸**：`padding` 是**内容内缩**，
   *   不是外框；`overflow: hidden` 裁的是 560，不是 530。
   *   判据：量具量到的 `.cs-sheet` rect 就是 `top 0 / bottom 560 / h 560`。
   *
   *   ⇒ 真正的余量必须拿 `.cs-pick` 的 rect 对 **560** 看
   *   （2026-10-16 量具 `ui/static/budget.html`，三档 iframe 逐个量）：
   *
   *     档位      `.cs-pick` 顶/底      档位格底     距纸底 560
   *     drafted   434.3 / 531.8        505.8        54px
   *     auto      459.0 / 556.6        530.6        29px   ← 最紧
   *
   *   `auto` 是最紧的一档（多了那行红字警告，起点 +25px）。
   *   一条纸片上下各一条撕边 ⇒ 每条候选多占 `2 × 16 = 32px`，三条 **+96px**。
   *   ⇒ 29px 装不下 96px，**差 67px**，必须从别处让出来。
   *
   * ⚠ **腾空间的顺序是硬规定：先减间隙、再减正文行数，不许回头砍撕边。**
   *   砍到 3px 就退回了第一版那个「用户明说读成黑框」的东西。
   *   改这里之前**必须回去重量具**，两个档都量，别靠出图目测。
   */
  .cs-act {
    position: relative;
    display: flex;
    align-items: baseline;
    gap: 6px;
    appearance: none;
    /* ⚠⚠⚠ **`margin: -8px` —— 撕边最外沿本来就该溢出布局框**（票 44 最终解法）。
     *
     * ## 为什么这是必须的，而不是微调
     *
     * 面板高 560，`.cs-sheet` 的 `padding-bottom: 30` 意味着**内容底在 y=530**
     * （`padding` 是内容内缩；`overflow: hidden` 裁的是 560，
     *   但内容不该越过 530 —— 那是留给撕口与投影的）。
     * 量具实测（`ui/static/budget.html`，`auto` 档）：
     *   基线 1px 描边按钮   档位格顶 459.0 / 底 530.6  ⇒ 余量 **0.6px**
     *   真撕边 16px（三条） 档位格顶 530.5 / 底 602.1  ⇒ **超 42px**
     *
     * 三条纸的撕边共 `6 × 16 = 96px`，而可用只有 0.6px。
     * ⇒ **在正常流里，16px 的撕边永远放不下。**
     *   上午那版把撕边压到 3px 正是为了塞进去，
     *   而用户明说那读成黑框 —— **两条路都不通，问题不在厚度上。**
     *
     * ## 关键观察：撕边最外侧**本来就是透明的**
     *
     * 实测面板自己的撕口 alpha 剖面（`_tearprobe.py`）：
     *     0 → 3 → 53~111 → 203~248 → 254
     * 最外面约 **3~6 源 px（1.5~3 CSS px）是全透明**，
     * 再往里才开始爬升；而内缩众数是 17~25 源 px（8.5~12.5 CSS px）。
     *
     * ⇒ 那最外侧的透明带**画出来什么也没有**，不承载任何视觉信息，
     *   却实实在在吃掉布局高度。**让它溢出即可**：
     *   `margin: -8px` 把纸片整体上提/下压 8px，
     *   透明的部分跑到布局框外。相邻两片各退 8px，
     *   它们的透明外沿互相**填进对方的位置**，
     *   而实心纸仍各自完整 —— 视觉上就是三条纸紧挨着撕。
     *
     * ⚠ **只有上下能给负边距。** 左右没有「让给纸边」的场景，
     *   纸片的左沿就是它的起点。
     *
     * ⚠ **`-8px` 的取值依据**：撕边 16px 里，实心纸约 5~8px
     *   （外沿全透明 1.5~3px + 爬升段 4~6px），
     *   退 8px 之后仍有 8px 纸芯在框内，够放 12.5px 的正文（实测正文高 37.5~56.3）。
     *   **再往大退就会让正文骑到渐变上** —— 那时的读法是「字贴在撕边上」，
     *   纸片就没有厚度了。若日后还要腾空间，**先动 `.cs-sheet` 的 padding-bottom**，
     *   别再加大这个负值。
     *
     * ⚠ **改这个值之前必须回去重量具**（`ui/static/budget.html`），
     *   两个档都量。上面每个数字都是量出来的，不是推演。
     */
    margin: -8px 0;
    /* ⚠ 内边距的量由**撕口内侧那段纸芯白**决定，不由「够不够放字」决定。
     *   撕口从外沿 0 爬到实心要 6~13 CSS px（实测），那 16px 的 border 里
     *   已经含有约 4~8px 的白纸。padding 再给 1px，字就骑在渐变的中段上，
     *   读出来是「字贴在撕口上」——纸片没有厚度。
     *   ⇒ 上下 3px：让字落在**实心纸**上，留一层纸的内缩边。
     *
     * ⚠⚠ **左边只给 10px，勾靠在绝对定位上（2026-10-16 第二次改）**。
     *   中间试过 `padding-left: 30px`（「给勾让出纸的白」），
     *   实测**正文宽从 237.3px 掉到 213.3px** ——
     *   勾改成 `position: absolute` 之后它**根本不占横向空间**，
     *   而 padding 还要为它加宽，正文反而更窄。
     *   首选正文于是 75px = **4 行**，比备选多 37.5px。
     *   ⇒ 让位的方式必须是「勾探进已有留白」，不是「给它开一格」。
     *   10px 与右边对称：三条纸的正文左缘对齐（它们是一组），
     *   勾在那 10px + 撕边纸白里往左探一点。 */
    padding: 3px 10px;
    border-style: solid;
    border-width: 16px;
    border-image: url('/strip.png') 32 fill / 16px / 0 stretch;
    color: var(--cs-ink);
    font: 600 13px/1.5 inherit;
    text-align: left;
    cursor: pointer;
    /* 常态：一片安静躺着的纸。
     *
     * ⚠⚠ **两层影决定的是「它有没有落在纸上」，不是装饰。**
     *   真撕边（16px）已经把「框」和「撕」分开了，而这两层影说的是另一件事：
     *     第一层 1px 的近影 = 纸边贴着桌面的暗线（决定「它在上层」）
     *     第二层 6px 的远影 = 纸片离桌面的高度（决定「它能拿起来」）
     *   去掉任何一层，出图立刻退回「一个贴上去的矩形」。 */
    box-shadow:
      0 1px 1px rgb(24 18 10 / 0.22),
      0 3px 7px rgb(24 18 10 / 0.12);
    transition:
      transform var(--cs-dur) var(--cs-ease),
      box-shadow var(--cs-dur) var(--cs-ease);
  }

  /* ── 悬停 / 聚焦：**把它拿起来** ────────────────────────────────
   *
   * ⚠ 位移只有 2px、旋转 0.5°：纸张很轻，抬得太高读成「浮起来的气泡」。
   *   抬起的作用只是**让手知道它可以被拿走**。
   *
   * 悬停与 `:focus-visible` 用**同一条规则**：键盘 Tab 到这一条时它也该被拿起来，
   * 否则鼠标用户知道「能点」而键盘用户只能靠焦点环猜——两条路径给出同一个反馈，
   * 才算「同一个东西的两种到达方式」，而不是两个控件。 */
  .cs-act:hover,
  .cs-act:focus-visible {
    transform: translateY(-2px) rotate(-0.5deg);
    box-shadow:
      0 2px 3px rgb(24 18 10 / 0.24),
      0 8px 16px rgb(24 18 10 / 0.18);
  }

  /* ── 按下：**压回纸面** ────────────────────────────────────────
   *
   * ⚠ 抬起与按下必须是**一对**：只做抬起的话，按下去纸条不动，
   *   反馈就只剩「变了个色」——而这片纸不会变色，它本来就是纸。
   *   压回去（1px + 影子收掉）读成「手指把它按在桌上」，
   *   是纸张唯一有的那一种「被操作了」。
   *
   * `:active` 抓的是**真实鼠标按住的那一瞬**，比 `:hover` 更窄，
   *   所以两者的位移量必须分开写：悬停 -2px，按下 +1px。 */
  .cs-act:active {
    transform: translateY(1px) rotate(0deg);
    box-shadow: 0 1px 1px rgb(24 18 10 / 0.24);
    transition-duration: var(--cs-dur-fast);
  }

  /* ⚠ 焦点环用墨不用红（规范 §5.4）。
   *   红是「不可逆」，而焦点环会出现在**每一个**候选上——
   *   用红就把不可逆说成了常态，跟红铅笔勾撞含义。
   *   ⚠ `outline-offset: -2px`：环画在这片纸自己的撕边之外，
   *   环与下面那张纸脱开，
   *   读成「另一个东西的一圈边」而不是「这一片被圈住了」。 */
  .cs-act:focus-visible {
    outline: 2px solid var(--cs-ink);
    outline-offset: -2px;
  }

  /* ── 禁用：**不降透明度**（规范 §4.2c，2026-10-14 `ui/scripts/disabledsep.py` 实测推翻 opacity）
   *
   * `opacity` 稀释的是整个元素——文字、边框、底色一起淡。
   * `.cs-btn-mini` 的边框正常态就只有 1.57 : 1，
   * `opacity: .34` 把它压到 **1.16 : 1**，
   * 而「禁用」恰恰是最该一眼看出来的状态。
   *
   * ⚠ **纸片没有线型可换。** 旧版靠 `border-style: dashed` 表达禁用，
   *   而撕边是位图，`border-style` 对它无效——`border-image` 一旦生效，
   *   `dashed` 就画不出来了。
   * ⇒ 改用**投影收掉**：一张**没有落在纸上**的纸片。
   *   它仍然看得见（撕边还在，文字还是墨色，对比度一分没掉），
   *   但它不再「压着」下面那张纸——这就是纸上语言里离「禁用」最近的东西。 */
  .cs-act:disabled {
    cursor: default;
    transform: none;
    box-shadow: none;
  }

  /* ── 变体一：备选。无记号——可逆、且不是推荐。 ─────────────── */
  .cs-act-alt {
    background: transparent;
  }

  /* ── 首选：墨色手绘勾。划重点。 ────────────────────────────────
   *
   * ⚠ 颜色是 `color`，勾是 `fill: currentColor`——所以记号与按钮共用一支色，
   *   而红的分档只需要改这一个 token。
   *
   * ⚠ **这里原先是「实墨」（墨底 + 纸色字），2026-10-15 拿掉。**
   *   为什么不用实墨：那一档出自 §4.2c 里「S5 的先不发 / 发送」那组
   *   **双按钮**语境，而 §4.2b v2 早就否决过同一个东西——
   *   「按钮不能拿现成色块当零件」，纯墨底是 Web 默认按钮语言。
   *   §4.2c 用「每屏最多一颗」缓解了色块互相压，但**一颗纯黑长条压在
   *   撕边纸上仍然比整张纸任何东西都重**（2026-10-15 出图确认），
   *   而它代表的只是一次「填进输入框」的建议。量级和风险严重不匹配。
   *   ⇒ 2026-10-16 票 44 改成撕边纸 + 手绘勾，与旧版的竖笔同一条通道。 */
  .cs-act-lead {
    background: transparent;
    color: var(--cs-ink);
  }

  /* ── `Auto` 档：红铅笔勾。撤不回来。 ────────────────────────────
   *
   * ⚠⚠ **红只给那枚勾，不再给整条按钮的 `color`**（2026-10-16）。
   *   旧写法是 `color: var(--cs-pencil-red)`，靠勾的 `fill: currentColor`
   *   跟着变红。**那时成立是因为首选不显示正文**——整条里除了勾就是空的，
   *   改 `color` 影响不到别的。
   *   而现在首选**也显示正文**了，同一写法会把**整段正文染红**。
   *   ⇒ 红铅笔在纸上就是「一道批」，不是「一整行字都是红的」。
   *     红遍整行的读法是「这条正文危险」，而危险属于**这一档**，
   *     不属于某一句话；真正说不可逆的通道是组级那行红字 + 纸底的大红勾。
   *
   * ⚠ 用 `--cs-pencil-red`（红铅笔，蜡质发闷）而不是 `--cs-live`（警报红）：
   *   这一族变体就叫「红批」，而红批在实物上就是一支红铅笔。
   * ⚠ `Auto` 档的正文走**墨色**，理由与 `.cs-act-lead .cs-act-text` 相同：
   *   它同样是被勾中的那一条。 */
  .cs-act-red {
    background: transparent;
    color: var(--cs-ink);
  }

  /* 红只落在记号槽的勾上。⚠ 用 `--cs-pencil-red` 而非 `--cs-live` 的理由同上。 */
  .cs-act-red .cs-act-mark {
    color: var(--cs-pencil-red);
  }

  /* 记号槽：⚠ **绝对定位**（票 44，2026-10-16 实测后改）。
   *
   * 它先是 flex 项，于是占 20px + `gap: 6px` = **26px 横向**，
   * 把首选正文压窄 26px。实测后果：
   *   首选正文 h = **75px = 4 行**（12.5px × 1.5 = 18.75/行）
   * 而两条备选都是 37.5px = 2 行。
   * ⇒ **首选这一条比备选高 37.5px**，纯粹是那枚勾买来的。
   *   而纵向已经见底（`auto` 档档位格顶被顶到 568，纸底 560）。
   *
   * ⚠ 为什么可以拿走这 26px：勾不是内容，是**批注**。
   *   批注压在句子上（`.cs-pick` 档位那行就是这个做法：取景框在上、
   *   标记从框里压到名上），而**压在句子上不需要给它留一格**。
   *   它坐在撕口内侧的纸白里往左探——那 6~13 CSS px 的纸白
   *   本来就是撕口渐变出来的（实测），**不是额外让出来的空间**。
   *
   * ⚠ `left: -6px`（负值）：勾往撕边里探一点。
   *   探出去的部分落在 border 区域，而 border 里画的是撕口——
   *   墨笔压在撕口的渐变上，正是「这一句被批了」的手绘读法。
   *   若探得更多会盖住撕口的参差轮廓，那才毁，-6 是量过的安全值。
   */
  .cs-act-mark {
    position: absolute;
    /* ⚠ 绝对定位的坐标相对 `.cs-act` 的 **padding box**，
     *   而 padding box 的顶就是 border 的内沿 —— 也就是**实心纸**的顶。 */
    left: -6px;
    top: 3px;
    flex: none;
    align-self: flex-start;
    /* 勾落在**首行的 x 高度**上，不是浮在条顶——
     * 批注压在句子上才像「这一句被批了」。 */
    pointer-events: none;
  }

  .cs-pencil-mark {
    display: block;
    color: inherit;
    fill: currentColor;
  }

  /* ⚠ **`.cs-act-tag`（「按这个 / 备选 N」）整条删掉了**（2026-10-16）。
   *   用户原话：「开头有什么必要加『备选X』的字？」
   *   理由与实测数据记在模板里那段 `{#if i === 0}` 之后的新注释里。
   *   核心三条：① 它占 37.9px + 6px gap = 44px 横向，
   *   而横向宽度直接决定备选折几行 ⇒ **横向就是纵向**；
   *   ② 首选/备选这件事已被手绘勾与 `aria-label` 说过两遍；
   *   ③ 编号暗示「顺序有讲究」，而用户挑的是「哪句话能直接发」。 */

  /* ⚠ **旧版的 `border-style: dashed` 禁用态整条删掉了**（票 44）。
   *   `border-image` 一旦生效，`dashed` 画不出来——纸片的边是位图，
   *   不是一条可以换线型的线。新的禁用表达在上面的 `.cs-act:disabled`：
   *   收掉投影，读成「一片没有落在纸上的纸」。 */

  /* 候选正文：颜色分工上备选是「机器认出来的」（规范 §4.2b），
   * 所以走铅笔灰而不是墨色。
   *
   * ⚠ 2026-10-16：首选也开始显示正文（原先只有备选显示），
   * 而它是被勾中的那一条 ⇒ 正文改走**墨色**。
   * 墨色 = 你写下的 / 铅笔灰 = 机器认出来的，这句分工原本就写着，
   * 只是原先首选不显示正文，那条分工在候选区里根本没体现过。
   * 现在它体现出来了：**勾 + 墨色正文**是首选，
   * **无勾 + 铅笔灰正文**是备选。两条规则同源，不是两套。 */
  .cs-act-text {
    font-size: 12.5px;
    font-weight: 400;
    line-height: 1.5;
    color: var(--cs-pencil);
  }

  /* 首选正文转墨色。⚠ 只改 `color`，不改 `border-image`：
   *   勾用 `fill: currentColor`（见 `.cs-pencil-mark`），
   *   所以「墨色 / 红铅笔」两档的记号差异由这一处颜色连带给出，
   *   不必再给记号单独设一次色 —— 一处说完，`.cs-act-red` 只改一个 token。 */
  .cs-act-lead .cs-act-text {
    color: var(--cs-ink);
  }

  /* 概率：等宽数字，省得两位数跳来跳去。 */
  .cs-act-prob {
    font-size: 12px;
    color: var(--cs-ink);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
    margin-left: auto;
  }

  /* 回执。成功走墨色、失败走红——红留给「没发成」这个必须看见的结果。 */
  .cs-act-note {
    margin: 2px 0 0;
    font-size: 12px;
    line-height: 1.4;
    color: var(--cs-ink);
  }
  .cs-act-note-bad {
    color: var(--cs-pencil-red);
  }

  .cs-pick {
    flex: none;
    display: flex;
    /* ⚠ 必须是 0：两档各占半栏（`.cs-pick-slot { flex: 1 }`），靠槽自己分栏，
     *  加 gap 只会让两档不再对称，而对称是「哪一档被勾了」能被一眼比出来的前提。 */
    gap: 0;
    /* 推到纸底。依赖 `.cs-body-col { flex: 1 }` 把这列撑满，
       否则 `auto` 无处可推，档位会贴回正文下方。
       ⚠ `padding-bottom` 不是装饰，按**标记下垂段**算不按文字算：
         档位格高 = markbox 40 + margin 2 + 11px 字 = 53，但 `big` 档勾的
         取景框 56×34、top 29，底边在 29 + 34 = 63——比槽底**低 10px**。
         纸底破口实测 7~11px，所以留 26px。给文字留的那点不够：
         实测出图里「只填入」骑在破口上。
       ⚠⚠ **26 → 8**（票 44，2026-10-16 真撕边后）。
         候选三条的撕边在正常流里要 96px，而 `auto` 档可用只有 0.6px，
         负边距 `-8px` 收回 48px（首尾各半）之后**仍差 24px**
         （量具实测档位格底 554.1，内容底 530）。
         而这 26px 里真正需要的是「档位格底离纸边别太近」：
           纸底破口实测 6~13.5 CSS px ⇒ 留 8px 已经够。
         26px 里有 18px 是按「没有撕边候选」的老预算留的。
         ⚠ 改这个数**会**让档位格更靠近纸边，所以改完必须重量具。 */
    margin-top: auto;
    padding-bottom: 8px;
  }

  .cs-pick-slot {
    flex: 1;
    display: flex;
    justify-content: center;
    /* 按钮化之后默认的 UA 样式必须全部抹掉，否则会多出一圈边框，
       而边框在撕边纸上等于第二个边界。 */
    appearance: none;
    margin: 0;
    padding: 0;
    border: 0;
    background: none;
    font: inherit;
    color: inherit;
    cursor: pointer;
  }

  /*  每档一格：**取景框在上、三字档位名在下**，标记从框里压到名上。
   *  这就是「底部怎么布局」的全部——与它在顶部时的形态**一模一样**，
   *  位置不同而已。2026-10-13 有一轮把标记改成字左边的小记号，
   *  那是把「位置」和「形态」两件事混成了一件，已恢复。 */
  .cs-slot-mark {
    display: flex;
    flex-direction: column;
    align-items: center;
  }

  /*  取景框只负责给标记一个固定高度，好让三行字对齐。它**不画任何东西**。
   *  ⚠ 60×40 是定案值。2026-10-13 有一轮缩到 24×16 当「行内小记号」用，
   *    连带动把幅面默认档从 `big` 降成 `base`——**两个定案被一个自己造的
   *    约束串起来推翻**，而那个约束本身就是「档位不占一整行」的副产物。
   *    档位挪到纸底之后每档有自己的高度，60×40 放得下，无需缩小。 */
  .cs-markbox {
    position: relative;
    display: block;
    width: calc(60px * var(--tk, 1));
    height: calc(40px * var(--tk, 1));
  }

  /*  标记。红铅笔，蜡质，每层的透明度是**笔迹数据的一部分**，
   *  由 `$lib/marks` 的生成器算出来。CSS 一旦兜底就会把嵌套的五级
   *  全部覆盖掉，羽化立刻塌回单层平涂——材质台 P1 与 P5 同框实测：
   *  落在中间密度带的像素 23.0% → 70.5%，总墨量反而少 17%。
   *  密度从芯往外逐级下降，才有「芯重边虚」。
   *
   *  **盖章式：笔盖在字上，不是悬在字上方。** 用户 2026-10-13 用手绘参考
   *  点破了这一点——标记靠**交叠**表达「这一档被批了」，躲开字反而变成
   *  一个跟内容无关的浮标。
   *
   *  ⚠ 盖住字**不会**读成删除线：删除线是一条横线穿过字中部，圈是包围，
   *    拓扑不同。2026-10-13 曾把「标记与字交叠」当成缺陷判掉，那是错判；
   *    2026-10-13 晚些时候又把这个定案本身推翻了（改成字左小记号），已恢复。
   *
   *  摆放偏移**全部按笔心反算**，不靠观感试：
   *      top         = 字心y - 1.1 - 笔心y
   *      margin-left = -笔心x
   *  字心y = markbox 40 + 标签 margin 2 + 字号 11 ÷ 2 = 47.5；-1.1 是现状圈
   *  对齐字心时本来就带着的那点偏上。笔心 = `marks.js` 里 path 的 bbox 中心。
   *
   *  ⚠ 这套公式替换掉过一个**试出来的**值：勾原来是 `margin-left:-21px`，
   *    注释写着「要右移 9px 才盖得住三个字」，但公式算出该是 -27——
   *    旧值把笔心推到了槽心**右侧 6px**。标一个东西却没对准它。
   *
   *  ⚠⚠ **原先这里写着「尺寸由 SVG 的 `width/height` 属性给（= `ACTIVE_VIEW`），
   *    不是 CSS」——那句话是错的，2026-10-15 真机拆穿。**
   *    下一行的 CSS `width: calc(var(--tick-w, 60px) * var(--tk, 1))` 直接压过
   *    SVG 属性，而 `--tick-w` 当时只对 `tick` 两支赋过值 ⇒「圈 + big」
   *    （`RING_VIEW` 84×52）被 fallback 的 60×40 压扁并偏左 13px。
   *    ⇒ 尺寸**确实由 CSS 给**，所以每一支都必须显式写 `--tick-w/--tick-h`，
   *    见下面那三支。写「不是 CSS」等于给下一个人留一个假前提。
   *  交叉验证：公式代回状态圈算出 (-30.7, 30.0)，与已上线的 -30 / 30 吻合。
   */
  .cs-tick {
    position: absolute;
    left: 50%;
    top: calc(26px * var(--tk, 1));
    margin-left: calc(-27px * var(--tk, 1));
    /* ⚠ 这里用 CSS 定尺寸是**有意的**：整格放大时笔必须跟着放大，
     *   而 SVG 的 `width`/`height` 属性写死成 `ACTIVE_VIEW` 的取景框。
     *   早先踩过的那个坑是 `width: 100%`——那会把笔**压回**框宽，
     *   症状是「选了大档但形状没变」。这里给的是显式的等比 px，不是百分比。 */
    width: calc(var(--tick-w, 60px) * var(--tk, 1));
    height: calc(var(--tick-h, 40px) * var(--tk, 1));
    opacity: 0;
    transition: opacity var(--cs-dur) var(--cs-ease);
  }

  .cs-tick[data-shape='ring'] {
    top: calc(30px * var(--tk, 1));
    margin-left: calc(-31px * var(--tk, 1));
  }

  /* ⚠ **每一支都必须显式给 `--tick-w` / `--tick-h`，不许落 fallback。**
   *
   * ⚠⚠ 2026-10-15 真机踩到：`.cs-tick` 的 CSS `width/height` **压过** SVG 的
   *   `width/height` 属性。而 `--tick-w` 的 fallback 是 `60px`，且原先**只有**
   *   `[data-shape='tick'][data-ring='mid'|'big']` 那两支给它赋过值。
   *   ⇒ 「圈 + big」这一支（`ACTIVE_VIEW = RING_VIEW = 84×52`）落进 fallback
   *   60×40，被硬压扁；更糟的是偏移 `margin-left:-43px` 是按 **84 宽**的
   *   笔心 x≈42 反算的，渲染宽只有 60 ⇒ **偏左约 13px**，
   *   垂直同理偏上约 6px。
   *   症状是「圈很扁 + 明显错位」，而门禁与测试全绿。
   *
   * ⚠ 本文件上面那段注释曾写着「尺寸由 SVG 的 `width/height` 属性给，
   *   **不是** CSS」——**那是错的**，CSS 就在下一行把它盖掉了。
   *   注释与代码矛盾、而真机表现跟着代码走的那种，必须改注释而不是改心。
   *
   * 下面的三支与 `marks.js` 的 `ACTIVE_VIEW` 逐支对应（1:1，不许有第四支漏掉）：
   *   base / mid 两支两形态都 = `MARK_VIEW` 60×40
   *   tick + mid/big           = `TICK_VIEW` 56×34
   *   ring + big               = `RING_VIEW` 84×52
   */
  .cs-tick[data-shape='ring'][data-ring='mid'],
  .cs-tick[data-shape='ring'][data-ring='base'],
  .cs-tick[data-shape='tick'][data-ring='base'] {
    --tick-w: 60px;
    --tick-h: 40px;
  }

  .cs-tick[data-shape='tick'][data-ring='mid'],
  .cs-tick[data-shape='tick'][data-ring='big'] {
    --tick-w: 56px;
    --tick-h: 34px;
    top: calc(29px * var(--tk, 1));
    margin-left: calc(-28px * var(--tk, 1));
  }

  .cs-tick[data-shape='ring'][data-ring='big'] {
    /* 84 宽 ⇒ 笔心 x = 42。原先写 -43 是按 84 估的，渲染宽 60 时偏左 13px。 */
    --tick-w: 84px;
    --tick-h: 52px;
    /* 字心 y 47.5 − 笔心 y 26 = 21.5。渲染宽高都对了之后这条自然对齐。 */
    top: calc(21px * var(--tk, 1));
    margin-left: calc(-42px * var(--tk, 1));
  }

  /*  只有 fill，不设 fill-opacity——每层的透明度是**笔迹数据的一部分**，
   *  由生成器算出来。CSS 一旦兜底就会把嵌套的五级全覆盖掉，
   *  羽化立刻塌回单层平涂。 */
  .cs-tick path {
    fill: var(--cs-pencil-red);
  }

  /*  ⚠ `margin-top: 2px` 是算式的一部分，不是间距微调：笔心反算公式里的
   *  「字心y = markbox 40 + 标签 margin 2 + 字号 11 ÷ 2 = 47.5」这一项
   *  就是它。改成 0 或 4，标记会整体偏上/偏下 2px ——在 1× 上看不出来，
   *  但「标一个东西却没对准它」正是当初 `-21px` 被推翻的那类错。 */
  .cs-pick-name {
    margin-top: calc(2px * var(--tk, 1));
    font-size: calc(11px * var(--tk, 1));
    line-height: 1;
    letter-spacing: 0.02em;
    color: var(--cs-pencil);
    /* ⚠ **600，不是 400**（2026-10-14，票 24 实测定案，见设计规范 §4.3）。
     *
     * 11px 的常规字重雅黑，**77.2% 的笔画细到 ≤1px** ——断笔不是「看着挤」，
     * 是「读不出来」。同样 11px 改成 600，细笔画掉到 **17.8%**。
     * 而把字号从 11 抬到 12 只买来约 7 个百分点。**杠杆在字重，不在字号。**
     *
     * ⚠⚠ **不要写 500**：本机雅黑没有 500 字重，浏览器渲成 400，
     *   实测两档数据**逐字节相同**（细笔画 79.6% / 77.2%）——写 500 等于没写。
     *
     * 未选中的那档退半档，让标记是唯一的高对比处。
     * 但 0.62 混的是透明度，落在纸上的实际对比度仍要够 ——
     * `--cs-pencil` 是 5.10 : 1，半透明后仍高于 AA 正文 4.5。 */
    font-weight: 600;
    opacity: 0.62;
    transition: opacity var(--cs-dur) var(--cs-ease);
  }

  .cs-pick-slot[aria-checked='true'] .cs-tick {
    opacity: 1;
  }
  .cs-pick-slot[aria-checked='true'] .cs-pick-name {
    opacity: 1;
  }

  /* ── 归属行 + 正文行 ──────────────────────────────────────────
   *
   *  内容栏 316px。写成 `max-width` 而不是 `width`，316 才 grep 得到——
   * 排版的第一变量是栏宽，栏宽错了后面量出的每个数字都错。
   */
  .cs-body-col {
    max-width: 316px;
    /* 撑满纸张的可用高度，档位那一行才能被 `margin-top: auto` 推到底。
       没有它，整列只有内容那么高，档位会紧贴正文、而下面留一大片空白
       ——那既没占住「纸底」，又让版面看着像没排完。 */
    flex: 1;
    margin: 0 auto;
    padding: 0 0 24px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }

  .cs-where {
    margin: 0;
    font-size: 12px;
    line-height: 1.45;
    color: var(--cs-pencil);
  }

  .cs-peer {
    /* 机器认出来的名字。**铅笔灰**，跟正文那支墨笔分开——
       墨 = 你写下的，铅笔 = 机器认出来的（app.css 的分工）。 */
    font-weight: 600;
    color: var(--cs-pencil-2);
  }

  /*  认不出会话名时的占位（票 30）。
   *
   *  用 `--cs-ink` 而不是 `--cs-mark`：标注档只有 3.00 : 1，够不上 12px
   *  正文（app.css 里写明了它只配给 ≥18.66px 粗体）。这里要的是一个
   *  「读得清、且一眼区别于正常会话名」的状态——18.05 : 1 的墨色加粗够。
   *  ⚠ 刻意**不用红**：红在本项目只给不可逆，而「认不出」是可恢复的提醒。
   *  HC 下 `--cs-ink` 已被接管成 `CanvasText`，自动跟着走。
   *
   * ⚠ **要带上 `letter-spacing: 0.18em`**（2026-10-14 八态图看出来）：
   * 正常会话名 `.cs-peer` 在默认的 `tracked` 下靠**疏排 + 加粗**读成标题，
   * 而这一类是独立选择器、不吃 `tracked` 那条规则的 `letter-spacing`。
   * 漏了它 ⇒ 认不出那一态在版式上比正常态**少一个层级手段**，两张图并排
   * 一眼就能看出疏排没有。层级既然不再靠字重，疏排就得两边都在。 */
  .cs-peer-unknown {
    font-weight: 600;
    letter-spacing: 0.18em;
    color: var(--cs-ink);
  }

  .cs-where-quote {
    display: block;
    margin-top: 3px;
    color: var(--cs-pencil);
    /* ⚠ **600，不是 400**（2026-10-14，票 24 实测定案，见设计规范 §4.3）。
     *
     * 引文是归属行里字数最多的一行，12px 常规字重下 **75.1% 的笔画 ≤1px**。
     * 加粗把它压到 15.8%；抬字号只买来约 7 个百分点。
     *
     * ⚠ 层级怎么办：名字、引文、档位名三行现在**都是 600**，所以
     *   **层级不再靠字重**，改由别的东西扛——
     *     · 名字：`.cs-peer` 的 **0.18em 疏排**（`tracked` 变体）+ 更深的
     *       `--cs-pencil-2`（6.13 : 1）
     *     · 引文：`--cs-pencil`（5.10 : 1）
     *   「机器认出来的名字」这层语义靠**疏排 + 更深的灰**，不是粗细。
     * ⚠ 同样**不要写 500**：本机雅黑没有 500 字重，渲出来等于 400。 */
    font-weight: 600;
  }

  /*  正文。**仿宋** 21px / 行高 1.5（§4.2a 定案，2026-10-14 由楷体改）。
   *  ⚠ 21px 是定案值，换字号前先看 §4.2a 那条取舍：19px 是唯一保持 4 行的
   *    档，但 48.4% 的笔画 ≤1px 会断笔——**断笔 = 读不出来**。
   *  ⚠ 字体由楷体改仿宋的理由（用户 2026-10-14 判「仿宋最合适」）：
   *    同屏其余文字（归属行 / 档位名 / 面板标签）全是黑体族，楷体是
   *    全屏唯一一支不同的笔，读起来「突兀」。仿宋的横细竖粗印刷感
   *    与纸材同源，又比黑体软。
   *  ⚠ 仿宋比楷体**细**。这条要盯：若实机上出现断笔，先加粗重或退回楷体，
   *    不要靠降字号解决（降字号正是 §4.2a 否掉的那条路）。
   */
  .cs-draft-text {
    margin: 0;
    font-family: FangSong, STFangsong, '仿宋', serif;
    font-size: 21px;
    line-height: 1.5;
    color: var(--cs-ink);
    letter-spacing: 0.01em;
    /* 正文是**要选中复制**的（用户可能想改），与面板整体 user-select:none
       不同——那是给档位控件设的。 */
    user-select: text;
  }

  /*  正文字体探针（`?draft=`）。默认（`data-draft-font` 为空）**不上任何
   *  规则**，也就是说默认走下面 `.cs-draft-text` 里那一条定案值——
   *  探针口只能由探针口触发，不能让默认路径自己漂走。
   *
   *  ⚠ **每个 key 都必须有一条同名规则**，这是本项目的第一条探针纪律。
   *    踩过：`DRAFT_FONTS` 里有过一个 `lxgw`，但没写对应规则——探针把
   *    `data-draft-font="lxgw"` 设上去了却没人消费，正文**静默回落**成默认。
   *    症状特别能骗人：出图看着「字体没生效」，而那张图**逐字节等于**
   *    默认字体那张。不比对哈希就会以为「这台字体效果不行」。
   *    判据：换了参数却出一模一样的图，先 `Get-FileHash` 比一遍，
   *    再怀疑字体。（`lxgw` 已随「正文走本机真有的字体」一并删除，
   *    体积数据与重开条件见票 22 与 `ui/scripts/subset_font.py`。） */
  .cs-sheet[data-draft-font='kai'] .cs-draft-text {
    font-family: KaiTi, STKaiti, '楷体', serif;
  }
  .cs-sheet[data-draft-font='fang'] .cs-draft-text {
    font-family: FangSong, STFangsong, '仿宋', serif;
  }
  .cs-sheet[data-draft-font='song'] .cs-draft-text {
    font-family: SimSun, '宋体', NSimSun, serif;
  }
  .cs-sheet[data-draft-font='yahei'] .cs-draft-text {
    font-family: 'Microsoft YaHei', '微软雅黑', sans-serif;
  }
  .cs-sheet[data-draft-font='hei'] .cs-draft-text {
    font-family: SimHei, '黑体', sans-serif;
  }

  /* ── 归属行「名字」的呈现（`?peer=`）───────────────────────────
   *
   *  ⚠ 原来只有一种做法：**黑体 600 加粗**。它能扛住 12px，但字号一放开
   *    到 15px 就塌了——粗细是这一层**唯一**的层级手段时，字号一大，
   *    600 的黑体在 21px 正文旁边读成「一块黑」，不是标题。
   *
   *  ⇒ 层级要另找手段。下面五种各有各的代价，别默认「降粗细」就对了：
   *    w500     粗细降一档。最省事，但 15px 下与 400 差别很小。
   *    w400     不加粗。层级全交给字号与灰阶，最平。
   *    ink      400 + 换成 `--cs-ink`。用**颜色**扛层级而不是粗细——
   *             代价是「机器认出来的」这层语义被削弱（见 `.cs-peer` 注释）。
   *    tracked  400 + 拉开字距。用**疏排**扛层级，像手写标题那样
   *             一个字一个字排开，最贴纸；代价是西文混排会散。
   *    kai/fang 换楷体/仿宋，名字与正文同族。层级交给字体族，
   *             但那样整屏就只剩一支笔，「UI 是 UI、正文是正文」
   *             这条分工也没了。
   */
  .cs-sheet[data-peer-look='w500'] .cs-peer {
    font-weight: 500;
  }
  .cs-sheet[data-peer-look='w400'] .cs-peer {
    font-weight: 400;
  }
  .cs-sheet[data-peer-look='ink'] .cs-peer {
    font-weight: 400;
    color: var(--cs-ink);
  }
  .cs-sheet[data-peer-look='tracked'] .cs-peer {
    /* ⚠ 2026-10-14：从 400 改成 **600 + 保留 0.18em 疏排**。
     *
     *  票 24 实测（11px 纯色底，`analysetype.py`）：`tracked` 原先的
     *  **400 + 疏排**下，名字的 **≤1px 细笔画 = 77.8%**——和引文（79.6%）、
     *  档位名（77.2%）一样读不出。疏排只改**字距**、不改**笔画粗细**，
     *  扛得住"层级"却扛不住"小字细笔画"。
     *  加到 600 后降到 **13.7%**，三行齐刷刷过线。
     *
     *  ⇒ 层级仍由**疏排 + 颜色**扛（见 `.cs-peer` 的 pencil-2 6.13:1 vs
     *  引文 pencil 5.10:1），字重这层从"唯一手段"退成"辅助"。
     *  500 在本机雅黑不存在、渲成 400，所以只能跳 600。 */
    font-weight: 600;
    letter-spacing: 0.18em;
  }
  .cs-sheet[data-peer-look='kai'] .cs-peer {
    font-weight: 400;
    font-family: KaiTi, STKaiti, '楷体', serif;
  }
  .cs-sheet[data-peer-look='fang'] .cs-peer {
    font-weight: 400;
    font-family: FangSong, STFangsong, '仿宋', serif;
  }

  /* ⛔ 正文**不**套 `url(#pencil)`——2026-10-14 实测否掉，探针已删。
   *
   *  试过：把记号用的 `url(#pencil)`（feTurbulence 颗粒 + feDisplacementMap
   *  边缘位移）套到正文文字上，零新依赖，是「不像印刷字」最便宜的一招。
   *  视觉上确实立刻多出手绘感。**但用户看出的是「缺墨」**，判断正确：
   *  位移把笔画边缘啃成断续碎点，尤其仿宋那几支细横细竖。
   *
   *  ⚠ 根因不是「位移太狠」，是**作用对象选错了**。纸面板上最淡的元素
   *    就是正文（仿宋字面率 12.7%，§4.2 表）——它已经在临界点上，
   *    再叠一层侵蚀等于在最弱的地方补刀。记号是手绘的，正文不是，
   *    这两件事本来就该由不同东西承担。
   *
   *  ⇒ `url(#pencil)` 留给记号与档位标记（那两处笔画粗、面积大，吃得住）。
   *    正文的「不是印刷体」交给**仿宋本身**，不再叠滤镜。
   *  复查口径：出图**放大**看细笔画有没有真的断开，不是看「有没有颗粒感」。
   */

  /*  归属行字号探针（`?peersz=`）。同理，默认不上规则。
   *  ⚠ `.cs-peer-unknown` 也要列进来（2026-10-14 加）：它是归属行字号的
   *    **另一种**形态，漏了它就等于「认不出」那一态在探针下不缩放——
   *    调这一行字号时只调到了认得出的那一半。 */
  .cs-sheet[data-peer-px]:not([data-peer-px='']) .cs-where,
  .cs-sheet[data-peer-px]:not([data-peer-px='']) .cs-peer,
  .cs-sheet[data-peer-px]:not([data-peer-px='']) .cs-peer-unknown {
    font-size: calc(var(--peer-probe, 12) * 1px);
  }

  .cs-draft-pending,
  .cs-draft-idle {
    /* 还没落笔的时候没有「墨」可说。用铅笔灰，且字号不缩——缩了会让
       真落笔时正文往下跳一格。 */
    color: var(--cs-pencil);
  }

  .cs-draft-failed {
    /* 红只给不可逆（app.css 文件头）。出错不算不可逆，所以**不用红**，
       沿用铅笔灰 + 一句人话。 */
    color: var(--cs-pencil-2);
  }

  /*  Auto：会把话发出去。笔从红铅笔换成**警报红**——这是红铅笔
   *  唯一让位给 `--cs-live` 的地方。
   *
   *  ⚠ 2026-10-15：砍掉二次确认那一档之后，`Auto` 成为唯一不可逆的档，
   *  「红＝不可逆」这条规则从它身上继续成立。
   *
   *  ⚠ 这里**只能改 fill，不能写 fill-opacity**。CSS 声明会盖掉
   *  每层 path 上的 `fill-opacity` 属性，五层嵌套立刻塌成单层平涂，
   *  不可逆的档就成了唯一一支不像铅笔的。要改浓度就改生成器里的
   *  `NEST` 剖面，让五级一起动。 */
  .cs-sheet[data-tier='Auto'] .cs-tick path {
    fill: var(--cs-live);
  }

  /*  聚焦环。撕边纸上不能靠描边，得用一圈**纸色的负空间**把
   *  底下的笔画顶开，否则键盘用户看不到焦点在哪。 */
  .cs-pick-slot:focus-visible {
    outline: none;
  }
  .cs-pick-slot:focus-visible .cs-markbox {
    outline: 2px solid var(--cs-tier-ink);
    outline-offset: 2px;
  }
</style>
