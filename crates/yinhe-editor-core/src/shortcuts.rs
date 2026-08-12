//! 快捷键配置数据模型（serde 持久化到设置文件）。
//!
//! 这里只定义数据与默认值；egui 侧的键名解析/匹配/显示在
//! `yinhe-egui::shortcuts` 中实现。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 单个快捷键组合。
///
/// `command` 是跨平台"主修饰键"：macOS 为 ⌘（Command），Windows/Linux 为 Ctrl，
/// 与 egui 的 `Modifiers::command` 语义一致，因此同一份配置在两端语义相同。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyCombo {
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
    /// 键名（如 "N"、"Space"、"Comma"、"Delete"、"ArrowUp"），
    /// 由 egui 侧提供 `str ↔ egui::Key` 映射表。
    pub key: String,
}

// ── 动作 id（快捷键表的 key）──

pub const ACTION_NEW_PROJECT: &str = "new_project";
pub const ACTION_OPEN: &str = "open";
pub const ACTION_SAVE: &str = "save";
pub const ACTION_SAVE_AS: &str = "save_as";
pub const ACTION_CLOSE_DOCUMENT: &str = "close_document";
pub const ACTION_EXPORT_AUDIO: &str = "export_audio";
pub const ACTION_EXPORT_MIDI: &str = "export_midi";
pub const ACTION_SETTINGS: &str = "settings";
pub const ACTION_EXIT: &str = "exit";

pub const ACTION_UNDO: &str = "undo";
pub const ACTION_REDO: &str = "redo";
pub const ACTION_CUT: &str = "cut";
pub const ACTION_COPY: &str = "copy";
pub const ACTION_PASTE: &str = "paste";
pub const ACTION_SELECT_ALL: &str = "select_all";
pub const ACTION_DUPLICATE: &str = "duplicate";
pub const ACTION_DELETE: &str = "delete";
pub const ACTION_TRANSPOSE_UP: &str = "transpose_up";
pub const ACTION_TRANSPOSE_DOWN: &str = "transpose_down";

pub const ACTION_TOGGLE_PLAY: &str = "toggle_play";
pub const ACTION_STOP: &str = "stop";

/// 全部可配置动作（设置页展示与默认值完整性检查共用）。
pub const ALL_ACTION_IDS: &[&str] = &[
    ACTION_NEW_PROJECT,
    ACTION_OPEN,
    ACTION_SAVE,
    ACTION_SAVE_AS,
    ACTION_CLOSE_DOCUMENT,
    ACTION_EXPORT_AUDIO,
    ACTION_EXPORT_MIDI,
    ACTION_SETTINGS,
    ACTION_EXIT,
    ACTION_UNDO,
    ACTION_REDO,
    ACTION_CUT,
    ACTION_COPY,
    ACTION_PASTE,
    ACTION_SELECT_ALL,
    ACTION_DUPLICATE,
    ACTION_DELETE,
    ACTION_TRANSPOSE_UP,
    ACTION_TRANSPOSE_DOWN,
    ACTION_TOGGLE_PLAY,
    ACTION_STOP,
];

/// 快捷键表：动作 id → 快捷键（`None` = 显式禁用该动作的快捷键）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keybindings {
    pub map: BTreeMap<String, Option<KeyCombo>>,
}

/// 某个动作的默认快捷键。`None` 表示该动作默认没有快捷键。
fn default_combo(action_id: &str) -> Option<KeyCombo> {
    let combo = |command: bool, shift: bool, alt: bool, key: &str| {
        Some(KeyCombo {
            command,
            shift,
            alt,
            key: key.to_string(),
        })
    };
    match action_id {
        ACTION_NEW_PROJECT => combo(true, false, false, "N"),
        ACTION_OPEN => combo(true, false, false, "O"),
        ACTION_SAVE => combo(true, false, false, "S"),
        ACTION_SAVE_AS => combo(true, true, false, "S"),
        ACTION_CLOSE_DOCUMENT => combo(true, false, false, "W"),
        ACTION_EXPORT_AUDIO => None,
        ACTION_EXPORT_MIDI => None,
        ACTION_SETTINGS => combo(true, false, false, "Comma"),
        ACTION_EXIT => combo(true, false, false, "Q"),
        ACTION_UNDO => combo(true, false, false, "Z"),
        ACTION_REDO => combo(true, true, false, "Z"),
        ACTION_CUT => combo(true, false, false, "X"),
        ACTION_COPY => combo(true, false, false, "C"),
        ACTION_PASTE => combo(true, false, false, "V"),
        ACTION_SELECT_ALL => combo(true, false, false, "A"),
        ACTION_DUPLICATE => combo(true, false, false, "D"),
        ACTION_DELETE => combo(false, false, false, "Delete"),
        ACTION_TRANSPOSE_UP => combo(false, true, false, "ArrowUp"),
        ACTION_TRANSPOSE_DOWN => combo(false, true, false, "ArrowDown"),
        ACTION_TOGGLE_PLAY => combo(false, false, false, "Space"),
        ACTION_STOP => combo(false, false, false, "Escape"),
        _ => None,
    }
}

impl Default for Keybindings {
    fn default() -> Self {
        Self {
            map: ALL_ACTION_IDS
                .iter()
                .map(|&id| (id.to_string(), default_combo(id)))
                .collect(),
        }
    }
}

impl Keybindings {
    /// 查询动作的快捷键。表中有显式条目（含 `None`）用条目值；
    /// 缺失（如旧配置升级）回退到该动作的默认值。
    pub fn get(&self, action_id: &str) -> Option<KeyCombo> {
        match self.map.get(action_id) {
            Some(combo) => combo.clone(),
            None => default_combo(action_id),
        }
    }

    /// 设置动作的快捷键（`None` 表示禁用）。
    pub fn set(&mut self, action_id: &str, combo: Option<KeyCombo>) {
        self.map.insert(action_id.to_string(), combo);
    }

    /// 恢复全部动作到平台默认值。
    pub fn reset_to_defaults(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let kb = Keybindings::default();
        let json = serde_json::to_string(&kb).unwrap();
        let back: Keybindings = serde_json::from_str(&json).unwrap();
        assert_eq!(kb, back);
    }

    #[test]
    fn defaults_cover_all_actions() {
        // 每个动作要么有默认快捷键、要么显式默认禁用；
        // 缺失的 id 必须能从 default_combo 回退。
        for &id in ALL_ACTION_IDS {
            let kb = Keybindings::default();
            assert!(kb.map.contains_key(id), "默认表缺少动作 {id}");
            let combo = kb.get(id);
            assert_eq!(
                combo,
                default_combo(id),
                "get() 回退逻辑与默认表不一致: {id}"
            );
        }
    }

    #[test]
    fn get_falls_back_for_missing_id() {
        let kb = Keybindings::default();
        assert_eq!(kb.get(ACTION_SAVE), default_combo(ACTION_SAVE));
        // 从 map 删除后仍回退默认
        let mut kb2 = kb.clone();
        kb2.map.remove(ACTION_SAVE);
        assert_eq!(kb2.get(ACTION_SAVE), default_combo(ACTION_SAVE));
    }

    #[test]
    fn explicit_none_disables_shortcut() {
        let mut kb = Keybindings::default();
        kb.set(ACTION_SAVE, None);
        assert_eq!(kb.get(ACTION_SAVE), None);
    }
}
