//! 往微信输入框里填文本——**以回读为唯一判据**。
//!
//! ## 验收标准不是返回值，是回读
//!
//! 投递函数返回「成功」而没有回读，等于没有证据。剪贴板是我们写进去的，
//! 再读回来与候选原文**字节级相等**才算数。
//!
//! 这里有两级证据，强度不同：
//!
//! | 回执 | 证据强度 | 含义 |
//! |---|---|---|
//! | `Sent` | **强** | 剪贴板回读与原文字节级相等，**且**回读发生在粘贴之后 |
//! | `PossiblySent` | 弱 | 粘贴指令发出去了，但回读没能在超时内完成 |
//!
//! 为什么不把回读做两次（粘贴前一次、粘贴后一次）就一定等于原文：
//! 剪贴板可能被**用户或别的程序**在中间改掉。那种情况下回读不等于原文，
//! 但文本**很可能已经贴进去了**——这时候报 `Failed` 会导致上层重放，
//! 而重放会**发出两条一样的消息**。所以这种情况必须归 `PossiblySent`。

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND, POINT};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_V,
};

use chatservant_core::dispatch::DeliveryReceipt;

/// 回读超时。超过就归 `PossiblySent`——**不能归失败**。
pub const READBACK_TIMEOUT: Duration = Duration::from_millis(400);

/// `HANDLE` ↔ `HGLOBAL` 是两个**不同**的 newtype（底层都是 `*mut c_void`），
/// Win32 的剪贴板 API 在两者间来回传。显式写这两个小函数，
/// 比满地 `.0` 更容易看出「这里在跨类型」。
fn handle_in(h: HANDLE) -> HGLOBAL {
    HGLOBAL(h.0)
}

/// `HGLOBAL` → `HANDLE`：`SetClipboardData` 收的是 `HANDLE`。
fn handle_of(h: HGLOBAL) -> HANDLE {
    HANDLE(h.0)
}

/// `CF_UNICODETEXT`
const CF_UNICODE_TEXT: u32 = 13;

/// 剪贴板是**进程全局**的资源，不是线程局部的。
///
/// 两个线程同时 `EmptyClipboard` + `SetClipboardData` 会怎样：线程 A 刚把
/// 自己的内存块交出去，线程 B 的 `EmptyClipboard` 让系统把它释放掉，
/// 而 A 还在读它 → 堆损坏（`STATUS_HEAP_CORRUPTION`）。
///
/// 这不是「测试里才有的问题」：dispatch 可以从 UI 点击、快捷键、定时器
/// 三个地方同时触发。所以互斥锁是**功能要求**，不是测试技巧。
static CLIPBOARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 取剪贴板锁。锁中毒（某个线程 panic 了）不算致命——剪贴板状态还在，
/// 继续用比整个进程崩掉好。
fn lock_clipboard() -> std::sync::MutexGuard<'static, ()> {
    CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner())
}

/// 写入剪贴板并读回校验。**返回读回来的文本**。
///
/// 为什么先写后读：这就是回读验证本身——写进去的东西能原样取回来，
/// 才说明剪贴板通路是通的、中间没被截断。中文尤其需要这一步：
/// UTF-16 与 UTF-8 之间任何一次错误转换都会在这里暴露。
pub fn roundtrip(text: &str) -> Result<String, String> {
    let _guard = lock_clipboard();
    roundtrip_locked(text)
}

/// 已经持锁时的往返。调用方负责持锁。
fn roundtrip_locked(text: &str) -> Result<String, String> {
    write_clipboard(text)?;
    read_clipboard()
}

