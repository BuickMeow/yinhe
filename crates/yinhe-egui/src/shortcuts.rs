//! 快捷键的 egui 侧工具：`egui::Key ↔ 字符串` 映射、显示文本、输入匹配。
//!
//! 数据模型（`KeyCombo`/`Keybindings`）在 `yinhe-editor-core::shortcuts`。

use eframe::egui::{Key, Modifiers};
use yinhe_editor_core::shortcuts::{KeyCombo, Keybindings};

/// 全部可录制/显示的 egui 键（排除纯修饰键，它们在 `KeyCombo` 里以 flag 表示）。
/// 顺序无关，仅用于设置页录制时的合法性判断。
pub(crate) const RECORDABLE_KEYS: &[Key] = &[
    Key::Num0,
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
    Key::A,
    Key::B,
    Key::C,
    Key::D,
    Key::E,
    Key::F,
    Key::G,
    Key::H,
    Key::I,
    Key::J,
    Key::K,
    Key::L,
    Key::M,
    Key::N,
    Key::O,
    Key::P,
    Key::Q,
    Key::R,
    Key::S,
    Key::T,
    Key::U,
    Key::V,
    Key::W,
    Key::X,
    Key::Y,
    Key::Z,
    Key::F1,
    Key::F2,
    Key::F3,
    Key::F4,
    Key::F5,
    Key::F6,
    Key::F7,
    Key::F8,
    Key::F9,
    Key::F10,
    Key::F11,
    Key::F12,
    Key::Space,
    Key::Enter,
    Key::Escape,
    Key::Tab,
    Key::Backspace,
    Key::Delete,
    Key::Insert,
    Key::Home,
    Key::End,
    Key::PageUp,
    Key::PageDown,
    Key::ArrowUp,
    Key::ArrowDown,
    Key::ArrowLeft,
    Key::ArrowRight,
    Key::Comma,
    Key::Period,
    Key::Minus,
    Key::Plus,
    Key::Equals,
    Key::OpenBracket,
    Key::CloseBracket,
    Key::Semicolon,
    Key::Quote,
    Key::Backtick,
    Key::Backslash,
    Key::Slash,
    // 打印键区：可作为快捷键（如 PrtSc 截图），放这里才能在键盘上绑定。
    Key::PrintScreen,
    Key::ScrollLock,
    Key::Pause,
];

/// 键名 → egui 键。与 [`key_to_str`] 互逆。
pub fn str_to_key(s: &str) -> Option<Key> {
    use Key::*;
    Some(match s {
        "0" => Num0,
        "1" => Num1,
        "2" => Num2,
        "3" => Num3,
        "4" => Num4,
        "5" => Num5,
        "6" => Num6,
        "7" => Num7,
        "8" => Num8,
        "9" => Num9,
        "A" => A,
        "B" => B,
        "C" => C,
        "D" => D,
        "E" => E,
        "F" => F,
        "G" => G,
        "H" => H,
        "I" => I,
        "J" => J,
        "K" => K,
        "L" => L,
        "M" => M,
        "N" => N,
        "O" => O,
        "P" => P,
        "Q" => Q,
        "R" => R,
        "S" => S,
        "T" => T,
        "U" => U,
        "V" => V,
        "W" => W,
        "X" => X,
        "Y" => Y,
        "Z" => Z,
        "F1" => F1,
        "F2" => F2,
        "F3" => F3,
        "F4" => F4,
        "F5" => F5,
        "F6" => F6,
        "F7" => F7,
        "F8" => F8,
        "F9" => F9,
        "F10" => F10,
        "F11" => F11,
        "F12" => F12,
        "Space" => Space,
        "Enter" => Enter,
        "Escape" => Escape,
        "Tab" => Tab,
        "Backspace" => Backspace,
        "Delete" => Delete,
        "Insert" => Insert,
        "Home" => Home,
        "End" => End,
        "PageUp" => PageUp,
        "PageDown" => PageDown,
        "ArrowUp" => ArrowUp,
        "ArrowDown" => ArrowDown,
        "ArrowLeft" => ArrowLeft,
        "ArrowRight" => ArrowRight,
        "Comma" => Comma,
        "Period" => Period,
        "Minus" => Minus,
        "Plus" => Plus,
        "Equals" => Equals,
        "OpenBracket" => OpenBracket,
        "CloseBracket" => CloseBracket,
        "Semicolon" => Semicolon,
        "Quote" => Quote,
        "Backtick" => Backtick,
        "Backslash" => Backslash,
        "Slash" => Slash,
        "PrintScreen" => PrintScreen,
        "ScrollLock" => ScrollLock,
        "Pause" => Pause,
        _ => return None,
    })
}

