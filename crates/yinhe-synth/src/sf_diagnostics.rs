//! SFZ 兼容性诊断：列出引擎尚不支持的 opcode。
//!
//! 解析器是自有实现（`crate::sfz`），未识别的键本来就记在
//! `SfzFile.unknown_opcodes` 里——这里只做措辞与排序，不再维护一份手写白名单
//! （那样会和解析器各自漂移）。

use std::path::Path;

/// 一条诊断：某个 opcode 尚不被引擎消费，以及它通常影响什么。
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
    let Ok(file) = crate::sfz::parse(path) else {
        return Vec::new();
    };
    let mut issues: Vec<SfzIssue> = file
        .unknown_opcodes
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

/// 缺失采样数（音色库引用了读不到的 wav）。
pub fn missing_samples(path: &Path) -> usize {
    crate::sfz::parse(path)
        .map(|f| f.missing_samples)
        .unwrap_or(0)
}

fn hint_for(opcode: &str) -> &'static str {
    match opcode {
        "sustain_sw" => "延音踏板开关语义未实现（CC64 仍按标准工作）",
        "pitch_keytrack" => "键位音高跟踪未实现（按 region 的 key 变调）",
        "transpose" => "整体移调未实现",
        "sw_last" | "sw_default" | "sw_label" => "keyswitch 默认值未完全实现",
        o if o.starts_with("sw_") => "keyswitch 变体未实现",
        "off_mode" => "off_by 的快速/正常释放模式未区分",
        "loop_crossfade" => "循环点交叉淡化未实现",
        "note_off_velocity" => "note-off 力度未接入",
        "width" | "position" | "direction" => "立体声宽度/读取位置未实现",
        "delay" | "end" | "start" | "count" => "采样区间偏移变体未实现",
        o if o.starts_with("fileg_") => "第二滤波包络未实现",
        o if o.starts_with("pitcheg_") => "音高包络未实现",
        o if o.starts_with("rt_") => "释放触发（rt_*）变体未实现",
        o if o.starts_with("eg") => "扩展包络未实现",
        o if o.starts_with("velocity_") => "力度曲线变体未实现",
        o if o.starts_with("controller_") => "CC 曲线变体未实现",
        o if o.starts_with("label_") || o.starts_with("hint_") => "UI 提示信息（不影响发声）",
        o if o.starts_with("xfin_lokey")
            || o.starts_with("xfin_hikey")
            || o.starts_with("xfout_lokey")
            || o.starts_with("xfout_hikey") =>
        {
            "键位区间的淡入淡出未实现"
        }
        o if o.starts_with("xfin_lovel")
            || o.starts_with("xfin_hivel")
            || o.starts_with("xfout_lovel")
            || o.starts_with("xfout_hivel") =>
        {
            "力度区间的淡入淡出未实现"
        }
        "bend_up" | "bend_down" => "弯音范围（region 级）未实现",
        _ => "未识别的 opcode 会被忽略",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reports_unimplemented_and_skips_supported() {
        let dir = tempfile::tempdir().unwrap();
        let sfz = dir.path().join("kit.sfz");
        fs::write(dir.path().join("a.wav"), b"RIFF").unwrap();
        fs::write(
            &sfz,
            "<region>\nsample=a.wav key=60 lokey=60 hikey=60 ampeg_attack=0.01\n\
             sustain_sw=off\nloop_crossfade=0.01\n",
        )
        .unwrap();

        let issues = scan(&sfz);
        let names: Vec<&str> = issues.iter().map(|i| i.opcode.as_str()).collect();
        assert!(names.contains(&"sustain_sw"));
        assert!(names.contains(&"loop_crossfade"));
        assert!(!names.contains(&"sample"));
        assert!(!names.contains(&"ampeg_attack"));
        assert!(!names.contains(&"key"));
    }

    #[test]
    fn follows_includes() {
        let dir = tempfile::tempdir().unwrap();
        let inc = dir.path().join("inc.sfz");
        fs::write(dir.path().join("b.wav"), b"RIFF").unwrap();
        fs::write(&inc, "<region> sample=b.wav sustain_sw=off\n").unwrap();
        let root = dir.path().join("root.sfz");
        fs::write(&root, "#include \"inc.sfz\"\n").unwrap();

        let issues = scan(&root);
        assert!(issues.iter().any(|i| i.opcode == "sustain_sw"));
    }

    #[test]
    fn missing_file_is_empty() {
        assert!(scan(Path::new("/nonexistent/definitely.sfz")).is_empty());
    }

    #[test]
    fn counts_missing_samples() {
        let dir = tempfile::tempdir().unwrap();
        let sfz = dir.path().join("t.sfz");
        fs::write(&sfz, "<region>\nsample=nope.wav\n").unwrap();
        assert_eq!(missing_samples(&sfz), 1);
    }
}
