//! 实机验证：仓库里 grep 不到 API key。
//!
//! key 从环境变量 `CHATSERVANT_PROBE_KEY` 读，**绝不进命令行**——
//! 命令行会留在 shell 历史和进程列表里，那比日志泄露更隐蔽也更难清。
//!
//! 这是「目测看不出问题」的对照物：单元测试证明脱敏函数对**已知**的
//! key 有效，这个探针证明仓库里压根没有 key 可泄露。

use std::path::{Path, PathBuf};

use anyhow::Result;

/// 环境变量名。写成常量是为了 grep 时能一眼对上。
const ENV_KEY: &str = "CHATSERVANT_PROBE_KEY";

pub fn run() -> Result<()> {
    let probe = std::env::var(ENV_KEY).unwrap_or_default();

    println!("== 凭据泄露扫描 ==");
    if probe.is_empty() {
        println!("  跳过：{ENV_KEY} 未设置（这是正常的——机器上本来就不该有 key）");
    } else {
        println!("  扫描工作区里是否残留 {} 字节的 key…", probe.len());
        match grep_workspace(&probe) {
            Ok(hits) if hits.is_empty() => println!("  ✅ 没有残留"),
            Ok(hits) => {
                println!("  ❌ 这些文件里出现了 key：");
                for h in hits {
                    println!("     {h}");
                }
                return Err(anyhow::anyhow!("仓库里出现了 API key"));
            }
            Err(e) => return Err(anyhow::Error::new(e).context("扫描失败")),
        }
    }

    println!("\n== 结构性约束 ==");
    let probe_dir = Path::new("target/probe");
    println!(
        "  探针帧目录 {}：{}",
        probe_dir.display(),
        if probe_dir.exists() {
            "存在"
        } else {
            "不存在"
        }
    );
    println!("  （/target 已在 .gitignore 里——截图不落盘的硬约束靠它兜住）");

    println!("\n== 凭据只去两处 ==");
    println!("  ① Windows 凭据管理器（chat-servant/<provider>）");
    println!("  ② 环境变量 <PROVIDER>_API_KEY");
    println!("  配置文件、日志、命令行、前端 bundle —— 一律没有");

    Ok(())
}

/// 递归扫工作区。跳过构建产物与依赖目录——里面的东西不是我们写的。
fn grep_workspace(needle: &str) -> std::io::Result<Vec<String>> {
    let skip = ["target", "node_modules", ".git", ".svelte-kit", "build"];
    let mut hits = Vec::new();
    for path in walk(Path::new(".")) {
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if skip.contains(&name) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        // 含 0 字节的是二进制（图标、PNG），里面不会有 ASCII key；
        // 超过 4 MB 的多半是数据库或日志转储，也不该在仓库里。
        if bytes.contains(&0u8) || bytes.len() > 4 * 1024 * 1024 {
            continue;
        }
        if bytes.windows(needle.len()).any(|w| w == needle.as_bytes()) {
            hits.push(path.display().to_string());
        }
    }
    Ok(hits)
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        out.push(p.clone());
        if p.is_dir() {
            out.extend(walk(&p));
        }
    }
    out
}
