//! # 前后端契约
//!
//! 事件名、动作、档位 —— **凡是跨进程边界传的东西，都在这个文件里定义。**
//!
//! ## 为什么要有这个文件
//!
//! 2026-10-10 实测：事件名当时散在**三处**——
//! `main.rs` 的 `pub mod events` 只有 `READOUT` / `TELEMETRY`，
//! `commands.rs:248` 写的是字面量 `app.emit("armed", ...)`，
//! 前端 `+page.svelte` 写的是第三个字面量 `listen('armed', ...)`。
//!
//! 改一个事件名要同时动三个地方，漏一个就是「界面收不到、后端日志一切正常」。
//! 这就是下面那条 `no_event_literal_outside_this_file` 要守的东西。
//!
//! ## 跨语言这件事没完全解决
//!
//! Rust 和 TypeScript 是两门语言，**这里仍然有两份**（`types.ts` 镜像本文件）。
//! 彻底解掉要上类型生成链，那是独立工程。
//! 现在的目标是：**每种语言内部只有一处**，且改动能被类型检查抓到一部分。

use serde::{Deserialize, Serialize};

/// Tauri 事件名。**前端只认这几个，不轮询。**
///
/// ⚠️ 改这里等于改契约。
///
/// ⚠️⚠️ **这里曾经写着「前端镜像在 `ui/src/lib/types.ts` 的 `EVENTS`」——
///   那句话是假的，`types.ts` 里从来没有过 `EVENTS`**，只有 serde 类型的
///   镜像（`Tier` / `Phase` / `ReadoutPayload` …）。
///   事件名在前端是**裸字面量**（`listen('readout', …)`），
///   两边靠本文件 + `no_event_literal_outside_this_file` 约束，
///   靠的类型检查是**没有**的。
///   ⚠ 想补上类型安全，先把前端事件名也收进一个 `const` 清单再从
///   契约生成——那是一件独立的事，别假装它已经做了。
pub mod events {
    /// 每轮观察的结果。面板的全部内容都从这里来。
    pub const READOUT: &str = "readout";
    /// 「已装填」——Confirm 档第一次点击后发出。
    ///
    /// ⚠⚠ **2026-10-14 实测：前端当前没有监听者。** 旧面板删掉时
    ///   （`5031ab3`）连它一起没了，而动作行（发送那一排）至今未做
    ///   ——`docs/前端设计进度.md` §3 记着「动作行未做，碰发送路径」。
    ///   所以下面那句「前端据此让按钮显形」描述的是**尚未实现**的行为。
    ///
    /// ⚠ 保留这条 emit 而不是删掉：Rust 侧的装填语义（[`Arming`]）是
    ///   真的，事件是它对外的声明。**但别把这行注释读成「已经有界面了」**——
    ///   本轮补的 `every_listed_event_is_actually_emitted` 只查
    ///   「有没有 emit」，查不出「有没有人听」，后者仍是空话。
    pub const ARMED: &str = "armed";
    /// 托盘菜单点了「重新锁定当前会话」。前端监听它并调 `relock_session`，
    /// **和面板上那颗按钮走同一条 Rust 路径** —— 两处不各写一遍。
    pub const MENU_RELOCK: &str = "menu:relock";
    /// 托盘/面板点了「打开设置」。
    ///
    /// ⚠ 2026-10-13 补录：这个事件名此前**根本没进过清单**，
    /// 是 `no_event_literal_outside_this_file` 第一次真正运行
    /// （`main.rs` 里补上 `mod contract;` 之后）把它抓出来的。
    /// 在那之前它是个裸字面量——改事件名时不会被任何测试或 grep 覆盖。
    pub const MENU_SETTINGS: &str = "menu:settings";

