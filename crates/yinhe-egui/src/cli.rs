//! 无 GUI 的只读 CLI：让 Agent 先了解工程与命令表，再决定是否启动编辑器。
//!
//! 只读命令不解压 `.yin` 的 data 段，也不加载 1 亿音符；这是它能安全用于
//! 大型工程的前提。写操作（apply/export）留给后续的控制 socket。

use std::path::Path;

use serde_json::{Value, json};
use yinhe_editor_core::shortcuts::{ALL_ACTION_IDS, CONTEXT_ACTION_IDS, KeyCombo, Keybindings};

/// 协议版本：外部脚本用它判断字段是否兼容。
pub(crate) const PROTOCOL: &str = "yinhe-cli/1";

/// 处理只读 CLI 参数。返回 `Some(exit_code)` 表示不应启动 GUI；
/// 返回 `None` 表示交给 GUI（文件参数/平台参数）。
pub(crate) fn run(argv: &[String]) -> Option<i32> {
    let mut args = argv.iter().skip(1);
    let first = args.next()?;
    match first.as_str() {
        "--help" | "-h" => {
            print_help();
            Some(0)
        }
        "--version" | "-V" => {
            print_version();
            Some(0)
        }
        "--list-commands" => Some(print_value(&commands_json())),
        "--inspect" => {
            let Some(path) = args.next() else {
                eprintln!("--inspect 需要一个 .yin 文件路径");
                return Some(2);
            };
            Some(match inspect_json(Path::new(path)) {
                Ok(value) => print_value(&value),
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            })
        }
        _ => None,
    }
}

fn print_help() {
    println!(
        "Yinhe {}\n\
         \n\
         用法:\n\
         \x20 yinhe <project.yin|song.mid>   启动 GUI 并打开文件\n\
         \x20 yinhe --inspect <file.yin>     只读输出工程/轨道元数据（JSON，不解压音符）\n\
         \x20 yinhe --list-commands          输出命令 ID 与默认快捷键（JSON）\n\
         \x20 yinhe --version                版本号\n\
         \x20 yinhe --help                   本帮助\n\
         \n\
         说明:\n\
         \x20 --inspect 只读 project.json 与 mapping.json，1 亿音符工程也是常数时间。\n\
         \x20 写操作/实时控制尚未开放；协议草案见 docs/2026-11/spec-agent-composition.md。",
        env!("CARGO_PKG_VERSION")
    );
}

fn print_version() {
    println!("Yinhe {}", env!("CARGO_PKG_VERSION"));
    println!("yinhe-yin format: {}", yinhe_yin::VERSION);
}

fn print_value(value: &Value) -> i32 {
    match serde_json::to_string_pretty(value) {
        Ok(text) => {
            println!("{text}");
            0
        }
        Err(e) => {
            eprintln!("序列化 JSON 失败: {e}");
            1
        }
    }
}

/// 命令表：ID、默认快捷键、作用域。`label_key` 是 i18n key，外部不要按它
/// 做稳定匹配；稳定匹配一律用 `id`。
fn commands_json() -> Value {
    let kb = Keybindings::default();
    let commands: Vec<Value> = ALL_ACTION_IDS
        .iter()
        .map(|id| {
            json!({
                "id": id,
                "label_key": crate::shortcuts::action_label_key(id),
                "scope": scope_of(id),
                "default": kb.get(id).iter().map(combo_text).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "protocol": PROTOCOL,
        "count": commands.len(),
        "commands": commands,
    })
}

/// 只读工程摘要：project.json + mapping.json 的轨道级摘要。
///
/// 不返回 `audio_clips`/`clips` 的完整内容：大型工程的片段列表可能很长，
/// Agent 需要时再按轨道查询。这里只给数量。
fn inspect_json(path: &Path) -> Result<Value, String> {
    let project = yinhe_yin::read_project_file(path)
        .map_err(|e| format!("无法读取 {} 的 project.json: {e}", path.display()))?;
    let mapping = yinhe_yin::read_mapping_file(path)
        .map_err(|e| format!("无法读取 {} 的 mapping.json: {e}", path.display()))?;
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    let mut tracks = Vec::new();
    for port in &mapping.ports {
        for channel in &port.channels {
            for track in &channel.tracks {
                tracks.push(json!({
                    "uuid": track.uuid,
                    "name": track.name,
                    "port": port.port,
                    "channel": channel.channel,
                    "kind": format!("{:?}", track.kind).to_lowercase(),
                    "muted": track.muted,
                    "soloed": track.soloed,
                    "folder_depth": track.folder_depth,
                    "audio_channel": track.audio_channel,
                    "clips": track.clips.len(),
                    "audio_clips": track.audio_clips.len(),
                }));
            }
        }
    }

    Ok(json!({
        "protocol": PROTOCOL,
        "file": {
            "path": path.display().to_string(),
            "bytes": bytes,
        },
        "format": {
            "magic": "YINH",
            "version": yinhe_yin::VERSION,
        },
        "project": project,
        "tracks": tracks,
        "mapping": {
            "version": mapping.version,
            "port_count": mapping.ports.len(),
            "channel_instrument_overrides":
                mapping.channel_instruments.iter().filter(|x| x.is_some()).count(),
            "synth_param_overrides": mapping.synth_params.len(),
        },
    }))
}

fn combo_text(combo: &KeyCombo) -> String {
    let mut text = String::new();
    if combo.command {
        text.push_str("Cmd+");
    }
    if combo.shift {
        text.push_str("Shift+");
    }
    if combo.alt {
        text.push_str("Alt+");
    }
    text.push_str(&combo.key);
    text
}

fn scope_of(id: &str) -> &'static str {
    if CONTEXT_ACTION_IDS.contains(&id) {
        "context"
    } else if id.starts_with("tool_") {
        "tool"
    } else if matches!(id, "toggle_play" | "stop") {
        "transport"
    } else if matches!(
        id,
        "new_project"
            | "open"
            | "import_audio"
            | "save"
            | "save_as"
            | "close_document"
            | "export_audio"
            | "export_midi"
            | "settings"
            | "project_settings"
            | "exit"
    ) {
        "file"
    } else {
        "edit"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_list_is_stable_and_contains_save() {
        let value = commands_json();
        assert_eq!(value["protocol"], PROTOCOL);
        assert_eq!(
            value["count"].as_u64().unwrap() as usize,
            ALL_ACTION_IDS.len()
        );
        let commands = value["commands"].as_array().expect("commands array");
        let save = commands
            .iter()
            .find(|c| c["id"] == "save")
            .expect("save 命令必须存在");
        assert_eq!(save["scope"], "file");
        assert!(
            save["default"].as_array().is_some_and(|a| !a.is_empty()),
            "save 应该有默认快捷键"
        );
    }

    #[test]
    fn combo_text_orders_modifiers() {
        let combo = KeyCombo {
            command: true,
            shift: true,
            alt: false,
            key: "S".to_string(),
        };
        assert_eq!(combo_text(&combo), "Cmd+Shift+S");
    }

    #[test]
    fn scope_of_context_and_tool() {
        assert_eq!(scope_of("track_delete"), "context");
        assert_eq!(scope_of("tool_pencil"), "tool");
        assert_eq!(scope_of("undo"), "edit");
    }
}
