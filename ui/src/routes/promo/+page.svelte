<script lang="ts">
  import PencilDefs from '$lib/PencilDefs.svelte';
  import {
    TICK_NESTED,
    RING_MARK,
    RING_LABEL_MARK_BIG,
    CONF_STROKES,
    TICK_VIEW,
    MARK_VIEW,
  } from '$lib/marks';

  let lang = $state<'zh' | 'en'>('zh');
  let activeState = $state('drafted');
  let craftShape = $state<'tick' | 'ring'>('tick');
  let craftFilter = $state(true);
  let confLevel = $state<'high' | 'mid' | 'low'>('high');
  let strokeKey = $state(0);
  const t = (zh: string, en: string) => (lang === 'zh' ? zh : en);
  const rows = (list: string[]) => list.map((s) => s.split('|'));

  const EXE_URL =
    'https://github.com/SeasonTemple/chat-servant/releases/download/v0.2.1/chat-servant_0.2.1_x64-setup.exe';
  const MSI_URL =
    'https://github.com/SeasonTemple/chat-servant/releases/download/v0.2.1/chat-servant_0.2.1_x64_en-US.msi';

  const basePrefix =
    typeof window !== 'undefined' && window.location.pathname.startsWith('/chat-servant')
      ? '/chat-servant'
      : '';
  const iframeSrc = $derived(`${basePrefix}/panel?demo=1&state=${activeState}`);

  const fixtures = rows([
    'drafted|起草就绪|Drafted|三条候选已落纸，点击任意一条即可填入输入框（默认仅填入档）。|Three replies drafted on paper; click any candidate to fill the chat box (Manual tier).',
    'auto|自动发送档|Auto Tier|显式升至自动发送档：点选候选即刻发出，会话结束自动回落仅填入档。|Explicitly raised to Auto tier: clicking sends immediately and resets to Manual after session.',
    'drafting|正在落笔|Drafting|检测到对方新消息，言侍正在识读来信并拟写回复。|New peer message detected; YanShi is reading and drafting replies.',
    'many|多候选滚动|Many Replies|候选较多时的纸面纵向排布与朱砂批注勾选体验。|Vertical paper flow and vermilion tick marks when multiple candidates exist.',
    'longdraft|长草稿渐隐|Long Draft|超长草稿在纸面底部的自然渐隐与余白控制。|Natural bottom fade and margin discipline for lengthy draft paragraphs.',
    'parked|会话停泊|Parked|切走聊天窗口时自动停泊，隐藏锁定内容且零后台推理。|Automatically parks when switching away: hides locked readout with zero inference.',
    'noprep|无密钥预置|No API Key|未配密钥时的开箱引导态，直接点击右上齿轮即可配置。|Out-of-box onboarding state before configuring an API key via top-right gear.',
  ]);
  const curFix = $derived(fixtures.find((f) => f[0] === activeState) ?? fixtures[0]);

  const promises = rows([
    '窗口级只读视觉捕获 + 本地离线 PP-OCRv6，零注入、零读库、零碰内存|Window-level capture + local offline PP-OCRv6: zero hooks, zero DB reads, zero memory access',
    'Rust 单一事实来源，截图常驻内存绝不落盘，静默期零模型调用|Rust as single source of truth; frames stay in RAM only, with zero token calls while idle',
    '两档发送（仅填入 / 自动发送），默认最低档且绝不碰钱|Two-tier dispatch (Manual fill / Auto send), defaulting to Manual and forbidding money topics',
  ]);

  const privacyCards = rows([
    '01|HeaderPrint · perceive|只读本机可见会话|Visible Session Only|像素级会话抬头指纹闸，切走或最小化即刻停泊，绝不跨窗代看。|Pixel-level header fingerprint gate parks immediately on window switch or minimize.',
    '02|WGC + PP-OCRv6 Local|零进程侵入采集|Zero Process Intrusion|仅用 Windows Graphics Capture 与本地 ONNX 推理，不挂钩子、不注入、不碰数据库。|Uses Windows Graphics Capture and local ONNX OCR only—no DLL hooks or DB decryption.',
    '03|check-real-chat.ps1|截图与真文不落盘|No Frames or Chat on Disk|捕获帧常驻内存不写盘；提交钩子含 8 字连续片段黑名单守卫，严防语料入库。|Captured frames live only in RAM; git hooks enforce an 8-gram blacklist against chat leaks.',
    '04|the_draft_prompt_still_forbids_money_topics|绝不碰钱硬约束|Strict No-Money Rule|转账、红包、收款界面一律忽略，起草系统提示词由编译期单测逐词锁死禁令。|Transfers, red packets, and payment UI are ignored; unit tests lock money bans in prompt.',
    '05|the_tier_set_is_exactly_two_and_named|两档发送默认最低|Two Tiers, Lowest Default|仅 Manual（仅填入）与 Auto（自动发送）两档；升档需显式操作，会话结束自动回落。|Only Manual and Auto tiers exist; upgrading is explicit and resets to Manual after session.',
    '06|Zero-Token Idle Loop|静默期零网络调用|Zero Calls While Idle|水位线几何比对确认无新消息时绝不出网，十分钟无人说话即十分钟零消耗。|Watermark geometry check ensures zero network requests when no new peer message arrives.',
    '07|Windows Credential + secret-scan|凭据隔离与报错脱敏|Isolated Keys & Redacted Logs|密钥仅存系统凭据管理器或环境变量，前端拿不到密钥，日志与错误全脱敏。|API keys reside only in Windows Credential Manager or env vars; logs never see raw keys.',
  ]);

  const pipelineStages = rows([
    '第一阶|Stage I|crates/capture|窗口发现与本地识读|Capture & Offline OCR|Win32 窗口锚定 + WGC 帧捕获 + 本地 PP-OCRv6 文本检测与识别，像素不出本机。|Win32 window anchoring + WGC frame capture + local PP-OCRv6 detection and recognition.',
    '第二阶|Stage II|crates/perceive|气泡归类与会话身份闸|Perceive & Session Gate|将 OCR 文本框归类为对方/己方/噪声，校验 HeaderPrint 会话身份与底部对齐水位线。|Classifies OCR boxes into peer/self/noise, verifying HeaderPrint identity and watermarks.',
    '第三阶|Stage III|crates/reason|唯一出网起草阶段|Isolated Reasoning Stage|全仓唯一允许出网的模型调用层，内置禁钱守卫与凭据脱敏，输出结构化候选草稿。|The sole crate permitted to make LLM network calls, enforcing money bans and redaction.',
    '第四阶|Stage IV|crates/core|纯逻辑状态机与分档派发|Core State & Tier Dispatch|持有会话状态、两档派发裁决，通过 Tauri 事件向纸面前端单向广播。|Owns session state, Manual/Auto dispatch rules, and broadcasts readouts to the paper UI.',
  ]);

  const proofs = rows([
    '3 crates #![forbid(unsafe_code)]|core / perceive / reason 编译期严禁 unsafe|Compile-time #![forbid(unsafe_code)] across core, perceive, and reason',
    'TLA+ DispatchAndSessionGate.tla|会话切换与分档派发状态机经形式化规约建模|Session gate and tier dispatch transitions modeled in TLA+',
    '16-step 4-lane gate.ps1|四轨并行门禁全绿准入，与 CI 逐条镜像对齐|16-step 4-lane parallel gate verifying Rust, UI, licenses, and hooks',
    '9 read-only cargo xtask probes|9 个只读开发者探针，支持真机帧与排版几何复核|9 read-only developer xtask probes for frame and layout geometry inspection',
  ]);

  const quicksteps = rows([
    '步骤一 · 安装包|01 · Direct Setup|客户端安装（Windows 10/11 · exe / msi）|Desktop Setup (Windows 10/11 · exe / msi)|# 点击页首按钮即可直接下载 exe 或 msi，或终端拉取：\ncurl -fLO ' +
      EXE_URL +
      '\n# 安装包已内嵌 PP-OCRv6 离线权重，启动后点右上角齿轮即可配置密钥',
    '步骤二 · 免密钥体验|02 · Keyless Demo|免密钥体验与本地 13 态夹具巡检|Keyless Mode & Local Fixture Tour|cd ui && pnpm install && pnpm dev\n# 浏览器打开 http://localhost:1420/panel?demo=1&state=drafted',
    '步骤三 · 源码与探针|03 · Build & Probes|从源码构建与 9 个只读诊断探针|Build from Source & 9 Read-Only Probes|cd ui && pnpm install && pnpm tauri dev\ncargo xtask caps && cargo xtask occluded && cargo xtask layout',
  ]);