/// egui 键 → 键名（序列化用）。与 [`str_to_key`] 互逆。
pub fn key_to_str(key: Key) -> String {
    str_to_key_rev(key).to_string()
}

fn str_to_key_rev(key: Key) -> &'static str {
    use Key::*;
    match key {
        Num0 => "0",
        Num1 => "1",
        Num2 => "2",
        Num3 => "3",
        Num4 => "4",
        Num5 => "5",
        Num6 => "6",
        Num7 => "7",
        Num8 => "8",
        Num9 => "9",
        A => "A",
        B => "B",
        C => "C",
        D => "D",
        E => "E",
        F => "F",
        G => "G",
        H => "H",
        I => "I",
        J => "J",
        K => "K",
        L => "L",
        M => "M",
        N => "N",
        O => "O",
        P => "P",
        Q => "Q",
        R => "R",
        S => "S",
        T => "T",
        U => "U",
        V => "V",
        W => "W",
        X => "X",
        Y => "Y",
        Z => "Z",
        F1 => "F1",
        F2 => "F2",
        F3 => "F3",
        F4 => "F4",
        F5 => "F5",
        F6 => "F6",
        F7 => "F7",
        F8 => "F8",
        F9 => "F9",
        F10 => "F10",
        F11 => "F11",
        F12 => "F12",
        Space => "Space",
        Enter => "Enter",
        Escape => "Escape",
        Tab => "Tab",
        Backspace => "Backspace",
        Delete => "Delete",
        Insert => "Insert",
        Home => "Home",
        End => "End",
        PageUp => "PageUp",
        PageDown => "PageDown",
        ArrowUp => "ArrowUp",
        ArrowDown => "ArrowDown",
        ArrowLeft => "ArrowLeft",
        ArrowRight => "ArrowRight",
        Comma => "Comma",
        Period => "Period",
        Minus => "Minus",
        Plus => "Plus",
        Equals => "Equals",
        OpenBracket => "OpenBracket",
        CloseBracket => "CloseBracket",
        Semicolon => "Semicolon",
        Quote => "Quote",
        Backtick => "Backtick",
        Backslash => "Backslash",
        Slash => "Slash",
        PrintScreen => "PrintScreen",
        ScrollLock => "ScrollLock",
        Pause => "Pause",
        _ => "Unknown",
    }
}

/// 按键的短显示名（不含修饰键）。
pub fn key_display(key: Key) -> String {
    let s = key_to_str(key);
    match s.as_str() {
        "Space" => "Space".to_string(),
        "Enter" => "Enter".to_string(),
        "Escape" => "Esc".to_string(),
        "Delete" => {
            if cfg!(target_os = "macos") {
                "⌫".to_string()
            } else {
                "Delete".to_string()
            }
        }
        "Backspace" => "Backspace".to_string(),
        "ArrowUp" => "↑".to_string(),
        "ArrowDown" => "↓".to_string(),
        "ArrowLeft" => "←".to_string(),
        "ArrowRight" => "→".to_string(),
        "Comma" => ",".to_string(),
        "Period" => ".".to_string(),
        "Minus" => "-".to_string(),
        "Plus" => "+".to_string(),
        "Equals" => "=".to_string(),
        "OpenBracket" => "[".to_string(),
        "CloseBracket" => "]".to_string(),
        "PageUp" => "PgUp".to_string(),
        "PageDown" => "PgDn".to_string(),
        _ => s,
    }
}

/// 组合键的显示文本：macOS 用符号（⌘⇧⌥），其他平台用 Ctrl+Shift+Alt+。
pub fn display_combo(combo: &KeyCombo) -> String {
    let Some(key) = str_to_key(&combo.key) else {
        return combo.key.clone();
    };
    if cfg!(target_os = "macos") {
        let mut s = String::new();
        if combo.command {
            s.push('⌘');
        }
        if combo.shift {
            s.push('⇧');
        }
        if combo.alt {
            s.push('⌥');
        }
        s.push_str(&key_display(key));
        s
    } else {
        let mut parts: Vec<String> = Vec::new();
        if combo.command {
            parts.push("Ctrl".to_string());
        }
        if combo.shift {
            parts.push("Shift".to_string());
        }
        if combo.alt {
            parts.push("Alt".to_string());
        }
        parts.push(key_display(key));
        parts.join("+")
    }
}