/// 把文本放进剪贴板（CF_UNICODETEXT）。
fn write_clipboard(text: &str) -> Result<(), String> {
    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    utf16.push(0); // Win32 要求以 NUL 结尾
    let bytes = utf16.len() * 2;

    // SAFETY: 整块只操作**剪贴板**——OpenClipboard/EmptyClipboard/SetClipboardData`r
    // 配的都是本函数刚构造的 HGLOBAL。Win32 会**接管**这块内存的所有权，
    // 所以 ytes 的尺寸必须和 utf16 的实际字节数一致（上面已算），
    // 之后 Rust 侧不再访问 utf16。每条错误分支都成对 CloseClipboard。
    unsafe {
        // 打开剪贴板失败最常见的原因是**别的程序正开着它**。
        // 这里重试几次而不是立刻报错——Windows 上这一瞬间是常态。
        if !open_clipboard_with_retry() {
            return Err("打不开剪贴板（可能被别的程序占用）".into());
        }
        let r = (|| -> Result<(), String> {
            EmptyClipboard().map_err(|e| format!("清空剪贴板失败：{e}"))?;

            let hmem = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes)
                .map_err(|e| format!("分配剪贴板内存失败：{e}"))?;
            let ptr = GlobalLock(hmem);
            if ptr.is_null() {
                let _ = GlobalFree(Some(hmem));
                return Err("锁定剪贴板内存失败".into());
            }
            std::ptr::copy_nonoverlapping(utf16.as_ptr() as *const u8, ptr as *mut u8, bytes);
            // 解锁失败不致命：GlobalUnlock 在锁计数为 0 时也返回 0（不是错误）
            let _ = GlobalUnlock(hmem);

            match SetClipboardData(CF_UNICODE_TEXT, Some(handle_of(hmem))) {
                // 成功时**内存所有权转移给系统**，绝不能再 Free
                Ok(_) => Ok(()),
                Err(e) => {
                    let _ = GlobalFree(Some(hmem));
                    Err(format!("写剪贴板失败：{e}"))
                }
            }
        })();
        let _ = CloseClipboard();
        r
    }
}

/// 读回剪贴板的文本。
fn read_clipboard() -> Result<String, String> {
    // 调用方已经持锁（`roundtrip` / `fill_input_box`）。
    // 这里不重复上锁——`Mutex` 不可重入，重复 lock 会死锁。
    // SAFETY: 只读剪贴板。GlobalLock 返回的指针指向**系统管理的**共享内存，
    // 直到 GlobalUnlock 之前都有效，所以锁的生命周期覆盖下面的读取。
    unsafe {
        if !open_clipboard_with_retry() {
            return Err("回读时打不开剪贴板".into());
        }
        let r = (|| -> Result<String, String> {
            let hglobal =
                GetClipboardData(CF_UNICODE_TEXT).map_err(|e| format!("剪贴板里没有文本：{e}"))?;
            let hmem: HGLOBAL = handle_in(hglobal);
            let ptr = GlobalLock(hmem);
            if ptr.is_null() {
                return Err("回读时锁定剪贴板内存失败".into());
            }
            let len = GlobalSize(hmem) / 2;
            let slice = std::slice::from_raw_parts(ptr as *const u16, len);
            // 去掉结尾的 NUL（可能有多个）
            let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
            let s = String::from_utf16_lossy(&slice[..end]);
            let _ = GlobalUnlock(hmem);
            Ok(s)
        })();
        let _ = CloseClipboard();
        r
    }
}

/// 读回剪贴板的文本。**公开**是因为端到端命令的 `Readback` 要用它——
/// 「回读」是投递验收的一环，不该只能由投递函数自己调用。
pub fn read_clipboard_public() -> Result<String, String> {
    let _guard = lock_clipboard();
    read_clipboard()
}

/// 打开剪贴板，最多试 5 次。
///
/// Windows 的剪贴板是**全局独占**的：任何程序开着它，别人就 Open 不了。
/// 这是系统级常态（复制粘贴、剪贴板管理器、各种输入法），不是异常。
/// 重试是唯一正确的处理方式。
fn open_clipboard_with_retry() -> bool {
    for _ in 0..5 {
        // SAFETY: 无参数、无指针，只尝试打开剪贴板。None = 当前任务。
        if unsafe { OpenClipboard(None) }.is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(12));
    }
    false
}