</script>

<svelte:head>
  <title>{t('言侍 — 侍于窗侧，执笔拟言', 'YanShi (chat-servant) — Desktop Chat Scribe')}</title>
</svelte:head>

<div class="promo-desk">
  <PencilDefs />

  <header class="topbar">
    <div class="topbar-inner">
      <a href="#stage" class="brand">
        <span class="brand-seal">言侍</span>
        <strong>{t('桌面单聊手笺', 'chat-servant')}</strong>
        <code>{t('v0.2.1 · 源码可见', 'v0.2.1 · BSL-1.1')}</code>
      </a>
      <nav class="topnav" aria-label="Page sections">
        <a href="#stage">{t('演示台', 'Stage')}</a>
        <a href="#craft">{t('器物美学', 'Craft')}</a>
        <a href="#privacy">{t('隐私边界', 'Privacy')}</a>
        <a href="#pipeline">{t('架构管线', 'Pipeline')}</a>
        <a href="#start">{t('快速开始', 'Quickstart')}</a>
      </nav>
      <div class="top-actions">
        <div class="lang-switch" role="group" aria-label="Language switch">
          <button
            type="button"
            class="lang-opt"
            class:active={lang === 'zh'}
            aria-pressed={lang === 'zh'}
            onclick={() => (lang = 'zh')}
          >
            {#if lang === 'zh'}
              <svg
                class="lang-ring-svg"
                viewBox="14 1 38 37"
                preserveAspectRatio="none"
                aria-hidden="true"
              >
                <g filter="url(#pencil)" fill="var(--cs-pencil-red)">
                  {#each RING_MARK as s, idx (idx)}<path d={s.d} fill-opacity={s.o} />{/each}
                </g>
              </svg>
            {/if}
            <span>中</span>
          </button>
          <span class="lang-sep" aria-hidden="true">/</span>
          <button
            type="button"
            class="lang-opt"
            class:active={lang === 'en'}
            aria-pressed={lang === 'en'}
            onclick={() => (lang = 'en')}
          >
            {#if lang === 'en'}
              <svg
                class="lang-ring-svg"
                viewBox="14 1 38 37"
                preserveAspectRatio="none"
                aria-hidden="true"
              >
                <g filter="url(#pencil)" fill="var(--cs-pencil-red)">
                  {#each RING_MARK as s, idx (idx)}<path d={s.d} fill-opacity={s.o} />{/each}
                </g>
              </svg>
            {/if}
            <span>EN</span>
          </button>
        </div>
        <a
          class="nav-link"
          href="https://github.com/SeasonTemple/chat-servant"
          target="_blank"
          rel="noreferrer">GitHub</a
        >
        <a class="btn-primary" href={EXE_URL} download>{t('下载言侍', 'Download')}</a>
      </div>
    </div>
  </header>

  <main class="desk-main">
    <section id="stage" class="hero">
      <div class="hero-copy">
        <div class="sheet-head">
          <span class="seal-tag">{t('卷壹 · 言侍台', 'VOL. I · YANSHI STAGE')}</span>
          <span class="kicker"
            >{t(
              'WINDOWS 桌面微信单聊副驾 · 本地离线识读 · 撕边棉纸手笺',
              'WINDOWS DESKTOP CHAT SCRIBE · LOCAL OFFLINE OCR · TORN COTTON PAPER',
            )}</span
          >
        </div>
        <div class="title-wrap">
          <h1>
            {t(
              '侍于窗侧，替你拟好下一句得体回音。',
              'Sitting beside your chat window, drafting your next fitting reply.',
            )}
          </h1>
          <svg
            class="hero-underline"
            viewBox="0 10 {MARK_VIEW.w} 16"
            preserveAspectRatio="none"
            aria-hidden="true"
          >
            <g filter="url(#pencil)" fill="var(--cs-pencil-red)">
              {#each CONF_STROKES.high as s, idx (idx)}<path d={s.d} fill-opacity={s.o} />{/each}
            </g>
          </svg>
        </div>
        <p class="lead">
          {t(
            '「言侍」——以言相侍。它不是冷冰冰的监控仪表盘，而是候在聊天窗口旁的一位文字侍从：默读屏幕上的来信，在暖黄棉纸便笺上替你拟出三两句得体草稿。你点一下，它替你填入输入框；无人说话时，它敛墨静候。',
            'YanShi ("attending with words") is not a monitoring dashboard, but a quiet attendant beside your chat window: it reads incoming messages on screen and drafts three thoughtful replies on a warm cotton-paper slip. Click once to fill; when the chat is quiet, it rests.',
          )}
        </p>

        <ul class="promise-list">
          {#each promises as p (p[1])}
            <li>
              <svg class="tick-svg" viewBox="0 0 {TICK_VIEW.w} {TICK_VIEW.h}" aria-hidden="true">
                <g filter="url(#pencil)" fill="var(--cs-pencil-red)">
                  {#each TICK_NESTED as s, idx (idx)}<path d={s.d} fill-opacity={s.o} />{/each}
                </g>
              </svg>
              <span>{t(p[0], p[1])}</span>
            </li>
          {/each}
        </ul>

        <div class="cta-row">
          <a class="btn-primary" href={EXE_URL} download
            >{t('下载 Windows 版（exe）', 'Download for Windows (exe)')}</a
          >
          <a class="btn-secondary" href={MSI_URL} download
            >{t('MSI 安装包（msi）', 'MSI Installer (msi)')}</a
          >
          <a
            class="btn-secondary"
            href="https://github.com/SeasonTemple/chat-servant"
            target="_blank"
            rel="noreferrer">{t('阅读源码', 'Source Code')}</a
          >
        </div>

        <div class="fixture-box">
          <div class="card-top">
            <strong
              >{t(
                '切换右侧真机面板 7 态夹具：',
                'Switch live PaperPanel 7-state fixtures:',
              )}</strong
            >
            <code class="card-guard">?demo=1&amp;state={activeState}</code>
          </div>
          <div class="fixture-tabs" role="tablist">
            {#each fixtures as f (f[0])}
              <button
                type="button"
                role="tab"
                aria-selected={activeState === f[0]}
                class="strip-tab"
                class:active={activeState === f[0]}
                onclick={() => (activeState = f[0])}
              >
                {#if activeState === f[0]}
                  <svg
                    class="ring-svg"
                    viewBox="0 1 {MARK_VIEW.w} 28"
                    preserveAspectRatio="none"
                    aria-hidden="true"
                  >
                    <g filter="url(#pencil)" fill="var(--cs-pencil-red)">
                      {#each RING_LABEL_MARK_BIG as s, idx (idx)}<path
                          d={s.d}
                          fill-opacity={s.o}
                        />{/each}
                    </g>
                  </svg>
                {/if}
                <span>{t(f[1], f[2])}</span>
              </button>
            {/each}
          </div>
          <p class="fixture-note">{t(curFix[3], curFix[4])}</p>
        </div>
      </div>

      <div class="stage-card">
        <div class="stage-bar">
          <div class="stage-bar-left">
            <span class="stage-dot"></span>
            <span>{t(`言侍手笺 · ${curFix[1]}`, `PaperPanel · ${activeState}`)}</span>
          </div>
          <span class="stage-flow"
            >{t('捕获 → 识读 → 身份闸 → 拟稿', 'WGC → OCR → Gate → Draft')}</span
          >
        </div>
        {#key activeState}
          <div class="stage-sheet-anim">
            <iframe title="YanShi interactive demo panel" src={iframeSrc} class="stage-frame"
            ></iframe>
          </div>
        {/key}
        <p class="stage-hint">
          {t(
            '↑ 真实生产面板实时运行：可直接点选纸面候选、切底栏档位、点右上角齿轮翻开设置屏',
            '↑ Live Svelte 5 PaperPanel: click candidates, toggle bottom tier, or open gear settings',
          )}
        </p>
      </div>
    </section>

    <section id="craft" class="section-block">
      <div class="sheet-head">
        <span class="seal-tag">{t('卷贰 · 器物美学', 'VOL. II · PAPER & PENCIL CRAFT')}</span>
        <span class="kicker"
          >{t(
            '拒绝千篇一律的监控仪表盘 · 每一笔皆可亲手拆解',
            'NO DASHBOARD TEMPLATES · INTERACTIVE MATERIAL ANATOMY',
          )}</span
        >
      </div>
      <h2>
        {t(
          '纸面物理与红铅笔笔迹解剖（点击下方纸条亲手试笔）',
          'Tactile Paper Physics & Red-Pencil Stroke Anatomy (Click to Test)',
        )}
      </h2>
      <div class="card-grid">
        <article class="paper-card">
          <div class="card-top">
            <span class="card-no">{t('器物一', 'CRAFT 01')}</span><code class="card-guard"
              >{t('五层同心羽化 + #pencil 滤镜', '#pencil + 5-Layer Feathering')}</code
            >
          </div>
          <h3>{t('落笔挑脚与蜡质颗粒滤镜', 'Pencil Pressure & Wax-Grain Filter')}</h3>
          <p>
            {t(
              '不用等宽 SVG 线条。笔迹由「中心线 + 压感宽度剖面」生成，5 层同心羽化（0.22→0.58）叠加 feTurbulence 石墨断续颗粒。',
              'No uniform stroke-width: paths are generated from a pressure profile with 5-layer core-to-edge feathering plus feTurbulence wax grain.',
            )}
          </p>
          <div class="lab-controls">
            <button
              type="button"
              class="strip-tab"
              class:active={craftShape === 'tick'}
              onclick={() => {
                craftShape = 'tick';
                strokeKey++;
              }}>{t('朱砂勾', 'Tick')}</button
            >
            <button
              type="button"
              class="strip-tab"
              class:active={craftShape === 'ring'}
              onclick={() => {
                craftShape = 'ring';
                strokeKey++;
              }}>{t('朱砂圈', 'Ring')}</button
            >
            <button
              type="button"
              class="strip-tab"
              class:active={craftFilter}
              onclick={() => {
                craftFilter = !craftFilter;
                strokeKey++;
              }}
              >{craftFilter
                ? t('蜡质颗粒：开', 'Grain: ON')
                : t('蜡质颗粒：关（平涂）', 'Grain: OFF')}</button
            >
          </div>
          {#key strokeKey}
            <button type="button" class="lab-stage lab-stage-btn" onclick={() => strokeKey++}>
              {#if craftShape === 'tick'}
                <svg class="lab-mark-svg" viewBox="0 0 56 39">
                  <defs>
                    <mask
                      id="tick-draw-{strokeKey}"
                      maskUnits="userSpaceOnUse"
                      x="0"
                      y="0"
                      width="56"
                      height="40"
                    >
                      <path
                        class="stroke-reveal-path"
                        d="M 12.8 8.6 L 15.6 9.5 L 23.2 36.2 L 42 5"
                        fill="none"
                        stroke="white"
                        stroke-width="6.2"
                        stroke-linecap="butt"
                        stroke-linejoin="round"
                        pathLength="100"
                      />
                    </mask>
                  </defs>
                  <path
                    class="stroke-guide-line"
                    d="M 12.8 8.6 L 15.6 9.5 L 23.2 36.2 L 42 5"
                    fill="none"
                  />
                  <g
                    mask="url(#tick-draw-{strokeKey})"
                    filter={craftFilter ? 'url(#pencil)' : undefined}
                    fill="var(--cs-pencil-red)"
                  >
                    {#each TICK_NESTED as s, idx (idx)}<path d={s.d} fill-opacity={s.o} />{/each}
                  </g>
                  <circle class="stroke-pen-tip tick-tip" r="2.3" />
                </svg>
              {:else}
                <svg class="lab-mark-svg" viewBox="12 0 42 40">
                  <defs>
                    <mask
                      id="ring-draw-{strokeKey}"
                      maskUnits="userSpaceOnUse"
                      x="0"
                      y="0"
                      width="60"
                      height="42"
                    >
                      <path
                        class="stroke-reveal-path"
                        d="M 21.5 25.5 C 18 14, 24 3.5, 34 3.5 C 45 3.5, 50.5 12, 49.5 20 C 48.5 30.5, 39 36.5, 31 36.5 C 22.5 36.5, 16.5 29, 18 21"
                        fill="none"
                        stroke="white"
                        stroke-width="6.5"
                        stroke-linecap="butt"
                        pathLength="100"
                      />
                    </mask>
                  </defs>
                  <path
                    class="stroke-guide-line"
                    d="M 21.5 25.5 C 18 14, 24 3.5, 34 3.5 C 45 3.5, 50.5 12, 49.5 20 C 48.5 30.5, 39 36.5, 31 36.5 C 22.5 36.5, 16.5 29, 18 21"
                    fill="none"
                  />
                  <g
                    mask="url(#ring-draw-{strokeKey})"
                    filter={craftFilter ? 'url(#pencil)' : undefined}
                    fill="var(--cs-pencil-red)"
                  >
                    {#each RING_MARK as s, idx (idx)}<path d={s.d} fill-opacity={s.o} />{/each}
                  </g>
                  <circle class="stroke-pen-tip ring-tip" r="2.3" />
                </svg>
              {/if}
              <span class="lab-caption"
                >{t(
                  '↻ 点击重放 1.2s 慢速运笔与中心线轨迹',
                  '↻ Click to replay 1.2s stroke & centerline',
                )}</span
              >
            </button>
          {/key}
        </article>

        <article class="paper-card">
          <div class="card-top">
            <span class="card-no">{t('器物二', 'CRAFT 02')}</span><code class="card-guard"
              >{t('-3° 楷体分值 + 26% 退隐叠印', '-3° Graphite Score + 26% Stamp Overlay')}</code
            >
          </div>
          <h3>
            {t(
              '不印百分号：铅笔斜体分值与朱砂叠印',
              'No % Sign: Graphite Marginal Score & Cinnabar Stamp',
            )}
          </h3>
          <p>
            {t(
              '言侍不在纸上印冷冰冰的「72%」进度条。每条候选右侧以 -3° 楷体铅笔写下两位数分值（72 / 54 / 28）；选中项分值自动退隐为 26% 浅底纹，原位叠盖一枚朱砂红勾或红圈。',
              'YanShi never prints a cold "72%" progress bar. Each strip carries a -3° italic graphite score (72 / 54 / 28); on the chosen strip, the score recedes to 26% opacity beneath a cinnabar red tick or ring.',
            )}
          </p>
          <div class="lab-controls">
            {#each [['high', '72 · 笃定首选', '72 · Top'], ['mid', '54 · 稳妥备选', '54 · Mid'], ['low', '28 · 补充角度', '28 · Alt']] as lv (lv[0])}
              <button
                type="button"
                class="strip-tab"
                class:active={confLevel === lv[0]}
                onclick={() => (confLevel = lv[0] as 'high' | 'mid' | 'low')}
                >{t(lv[1], lv[2])}</button
              >
            {/each}
          </div>
          {#key `${confLevel}-${craftShape}`}
            <div class="lab-stage">
              <div class="lab-score-row">
                {#each [['high', '72'], ['mid', '54'], ['low', '28']] as item (item[0])}
                  {@const picked = confLevel === item[0]}
                  <button
                    type="button"
                    class="lab-score-slot"
                    class:picked
                    onclick={() => (confLevel = item[0] as 'high' | 'mid' | 'low')}
                  >
                    <span class="lab-score-num" class:faded={picked}>{item[1]}</span>
                    {#if picked}
                      <svg
                        class="lab-score-mark"
                        viewBox={craftShape === 'ring'
                          ? `14 1 38 37`
                          : `0 0 ${TICK_VIEW.w} ${TICK_VIEW.h}`}
                        preserveAspectRatio="none"
                        aria-hidden="true"
                      >
                        <g filter="url(#pencil)" fill="var(--cs-pencil-red)">
                          {#each craftShape === 'ring' ? RING_MARK : TICK_NESTED as s, idx (idx)}<path
                              d={s.d}
                              fill-opacity={s.o}
                            />{/each}
                        </g>
                      </svg>
                    {/if}
                  </button>
                {/each}
              </div>
              <span class="lab-caption"
                >{t('← 点选分值看退隐与落印', '← Click score to stamp')}</span
              >
            </div>
          {/key}
        </article>

        <article class="paper-card">
          <div class="card-top">
            <span class="card-no">{t('器物三', 'CRAFT 03')}</span><code class="card-guard"
              >probe-sheet.png · 5 Layers</code
            >
          </div>
          <h3>
            {t(
              '五层撕口纸边与右下偏心指压反馈',
              '5-Layer Deckle Edge & Right-Weighted Finger Press',
            )}
          </h3>
          <p>
            {t(
              '不用 CSS 直线边框，也不用 SetWindowRgn 二值硬阶梯。整张棉纸含五层物理细节，悬停整条微抬（-2px），按下时整条联动下沉、且受力重心偏向右侧/右下角压入纸面。',
              'No CSS solid borders or jagged SetWindowRgn masks. Real cotton paper with 5 edge layers, instant hover lift (-2px), and a full-strip press weighted toward the bottom-right finger contact.',
            )}
          </p>
          <ul class="layer-list">
            <li>
              <code>①</code>
              {t('暖黄棉纤维表面 rgb(243 230 192)', 'Warm cotton fiber surface')}
            </li>
            <li><code>②</code> {t('1px 切面顶部纤维反光亮口', '1px inner fiber highlight lip')}</li>
            <li><code>③</code> {t('三维纸张断面侧壁纹理', '3D cut-fiber sidewall')}</li>
            <li>
              <code>④+⑤</code>
              {t('1px 外缘暗线 + 合成器抗锯齿撕口', '1px outer occlusion + AA deckle edge')}
            </li>
          </ul>
        </article>
      </div>
    </section>

    <section id="privacy" class="section-block">
      <div class="sheet-head">
        <span class="seal-tag">{t('卷叁 · 铁律校样', 'VOL. III · HARD CONSTRAINTS')}</span>
        <span class="kicker"
          >{t('七条产品底线 · 机器守卫逐条锁死', 'SEVEN HARD CONSTRAINTS · MACHINE-GUARDED')}</span
        >
      </div>
      <h2>
        {t(
          '隐私与安全硬边界（不靠自觉，靠编译期与门禁守卫）',
          'Privacy & Safety Boundaries Enforced by Code and CI',
        )}
      </h2>
      <div class="card-grid">
        {#each privacyCards as c (c[0])}
          <article class="paper-card">
            <div class="card-top">
              <span class="card-no">{c[0]}</span><code class="card-guard">{c[1]}</code>
            </div>
            <h3>{t(c[2], c[3])}</h3>
            <p>{t(c[4], c[5])}</p>
          </article>
        {/each}
      </div>
    </section>

    <section id="pipeline" class="section-block">
      <div class="sheet-head">
        <span class="seal-tag">{t('卷肆 · 架构管线', 'VOL. IV · RUST PIPELINE')}</span>
        <span class="kicker"
          >{t(
            'RUST 单一事实来源 · 四阶段流水线',
            'RUST SINGLE SOURCE OF TRUTH · 4-STAGE PIPELINE',
          )}</span
        >
      </div>
      <h2>
        {t('四阶段 Rust 架构管线与工程背书', 'Four-Stage Rust Pipeline & Engineering Proofs')}
      </h2>
      <div class="card-grid">
        {#each pipelineStages as s (s[2])}
          <article class="paper-card">
            <div class="card-top">
              <span class="card-no">{t(s[0], s[1])}</span><code class="card-guard">{s[2]}</code>
            </div>
            <h3>{t(s[3], s[4])}</h3>
            <p>{t(s[5], s[6])}</p>
          </article>
        {/each}
      </div>
      <div class="proof-strip">
        {#each proofs as pf (pf[0])}
          <div class="proof-item">
            <code class="card-guard">{pf[0]}</code><span>{t(pf[1], pf[2])}</span>
          </div>
        {/each}
      </div>
    </section>

    <section id="start" class="section-block">
      <div class="sheet-head">
        <span class="seal-tag">{t('卷伍 · 开箱探针', 'VOL. V · QUICKSTART')}</span>
        <span class="kicker">{t('三步上手 · 内嵌离线权重开箱即用', 'GET STARTED IN MINUTES')}</span>
      </div>
      <h2>{t('快速开始与本地诊断探针', 'Quickstart & Developer Probes')}</h2>
      <div class="card-grid">
        {#each quicksteps as qs (qs[0])}
          <div class="paper-card">
            <span class="card-no">{t(qs[0], qs[1])}</span>
            <h3>{t(qs[2], qs[3])}</h3>
            <pre class="code-box"><code>{qs[4]}</code></pre>
          </div>
        {/each}
      </div>
    </section>
  </main>

  <footer class="desk-footer">
    <strong>{t('言侍', 'YanShi · chat-servant')}</strong> — {t(
      '许可协议 BSL-1.1（2029-10-12 自动转为 Apache-2.0）。Rust 侧为唯一事实来源。',
      'Licensed under BSL-1.1 (converts to Apache-2.0 on 2029-10-12). Rust backend is the single source of truth.',
    )}
  </footer>
</div>

<!-- prettier-ignore -->
<style>
	:global(html), :global(body) { overflow-y: auto !important; overflow-x: hidden !important; height: auto !important; user-select: text !important; background: #0b1014; scroll-behavior: smooth; }
	@keyframes drawMark {
		from { clip-path: inset(0 100% 0 0); opacity: 0.35; }
		to { clip-path: inset(0 0% 0 0); opacity: 1; }
	}
	@keyframes sheetFlip {
		from { opacity: 0.55; transform: translateY(5px) scale(0.992); }
		to { opacity: 1; transform: translateY(0) scale(1); }
	}
	.promo-desk {
		min-height: 100vh;
		background:
			radial-gradient(1100px 640px at 18% 6%, rgb(36 52 64 / 0.58) 0%, transparent 70%),
			radial-gradient(960px 600px at 85% 24%, rgb(26 38 48 / 0.68) 0%, transparent 72%),
			repeating-linear-gradient(115deg, rgb(255 255 255 / 0.012) 0px, rgb(255 255 255 / 0.012) 1px, transparent 1px, transparent 6px),
			linear-gradient(180deg, #12181e 0%, #0b1014 55%, #090d11 100%);
		color: var(--cs-ink);
		font-family: 'FangSong', 'STFangsong', '仿宋', 'Noto Serif SC', Georgia, serif;
		padding-bottom: 3.5rem;
	}
	.topbar {
		position: sticky;
		top: 0;
		z-index: 30;
		background: rgb(14 19 24 / 0.92);
		backdrop-filter: blur(8px);
		border-bottom: 1px solid rgb(76 90 99 / 0.38);
		box-shadow: 0 8px 24px rgb(0 0 0 / 0.45);
	}
	.topbar-inner { width: min(92vw, 1720px); margin: 0 auto; padding: 0.75rem 1.5rem; display: flex; align-items: center; justify-content: space-between; gap: 1rem; flex-wrap: wrap; }
	.brand, .topnav, .top-actions, .cta-row, .fixture-tabs, .lab-controls { display: flex; align-items: center; gap: 0.7rem; flex-wrap: wrap; }
	.brand { text-decoration: none; color: var(--cs-paper); font-size: clamp(1.02rem, 1.08vw, 1.18rem); }
	.brand code { font: 0.76rem ui-monospace, Consolas, monospace; color: var(--cs-paper); opacity: 0.78; border: 1px solid rgb(243 230 192 / 0.28); padding: 0.08rem 0.42rem; border-radius: 2px; }
	.brand-seal { background: var(--cs-pencil-red); color: var(--cs-paper-raised); font-weight: 700; padding: 0.16rem 0.55rem; border-radius: 2px; box-shadow: inset 0 0 0 1px rgb(250 242 214 / 0.35); letter-spacing: 0.08em; }
	.topnav { gap: clamp(0.8rem, 1.4vw, 1.65rem); }
	.topnav a, .nav-link { color: var(--cs-paper); opacity: 0.86; text-decoration: none; font-weight: 600; font-size: clamp(0.92rem, 0.98vw, 1.05rem); transition: transform 45ms ease-out, opacity 45ms ease-out; }
	.topnav a:hover, .nav-link:hover { opacity: 1; color: #fff; transform: translateY(-1px); }
	.lang-switch {
		display: inline-flex;
		align-items: center;
		gap: 0.2rem;
		background: none;
		border: 8px solid transparent;
		border-image: url('/strip.png') 32 fill / 8px / 0 stretch;
		padding: 0.04rem 0.32rem;
		filter: drop-shadow(0 2px 4px rgb(0 0 0 / 0.35));
	}
	.lang-opt {
		position: relative;
		padding: 0.08rem 0.44rem;
		font: 600 0.86rem inherit;
		color: var(--cs-pencil);
		opacity: 0.65;
		cursor: pointer;
		transition: transform 45ms cubic-bezier(0.22, 1, 0.36, 1), opacity 45ms ease-out, color 45ms ease-out;
	}
	.lang-opt:hover { opacity: 1; transform: translateY(-1px); }
	.lang-opt.active { color: var(--cs-pencil-red); font-weight: 700; opacity: 1; }
	.lang-sep { font-size: 0.78rem; color: var(--cs-pencil); opacity: 0.45; user-select: none; }
	.lang-ring-svg { position: absolute; inset: -4px -5px; width: calc(100% + 10px); height: calc(100% + 8px); pointer-events: none; animation: drawMark 260ms cubic-bezier(0.22, 1, 0.36, 1) both; }
	.ring-svg { position: absolute; inset: -4px -6px; width: calc(100% + 12px); height: calc(100% + 8px); pointer-events: none; animation: drawMark 280ms cubic-bezier(0.22, 1, 0.36, 1) both; }
	.desk-main { width: min(92vw, 1720px); margin: 0 auto; padding: clamp(2rem, 2.6vw, 3.2rem) 1.5rem; display: flex; flex-direction: column; gap: clamp(2.4rem, 3vw, 3.6rem); }
	.hero, .section-block {
		position: relative;
		isolation: isolate;
		background: none;
		padding: clamp(2rem, 2.8vw, 3.3rem) clamp(2.1rem, 3.2vw, 3.8rem);
	}
	.hero::before, .section-block::before {
		content: '';
		position: absolute;
		inset: 0;
		z-index: -1;
		pointer-events: none;
		border: 24px solid transparent;
		border-image: url('/probe-sheet.png') 76 68 76 68 fill / 24px 22px 24px 22px / 0 stretch;
		filter: drop-shadow(0 18px 32px rgb(3 6 9 / 0.68));
	}
	.hero { display: grid; grid-template-columns: 1fr 420px; gap: clamp(2rem, 3vw, 3.6rem); align-items: center; }
	.sheet-head {
		display: flex;
		align-items: center;
		gap: 0.85rem;
		flex-wrap: wrap;
		padding-bottom: 0.65rem;
		margin-bottom: 0.9rem;
		border-bottom: 3px double rgb(46 34 18 / 0.24);
	}
	.seal-tag {
		font: 700 clamp(0.74rem, 0.82vw, 0.88rem) ui-monospace, Consolas, monospace;
		color: var(--cs-pencil-red);
		border: 1.5px solid var(--cs-pencil-red);
		padding: 0.1rem 0.52rem;
		letter-spacing: 0.08em;
		background: rgb(200 50 42 / 0.06);
		transform: rotate(-1.2deg);
	}
	.kicker { font: 700 clamp(0.76rem, 0.84vw, 0.92rem) ui-monospace, Consolas, monospace; letter-spacing: 0.06em; color: var(--cs-tier-ink); opacity: 0.86; }
	.title-wrap { position: relative; display: inline-block; margin-bottom: 0.9rem; }
	h1 { font-size: clamp(2.15rem, 2.45vw, 3.1rem); line-height: 1.24; margin: 0; color: var(--cs-ink); letter-spacing: 0.01em; }
	.hero-underline { width: 68%; height: 15px; display: block; margin-top: 0.2rem; animation: drawMark 520ms cubic-bezier(0.22, 1, 0.36, 1) both; }
	h2 { font-size: clamp(1.52rem, 1.72vw, 2.08rem); margin: 0 0 1.3rem; color: var(--cs-ink); }
	.lead { font-size: clamp(1.04rem, 1.14vw, 1.26rem); line-height: 1.72; color: var(--cs-ink-2); margin: 0 0 1.35rem; font-weight: 600; max-width: 66ch; }
	.promise-list { list-style: none; padding: 0; margin: 0 0 1.5rem; display: flex; flex-direction: column; gap: 0.62rem; }
	.promise-list li { display: flex; align-items: flex-start; gap: 0.65rem; font-size: clamp(0.95rem, 1.04vw, 1.14rem); color: var(--cs-ink); font-weight: 600; }
	.tick-svg { width: 24px; height: 17px; flex-shrink: 0; margin-top: 0.16rem; animation: drawMark 420ms cubic-bezier(0.22, 1, 0.36, 1) both; }
	.btn-primary {
		text-decoration: none;
		padding: 0.58rem 1.2rem;
		font-weight: 700;
		border-radius: 2px;
		font-size: clamp(0.9rem, 0.96vw, 1.04rem);
		background: var(--cs-pencil-red);
		color: var(--cs-paper-raised);
		border: 1px solid rgb(120 22 18);
		box-shadow: inset 0 0 0 1px rgb(250 242 214 / 0.28), 0 3px 8px rgb(120 22 18 / 0.28);
		transition: transform 45ms cubic-bezier(0.22, 1, 0.36, 1), box-shadow 45ms ease-out;
	}
	.btn-primary:hover { transform: translateY(-2px); box-shadow: inset 0 0 0 1px rgb(250 242 214 / 0.35), 0 6px 14px rgb(120 22 18 / 0.38); }
	.btn-primary:active { transform: translateY(1px) scale(0.99, 0.98); box-shadow: inset 0 0 0 1px rgb(250 242 214 / 0.2), 0 1px 2px rgb(120 22 18 / 0.25); transition-duration: 25ms; }
	.btn-secondary {
		text-decoration: none;
		background: none;
		border: 10px solid transparent;
		border-image: url('/strip.png') 32 fill / 10px / 0 stretch;
		padding: 0.18rem 0.72rem;
		font-weight: 700;
		font-size: clamp(0.88rem, 0.94vw, 1rem);
		color: var(--cs-tier-ink);
		filter: drop-shadow(0 2px 3px rgb(20 26 31 / 0.18));
		transition: transform 45ms cubic-bezier(0.22, 1, 0.36, 1), filter 45ms ease-out;
	}
	.btn-secondary:hover, .strip-tab:hover { transform: translateY(-2px); filter: drop-shadow(0 6px 10px rgb(20 26 31 / 0.26)); }
	.btn-secondary:active, .strip-tab:active { transform: translateY(1px) scale(0.988, 0.975); filter: drop-shadow(0 1px 2px rgb(20 26 31 / 0.14)); transition-duration: 25ms; }
	.fixture-box, .paper-card, .proof-strip {
		background: none;
		border: 14px solid transparent;
		border-image: url('/probe-sheet.png') 64 56 64 56 fill / 14px 12px 14px 12px / 0 stretch;
		padding: clamp(0.55rem, 0.8vw, 0.9rem) clamp(0.65rem, 0.95vw, 1.1rem);
		filter: drop-shadow(0 3px 6px rgb(20 26 31 / 0.15));
		transition: transform 55ms cubic-bezier(0.22, 1, 0.36, 1);
	}
	.paper-card:hover { transform: translateY(-2px); }
	.fixture-box { margin-top: 1.4rem; display: flex; flex-direction: column; gap: 0.75rem; }
	.strip-tab {
		position: relative;
		background: none;
		border: 10px solid transparent;
		border-image: url('/strip.png') 32 fill / 10px / 0 stretch;
		padding: 0.16rem 0.65rem;
		font: 700 clamp(0.84rem, 0.9vw, 0.96rem) inherit;
		cursor: pointer;
		color: var(--cs-ink);
		filter: drop-shadow(0 2px 3px rgb(20 26 31 / 0.15));
		transition: transform 45ms cubic-bezier(0.22, 1, 0.36, 1), filter 45ms ease-out;
	}
	.strip-tab.active { color: var(--cs-pencil-red); }
	.fixture-note, .paper-card p { margin: 0; font-size: clamp(0.9rem, 0.96vw, 1.04rem); line-height: 1.62; color: var(--cs-ink-2); font-weight: 600; }
	.stage-card {
		background: linear-gradient(180deg, #141b22 0%, #0d1217 100%);
		border: 1px solid rgb(76 90 99 / 0.45);
		box-shadow: inset 0 2px 12px rgb(0 0 0 / 0.7), 0 12px 26px rgb(10 14 18 / 0.38);
		padding: 0.95rem;
		border-radius: 6px;
		width: 420px;
		box-sizing: border-box;
	}
	.stage-bar { display: flex; align-items: center; justify-content: space-between; gap: 0.4rem; margin-bottom: 0.55rem; color: var(--cs-paper); font: 0.74rem ui-monospace, Consolas, monospace; white-space: nowrap; }
	.stage-bar-left { display: flex; align-items: center; gap: 0.4rem; }
	.stage-flow { font-size: 0.68rem; color: var(--cs-pencil-red); border: 1px solid rgb(200 50 42 / 0.45); padding: 0.05rem 0.36rem; border-radius: 2px; }
	.stage-dot { width: 8px; height: 8px; border-radius: 50%; background: var(--cs-pencil-red); box-shadow: 0 0 8px var(--cs-pencil-red); }
	.stage-sheet-anim { animation: sheetFlip 220ms cubic-bezier(0.22, 1, 0.36, 1) both; }
	.stage-frame { width: 380px; height: 560px; border: 0; display: block; margin: 0 auto; border-radius: 4px; }
	.stage-hint { margin: 0.65rem 0 0; font-size: 0.82rem; color: var(--cs-paper); opacity: 0.88; text-align: center; }
	@keyframes revealStrokePath {
		0% { stroke-dashoffset: 100; }
		100% { stroke-dashoffset: 0; }
	}
	@keyframes movePenTip {
		0% { offset-distance: 0%; opacity: 1; }
		88% { offset-distance: 100%; opacity: 1; }
		100% { offset-distance: 100%; opacity: 0; }
	}
	.stroke-reveal-path {
		stroke-dasharray: 100;
		stroke-dashoffset: 100;
		animation: revealStrokePath 1200ms cubic-bezier(0.4, 0, 0.2, 1) forwards;
	}
	.stroke-guide-line {
		stroke: rgb(46 34 18 / 0.22);
		stroke-width: 0.75;
		stroke-dasharray: 2 2;
	}
	.stroke-pen-tip {
		fill: var(--cs-pencil-red);
		stroke: #fff8ea;
		stroke-width: 0.9;
		filter: drop-shadow(0 0 3px rgb(200 50 42 / 0.75));
		animation: movePenTip 1320ms cubic-bezier(0.4, 0, 0.2, 1) forwards;
	}
	.tick-tip { offset-path: path('M 12.8 8.6 L 15.6 9.5 L 23.2 36.2 L 42 5'); }
	.ring-tip { offset-path: path('M 21.5 25.5 C 18 14, 24 3.5, 34 3.5 C 45 3.5, 50.5 12, 49.5 20 C 48.5 30.5, 39 36.5, 31 36.5 C 22.5 36.5, 16.5 29, 18 21'); }
	.lab-stage { display: flex; align-items: center; justify-content: space-between; gap: 0.75rem; margin-top: 0.45rem; padding: 0.55rem 0.75rem; border-top: 1px dashed rgb(46 34 18 / 0.2); }
	.lab-stage-btn { width: 100%; background: none; border-left: 0; border-right: 0; border-bottom: 0; font: inherit; cursor: pointer; text-align: left; }
	.lab-mark-svg { width: 68px; height: 44px; overflow: visible; }
	.lab-score-row { display: inline-flex; align-items: center; gap: 0.65rem; }
	.lab-score-slot {
		position: relative;
		width: 44px;
		height: 30px;
		display: inline-flex;
		align-items: center;
		justify-content: center;
		background: rgb(46 34 18 / 0.05);
		border: 1px dashed rgb(46 34 18 / 0.24);
		border-radius: 3px;
		cursor: pointer;
		padding: 0;
	}
	.lab-score-slot.picked { border-style: solid; border-color: rgb(200 50 42 / 0.45); background: rgb(200 50 42 / 0.05); }
	.lab-score-num {
		font-family: 'STKaiti', 'KaiTi', 'FangSong', Georgia, serif;
		font-style: italic;
		font-weight: 700;
		font-size: 16px;
		color: #544d42;
		transform: rotate(-3deg);
		filter: url(#pencil);
		transition: opacity 80ms ease-out;
	}
	.lab-score-num.faded { opacity: 0.26; color: var(--cs-ink); }
	.lab-score-mark {
		position: absolute;
		width: 38px;
		height: 26px;
		overflow: visible;
		pointer-events: none;
		animation: drawMark 320ms cubic-bezier(0.22, 1, 0.36, 1) both;
	}
	.lab-caption { font: 700 0.78rem ui-monospace, Consolas, monospace; color: var(--cs-pencil-red); }
	.layer-list { list-style: none; padding: 0; margin: 0.35rem 0 0; display: flex; flex-direction: column; gap: 0.38rem; font-size: clamp(0.84rem, 0.9vw, 0.96rem); font-weight: 600; color: var(--cs-ink-2); }
	.layer-list code { color: var(--cs-pencil-red); font-weight: 700; margin-right: 0.3rem; }
	.card-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(290px, 1fr)); gap: clamp(1rem, 1.35vw, 1.45rem); }
	.proof-strip { margin-top: 1.25rem; display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 1.1rem; }
	.paper-card, .proof-item { display: flex; flex-direction: column; gap: 0.5rem; font-size: clamp(0.88rem, 0.94vw, 1rem); min-width: 0; }
	.card-top { display: flex; justify-content: space-between; align-items: flex-start; gap: 0.4rem; flex-wrap: wrap; min-width: 0; border-bottom: 1px dashed rgb(46 34 18 / 0.22); padding-bottom: 0.4rem; }
	.card-no { font: 700 0.85rem ui-monospace, Consolas, monospace; color: var(--cs-pencil-red); flex-shrink: 0; }
	.card-guard { font: 0.75rem ui-monospace, Consolas, monospace; background: rgb(46 34 18 / 0.07); color: var(--cs-tier-ink); padding: 0.14rem 0.48rem; border: 1px solid rgb(46 34 18 / 0.28); border-radius: 2px; max-width: 100%; overflow-wrap: anywhere; word-break: break-word; }
	.paper-card h3 { margin: 0.1rem 0 0; font-size: clamp(1.06rem, 1.14vw, 1.24rem); color: var(--cs-ink); }
	.code-box { margin: 0.3rem 0 0; background: #10161b; color: var(--cs-paper-raised); border: 1px solid rgb(76 90 99 / 0.4); padding: 0.8rem; border-radius: 3px; font-size: clamp(0.75rem, 0.82vw, 0.86rem); line-height: 1.56; white-space: pre-wrap; word-break: break-word; }
	.desk-footer { width: min(92vw, 1720px); margin: 0 auto; padding: 0 1.5rem; text-align: center; color: var(--cs-paper); opacity: 0.85; font-size: 0.92rem; }
	@media (min-width: 1560px) {
		.hero { grid-template-columns: 1fr 484px; }
		.stage-card { width: 484px; padding: 1.15rem; }
		.stage-bar { font-size: 0.8rem; margin-bottom: 0.7rem; }
		.stage-flow { font-size: 0.72rem; }
		.stage-sheet-anim { height: 638px; display: flex; justify-content: center; }
		.stage-frame { transform: scale(1.14); transform-origin: top center; flex-shrink: 0; }
	}
	@media (max-width: 920px) {
		.hero { grid-template-columns: 1fr; }
		.stage-card { width: 100%; max-width: 420px; margin: 0 auto; }
	}
	@media (max-width: 520px) {
		.topbar-inner { width: 100%; padding: 0.5rem 0.75rem; gap: 0.45rem; }
		.brand code { display: none; }
		.topnav { width: 100%; justify-content: space-between; font-size: 0.82rem; order: 3; border-top: 1px solid rgb(76 90 99 / 0.28); padding-top: 0.35rem; gap: 0.35rem; }
		.desk-main { width: 100%; padding: 1rem 0.5rem; gap: 1.5rem; }
		.hero, .section-block { padding: 1.15rem 1.05rem; }
		.hero::before, .section-block::before { border-width: 12px; border-image-width: 12px; }
		h1 { font-size: 1.5rem; }
		h2 { font-size: 1.22rem; }
		.lead { font-size: 0.94rem; }
		.card-grid, .proof-strip { grid-template-columns: 1fr; }
		.stage-card { max-width: 100%; padding: 0.55rem 0.3rem; overflow: hidden; }
		.stage-bar { font-size: 0.68rem; flex-wrap: wrap; }
		.stage-sheet-anim { height: 442px; display: flex; justify-content: center; overflow: hidden; }
		.stage-frame { transform: scale(0.78); transform-origin: top center; flex-shrink: 0; }
	}
</style>
