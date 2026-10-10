# 19: 10 个文件里有 14 处不可逆的乱码

**What to fix:** 把源码与文档里被永久损坏的中文字符修回去，或在确认无法复原时改成如实描述。

**Status:** ready-for-agent

## 症状

全仓扫 `U+FFFD`（Unicode 替换字符，编码损坏后**无法还原**的标记），
命中 10 个文件。成因是 §11.7 记的那个坑：PS 5.1 的
`(Get-Content -Raw) -replace … | Set-Content -Encoding UTF8`
做机械替换，UTF-8 被按 ANSI 读回再写。

⚠ 这些位置**编译得过、测试也跑得过**——受损的全是注释与文档。
所以门禁永远不会报它们，只能靠扫 `U+FFFD`。

## 为什么单独立票而不直接改

字符已经丢了，**原字无法从文件里恢复**。只能靠上下文猜，
而猜出来的字写进文档比留着可见的损坏更危险：
读者分不清「原文如此」还是「有人补的」。

所以每处都给出**候选复原 + 把握程度**，由人确认。

## 清单

| 文件 | 行 | 现状 | 候选复原 | 把握 |
|---|---|---|---|---|
| `crates/core/src/frame.rs` | 4 | `//! ��那条路径要打���提示。` | 行首可能是「但」或无；「打___提示」→「打印提示」 | 低 / 中 |
| `ui/src/routes/+page.svelte` | 812 | `把��拿掉等于让他凭记忆去撤回一句他看不见的话。` | 「把**证据**拿掉」？ | 低 |
| `xtask/src/e2e.rs` | 31 | `**报���时按它定位。**` | 「**报错时**按它定位。」 | **高**（只有「错」通顺） |
| `xtask/src/main.rs` | 1275 | `// 混�� DPI 下……` | 「**混合** DPI」 | **高** |
| `xtask/src/main.rs` | 1330 | `// …对本机对所有窗口一��失败…` | 「**一律**失败」 | 中 |
| `docs/adr/0002-frame-conditioning-pipeline.md` | 36 | `由���得到本 ADR 的核心结论：` | 「由**实测**得到」 | 中高 |
| `.scratch/…/07-conditioning-mask-and-scale.md` | 257 | `「置���」是**结构事实**` | 无法从上下文确定 | 低 |
| `crates/perceive/src/fixture.rs` | — | 待定位 | — | — |
| `crates/perceive/src/identity.rs` | — | 待定位 | — | — |
| `crates/perceive/src/pipeline.rs` | — | 待定位 | — | — |
| `crates/reason/src/model.rs` | — | 待定位 | — | — |

后四项**行号待补**（本轮正在被文档补齐任务改动，改完再定位）。

## 验收项

- [ ] 全仓扫 `U+FFFD` 输出为空。给出这条命令：

  ```powershell
  cd E:\devSpace\chat-servant-m31f
  Get-ChildItem -Recurse -Include *.rs,*.svelte,*.ts,*.md -File |
    Where-Object { $_.FullName -notmatch '\\target\\|node_modules|\.svelte-kit' } |
    ForEach-Object { $c = [IO.File]::ReadAllText($_.FullName); if ($c -match "\uFFFD") { $_.FullName } }
  ```

- [ ] 每处要么按确认过的原文复原，要么改成**如实描述**（例：
  「此处原有一句关于 X 的说明，字符已损坏，无法复原」），**不许留半句**
- [ ] 复原过的行在同一个 commit 里注明「按上下文复原，非原文」——
  下一个读代码的人有权知道这句话不是当年写的
- [ ] ⚠ **这条不许靠 `scripts/fix-mojibake-names.ps1` 修**。那个脚本处理的是
      **文件名**乱码，靠 `core.quotepath` 解码；这里坏的是**文件内容**，
      两者机制不同、后果也不同（后者不可逆）
- [ ] 顺手把这条扫描加进 `gate.ps1` —— 注释损坏能静默通过十几年，
      靠人想起来扫是不现实的