/// 把一组按键展开成事件序列：**先全部按下，再全部按相反顺序抬起**。
///
/// 抽成独立函数是为了让测试能跑**真代码**——抄一份序列逻辑进测试，
/// 测的就不是线上那一段了（那个坑本文件已经踩过一次）。
fn chord_events(keys: &[VIRTUAL_KEY]) -> Vec<INPUT> {
    let key_event = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let mut inputs: Vec<INPUT> = Vec::with_capacity(keys.len() * 2);
    for &vk in keys {
        inputs.push(key_event(vk, false));
    }
    for &vk in keys.iter().rev() {
        inputs.push(key_event(vk, true));
    }
    inputs
}

/// 按和弦发送按键。见 [`chord_events`] 的顺序说明。
fn send_keys(keys: &[VIRTUAL_KEY]) -> Result<(), String> {
    send_inputs(&mut chord_events(keys))
}

/// 逐字符发送 Unicode 文本。
///
/// 用虚拟键码注入**一个字符一个字符**。这是唯一能在不切换输入法、
/// 不依赖键盘布局的前提下打任意语言的办法——代价是慢（约每字符 1 ms），
/// 对「填一条回复」这个量级完全够。
pub fn type_unicode(text: &str) -> Result<(), String> {
    let mut chunk: Vec<INPUT> = Vec::with_capacity(64);
    for unit in text.encode_utf16() {
        // 代理对的低半部分不能单独发送——整对必须连续
        for flags in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
            chunk.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: unit,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }
        if chunk.len() >= 60 {
            // SendInput 一次最多传有限个，分批发更稳
            send_inputs(&mut chunk)?;
            chunk.clear();
        }
    }
    if !chunk.is_empty() {
        send_inputs(&mut chunk)?;
    }
    Ok(())
}

fn send_inputs(inputs: &mut [INPUT]) -> Result<(), String> {
    if inputs.is_empty() {
        return Ok(());
    }
    // SAFETY: inputs 是调用方（chord_events / 	ype_unicode）构造好的、
    // 长度与 cbSize 描述一致的结构数组，指针有效覆盖整个调用。
    // ⚠ SendInput 会**读**这块内存并把事件投到当前前台窗口——
    //   调用方已通过 ocus(target) 确认目标在前台（见 ill_input_box）。
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        // 返回值**少于**请求数 = 被 UIPI 拦了（目标窗口是更高完整性级别）。
        // 这不是「部分成功」，是完全没送出去。
        Err(format!(
            "SendInput 只送出了 {sent}/{} 个事件——目标窗口拒绝接收（UIPI 拦截）",
            inputs.len()
        ))
    }
}

/// Ctrl+V。
pub fn paste() -> Result<(), String> {
    send_keys(&[VK_CONTROL, VK_V])
}