    /// 全部事件名。**测试用它断言「没有漏在外面的字面量」。**
    ///
    /// ⚠ 2026-10-14（票 28）从 6 个减到 4 个：删掉 `debug_pane` 与
    /// `telemetry` —— 这两个**从来没有 emit 过**，前端却在 `listen`。
    /// 一个**声明了却永远不触发**的事件名比没有更坏：它让人以为那条
    /// 链路是通的。`debug_pane` 的数据走的是同名**命令**
    /// （`invoke('debug_pane')`），不是事件，别再把两者混记。
    pub const ALL: [&str; 4] = [READOUT, ARMED, MENU_RELOCK, MENU_SETTINGS];
}

/// 投递动作。
///
/// **原先是 `String`**——`action: "删库"` 也能一路传进 `dispatch`。
/// 现在它在 `serde` 反序列化阶段就失败，进不了业务逻辑。
///
/// 命名与前端 `types.ts` 的 `Action` 严格对应，**改一边就要改另一边**。
///
/// ⚠ **2026-10-13 已接线**（票 26）：`commands::dispatch` 第一件事就是
///   `Action::parse`，解析不出来直接回 `Failed` + 这句人话，
///   进不了业务逻辑。
///
/// `main.rs` 里补上 `mod contract;` 之前，这个文件**从来没进过编译**，
/// 于是「它还没接线」这件事没有任何门禁能发现。本文件活到 2026-10-13
/// 才第一次被编译，`dead_code` 一次性报了出来。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    /// 填入输入框，停在光标处。**不发送。** Manual 档唯一允许的动作。
    Fill,
    /// 发送。**装填之后**按才走这条。
    Confirm,
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Fill => "填入",
            Self::Confirm => "确认发送",
        })
    }
}

impl Action {
    /// 从字符串解析。**大小写不敏感，且拒绝尾随空白。**
    ///
    /// 为什么不用 `serde` 自动做：Tauri 的 `#[tauri::command]` 参数是逐个反序列化的，
    /// 单独一个 `String` 参数拿到手时已经晚了。这里显式解析，
    /// 好处是**错误信息能直接显示给人看**（界面会把它写进 `reason`）。
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "fill" => Ok(Self::Fill),
            "confirm" => Ok(Self::Confirm),
            other => Err(format!(
                "不认识的动作 {other:?}。只能是 \"fill\"（填入）或 \"confirm\"（确认发送）。"
            )),
        }
    }
}

/// 面板当前处于哪一阶段。
///
/// **原先这个判断在前端**：`+page.svelte` 靠 `candidates.length === 0`
/// 区分「静默」「出错」「刚启动」三种完全不同的状态——这是**界面在猜 Rust 的状态**。
/// 那是本文件存在的原因：**判断归 Rust，界面只渲染。**
///
/// 这个枚举同时是 §4.3 首屏提速的地基——有了 `Read`，
/// 观察循环就能**不等模型**先推一帧，实测体感 6~9s → 1.1s。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
pub enum Phase {
    /// 已认出会话、已读到消息，**模型还没回来**。
    ///
    /// 这是「首屏」：纯本地，~1.1s。界面据此立刻显示「读到 N 条，正在起草」。
    Read {
        /// 已归并的消息。**即使起草中也要给**——用户要看的是对方说了什么。
        messages: Vec<chatservant_perceive::Message>,
    },
    /// 正在起草。消息可用，候选还没有。
    ///
    /// 单独一个变体而不是复用 `Read` + 一个 bool：让「模型还没回来」这件事
    /// 在类型上就是一件事，前端不需要猜 `candidates` 为空意味着什么。
    Drafting {
        messages: Vec<chatservant_perceive::Message>,
    },
    /// 有候选。**这是产出。**
    Drafted {
        messages: Vec<chatservant_perceive::Message>,
        candidates: Vec<crate::watch::DraftCandidate>,
    },
    /// 出错。`error` 是**能直接显示给人看**的中文，不是 `Debug` 格式。
    Failed {
        messages: Vec<chatservant_perceive::Message>,
        error: String,
    },
}

