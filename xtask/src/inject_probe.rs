//! 实机验证 `inject`：真的往系统剪贴板写-读往返，并证明**鼠标指针没动**。
//!
//! 这条命令**不碰微信**：它只验「剪贴板通路 + 不动鼠标」这两件事。
//! 真往微信输入框里打字需要目标窗口在前台，那是人工验证，不是自动化能做的。
//!
//! 跑法：`cargo run -p xtask -- inject-probe`

use std::process::ExitCode;

/// 必须能**逐字节**往返的样本。中文、emoji（代理对）、换行、控制字符
/// 都在里面——UTF-16 ↔ UTF-8 的任何一次错误转换都会在这里露出来。
const SAMPLES: &[(&str, &str)] = &[
    ("纯 ASCII", "hello world 123"),
    ("中日文", "这是候选回复，含中英文 mixed 与符号 ✓"),
    ("换行与制表", "第一行\n第二行\t制表符"),
    ("emoji 代理对", "前面 🚀 中间 🎯 后面"),
    ("空串", ""),
    ("长文本", "字"),
];

pub fn run() -> Result<(), String> {
    use chatservant_capture::inject;

    println!("== 鼠标指针基线 ==");
    let before = inject::cursor_pos().ok_or("读不到鼠标位置")?;
    println!("  起点 ({}, {})", before.x, before.y);

    println!("\n== 剪贴板往返（逐字节相等）==");
    let mut bad = 0usize;
    for (name, text) in SAMPLES {
        let long = if *text == "字" {
            // 长文本单独放大，逼近剪贴板大小限制
            "字".repeat(50_000)
        } else {
            (*text).to_string()
        };
        match inject::roundtrip(&long) {
            Ok(back) if back == long => {
                println!("  OK    {name:<14} {} 字节往返相等", long.len());
            }
            Ok(back) => {
                bad += 1;
                println!("  FAIL  {name:<14} 写 {} 读 {}", long.len(), back.len());
            }
            Err(e) => {
                bad += 1;
                println!("  FAIL  {name:<14} {e}");
            }
        }
    }

    println!("\n== 指针是否被移动过 ==");
    let after = inject::cursor_pos().ok_or("读不到鼠标位置")?;
    println!("  终点 ({}, {})", after.x, after.y);
    if before.x != after.x || before.y != after.y {
        bad += 1;
        println!("  FAIL  指针动了！注入过程绝不允许移动用户的鼠标");
    } else {
        println!("  OK    指针没动过 —— 整条路径没有一次 mouse_event / SetCursorPos");
    }

    println!("\n== 结论 ==");
    if bad == 0 {
        println!("INJECT_PROBE_OK");
        Ok(())
    } else {
        Err(format!("{bad} 项不通过"))
    }
}

/// 让 main 能用 `ExitCode` 收尾。
#[allow(dead_code)]
pub type Code = ExitCode;
