# 99: 「关闭时」那个设置只写不读 —— 选哪一档都关不掉面板

**Status:** 5/5 · 2026-11-01 code landed + guarded（`CloseRequested` 先读偏好），并在真二进制上验完两档（`WM_CLOSE` 代替手按 `Alt+F4`，分岔点相同）

## 一、事实（怎么发现的）

2026-11-01 核 `docs/本机验收清单.md` 第 2 节「窗口出口」时，`rg 'CloseRequested|close_behavior|CloseBehavior' ui/src-tauri/src/*.rs` 一次零命中——
那不像「没实现」，像「用的不是这几个词」。换路径读 `ui/src-tauri/src/main.rs:85` 起那段 `on_window_event`，看到的是：

```rust
if let WindowEvent::CloseRequested { api, .. } = event {
    match label.as_str() {
        "debug" => {}
        _ => {
            let _ = commands::hide_panel(app);
            api.prevent_close();
        }
    }
}
```

**它不读任何偏好。** 而设置页「窗口 → 按窗口控制键时：退出 / 收进托盘」那一档、
`get_close_behavior` / `set_close_behavior` 两个命令、`%APPDATA%\dev.chatservant.app\close_behavior.txt`
那个文件——全都在，只是**没有任何关闭路径消费它**。

⇒ 后果是一句**假承诺**：设置页写着「按 × 直接退出。程序不会在后台继续跑。」
（`ui/src/lib/PanelSettings.svelte:460`），而实际两档结果一样，`Alt+F4` / 系统关闭一律变成「收进托盘」。

## 二、这不是回归

- `git log -S 'api.prevent_close' --all` 只有一条：`05521b6`（当初加拦截那次）。
- `git log -S 'read_close_behavior' -- ui/src-tauri/src/main.rs` **零命中** ⇒ 这个偏好从落地那天起就没人读过。
- `.scratch/wechat-reply-copilot/issues/66-*.md`（done，9/9）里明写「「关闭窗口时怎么办」是**偏好**」，
  那一轮只改了 `−` 的显形判据（`trayReady`），没碰关闭路径——缺口就是那时留下的。

## 三、处置（2026-11-01）

**让 `CloseRequested` 真的读偏好**，而不是删掉这个设置项。理由：`docs/面板设计.md:132` 记着
原始意图就是「退出（默认）/ 收进托盘」，同段还写「一颗不能有两种含义：× 永远退出，— 永远收起」
——删掉设置项会把 `×` 与「收进托盘」之间那层语义一起删掉，而用户要的正是能选。

代码：
- `ui/src-tauri/src/commands.rs`：新增 `pub enum CloseAction { Quit, Hide }`、纯函数 `close_action_from(CloseBehavior)`、
  以及 `pub(crate) fn close_action(app: &AppHandle) -> CloseAction`（就是 `close_action_from(read_close_behavior(app))`）。
  判据落在 commands 而不是 main，是因为**纯函数可测、`AppHandle` 不可测**。
- `ui/src-tauri/src/main.rs:100-110`：主面板那支改成
  `if commands::close_action(&app) == commands::CloseAction::Quit { quit_app } else { hide_panel + prevent_close }`。
  `quit_app` 走 `app.exit(0)`，窗口随进程一起没了 ⇒ 那条分支不需要也不该再 `prevent_close`。
- `read_close_behavior` 已经把「选了 tray 但托盘没了」折回 `Exit`（`ui/src-tauri/src/commands.rs:994`）
  ⇒ 返回 `Hide` 时 `hide_panel` 的前置条件一定成立，不需要第二道判断。

文案：
- `ui/src/lib/PanelSettings.svelte:461` 两支提示改成说清「系统关闭（Alt+F4）也跟着这一档」。
- `ui/src/lib/PanelSettings.svelte:151-157` 的 Props 注释删掉「`−` 就是靠它显形」（那是假的，`−` 靠 `trayReady`），
  改成「它只管系统关闭请求」。
- `ui/src/lib/PaperPanel.svelte:1133` 那颗 `×` 的 tooltip 不再随偏好变——`×` 永远退出，两档下文案本来就该一样。

## 四、守卫（两条，各量一半）

| 守卫 | 位置 | 量什么 |
|---|---|---|
| `close_action_follows_the_preference` | `ui/src-tauri/src/commands.rs` | 偏好与动作一一对应，没有第三支把两档并成一处 |
| `the_close_request_reads_the_preference_before_hiding` | `ui/src-tauri/src/main.rs` | **接线**：`CloseRequested` 分支里读了偏好、有真退出那支、且 `prevent_close` 在读之后 |

第二条必须是源码扫描：判据本体由第一条量，可它证明不了「这里有没有调它」——`AppHandle` 起不来，闭包也没法单独执行。

**量具自检（已做）**：把 `if commands::close_action(&app) == commands::CloseAction::Quit {` 换成 `if false {`
⇒ `the_close_request_reads_the_preference_before_hiding` **红**（`test result: FAILED. 0 passed; 1 failed; 132 filtered out`）；
写回 ⇒ 绿。`cargo test -p chat-servant-app` 全量 **133 passed / 0 failed**。

## 五、验收

- [x] `CloseRequested` 在两档下行为不同（`Exit` → `quit_app`，`Tray` → `hide_panel` + `prevent_close`）
- [x] 判据是纯函数、可单测（`close_action_from`）
- [x] 接线有源码守卫，且自证过会红
- [x] 设置页文案不再承诺做不到的事（`Alt+F4` 现在跟着那一档走）
- [x] **真机验过两档**（2026-11-01，真二进制 + 真窗口，没有人点）：
      `Exit` 档 `action=Quit` ⇒ 窗口被销毁（`IsWindow=False`）、进程 exit code **0**；
      `Tray` 档 `action=Hide` ⇒ 窗口还在但不可见（`IsWindow=True` / `IsVisible=False`）、
      进程活着且观察循环继续走 tick。
      量法：`target/probe/_wmclose2.ps1`（gitignored）启动 `target/debug/chat-servant-app.exe`，
      向主窗口发 `WM_CLOSE`（0x0010），再用 `IsWindow` / `IsWindowVisible` / `HasExited` 取证。
      ⚠ **用的是 `WM_CLOSE`，不是真的按 `Alt+F4`**：系统快捷键走的也是
      `WM_SYSCOMMAND` ⇒ `WM_CLOSE` ⇒ tao 的 `CloseRequested`，分岔点在同一个地方；
      不同的只是「键盘那一段路」，而它不属于本仓代码。
      ⚠ 取证前必须**重建过** `target/debug/chat-servant-app.exe`：本轮第一次跑的是
      一份早于本修的旧二进制（表现正好是**旧行为**：窗口隐藏、进程不退）
      ⇒ 差点把修好的行为当成没修好，就是「拿陈旧产物去验新代码」这个老坑。
      → 同时给 `main.rs` 的分支补了一行 `tracing::info!`（`target: "chat_servant_app::close"`，
      打 `label` + `action`）：这条分支之前在日志里**一个字都没有**，
      所以上面那个差点只能靠猜。

## 六、口径变化（本票之后）

`docs/本机验收清单.md` 第 2 节原写「主面板选 `Alt+F4` → **面板关不掉**，只能从托盘退」，
那在当时是**对的**实测（`ALT_F4_BLOCKED`），但它描述的是**缺陷态**。本票起该行拆成两行：
`Exit` 档应真退、`Tray` 档才只是收起来。同一节「点 `×`」与「`−` 只在『收进托盘』档出现」两行也一并订正
（`−` 的判据一直是 `trayReady`，与偏好无关，票 66 就改了，只是清单没跟上）。