/// 本帧按下的键是否匹配该组合（用于键盘事件匹配）。
pub fn matches_combo(combo: &KeyCombo, modifiers: Modifiers, key: Key) -> bool {
    let Some(k) = str_to_key(&combo.key) else {
        return false;
    };
    if key != k {
        return false;
    }
    let command = modifiers.command || modifiers.ctrl;
    command == combo.command && modifiers.shift == combo.shift && modifiers.alt == combo.alt
}

/// 快捷键显示文本在 egui 临时数据里的键。
///
/// 右键菜单分布在很深的调用栈里（事件表、图层、时间标尺……），
/// 为了让菜单项显示快捷键而把整个 `App` 一路透传下去不值得；
/// 由 `App` 每帧写一份 `action_id → 显示文本` 的快照，菜单按需查。
fn shortcut_text_cache_id() -> egui::Id {
    egui::Id::unique("yinhe_shortcut_text_cache")
}

/// 每帧刷新快捷键显示文本（`App` 调用；键位设置变化后自动跟上）。
pub fn publish_shortcut_text(ctx: &egui::Context, keybindings: &Keybindings) {
    let mut map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let ids = yinhe_editor_core::shortcuts::ALL_ACTION_IDS
        .iter()
        .chain(yinhe_editor_core::shortcuts::CONTEXT_ACTION_IDS);
    for &id in ids {
        // 只显示第一个快捷键：菜单项右栏宽度有限，多个组合会挤掉主文本。
        if let Some(combo) = keybindings.get(id).first() {
            map.insert(id.to_string(), display_combo(combo));
        }
    }
    ctx.data_mut(|d| d.insert_temp(shortcut_text_cache_id(), map));
}

/// 查某动作的快捷键显示文本（未绑定返回 None）。
pub fn shortcut_text_for(ctx: &egui::Context, action_id: &str) -> Option<String> {
    if action_id.is_empty() {
        return None;
    }
    ctx.data(|d| d.get_temp::<std::collections::HashMap<String, String>>(shortcut_text_cache_id()))
        .and_then(|map| map.get(action_id).cloned())
}

/// macOS 原生菜单是否负责处理该快捷键组合。
///
/// AppKit 的菜单加速键只对带修饰键的组合可靠拦截（⌘/⇧/⌥）；无修饰键的
/// 组合（如 Space、Esc）对可打印字符不会触发菜单项，必须由 egui 自己处理，
/// 否则按键会两头落空（原生菜单收不到、egui 又跳过）。
pub fn native_menu_handles(combo: &KeyCombo) -> bool {
    combo.command || combo.shift || combo.alt
}

/// 是否是可录制的主键（非纯修饰键）。
pub fn is_recordable_key(key: Key) -> bool {
    RECORDABLE_KEYS.contains(&key)
}