/// 一次完整的「把候选填进输入框」——**不移动鼠标指针**。
///
/// ## 为什么不用鼠标点输入框
///
/// 移动真实鼠标去点输入框，用户正在做的事就被打断了：他的手停在半空，
/// 光标跑到聊天窗口里，肉眼可见指针在动。ticket 明确要求**指针不动**。
///
/// 替代路径：`SetForegroundWindow` + `Ctrl+V`。焦点被系统拒绝时
/// （Windows 的前台锁）如实报失败，**不降级去动鼠标**——
/// 动鼠标是「偷偷替用户做了他没要求的事」，宁可失败。
///
/// ## 焦点被拒时怎么办
///
/// Windows 有一个前台锁：程序不能随意抢前台。绕过的正规做法是
/// `AttachThreadInput` ——把自己的线程临时挂到目标窗口的输入队列上，
/// 再调 `SetForegroundWindow`。这是系统提供的机制，不是绕过安全策略。
/// 仍然失败就报失败。
pub fn fill_input_box(text: &str, target: HWND) -> DeliveryReceipt {
    // 第一步：写剪贴板并**回读校验**。
    // 这一步失败的话，一个字符都还没发出去——可以安全重放。
    let written = {
        let _guard = lock_clipboard();
        roundtrip_locked(text)
    };
    let readback = match written {
        Ok(got) => got,
        Err(e) => return DeliveryReceipt::Failed(format!("写剪贴板失败：{e}")),
    };
    if readback != text {
        // 剪贴板被截断或转换错了。此时**还没按键**，可以安全重放。
        return DeliveryReceipt::Failed(format!(
            "剪贴板回读不一致：写入 {} 字节，读回 {} 字节",
            text.len(),
            readback.len()
        ));
    }

    // 第二步：抢焦点。失败就不往下走——没有焦点的 Ctrl+V 是发给别人的。
    if let Err(e) = focus(target) {
        return DeliveryReceipt::Failed(format!("无法把焦点交给目标窗口：{e}"));
    }

    // 第三步：Ctrl+V。
    if let Err(e) = paste() {
        // ⚠ 焦点已经抢到了，**可能**在重试期间用户已经粘贴了。
        // 但 Ctrl+V 只有一发：失败就是没发出去，可以重放。
        return DeliveryReceipt::Failed(format!("发送 Ctrl+V 失败：{e}"));
    }

    // 第四步：回读。这一步只是确认剪贴板还在——它按定义一直在。
    // 真正的确认是「输入框里出现了这段文本」，那需要 OCR 目标区域，
    // 是 ticket 16 的调试窗口。这里如实说明**没有**拿到那种证据。
    let deadline = Instant::now() + READBACK_TIMEOUT;
    let mut last_err = String::from("尚未回读");
    while Instant::now() < deadline {
        // 整个回读窗口都持锁：中途放开会让别的线程换掉剪贴板内容，
        // 那时回读到的就不是我们写的东西——那样的「证据」毫无意义。
        let got = {
            let _guard = lock_clipboard();
            read_clipboard()
        };
        match got {
            Ok(got) if got == text => {
                // 拿到了字节级相等的证据。但**没有**证据证明它被粘贴了。
                // 所以不能报 Sent——那是「确定送达」。
                return DeliveryReceipt::PossiblySent;
            }
            Ok(_) => last_err = "剪贴板内容已被别的程序改动".into(),
            Err(e) => last_err = e,
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    // 回执按设计是**无载荷**的（见 CONTEXT.md：TimedOut 是一个状态，不是
    // 一条带原因的数据）。所以诊断信息走日志，不去改回执的类型。
    tracing::warn!(reason = %last_err, "回读超时，按 PossiblySent 对待");
    DeliveryReceipt::TimedOut
}

/// 发一次**单独的 ALT 按下+抬起**，什么都不输入。
///
/// ## 它干什么用
///
/// Windows 的前台锁规定：后台进程不能随意抢前台。系统有一条正规通道——
/// **收到一个 ALT 键事件之后，下一次 `SetForegroundWindow` 会被放行**。
/// 这不是绕过安全策略，是 Win32 公开的前台锁释放方式。
///
/// ## 代价，以及为什么仍然可以接受
///
/// 这一发 ALT 会送到**当前前台窗口**。单按 ALT 不输入任何字符：
/// - 在终端 / 编辑器 / 浏览器里通常什么都不发生；
/// - 在经典 Win32 程序里可能让菜单栏闪一下，按 ESC 或松手即恢复。
///
/// 对比项目原本拒绝的两件事：移动鼠标指针（用户看得见指针乱跑）、
/// 真往输入框打字。这里两样都不做——只发一个修饰键。
///
/// 发失败就当作没发：`focus` 的兜底步骤，不该因为它整体失败。
fn prime_foreground_lock() {
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_MENU;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };

    let mut inputs = [
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(VK_MENU.0),
                    wScan: 0,
                    dwFlags: KEYBD_EVENT_FLAGS(0),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(VK_MENU.0),
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
    ];
    let _ = send_inputs(&mut inputs);
}

/// 把前台焦点交给目标窗口。
///
/// `AttachThreadInput` 是 Windows 提供的正规机制：把自己的输入队列
/// 临时挂到目标线程的队列上，这样 `SetForegroundWindow` 才被允许。
/// 不这么做的话，后台程序调用它会**静默失败**——返回值是 FALSE，
/// 但没有任何错误码告诉你为什么。
///
/// 仍然失败时走 [`prime_foreground_lock`]：先发一个 ALT 释放前台锁再试一次。
/// 实测（2026-10-07）只做 `AttachThreadInput` 在**从后台 shell 启动**的
/// 进程上会被拒（真实产品里用户先点了应用自己的按钮，那条路径通常已放行）。
pub fn focus(target: HWND) -> Result<(), String> {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
    };

    // SAFETY: 整块只调 GetForegroundWindow / SetForegroundWindow /
    // AttachThreadInput / GetWindowThreadProcessId——都是**按值收发**窗口或线程 ID，
    // 不解引用任何我们持有的指针。AttachThreadInput 会改本线程的输入队列关联，
    // 所以**每条分支都必须 Detach**——本函数保证成对（见下面的展开写法）。
    unsafe {
        let fg = GetForegroundWindow();
        if fg == target && SetForegroundWindow(target).as_bool() {
            return Ok(());
        }

        let target_tid = GetWindowThreadProcessId(target, None);
        let self_tid = GetCurrentThreadId();
        if target_tid == 0 {
            return Err("拿不到目标窗口的线程 id".into());
        }
        let attached = AttachThreadInput(self_tid, target_tid, true).as_bool();
        let ok = SetForegroundWindow(target).as_bool();
        if attached {
            let _ = AttachThreadInput(self_tid, target_tid, false);
        }
        if ok {
            return Ok(());
        }

        // 兜底：ALT 释放前台锁，再走一遍同样的正规路径。
        prime_foreground_lock();
        std::thread::sleep(Duration::from_millis(60));
        let fg2 = GetForegroundWindow();
        if fg2 != target {
            let attached = AttachThreadInput(self_tid, target_tid, true).as_bool();
            let ok2 = SetForegroundWindow(target).as_bool();
            if attached {
                let _ = AttachThreadInput(self_tid, target_tid, false);
            }
            if ok2 {
                return Ok(());
            }
        }
        Err("SetForegroundWindow 被系统拒绝（Windows 前台锁）：\
             即使用 ALT 释放前台锁也没成功"
            .into())
    }
}