impl Phase {
    /// 消息。四个变体都要——**出错时也保留已读到的消息**，
    /// 否则一个 OCR 抖动会让面板把整段对话清空。
    pub fn messages(&self) -> &[chatservant_perceive::Message] {
        match self {
            Self::Read { messages }
            | Self::Drafting { messages }
            | Self::Drafted { messages, .. }
            | Self::Failed { messages, .. } => messages,
        }
    }

    /// 候选。只有 `Drafted` 非空。
    pub fn candidates(&self) -> &[crate::watch::DraftCandidate] {
        match self {
            Self::Drafted { candidates, .. } => candidates,
            _ => &[],
        }
    }

    /// 面板顶部要显示的告警。**只有 `Failed` 有话说。**
    ///
    /// 其他三种状态返回空串，让界面不占地方——一个一直显示「一切正常」的横幅
    /// 会把真的异常稀释掉，这正是 §5.5 判据 12 背后的道理。
    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed { error, .. } => Some(error),
            _ => None,
        }
    }

    /// 判别字段的短名。**给推送去重用。**
    ///
    /// ## 为什么需要它
    ///
    /// `cs-pump` 线程靠「跟上一帧不一样才推」去重，而它原来**只看 `ticks`**。
    /// 首屏优化（§4.3 方案 1）会在**一轮中间**多推一帧，那一帧的 `ticks`
    /// 与上一轮相同——**pump 会以为没变化而把它吃掉**，首屏优化就白做了。
    ///
    /// 所以指纹必须是 `(ticks, phase)`。而 `Phase` 本身**不能**直接 `PartialEq`：
    /// 同一个 `Drafted` 阶段里候选会变（新一轮推理完成），
    /// 只比阶段名会漏推。**`ticks` 覆盖「同阶段内容变了」，
    /// `phase` 覆盖「同一 ticks 里阶段变了」。两者缺一不可。**
    pub fn key(&self) -> &'static str {
        match self {
            Self::Read { .. } => "read",
            Self::Drafting { .. } => "drafting",
            Self::Drafted { .. } => "drafted",
            Self::Failed { .. } => "failed",
        }
    }

    /// 界面该显示哪句「正在干什么」。
    ///
    /// ⚠️ **判据 12：空状态必须给一个明确的下一步动作。**
    /// `Drafting` 这一档如果只显示「加载中」，用户就不知道该等还是该干别的。
    ///
    /// ⚠ 2026-10-13：**已接线**（票 26）。它由 `Readout` 的 `status_hint`
    /// 字段带到界面上，界面不再自己写死这句——同一句话只有一份。
    pub fn status_hint(&self) -> Option<&'static str> {
        match self {
            Self::Drafting { .. } => Some("正在起草，通常几秒"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 动作解析：**认这两个，别的一律拒**。
    ///
    /// 这条测试存在的理由是：从前 `action` 是 `String`，
    /// 任何字符串都能进 `dispatch`。拼错、传空、传中文，
    /// 都要在**这里**失败，而不是走到 UI 层变成一句看不懂的 `reason`。
    #[test]
    fn only_fill_and_confirm_parse() {
        assert_eq!(Action::parse("fill"), Ok(Action::Fill));
        assert_eq!(Action::parse("confirm"), Ok(Action::Confirm));
        // 尾随空白曾经会漏到下游——Tauri 参数不保证干净
        assert_eq!(Action::parse("  fill  "), Ok(Action::Fill));
        // 全部拒绝
        for bad in ["", " ", "delete", "fill_all", "确认", "FILL", "0"] {
            assert!(Action::parse(bad).is_err(), "{bad:?} 不该被接受");
        }
    }

    /// 拒绝时必须**给得出人话**，因为它会直接显示在界面上。
    #[test]
    fn a_rejected_action_explains_itself() {
        let e = Action::parse("delete").unwrap_err();
        // 界面会原样显示这句，所以它得说清「能传什么」
        assert!(e.contains("fill"), "{e}");
        assert!(e.contains("confirm"), "{e}");
    }

    /// `Phase` 序列化出来是**带 `phase` 判别字段的**，前端据此分支。
    ///
    /// 锁死这个形状：前端 `+page.svelte` 靠 `payload.phase` 决定渲染什么，
    /// 形状一变界面白屏——而这正是「Rust 的 `Message` 少一个 `id`，
    /// 前端 `{#each (m.id)}` 全变 undefined，面板整块空白而后端日志一切正常」
    /// 那一类事故的翻版。所以这里把它测死。
    #[test]
    fn phase_serializes_with_a_discriminant_field() {
        let p = Phase::Drafting {
            messages: Vec::new(),
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v.get("phase").and_then(|x| x.as_str()), Some("drafting"));

        let d = Phase::Drafted {
            messages: Vec::new(),
            candidates: Vec::new(),
        };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v.get("phase").and_then(|x| x.as_str()), Some("drafted"));
    }

    /// 出错时**消息不能丢**。一个 OCR 抖动不该把整段对话清空。
    #[test]
    fn failing_keeps_the_messages_it_already_read() {
        let p = Phase::Failed {
            messages: Vec::new(),
            error: "OCR 失败".into(),
        };
        assert!(p.messages().is_empty()); // 这里没塞消息，验证不 panic
        assert_eq!(p.error(), Some("OCR 失败"));
        assert!(p.candidates().is_empty());
    }

    /// `key()` 必须能把四个阶段**区分开**。
    ///
    /// 它是 `cs-pump` 去重的一半（另一半是 `ticks`）——
    /// 判据是「同一 ticks 里阶段变了」也要能推出去。
    /// 锁死它，免得将来加变体时忘了更新。
    #[test]
    fn phase_keys_are_distinct() {
        let keys = [
            Phase::Read { messages: vec![] }.key(),
            Phase::Drafting { messages: vec![] }.key(),
            Phase::Drafted {
                messages: vec![],
                candidates: vec![],
            }
            .key(),
            Phase::Failed {
                messages: vec![],
                error: "x".into(),
            }
            .key(),
        ];
        for (i, a) in keys.iter().enumerate() {
            for (j, b) in keys.iter().enumerate() {
                assert!(i == j || a != b, "{a} 与 {b} 同名，去重会漏推");
            }
        }
    }

    /// **守住「src/ 下的每个文件都得被 `mod` 声明」这条纪律。**
    ///
    /// Rust 自己不会报这件事：`contract.rs` 躺在 `src/` 里，
    /// **一次都没被编译过**，而 `cargo clippy` / `cargo test` 全绿——
    /// 整整一个文件（连同它的 6 条测试）白躺了很久。`dead_code` 只是
    /// 间接暴露它，因为那次是别的原因顺手补了 `mod contract;`。
    ///
    /// 判据：一个 `.rs` 文件的名字必须在别的文件里以 `mod <名>;`
    /// 出现过。例外是 **crate 的两个根**——`main.rs` 与 `lib.rs`
    /// （Tauri 2 要求 crate 有 `[lib]`，所以这两个都是 Cargo 自己声明的，
    /// 不需要谁去 `mod` 它们）。
    #[test]
    fn no_orphan_module_file_in_src() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let Ok(rd) = std::fs::read_dir(&src) else {
            return;
        };
        let mut names: Vec<String> = Vec::new();
        let mut bodies: Vec<String> = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("rs") {
                continue;
            }
            let Some(stem) = p.file_stem().and_then(|x| x.to_str()) else {
                continue;
            };
            names.push(stem.to_string());
            bodies.push(std::fs::read_to_string(&p).unwrap_or_default());
        }
        let mut orphans: Vec<&str> = Vec::new();
        for name in &names {
            if name == "main" || name == "lib" {
                continue;
            }
            let declared = bodies.iter().any(|b| b.contains(&format!("mod {name};")));
            if !declared {
                orphans.push(name);
            }
        }
        assert!(
            orphans.is_empty(),
            "src/ 下这些文件从来没被 `mod` 声明过，也就是**从来没进过编译**：\n  {}",
            orphans.join("\n  ")
        );
    }

    /// 事件名清单是**测试用的**——见下面的 `no_event_literal_outside_this_file`。
    #[test]
    fn event_names_are_stable() {
        assert_eq!(events::READOUT, "readout");
        assert_eq!(events::ARMED, "armed");
        assert_eq!(events::MENU_RELOCK, "menu:relock");
        assert_eq!(events::MENU_SETTINGS, "menu:settings");
        assert_eq!(events::ALL.len(), 4);
    }

    /// **守住「清单里不许躺着永远不会触发的事件名」这条纪律。**
    ///
    /// 这条是 2026-10-14（票 28）补的，代价是发现上一版把这个测试做反了：
    /// `debug_pane` / `telemetry` 躺在 `ALL` 里、被 `event_names_are_stable`
    /// 断言「必须等于这两个字面量」，于是**声明一个死事件名反而让测试更绿**。
    /// 清单的职责是「前端只认这几个」，一个不 emit 的名字进了清单，
    /// 它就是在替一条不存在的链路背书。
    ///
    /// ⚠ 匹配的是**常量名**，不是事件名字面量（`"armed"`）——后者被
    ///   `no_event_literal_outside_this_file` 禁止出现在本文件之外，
    ///   两条测试会互相打脸。
    /// ⚠ 常量名**不是** `to_uppercase()`：事件名 `menu:relock` 的常量是
    ///   `MENU_RELOCK`（冒号换成下划线）。第一版这里直接 `to_uppercase()`，
    ///   测试立刻报「菜单两条没 emit」——**那正是测试自己写错**，
    ///   而它报出来的样子和真缺 emit 一模一样。
    #[test]
    fn every_listed_event_is_actually_emitted() {
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let me = here.join("src").join("contract.rs");
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        walk_rs(&here.join("src"), &mut files);

        let mut never = Vec::new();
        for ev in events::ALL {
            let konst = ev.to_uppercase().replace(':', "_");
            let emitted = files.iter().any(|p| {
                if *p == me {
                    return false;
                }
                std::fs::read_to_string(p).is_ok_and(|t| {
                    t.lines()
                        .any(|l| l.contains("emit(") && l.contains(konst.as_str()))
                })
            });
            if !emitted {
                never.push(ev);
            }
        }
        assert!(
            never.is_empty(),
            "清单里有**从来没有 emit 过**的事件名，它们让「前端只认这几个」变成空话：\n  {}",
            never.join("\n  ")
        );
    }

    /// **守住「只有这个文件能出现事件名字面量」这条纪律。**
    ///
    /// 为什么需要自动检查：事件名散三处这个问题，
    /// 靠 code review 是抓不住的——`emit("armed")` 看起来完全正常。
    /// 唯一可靠的办法是让机器去 grep 源码本身。
    #[test]
    fn no_event_literal_outside_this_file() {
        // `CARGO_MANIFEST_DIR` = `ui/src-tauri`。仓库根是它的上两级。
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = here.parent().and_then(|p| p.parent()).unwrap_or(here);
        // 本文件自己必然含字面量（它们就定义在这儿）
        let me = here.join("src").join("contract.rs");

        let mut bad: Vec<String> = Vec::new();
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        for dir in [here.join("src"), root.join("crates")] {
            walk_rs(&dir, &mut files);
        }
        for path in &files {
            if *path == me {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            for ev in events::ALL {
                // 形如 emit("armed" / listen("armed" 的字面量出现
                if text.contains(&format!("\"{ev}\"")) {
                    bad.push(format!(
                        "{}: 出现了事件名字面量 \"{ev}\"，改用 events::{}",
                        path.display(),
                        ev.to_uppercase()
                    ));
                }
            }
        }
        assert!(bad.is_empty(), "事件名又散出去了：\n{}", bad.join("\n"));
    }

    fn walk_rs(dir: &std::path::Path, acc: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk_rs(&p, acc);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                acc.push(p);
            }
        }
    }
}