/// 动作 id → 动作名的 i18n key（设置页/标题栏/菜单共用，单一来源）。
pub fn action_label_key(action_id: &str) -> &'static str {
    use yinhe_editor_core::shortcuts as sc;
    match action_id {
        sc::ACTION_NEW_PROJECT => "file.new_project",
        sc::ACTION_OPEN => "file.open",
        sc::ACTION_IMPORT_AUDIO => "file.import_audio",
        sc::ACTION_SAVE => "file.save",
        sc::ACTION_SAVE_AS => "file.save_as",
        sc::ACTION_CLOSE_DOCUMENT => "file.close",
        sc::ACTION_EXPORT_AUDIO => "file.export_audio",
        sc::ACTION_EXPORT_MIDI => "file.export_midi",
        sc::ACTION_SETTINGS => "file.settings",
        sc::ACTION_PROJECT_SETTINGS => "file.project_settings",
        sc::ACTION_EXIT => "file.exit",
        sc::ACTION_UNDO => "menu.undo",
        sc::ACTION_REDO => "menu.redo",
        sc::ACTION_CUT => "menu.cut",
        sc::ACTION_COPY => "menu.copy",
        sc::ACTION_PASTE => "menu.paste",
        sc::ACTION_PASTE_AT_ORIGINAL => "menu.paste_at_original",
        sc::ACTION_PASTE_FLIPPED => "menu.paste_flipped",
        sc::ACTION_SELECT_ALL => "menu.select_all",
        sc::ACTION_SELECT_NOTES_ONLY => "menu.select_notes_only",
        sc::ACTION_FILTER_SELECTION => "menu.filter_selection",
        sc::ACTION_DUPLICATE => "menu.duplicate",
        sc::ACTION_DELETE => "menu.delete",
        sc::ACTION_TRANSPOSE_UP => "menu.octave_up",
        sc::ACTION_TRANSPOSE_DOWN => "menu.octave_down",
        sc::ACTION_DEDUP_WITHIN_TRACK => "menu.dedup_within_track",
        sc::ACTION_DEDUP_ACROSS_TRACKS => "menu.dedup_across_tracks",
        sc::ACTION_HUMANIZE => "menu.humanize",
        sc::ACTION_FILL_GAPS => "menu.fill_gaps",
        sc::ACTION_MERGE_NOTES => "menu.merge_notes",
        sc::ACTION_LENGTH_DOUBLE => "menu.length_double",
        sc::ACTION_LENGTH_HALF => "menu.length_half",
        sc::ACTION_FLIP_HORIZONTAL => "sel.flip_horizontal",
        sc::ACTION_FLIP_VERTICAL => "sel.flip_vertical",
        sc::ACTION_GRID_SPLIT => "menu.grid_split",
        sc::ACTION_TOGGLE_PLAY => "shortcuts.play_toggle",
        sc::ACTION_STOP => "shortcuts.stop",
        sc::ACTION_TOOL_SELECT => "shortcuts.tool_select",
        sc::ACTION_TOOL_SELECT_VERTICAL => "shortcuts.tool_select_vertical",
        sc::ACTION_TOOL_PAN => "shortcuts.tool_pan",
        sc::ACTION_TOOL_PENCIL => "shortcuts.tool_pencil",
        sc::ACTION_TOOL_SCISSORS => "shortcuts.tool_scissors",
        sc::ACTION_TOOL_ERASER => "shortcuts.tool_eraser",
        sc::ACTION_TOOL_LINE => "shortcuts.tool_line",
        sc::ACTION_TOOL_BRUSH => "shortcuts.tool_brush",
        // 轨道操作（菜单可达，默认无快捷键）
        sc::ACTION_TRACK_ADD_BELOW => "arrange.add_below",
        sc::ACTION_TRACK_ADD_ABOVE => "arrange.add_above",
        sc::ACTION_TRACK_MOVE_UP => "arrange.move_up",
        sc::ACTION_TRACK_MOVE_DOWN => "arrange.move_down",
        sc::ACTION_TRACK_DELETE => "arrange.delete_track",
        sc::ACTION_TRACK_RENAME => "arrange.rename_track",
        sc::ACTION_TRACK_PROPERTIES => "arrange.track_properties",
        sc::ACTION_ADD_AUTOMATION => "arrange.add_automation",
        sc::ACTION_DELETE_AUTOMATION => "arrange.delete_automation",
        // 音频片段操作
        sc::ACTION_AUDIO_SPLIT => "arrange.audio_split",
        sc::ACTION_AUDIO_DUPLICATE => "arrange.audio_duplicate",
        sc::ACTION_AUDIO_REVERSE => "arrange.audio_reverse",
        sc::ACTION_AUDIO_NORMALIZE => "arrange.audio_normalize",
        sc::ACTION_AUDIO_GAIN_UP => "arrange.audio_gain_up",
        sc::ACTION_AUDIO_GAIN_DOWN => "arrange.audio_gain_down",
        sc::ACTION_AUDIO_GAIN_RESET => "arrange.audio_gain_reset",
        sc::ACTION_AUDIO_DELETE => "arrange.audio_delete",
        // 事件 / 标记
        sc::ACTION_EVENT_INSERT_ABOVE => "event_browser.menu.insert_above",
        sc::ACTION_EVENT_INSERT_BELOW => "event_browser.menu.insert_below",
        sc::ACTION_DELETE_MARKER => "menu.delete_marker",
        sc::ACTION_INSERT_MARKER => "menu.insert_marker",
        _ => "shortcuts.unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_menu_handles_requires_modifier() {
        let bare = KeyCombo {
            command: false,
            shift: false,
            alt: false,
            key: "Space".to_string(),
        };
        assert!(!native_menu_handles(&bare), "无修饰键快捷键应由 egui 处理");

        let with_cmd = KeyCombo {
            command: true,
            shift: false,
            alt: false,
            key: "S".to_string(),
        };
        assert!(native_menu_handles(&with_cmd), "⌘ 组合应由原生菜单处理");

        let with_shift = KeyCombo {
            command: false,
            shift: true,
            alt: false,
            key: "ArrowUp".to_string(),
        };
        assert!(native_menu_handles(&with_shift), "⇧ 组合应由原生菜单处理");
    }
}
