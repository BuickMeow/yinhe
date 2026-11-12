//! `cargo xtask layers`：审计 crate 分层，找越层依赖与 UI 泄漏。
//!
//! 判据只有两条，都不靠人工维护清单：
//!
//! 1. **UI 层以下不认识 egui**：除 `yinhe-egui`（桌面壳）外，任何 crate 的
//!    正常依赖里出现 `egui` 就算越层。`yinhe-theme` 的 egui 是可选 feature，
//!    单独构建不带 egui，因此不算；`yinhe-fonts` 硬依赖 egui，算越层。
//! 2. **没有循环依赖**：按各 `Cargo.toml` 里声明的 workspace 内部 path 依赖
//!    建图做 DFS。
//!
//! 依赖图直接从 `crates/*/Cargo.toml` 的行级内容解析，不引入 `toml` crate
//!（xtask 只允许标准库）。仓库的 Cargo.toml 写法很规整：
//! `yinhe-xxx = { path = "../yinhe-xxx" }`，一行一条。
//!
//! 用法：
//!   cargo xtask layers             打印分层与越层报告
//!   cargo xtask layers --check     有越层或环时返回失败（CI 用）

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// 桌面壳：唯一允许依赖 egui 的 crate。
const UI_CRATE: &str = "yinhe-egui";

/// 一个 crate 的依赖信息。
struct CrateInfo {
    name: String,
    /// workspace 内部依赖（crate 名）。
    internal: Vec<String>,
    /// 是否直接声明了 egui / eframe 依赖（含 optional 与 dev）。
    egui_normal: bool,
    /// 是否只在 dev-dependencies 里依赖 egui 相关 crate。
    egui_dev_only: bool,
    /// dev-dependencies 里的内部依赖。
    internal_dev: Vec<String>,
}