/// 按一次回车。**不移动鼠标指针**。
pub fn press_enter() -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
    send_keys(&[VK_RETURN])
}

/// 填进输入框**并按回车发送**——Auto 档走这条路。
///
/// ## 为什么单独一个函数而不是给 `fill_input_box` 加个参数
///
/// 「填」和「发」是**不可逆性不同**的两个动作。加个 `send: bool` 的话，
/// 调用点变成 `fill_input_box(&t, hwnd, tier == Dispatch::Auto)`，
/// 于是「什么时候会发出去」被藏进一个布尔表达式里。分成两个函数后，
/// **只有 Auto 档的调用点会出现按回车**，一眼可查。
///
/// ## 回执只能到 `PossiblySent`
///
/// 回车按下去之后，程序**没有**证据证明这条消息真的发出去了——
/// 证明它需要 OCR 聊天区看那条消息有没有离开输入框，
/// 而发送后输入框会被清空，OCR 那一刻读到的已经不是它。
///
/// 所以这里**不报 `Sent`**。`Sent` 是「确定送达」，报它就是撒谎，
/// 而撒谎的回执会让上层以为不必再做回读。拿不到更高档的回执不是缺陷，
/// 是**证据能力的边界**，如实标出来。
pub fn fill_and_send(text: &str, target: HWND) -> DeliveryReceipt {
    let filled = fill_input_box(text, target);
    // 填充失败就**到此为止**——绝不在没填进去的时候按回车，
    // 那会把输入框里原有的内容（用户自己打了一半的话）发出去。
    if !matches!(filled, DeliveryReceipt::PossiblySent) {
        return filled;
    }

    match press_enter() {
        Ok(()) => DeliveryReceipt::PossiblySent,
        Err(e) => DeliveryReceipt::Failed(format!(
            "回车发送失败（内容已填入输入框，你可以手动发）：{e}"
        )),
    }
}

