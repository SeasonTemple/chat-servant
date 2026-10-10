# 17: 设置窗口停在离屏，面板进不去设置

**What to fix:** `settings` 窗口被创建了，但停在离屏位置且不可见。

**Status:** ready-for-agent

## 症状（2026-10-11 实机枚举该进程**全部**顶层窗口，含不可见的）

```
0xB11EC  vis=True   Tauri Window  '副驾'      380x560  @ (312,312)
0x70C70  vis=False  Tauri Window  '识别画布'  776x759  @ (104,104)
0x30E3E  vis=False  Tauri Window  '设置'      656x559  @ (-32000,-32000)
```

`tauri.conf.json` 里 `settings` 写的是 `"visible": true`。
`(-32000,-32000)` 是 Windows **最小化**窗口的标准位置。
同一份配置里 `识别画布`（`visible: false`）停在该停的地方，
主面板正常可见——**所以不是「没建」，是「建了之后被藏起来了」**。

## 为什么这条单独开票

`scripts/winscan.ps1` 只报可见窗口，于是「窗口没被创建」和
「窗口被藏起来了」在诊断上长得一模一样。`hittrace.ps1` 也只列
**盖住某一点**的窗口，不是完整枚举。**两者都会把人引向错误的根因。**

## 验收项

- [ ] `scripts/winscan.ps1` 能报出不可见的本进程窗口，或旁边放一句
      「本脚本只报可见窗口；要看隐藏的用 `target/probe/pidwins.ps1`」
      ——诊断脚本本身不许把人带偏
- [ ] 启动后设置窗口可见，或在设置不可达时**面板上有明确提示**，
      而不是让用户去找一个看不见的窗口
- [ ] 复现一次：是**稳定停在离屏**，还是**被最小化**。
      ⚠ 未定位前不要写「已修」，两种根因的处理方式完全不同
- [ ] `open_settings` 之后窗口真的可见（截图证据）

## 待查的方向（按怀疑度排序）

1. `open_settings` 里调了 `show()` 但没调 `unminimize()` ——
   `show_panel` 就同时调了 `show` + `unminimize` + `set_focus`，
   设置窗口这条路径**没有** `unminimize`
2. Tauri 2 在 `visible: true` 的窗口创建后又被某个路径最小化
3. `focus: false` + 无边框组合下的初始化时序

- [ ] 三条方向逐条排除，并写明是哪一条