pub fn run(args: &[String]) -> ExitCode {
    let mut check = false;
    for arg in args {
        match arg.as_str() {
            "--check" => check = true,
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("未知参数: {other}（--help 看用法）");
                return ExitCode::FAILURE;
            }
        }
    }
    match audit() {
        Ok(report) => {
            print!("{}", report.text);
            if check && report.problem {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("layers 审计失败: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!(
        "cargo xtask layers [--check]\n\n  \
         审计 crate 分层：UI 层以下不得依赖 egui、不得有循环依赖。\n  \
         --check  发现问题时返回非零退出码（CI 用）"
    );
}

/// 审计结果：可打印文本 + 是否存在需要拦截的问题。
struct Report {
    text: String,
    problem: bool,
}

fn audit() -> Result<Report, String> {
    let root = crate::cmd::workspace_root();
    let members = workspace_members(&root)?;
    let crates = members
        .iter()
        .map(|name| read_crate(&root, name))
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = String::new();
    let mut problem = false;
    writeln!(out, "workspace 成员：{} 个\n", crates.len()).ok();

    // ── UI 泄漏 ──
    let mut leaks: Vec<&CrateInfo> = crates
        .iter()
        .filter(|c| c.name != UI_CRATE && c.egui_normal)
        .collect();
    leaks.sort_by(|a, b| a.name.cmp(&b.name));
    let dev_only: Vec<&CrateInfo> = crates
        .iter()
        .filter(|c| c.name != UI_CRATE && !c.egui_normal && c.egui_dev_only)
        .collect();
    writeln!(out, "## egui 依赖").ok();
    if leaks.is_empty() {
        writeln!(out, "OK：除 {UI_CRATE} 外没有 crate 正常依赖 egui。").ok();
    } else {
        problem = true;
        writeln!(out, "越层：以下非 UI crate 直接依赖 egui/eframe：").ok();
        for c in &leaks {
            writeln!(out, "  - {}", c.name).ok();
        }
    }
    if !dev_only.is_empty() {
        let names: Vec<&str> = dev_only.iter().map(|c| c.name.as_str()).collect();
        writeln!(
            out,
            "提示：仅 dev-dependencies 依赖 egui：{}",
            names.join(", ")
        )
        .ok();
    }

    // ── 循环依赖 ──
    writeln!(out, "\n## 依存方向").ok();
    if let Some(cycle) = find_cycle(&crates) {
        problem = true;
        writeln!(out, "循环依赖：{}", cycle.join(" → ")).ok();
    } else {
        writeln!(out, "OK：workspace 内部依赖无环。").ok();
    }

    // ── 分层 ──
    let depth = compute_depth(&crates);
    let mut by_depth: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
    for c in &crates {
        let d = depth.get(&c.name).copied().unwrap_or(0);
        by_depth.entry(d).or_default().push(c.name.as_str());
    }
    writeln!(out, "\n## 分层（按最长路径深度）").ok();
    for (d, mut names) in by_depth {
        names.sort_unstable();
        writeln!(out, "L{d}: {}", names.join(" · ")).ok();
    }

    // ── dev 反向边（提示，不拦截）──
    let mut back: Vec<String> = Vec::new();
    for c in &crates {
        let d_self = depth.get(&c.name).copied().unwrap_or(0);
        for dep in &c.internal_dev {
            let d_dep = depth.get(dep).copied().unwrap_or(0);
            if d_self < d_dep {
                back.push(format!("{} (L{d_self}) dev → {dep} (L{d_dep})", c.name));
            }
        }
    }
    if !back.is_empty() {
        writeln!(out, "\n## 测试期反向上层依赖（仅提示）").ok();
        for line in back {
            writeln!(out, "  - {line}").ok();
        }
    }
    Ok(Report { text: out, problem })
}

/// 读根 `Cargo.toml` 的 `members` 列表，返回 crate 名。
fn workspace_members(root: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(root.join("Cargo.toml"))
        .map_err(|e| format!("读取根 Cargo.toml 失败: {e}"))?;
    let mut names = Vec::new();
    let mut in_members = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("members") {
            in_members = true;
            continue;
        }
        if in_members {
            if line.starts_with(']') {
                break;
            }
            let Some(start) = line.find('"') else {
                continue;
            };
            let Some(end) = line[start + 1..].find('"') else {
                continue;
            };
            let path = &line[start + 1..start + 1 + end];
            // "crates/yinhe-core" → "yinhe-core"
            if let Some(name) = Path::new(path).file_name() {
                names.push(name.to_string_lossy().into_owned());
            }
        }
    }
    Ok(names)
}

/// 解析单个 crate 的 Cargo.toml。
fn read_crate(root: &Path, name: &str) -> Result<CrateInfo, String> {
    let dir = if name == "xtask" {
        root.join("xtask")
    } else {
        root.join("crates").join(name)
    };
    let path: PathBuf = dir.join("Cargo.toml");
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;

    let mut info = CrateInfo {
        name: name.to_string(),
        internal: Vec::new(),
        egui_normal: false,
        egui_dev_only: false,
        internal_dev: Vec::new(),
    };

    // 当前处于哪个依赖段：[dependencies] / [dev-dependencies] / [target.*.dependencies] ……
    let mut in_dev = false;
    let mut in_deps = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            let section = line.trim_start_matches('[').trim_end_matches(']');
            in_dev = section.contains("dev-dependencies");
            // target 特定依赖段也属于正常依赖（如 ohos 专属 cpal）。
            in_deps = section.ends_with("dependencies");
            continue;
        }
        if !in_deps || line.starts_with('#') {
            continue;
        }
        let Some((dep_name, rest)) = line.split_once('=') else {
            continue;
        };
        let dep_name = dep_name.trim();
        let rest = rest.trim();
        // 只关心 path 依赖（workspace 内部）。
        let is_internal = dep_name.starts_with("yinhe-") && rest.contains("path");
        if is_internal {
            if in_dev {
                info.internal_dev.push(dep_name.to_string());
            } else {
                info.internal.push(dep_name.to_string());
            }
            continue;
        }
        let is_egui = dep_name == "egui" || dep_name == "eframe" || dep_name == "egui_extras";
        if is_egui {
            // optional 的 egui 不算正常依赖（yinhe-theme 的 feature 开关）。
            let optional = rest.contains("optional = true");
            if in_dev {
                info.egui_dev_only = true;
            } else if !optional {
                info.egui_normal = true;
            }
        }
    }
    Ok(info)
}