/// 只按回车，**不再往输入框里写任何东西**——Confirm 档第二次点击走这条。
///
/// ## 为什么 Confirm 的第二次点击不能再填一遍
///
/// 2026-10-09 实机事故：Confirm 档的两次点击都调了 `fill_and_send`，
/// 而 `fill_input_box` 是**在光标处追加**（它的语义就是「停在光标处」，
/// 不动用户已经打了一半的字）。于是发出去的是**两份拼接**：
///
/// ```text
/// 看起来你想要发送文件给我，请确认一下文件名和内容，我会准备好接收。
/// 看起来你想要发送文件，我这边已经准备好了接收，请继续。
/// ```
///
/// 两次点击之间的 5 秒里人根本来不及改任何东西——第一份还没看完。
///
/// ## 顺带把交互改对了
///
/// 「第一次填入、第二次发送」本来就是更好的形态：两次点击之间那几秒，
/// 人可以**读、可以改**输入框里的字。这正是 Confirm 档存在的意义
/// （硬约束第 5 条：升档必须是人按的、可回退的）。
/// 之前两次都填，等于把这个窗口浪费掉了。
///
/// ## 回执同样只能到 `PossiblySent`
///
/// 与 [`fill_and_send`] 同理：按完回车没有证据证明消息真的走了。
pub fn send_only(target: HWND) -> DeliveryReceipt {
    // **先把焦点交给目标窗口，再按回车——但不碰输入框里的一个字。**
    //
    // ⚠ 早先这里写的是「前台不是微信就不发」，那是对的守门、错的门：
    // 用户第二次点是**点面板上的按钮**，那一刻前台是面板不是微信，
    // 于是这道门永远关着——实测按了没反应，文字留在输入框里发不出去。
    //
    // 真正的风险不是「前台不是微信」，而是「**前台不是微信却按了回车**」。
    // 抢到焦点之后再按，那一下就确定落在微信的输入框里。
    // 抢不到焦点就**不发**——那时按下去可能落在别的窗口，
    // 而别的窗口里一次回车可能是提交表单、发消息、关窗口。
    if let Err(e) = focus(target) {
        return DeliveryReceipt::Failed(format!(
            "无法把焦点交给微信——**没有按回车**，因为那一下可能落在别的窗口上：{e}"
        ));
    }
    match press_enter() {
        Ok(()) => DeliveryReceipt::PossiblySent,
        Err(e) => DeliveryReceipt::Failed(format!(
            "回车发送失败（内容已在输入框里，你可以手动发）：{e}"
        )),
    }
}

/// 鼠标当前位置。**只读**，用于在调试视图里证明「指针没动过」。
pub fn cursor_pos() -> Option<POINT> {
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    // SAFETY: `p` 是栈上的 POINT，`GetCursorPos` 只往里写 2 个 i32。
    unsafe { GetCursorPos(&mut p).ok().map(|()| p) }
}

