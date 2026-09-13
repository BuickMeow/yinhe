//! VST3 bundle 的 `moduleinfo.json` 解析（VST3 SDK 3.6.14+）。
//!
//! 只解析宿主需要的字段，未知字段忽略。文件缺失返回 `Ok(None)`，
//! 读取/解析失败返回 Err（调用方决定降级策略：加载 factory 枚举或标记不支持）。

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// `moduleinfo.json` 解析失败（IO 或 JSON 结构错误）。
#[derive(Debug, thiserror::Error)]
pub enum ModuleInfoError {
    #[error("读取 moduleinfo 失败: {0}")]
    Io(#[from] std::io::Error),
    #[error("解析 moduleinfo 失败: {0}")]
    Json(#[from] serde_json::Error),
}

/// 一个 VST3 bundle 的元数据（宿主所需子集）。
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleInfo {
    /// module 显示名（通常等于插件名，但一个 bundle 可能有多个类）。
    pub name: String,
    pub version: String,
    pub vendor: String,
    /// 音频处理器类（"Audio Module Class"）；其他辅助类已过滤。
    pub classes: Vec<ModuleClassInfo>,
    /// 读取来源（诊断用）。
    pub source: PathBuf,
}

/// 一个可加载的 VST3 音频处理器类。
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleClassInfo {
    /// 32 位十六进制类 ID（VST3 CID）。大小写按文件原样保留，
    /// 比较时须忽略大小写。
    pub class_id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// 子类别（"Instrument" / "Fx" / "Synth" 等）。
    pub sub_categories: Vec<String>,
    /// 是否为乐器（子类别含 "Instrument"）。
    pub is_instrument: bool,
    /// 是否为效果器（子类别含 "Fx"）。
    pub is_effect: bool,
}

/// 读取并解析 bundle 的 `moduleinfo.json`。
///
/// 查找顺序（与 SDK 布局一致）：
/// 1. `Contents/Resources/moduleinfo.json`（当前推荐位置）；
/// 2. `Contents/moduleinfo.json`（SDK 3.7.5 旧位置）。
///
/// 两处都不存在时返回 `Ok(None)`（旧插件，需要加载 factory 枚举）。
pub fn read_moduleinfo(bundle: &Path) -> Result<Option<ModuleInfo>, ModuleInfoError> {
    let candidates = [
        bundle.join("Contents/Resources/moduleinfo.json"),
        bundle.join("Contents/moduleinfo.json"),
    ];
    let Some(source) = candidates.iter().find(|p| p.is_file()) else {
        return Ok(None);
    };
    let bytes = std::fs::read(source)?;
    // VST3 SDK 用 JSON5 解析 moduleinfo：部分插件（如 Kushview Element）会写
    // 尾随逗号。严格解析失败后走宽容预处理重试，仍失败才报错。
    let raw: RawModuleInfo = match serde_json::from_slice(&bytes) {
        Ok(raw) => raw,
        Err(strict_err) => {
            let lenient = strip_json5_noise(&bytes);
            serde_json::from_slice(&lenient).map_err(|_| ModuleInfoError::Json(strict_err))?
        }
    };
    Ok(Some(ModuleInfo {
        name: raw.name,
        version: raw.version,
        vendor: raw.factory.vendor,
        classes: raw
            .classes
            .into_iter()
            .filter(|c| c.category == "Audio Module Class")
            .map(RawClass::into_class)
            .collect(),
        source: source.clone(),
    }))
}

/// 去除 JSON5 噪声（尾随逗号、`//` 行注释），产出严格 JSON 文本。
///
/// 只处理宿主遇到的最小集合；块注释等罕见语法不做处理（解析失败照常报错）。
fn strip_json5_noise(bytes: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                // 行注释：吞到行尾（保留换行便于错误定位）。
                for c2 in chars.by_ref() {
                    if c2 == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ',' => {
                // 尾随逗号：逗号后（跳过空白）若是 } 或 ] 则丢弃。
                let mut lookahead = chars.clone();
                let next = lookahead.by_ref().find(|c2| !c2.is_whitespace());
                if !matches!(next, Some('}') | Some(']')) {
                    out.push(',');
                }
            }
            _ => out.push(c),
        }
    }
    out.into_bytes()
}

// ── serde 中间结构（字段名带空格，全部显式 rename + default 容错）──

#[derive(Deserialize, Default)]
struct RawModuleInfo {
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Version", default)]
    version: String,
    #[serde(rename = "Factory Info", default)]
    factory: RawFactoryInfo,
    #[serde(rename = "Classes", default)]
    classes: Vec<RawClass>,
}

#[derive(Deserialize, Default)]
struct RawFactoryInfo {
    #[serde(rename = "Vendor", default)]
    vendor: String,
}

#[derive(Deserialize, Default)]
struct RawClass {
    #[serde(rename = "CID", default)]
    class_id: String,
    #[serde(rename = "Category", default)]
    category: String,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Vendor", default)]
    vendor: String,
    #[serde(rename = "Version", default)]
    version: String,
    #[serde(rename = "Sub Categories", default)]
    sub_categories: Vec<String>,
}

impl RawClass {
    fn into_class(self) -> ModuleClassInfo {
        let has = |needle: &str| self.sub_categories.iter().any(|s| s == needle);
        ModuleClassInfo {
            class_id: self.class_id,
            name: self.name,
            vendor: self.vendor,
            version: self.version,
            is_instrument: has("Instrument"),
            is_effect: has("Fx"),
            sub_categories: self.sub_categories,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "Name": "DemoBundle",
        "Version": "1.2.3",
        "Factory Info": { "Vendor": "DemoVendor", "URL": "https://example.com" },
        "Classes": [
            {
                "CID": "ABCDEF0123456789ABCDEF0123456789",
                "Category": "Audio Module Class",
                "Name": "Demo Synth",
                "Vendor": "DemoVendor",
                "Version": "1.2.3",
                "Sub Categories": ["Instrument", "Synth"]
            },
            {
                "CID": "00000000000000000000000000000000",
                "Category": "Component Controller Class",
                "Name": "Ignored"
            }
        ]
    }"#;

    #[test]
    fn parses_and_filters_audio_module_class() {
        let raw: RawModuleInfo = serde_json::from_str(SAMPLE).expect("sample should parse");
        let classes: Vec<_> = raw
            .classes
            .into_iter()
            .filter(|c| c.category == "Audio Module Class")
            .map(RawClass::into_class)
            .collect();
        assert_eq!(classes.len(), 1);
        let c = &classes[0];
        assert_eq!(c.name, "Demo Synth");
        assert_eq!(c.class_id, "ABCDEF0123456789ABCDEF0123456789");
        assert!(c.is_instrument);
        assert!(!c.is_effect);
    }

    #[test]
    fn missing_file_returns_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = read_moduleinfo(dir.path()).expect("read should not fail");
        assert!(result.is_none());
    }

