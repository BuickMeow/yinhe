//! SFZ 行级词法：注释、标签、`opcode=value`、`#include`、`#define`。
//!
//! SFZ 是行导向的：一行可含多个 `opcode=value`，标签 `<group>` 也可与 opcode
//! 同行。这里按空白切分 token 而不是按行，能同时覆盖两种写法。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 词法 token。
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Token {
    /// `<region>` / `<group>` / `<master>` / `<global>` / `<control>` / 其它。
    Group(String),
    Opcode(String, String),
    /// `#include` 在词法阶段就地展开，不产生 token；保留变体只为文档语义。
    #[allow(dead_code)]
    Include(PathBuf),
}

/// `#include` 深度上限（畸形链路保护）。
const MAX_DEPTH: usize = 16;

/// 读取并展开一个 SFZ（含 include），产出扁平的 token 流。
pub(super) fn read_tokens(path: &Path) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let mut defines = HashMap::<String, String>::new();
    let mut stack = Vec::new();
    read_file(path, &mut defines, &mut out, &mut stack, 0)?;
    Ok(out)
}

fn read_file(
    path: &Path,
    defines: &mut HashMap<String, String>,
    out: &mut Vec<Token>,
    stack: &mut Vec<PathBuf>,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("无法读取 SFZ {:?}: {e}", path))?;
    if stack.contains(&canonical) {
        // 循环 include：跳过而不是报错（真实音色库偶尔有自引用）。
        return Ok(());
    }
    let text = std::fs::read_to_string(&canonical)
        .map_err(|e| format!("无法读取 SFZ {:?}: {e}", canonical))?;
    let dir = canonical.parent().unwrap_or(Path::new(".")).to_path_buf();
    stack.push(canonical);

    for raw_line in text.lines() {
        let line = strip_comment(raw_line);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#include") {
            let inc = rest.trim().trim_matches(['"', '\'']).replace('\\', "/");
            if !inc.is_empty() {
                let inc = substitute(&inc, defines);
                let inc_path = dir.join(&inc);
                read_file(&inc_path, defines, out, stack, depth + 1)?;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("#define") {
            let rest = rest.trim();
            if let Some((name, value)) = rest.split_once(char::is_whitespace) {
                defines.insert(name.trim().to_string(), value.trim().to_string());
            }
            continue;
        }
        if line.starts_with('#') {
            continue;
        }

        for piece in split_pieces(line) {
            if piece.starts_with('<') && piece.ends_with('>') {
                let tag = piece[1..piece.len() - 1].trim().to_ascii_lowercase();
                out.push(Token::Group(tag));
            } else if let Some((name, value)) = piece.split_once('=') {
                let name = name.trim().to_ascii_lowercase();
                if name.is_empty() {
                    continue;
                }
                let value = substitute(value.trim(), defines);
                out.push(Token::Opcode(name, value));
            }
        }
    }

    stack.pop();
    Ok(())
}

/// 去掉 `//` 注释（SFZ 只有一个注释语法）。
fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// 按空白切 token，但保留 `"…"` 引号内的整体（采样路径可含空格）。
fn split_pieces(line: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    for ch in line.chars() {
        match ch {
            '"' => {
                in_quote = !in_quote;
                cur.push(ch);
            }
            c if c.is_whitespace() && !in_quote => {
                if !cur.is_empty() {
                    pieces.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        pieces.push(cur);
    }
    pieces
}

/// `$VAR` 替换（`#define $NAME value`）。
fn substitute(s: &str, defines: &HashMap<String, String>) -> String {
    if !s.contains('$') || defines.is_empty() {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('$') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let end = rest
            .char_indices()
            .skip(1)
            .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        let name = &rest[..end];
        match defines.get(name) {
            Some(v) => out.push_str(v),
            None => out.push_str(name),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_tags_and_multiple_opcodes_per_line() {
        let pieces = split_pieces("<region> sample=a.wav key=60 volume=-6");
        assert_eq!(
            pieces,
            vec!["<region>", "sample=a.wav", "key=60", "volume=-6"]
        );
    }

    #[test]
    fn keeps_quoted_paths_with_spaces() {
        let pieces = split_pieces("sample=\"my samples/a.wav\" key=60");
        assert_eq!(pieces, vec!["sample=\"my samples/a.wav\"", "key=60"]);
    }

    #[test]
    fn strips_comments() {
        assert_eq!(strip_comment("key=60 // hi"), "key=60 ");
    }

    #[test]
    fn substitutes_defines() {
        let mut d = HashMap::new();
        d.insert("$DIR".to_string(), "Samples".to_string());
        assert_eq!(substitute("$DIR/a.wav", &d), "Samples/a.wav");
        assert_eq!(substitute("$MISSING/a.wav", &d), "$MISSING/a.wav");
    }
}