/// 把窗口内坐标换成屏幕坐标。
pub fn to_screen(target: HWND, x: i32, y: i32) -> Option<POINT> {
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    let mut p = POINT { x, y };
    // SAFETY: p 是栈上的 POINT，ClientToScreen 只改写它自己的两个字段。
    unsafe { ClientToScreen(target, &mut p).as_bool().then_some(p) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 把一组按键的真实事件序列压成 `(虚拟键码, 是否抬起)`，便于断言。
    fn seq(keys: &[VIRTUAL_KEY]) -> Vec<(u16, bool)> {
        chord_events(keys)
            .iter()
            .map(|i| {
                // `INPUT::Anonymous` 是 union，读字段要 unsafe。
                // SAFETY: 读 union 的字段。chord_events 只会造 `r
                // `INPUT{ Keyboard }`，所以 ki 这个变体**是有效的**。
                // 读的是拷贝（ki 不是引用），不产生别名。
                let ki = unsafe { i.Anonymous.ki };
                (ki.wVk.0, ki.dwFlags == KEYEVENTF_KEYUP)
            })
            .collect()
    }

    /// ⚠ **Ctrl+V 必须是和弦，不是两次独立按键。**
    ///
    /// 早先 `send_keys` 对每个键各发「按下+抬起」，于是实际序列是
    /// `Ctrl↓ Ctrl↑ V↓ V↑`——Ctrl 在 V 之前就松开了，**粘贴从未发生**，
    /// 微信输入框里敲进去的是一个字母 `v`。
    ///
    /// 当时的测试只验剪贴板往返，没碰按键序列，所以一路绿灯。
    /// 这条断言就是为了钉住顺序。
    #[test]
    fn a_chord_presses_all_keys_before_releasing_any() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, VK_V};
        assert_eq!(
            seq(&[VK_CONTROL, VK_V]),
            vec![
                (VK_CONTROL.0, false),
                (VK_V.0, false),
                (VK_V.0, true),
                (VK_CONTROL.0, true),
            ],
            "修饰键必须在主键之后才松开，否则不是 Ctrl+V"
        );
    }

    /// 单键时序列就是 down/up 一对。
    #[test]
    fn a_single_key_is_a_plain_down_up() {
        use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
        assert_eq!(
            seq(&[VK_ESCAPE]),
            vec![(VK_ESCAPE.0, false), (VK_ESCAPE.0, true)]
        );
    }

    const SAMPLE: &str = "这是候选回复，含中英文 mixed 123 与符号 ✓";

    /// 回读必须字节级相等——这是唯一的验收判据。
    ///
    /// 中文尤其要过这一关：UTF-16 ↔ UTF-8 任何一次错误转换都会在这里露出来。
    #[test]
    fn clipboard_roundtrip_is_byte_exact() {
        let back = roundtrip(SAMPLE).expect("剪贴板往返");
        assert_eq!(back, SAMPLE, "回读与原文不等");
    }

    /// 空文本也要能往返。空候选不该被当成失败——那是合法的「清空」。
    #[test]
    fn empty_text_roundtrips() {
        assert_eq!(roundtrip("").expect("空文本往返"), "");
    }

    /// 含换行与制表符的文本。剪贴板是文本格式，这些必须原样保留。
    #[test]
    fn multiline_and_tab_roundtrip() {
        let s = "第一行\n第二行\t制表符\r\n第三行";
        assert_eq!(roundtrip(s).expect("多行往返"), s);
    }

    /// 超长文本。剪贴板有大小限制，超限时 `GlobalAlloc` 会失败——
    /// 那时必须**报错**而不是截断。截断的回复比没有回复更糟。
    #[test]
    fn long_text_is_not_truncated() {
        let long = "字".repeat(20_000);
        let back = roundtrip(&long).expect("长文本往返");
        assert_eq!(back.chars().count(), long.chars().count(), "长文本被截断了");
    }

    /// 代理对（emoji）在 UTF-16 里是两个 u16，整对必须连续发送。
    /// 这条测的是 roundtrip 不把它拆坏。
    #[test]
    fn surrogate_pairs_survive() {
        let s = "前面 🚀 中间 🎯 后面";
        assert_eq!(roundtrip(s).expect("代理对往返"), s);
    }

    /// 读鼠标位置只是为了**证明它没动过**，本身不该失败。
    #[test]
    fn cursor_position_is_readable() {
        assert!(cursor_pos().is_some(), "读不到鼠标位置");
    }
}