    #[test]
    fn reads_from_resources_or_contents() {
        let dir = tempfile::tempdir().expect("tempdir");
        let contents = dir.path().join("Contents");
        std::fs::create_dir_all(contents.join("Resources")).expect("mkdir");
        std::fs::write(contents.join("Resources/moduleinfo.json"), SAMPLE).expect("write");
        let info = read_moduleinfo(dir.path()).expect("read").expect("some");
        assert_eq!(info.name, "DemoBundle");
        assert_eq!(info.vendor, "DemoVendor");
        assert_eq!(info.classes.len(), 1);
    }

    #[test]
    fn invalid_json_is_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let contents = dir.path().join("Contents");
        std::fs::create_dir_all(&contents).expect("mkdir");
        std::fs::write(contents.join("moduleinfo.json"), b"{ not json").expect("write");
        assert!(read_moduleinfo(dir.path()).is_err());
    }

    #[test]
    fn parses_json5_trailing_commas_and_line_comments() {
        let dir = tempfile::tempdir().expect("tempdir");
        let contents = dir.path().join("Contents");
        std::fs::create_dir_all(&contents).expect("mkdir");
        // 模拟 Kushview Element 风格：尾随逗号 + 行注释 + 字符串内逗号/斜杠。
        let json5 = r#"{
            // 模块信息
            "Name": "Element FX, v2",
            "Version": "1.0.0",
            "Factory Info": {
                "Vendor": "Kushview",
                "Flags": { "Unicode": true, },
            },
            "Classes": [
                {
                    "CID": "ABCDEF019182FAEB4B736856456C4658",
                    "Category": "Audio Module Class",
                    "Name": "Element FX",
                    "Sub Categories": [ "Fx", ],
                },
            ],
        }"#;
        std::fs::write(contents.join("moduleinfo.json"), json5).expect("write");
        let info = read_moduleinfo(dir.path()).expect("read").expect("some");
        assert_eq!(info.name, "Element FX, v2");
        assert_eq!(info.classes.len(), 1);
        assert!(info.classes[0].is_effect);
    }
}