/// DFS 找环（含 dev 边：测试期反向依赖同样会让层次失效）。
fn find_cycle(crates: &[CrateInfo]) -> Option<Vec<String>> {
    let graph: BTreeMap<&str, Vec<&str>> = crates
        .iter()
        .map(|c| {
            let mut deps: Vec<&str> = c.internal.iter().map(String::as_str).collect();
            deps.extend(c.internal_dev.iter().map(String::as_str));
            (c.name.as_str(), deps)
        })
        .collect();

    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Active,
        Done,
    }
    let mut marks: BTreeMap<&str, Mark> = graph.keys().map(|k| (*k, Mark::New)).collect();
    let mut stack: Vec<&str> = Vec::new();

    fn visit<'a>(
        node: &'a str,
        graph: &BTreeMap<&'a str, Vec<&'a str>>,
        marks: &mut BTreeMap<&'a str, Mark>,
        stack: &mut Vec<&'a str>,
    ) -> Option<Vec<String>> {
        match marks.get(node) {
            Some(Mark::Done) => return None,
            Some(Mark::Active) => {
                let start = stack.iter().position(|n| *n == node).unwrap_or(0);
                let mut cycle: Vec<String> =
                    stack[start..].iter().map(|s| (*s).to_string()).collect();
                cycle.push(node.to_string());
                return Some(cycle);
            }
            _ => {}
        }
        marks.insert(node, Mark::Active);
        stack.push(node);
        if let Some(deps) = graph.get(node) {
            for dep in deps {
                if let Some(c) = visit(dep, graph, marks, stack) {
                    return Some(c);
                }
            }
        }
        stack.pop();
        marks.insert(node, Mark::Done);
        None
    }

    for node in graph.keys() {
        if let Some(cycle) = visit(node, &graph, &mut marks, &mut stack) {
            return Some(cycle);
        }
    }
    None
}

/// 每个 crate 的最长路径深度（只算正常依赖；dev 用于提示而非分层）。
fn compute_depth(crates: &[CrateInfo]) -> BTreeMap<String, u32> {
    let graph: BTreeMap<&str, &Vec<String>> = crates
        .iter()
        .map(|c| (c.name.as_str(), &c.internal))
        .collect();
    let mut depth: BTreeMap<String, u32> = BTreeMap::new();

    fn depth_of(
        node: &str,
        graph: &BTreeMap<&str, &Vec<String>>,
        depth: &mut BTreeMap<String, u32>,
    ) -> u32 {
        if let Some(d) = depth.get(node) {
            return *d;
        }
        let d = match graph.get(node) {
            None => 0,
            Some(deps) if deps.is_empty() => 0,
            Some(deps) => {
                let mut max = 0;
                for dep in deps.iter() {
                    max = max.max(depth_of(dep, graph, depth) + 1);
                }
                max
            }
        };
        depth.insert(node.to_string(), d);
        d
    }

    for c in crates {
        depth_of(&c.name, &graph, &mut depth);
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str, deps: &[&str]) -> CrateInfo {
        CrateInfo {
            name: name.to_string(),
            internal: deps.iter().map(|s| (*s).to_string()).collect(),
            egui_normal: false,
            egui_dev_only: false,
            internal_dev: Vec::new(),
        }
    }

    #[test]
    fn detects_cycle() {
        let crates = vec![info("a", &["b"]), info("b", &["c"]), info("c", &["a"])];
        let cycle = find_cycle(&crates).expect("应检测到环");
        assert_eq!(
            cycle.first().map(String::as_str),
            cycle.last().map(String::as_str)
        );
    }

    #[test]
    fn acyclic_graph_reports_none() {
        let crates = vec![info("a", &["b"]), info("b", &[])];
        assert!(find_cycle(&crates).is_none());
    }

    #[test]
    fn depth_counts_longest_path() {
        let crates = vec![
            info("top", &["mid1", "mid2"]),
            info("mid1", &["leaf"]),
            info("mid2", &[]),
            info("leaf", &[]),
        ];
        let depth = compute_depth(&crates);
        assert_eq!(depth["leaf"], 0);
        assert_eq!(depth["mid2"], 0);
        assert_eq!(depth["mid1"], 1);
        assert_eq!(depth["top"], 2);
    }
}
