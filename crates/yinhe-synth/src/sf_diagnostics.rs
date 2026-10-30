//! SFZ 兼容性诊断：列出 `xsynth-soundfonts` 不会识别的 opcode。
//!
//! 解析本身委托 xsynth；未知 opcode 会被静默忽略，用户只能听到"不对劲"。
//! 这里在加载前扫描源文件（含 `#include` 递归），把会被忽略的 opcode 汇总
//! 出来，供设置页提示。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// `xsynth-soundfonts 0.4` 实际会解析的 opcode（与 `sfz/parse.rs` 对齐）。
const SUPPORTED: &[&str] = &[
    "lokey",
    "hikey",
    "lovel",
    "hivel",
    "volume",
    "pan",
    "pitch_keycenter",
    "key",
    "cutoff",
    "resonance",
    "amp_keycenter",
    "amp_keytrack",
    "amp_veltrack",
    "pan_keycenter",
    "pan_keytrack",
    "pan_veltrack",
    "fil_veltrack",
    "fil_keytrack",
    "fil_keycenter",
    "fil_type",
    "loop_mode",
    "loopmode",
    "loop_start",
    "loopstart",
    "loop_end",
    "loopend",
    "offset",
    "default_path",
    "tune",
    "ampeg_delay",
    "ampeg_start",
    "ampeg_attack",
    "ampeg_hold",
    "ampeg_decay",
    "ampeg_sustain",
    "ampeg_release",
    "ampeg_vel2release",
    "sample",
];

/// 一条诊断：某个 opcode 会被忽略，以及它通常影响什么。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SfzIssue {
    pub opcode: String,
    pub count: usize,
    pub hint: &'static str,
}

impl SfzIssue {
    /// 面向 UI 的一行文本，例如 `group ×12：鼓组/断音互斥不会生效`。
    pub fn message(&self) -> String {
        format!("{} ×{}：{}", self.opcode, self.count, self.hint)
    }
}

/// 扫描一个 SFZ（含 `#include`），返回按出现次数降序的诊断。
///
/// 读取失败或非 SFZ 路径返回空（诊断不应影响加载）。
pub fn scan(path: &Path) -> Vec<SfzIssue> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    scan_file(path, &mut counts, &mut seen, 0);

    let mut issues: Vec<SfzIssue> = counts
        .into_iter()
        .map(|(opcode, count)| SfzIssue {
            hint: hint_for(&opcode),
            opcode,
            count,
        })
        .collect();
    issues.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.opcode.cmp(&b.opcode)));
    issues
}

fn scan_file(
    path: &Path,
    counts: &mut HashMap<String, usize>,
    seen: &mut HashSet<PathBuf>,
    depth: usize,
) {
    // 深度上限防止畸形 include 链拖住 UI。
    if depth > 8 {
        return;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let dir = path.parent().unwrap_or_else(|| Path::new("."));

    for line in text.lines() {
        let line = strip_comment(line);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#include") {
            let inc = rest.trim().trim_matches('"').trim_matches('\'');
            // 含 `$` 的路径依赖当前 `#define`，不展开（避免误报）。
            if !inc.is_empty() && !inc.contains('$') {
                let inc_path = dir.join(inc);
                if seen.insert(inc_path.clone()) {
                    scan_file(&inc_path, counts, seen, depth + 1);
                }
            }
            continue;
        }
        if line.starts_with('#') {
            continue;
        }

        // 同一行可能既有 `<region>` 又有多个 `opcode=value`。
        let stripped = strip_tags(line);
        for token in stripped.split_whitespace() {
            let Some((opcode, _)) = token.split_once('=') else {
                continue;
            };
            let opcode = opcode.trim().to_ascii_lowercase();
            if opcode.is_empty() || SUPPORTED.contains(&opcode.as_str()) {
                continue;
            }
            *counts.entry(opcode).or_insert(0) += 1;
        }
    }
}

/// 去掉 `//` 注释（SFZ 的注释语法）。
fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// 把 `<region>` / `<group>` 等标签替换成空格，保留后面的 opcode。
fn strip_tags(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        match rest[start..].find('>') {
            Some(end) => {
                out.push(' ');
                rest = &rest[start + end + 1..];
            }
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

fn hint_for(opcode: &str) -> &'static str {
    if matches!(opcode, "group" | "off_by") {
        "鼓组/断音互斥不会生效"
    } else if matches!(opcode, "seq_position" | "seq_length" | "lorand" | "hirand") {
        "round-robin 不会轮换"
    } else if opcode.starts_with("sw_") {
        "keyswitch 不会切换"
    } else if opcode == "trigger" {
        "release/legato 触发不会生效"
    } else if matches!(opcode, "on_locc" | "on_hicc" | "on_loch" | "on_hicch") {
        "CC 触发的 articulation 不会生效"
    } else if opcode.starts_with("eg2") || opcode.starts_with("lfo") {
        "第二包络/LFO 不会生效"
    } else {
        "未识别的 opcode 会被忽略"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reports_unsupported_and_ignores_supported() {
        let dir = tempfile::tempdir().unwrap();
        let sfz = dir.path().join("kit.sfz");
        fs::write(
            &sfz,
            "<control>\ndefault_path=Samples/\n\
             <group> group=1 off_by=1\n\
             <region> sample=a.wav key=60 lokey=60 hikey=60 ampeg_attack=0.01 seq_position=1 seq_length=4\n\
             // <region> sample=commented.wav group=9\n",
        )
        .unwrap();

        let issues = scan(&sfz);
        let names: Vec<&str> = issues.iter().map(|i| i.opcode.as_str()).collect();
        assert!(names.contains(&"group"));
        assert!(names.contains(&"off_by"));
        assert!(names.contains(&"seq_position"));
        assert_eq!(names.iter().filter(|n| **n == "group").count(), 1);
        assert!(!names.contains(&"sample"));
        assert!(!names.contains(&"ampeg_attack"));
        assert!(!names.contains(&"default_path"));
    }

    #[test]
    fn follows_includes() {
        let dir = tempfile::tempdir().unwrap();
        let inc = dir.path().join("inc.sfz");
        fs::write(&inc, "<region> sample=b.wav sw_last=60\n").unwrap();
        let root = dir.path().join("root.sfz");
        fs::write(&root, "#include \"inc.sfz\"\n").unwrap();

        let issues = scan(&root);
        assert!(issues.iter().any(|i| i.opcode == "sw_last"));
    }

    #[test]
    fn missing_file_is_empty() {
        assert!(scan(Path::new("/nonexistent/definitely.sfz")).is_empty());
    }
}
