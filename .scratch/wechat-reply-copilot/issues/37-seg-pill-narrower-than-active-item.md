# 分段控件的指示药丸在窄视口下比激活项窄，最后一个字挂到框外

- **状态**: done
- **标签**: ui, bug
- **发现时间**: 2026-10-15
- **修完**: 2026-10-15

## 根因（一句话）

`slideTo` 把「药丸还是初值 `left:0/width:0`」当成了「有位移」，
于是**挂载那一刻从 0 补间 240ms 到目标**——药丸是凭空长出来、不是滑动；
无头截图截到的就是这半截帧，于是看着比激活按钮窄。

**与视口宽度无关**（原判断「差异随视口宽度变」是巧合，见下）。

## 定位过程（可复现）

### 1. 先证伪「量错了」

用 CDP 连真实无头 Chromium（`Runtime.evaluate`，不用改源码）读同一页面的 DOM：

| 控件 | 槽 `offsetWidth` | 激活项 `offsetLeft/offsetWidth` | 药丸内联值 |
|---|---|---|---|
| 发送档位 | 141 | `2 / 61` | `width: 61px; left: 2px` |
| 标记形状 | 79 | `38 / 36` | `width: 36px; left: 38px` |
| 关闭窗口 | 129 | `2 / 49` | `width: 49px; left: 2px` |

⇒ 量出来的是**对的**。所以「药丸量到的值和 CSS 算出来的对不上」
（`left:1` vs 应为 `3`）这个前提本身是错的：`offsetLeft` 不含父级
`border`，槽 `padding: 2px` + `border: 1px` 时它就是 **2**，实测 2。

### 2. 再证伪「窄视口改了布局」

槽宽在 520 与 671 两个视口下都是 141/79/129，与视口无关；
`?demo=1` 的 `cs-demo-frame` 是内联 `width: 380px`，没被任何规则改掉。
**「差异随视口宽度变」是把两件事混了**：520 那次量的是截图（有偏差），
671 那次量的是 DOM（无偏差）。

### 3. 决定性证据：药丸宽度随截图预算单调变长

`scripts/shoturl.ps1 -Scale 1`（1 px = 1 CSS px），数药丸**上边线
那一行**（y=104）的墨线跨度（那一行是药丸顶边，两端被 4px 圆角切掉）：

| `--virtual-time-budget` | 药丸上边线跨度 | 药丸实际宽 | 目标（激活项） |
|---|---|---|---|
| 4000 | `x=122..157`（36） | 44 | 61 |
| 15000 | `x=122..157`（36） | 44 | 61 |
| **40000** | `x=123..168`（46） | **54** | 61 |

⇒ 随预算单调变长 = **补间在跑、没跑完**。三组同时被截在同一进度上
（另两组药丸实测 26/35 px，对 36/49 的比值同样是 ≈0.71）。

## 修法

`ui/src/lib/attach.ts` 的 `slideTo`：把「offset 全 0」判为**还没落位**，
直接 `gsap.set` 落位；之后的切换照常滑动。

```ts
const unplaced = pill.offsetLeft === 0 && pill.offsetWidth === 0;
if (unplaced || (Math.abs(left - pill.offsetLeft) < 0.5 && Math.abs(width - pill.offsetWidth) < 0.5)) {
  gsap.set(pill, { left, width });
  return;
}
```

挂载时没有「从哪来」，那段 240ms 本来就没有表达任何东西；
改完之后药丸从第一帧起就是对的（真机上也不再有一段 240ms 的错位）。

## 修好的证据（`-Scale 1` 出图，量像素）

同一 URL：`?demo=1&state=drafted&settings=1&shape=ring&size=big`，
520×760。出图见 `target/probe/seg37-after.png`（`target/` 已 gitignore，
重出命令见下）。

| 控件 | 药丸左右边线 | 激活项左右边线 | 对齐 |
|---|---|---|---|
| 发送档位「只填入」 | `x=119 / 179`（61） | `x=119 / 180`（61） | ✅ |
| 标记形状「圈」 | `x=155 / 190`（36） | `x=155 / 191`（36） | ✅ |
| 关闭窗口「退出」 | `x=119 / 167`（49） | `x=119 / 168`（49） | ✅ |

（修前分别是 43 / 26 / 35 px。）

重出命令：

```bash
pwsh -NoProfile -File scripts\shoturl.ps1 `
  -Url "http://localhost:1420/?demo=1&state=drafted&settings=1&shape=ring&size=big" `
  -OutPng "<abs>\target\probe\seg37-after.png" -Scale 1 -Wait 4000
```

⚠ `-OutPng` **要绝对路径**：相对路径那次没出图（`no screenshot produced`
而 exit=0），不是没生效，是 chrome 没写到那儿。

## 门禁

`svelte-autofixer` 0 issue 0 suggestion · `prettier --check` 干净 ·
`pnpm check` 0 errors 0 warnings（均 2026-10-15 实跑）。

## 遗留的判断，供下一个人参考

- 本票原先挂着的三条「已排除」（flex 收缩 / 字体就绪 / 动画时长）**其实
  都没排除掉真因**：它们量的是同一张截图，而截图一直在截半截帧。
  `flex: none` 与两条重量钩子仍然留着，但它们不是本票的解，
  理由写在代码注释里。
- `scripts/shoturl.ps1` 用 `--virtual-time-budget` 出图，**任何带补间的
  界面都可能截到中间帧**。这不是本票的缺陷，但下一次有人拿它验动画
  终点时会再踩一遍——值得考虑在脚本里加一条「补间跑完再截」的等待，
  **但那是 `scripts/` 的文件，不在本票所有权内，没有动**。
