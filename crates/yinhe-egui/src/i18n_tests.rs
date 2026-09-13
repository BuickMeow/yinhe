//! 语言文件一致性测试：6 个 locale 的 key 集合必须完全一致。
//!
//! 只做静态文本解析（`^key:` 非缩进行），不依赖 YAML 库：
//! 项目内文案均为单行 `key: "value"` 格式，无需处理块语法。

use std::collections::BTreeSet;
use std::path::PathBuf;

const LOCALES: [&str; 6] = [
    "en-US.yml",
    "zh-CN.yml",
    "zh-HK.yml",
    "zh-TW.yml",
    "ja-JP.yml",
    "ko-KR.yml",
];

/// 提取一个 locale 文件的所有 key。
fn keys_of(path: &PathBuf) -> BTreeSet<String> {
    let content =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("读取 {path:?} 失败: {e}"));
    content
        .lines()
        .filter_map(|line| {
            if line.starts_with(|c: char| c.is_whitespace()) || line.starts_with('#') {
                return None;
            }
            let (key, _) = line.split_once(':')?;
            let key = key.trim();
            if key.is_empty() || key.contains(char::is_whitespace) {
                return None;
            }
            Some(key.to_string())
        })
        .collect()
}

#[test]
fn all_locales_have_same_keys() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("locales");
    let mut names = LOCALES.iter();
    let base_name = names.next().expect("至少一个基准语言");
    let base = keys_of(&dir.join(base_name));
    assert!(!base.is_empty(), "基准语言 {base_name} 未解析出任何 key");

    let mut problems = Vec::new();
    for name in names {
        let keys = keys_of(&dir.join(name));
        let missing: Vec<&String> = base.difference(&keys).collect();
        let extra: Vec<&String> = keys.difference(&base).collect();
        if !missing.is_empty() {
            problems.push(format!(
                "{name} 缺少 {} 个 key: {:?}",
                missing.len(),
                missing
            ));
        }
        if !extra.is_empty() {
            problems.push(format!("{name} 多出 {} 个 key: {:?}", extra.len(), extra));
        }
    }
    assert!(
        problems.is_empty(),
        "各语言 key 不一致（基准 {base_name}）:\n{}",
        problems.join("\n")
    );
}

/// 源码里引用的 `t!("...")` key 必须在所有语言中存在（防手滑写错 key）。
#[test]
fn referenced_keys_exist() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("locales");
    let mut all_keys: Option<BTreeSet<String>> = None;
    for name in LOCALES {
        let keys = keys_of(&dir.join(name));
        all_keys = Some(match all_keys {
            Some(acc) => acc.intersection(&keys).cloned().collect(),
            None => keys,
        });
    }
    let known = all_keys.unwrap_or_default();

    let src_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut referenced = BTreeSet::new();
    collect_t_keys(&src_dir, &mut referenced);

    let missing: Vec<&String> = referenced.difference(&known).collect();
    assert!(
        missing.is_empty(),
        "源码引用了不存在的 i18n key: {missing:?}"
    );
}

/// 递归扫描 rs 文件里的 `t!("key"` 字面量。
fn collect_t_keys(dir: &PathBuf, out: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_t_keys(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            collect_t_keys_in_file(&text, out);
        }
    }
}

/// 扫描单个源文件：先剥掉行内注释，再匹配 `t!("key")`。
fn collect_t_keys_in_file(text: &str, out: &mut BTreeSet<String>) {
    for line in text.lines() {
        let code = match line.find("//") {
            Some(p) => &line[..p],
            None => line,
        };
        let mut rest = code;
        while let Some(pos) = rest.find("t!(\"") {
            // `format!("` 等标识符结尾也含 `t!(`，只认前面不是标识符字符的 `t!`。
            let before = if pos > 0 {
                rest.as_bytes()[pos - 1]
            } else {
                b' '
            };
            let is_ident = before.is_ascii_alphanumeric() || before == b'_';
            let after = &rest[pos + 4..];
            if !is_ident && let Some(end) = after.find('"') {
                let key = &after[..end];
                if !key.is_empty() {
                    out.insert(key.to_string());
                }
            }
            rest = &rest[pos + 4..];
        }
    }
}
