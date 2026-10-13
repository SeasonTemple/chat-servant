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
    { key: 'Confirm', label: '确认后发' },
  ] as const;

  type TierKey = (typeof TIERS)[number]['key'];

  /**
   *  档位。**这是 Rust 那份的镜像，不是第二份真值**（硬约束第 5 条）。
   *
   *  初值分两种情况，差别就是安全观感：
   *    · 脱离 Tauri（演示）→ 认 `?tier=`，那是**探针口**，
   *      本来就没有「真档位」可言。
   *    · 真 Tauri → **一律从最低档起步**，哪怕 URL 写了 `?tier=Confirm`。
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
      // 界面先动的话，用户点了「确认后发」、勾已经挪过去了，
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
    // ⚠ **`tier` 原来根本没读**（2026-10-14 实测）。于是 `confirm.json`
    // 与 `drafted.json` 渲染出来**逐字节相同**——两个夹具的唯一差别就是
    // `tier` 与 `reason`，而后者一个都没接。演示页于是只演得出四态，
    // 文档里写的「七态」有一半是假的。
    //
    // 边界要写清楚：这里**没有第二份真值**。脱离 Tauri 时压根没有 Rust
    // （`HAS_TAURI` 为 false），`chooseTier` 也是直接改本地 state 的——
    // 同一个「演示下本地说了算」的约定，不是新增的一套。
    if (!HAS_TAURI && r.tier && TIERS.some((t) => t.key === r.tier)) {
      tier = r.tier as TierKey;
    }
    readout =
      r.phase === 'drafting'
        ? { phase: 'drafting', messages: msgs, status_hint: hint }
        : r.phase === 'failed'
          ? { phase: 'failed', messages: msgs, error: r.error ?? '未知原因' }
          : r.phase === 'drafted'
            ? { phase: 'drafted', messages: msgs, candidates: cands }
            : { phase: 'read', messages: msgs };
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

    ⚠ 设计规范 §1：设置页用**中性白**，刻意不参与「纸 + 墨 + 红批注」
    这套叙事。`PanelSettings` 自带那份配色，这里不覆写它——
    在撕边纸上叠一层同色系的设置，读成「纸上又贴了一张纸」。
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
        <span class="cs-peer">{peer || '—'}</span>
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
        {:else}
          <p class="cs-draft-text cs-draft-idle">对方说完话，这里才落笔。</p>
        {/if}
      </div>

      <!--
        档位行 —— **纸的底部**，紧贴将来的动作行。

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
       撕口实测 7~11 CSS px（随 x 变，对 alpha 阈值不敏感），
       量法见 ui/scripts/sheetprobe.ps1。原注释写「17.5px」，对不上。 */
    padding: 40px 32px 30px;
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
         实测出图里「只填入」骑在破口上。 */
    margin-top: auto;
    padding-bottom: 26px;
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
   *  ⚠ 尺寸由 SVG 的 `width`/`height` 属性给（= `ACTIVE_VIEW`），**不是** CSS。
   *    这里若写 `width: 100%`，`big` 档会被压回 60 宽而不是被裁——
   *    两者症状不同：裁是「放大了但没变大」，压是「选了大档但形状没变」。
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

  /*  放大档换了取景框，笔心坐标随之改变，偏移**重算**而不是在旧值上加减。
   *  实测（解析 marks.js 的 path 坐标，非估算）：mid 笔心 (27.70, 17.50)、
   *  big 笔心 (27.75, 17.50)——两档同框同高，偏移相同。代回公式
   *  top = 46.4 - 笔心y = 29，margin-left = -笔心x = -28。 */
  .cs-tick[data-shape='tick'][data-ring='mid'],
  .cs-tick[data-shape='tick'][data-ring='big'] {
    --tick-w: 56px;
    --tick-h: 34px;
    top: calc(29px * var(--tk, 1));
    margin-left: calc(-28px * var(--tk, 1));
  }

  .cs-tick[data-shape='ring'][data-ring='big'] {
    top: calc(21px * var(--tk, 1));
    margin-left: calc(-43px * var(--tk, 1));
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
    /* 未选中的那档退半档，让标记是唯一的高对比处。
     * 但 0.62 混的是透明度，落在纸上的实际对比度仍要够 ——
     * `--cs-pencil` 是 5.10 : 1，半透明后仍高于 AA 正文 4.5。 */
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

  .cs-where-quote {
    display: block;
    margin-top: 3px;
    color: var(--cs-pencil);
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
    font-weight: 400;
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

  /*  归属行字号探针（`?peersz=`）。同理，默认不上规则。 */
  .cs-sheet[data-peer-px]:not([data-peer-px='']) .cs-where,
  .cs-sheet[data-peer-px]:not([data-peer-px='']) .cs-peer {
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

  /*  Confirm：会把话发出去。笔从红铅笔换成**警报红**——这是红铅笔
   *  唯一让位给 `--cs-live` 的地方。
   *
   *  ⚠ 2026-10-13：`Auto` 取消后，`Confirm` 成为唯一不可逆的档，
   *  「红＝不可逆」这条规则从它身上继续成立。
   *
   *  ⚠ 这里**只能改 fill，不能写 fill-opacity**。CSS 声明会盖掉
   *  每层 path 上的 `fill-opacity` 属性，五层嵌套立刻塌成单层平涂，
   *  不可逆的档就成了唯一一支不像铅笔的。要改浓度就改生成器里的
   *  `NEST` 剖面，让五级一起动。 */
  .cs-sheet[data-tier='Confirm'] .cs-tick path {
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
