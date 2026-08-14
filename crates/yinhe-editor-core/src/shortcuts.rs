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

pub const ACTION_TOOL_SELECT: &str = "tool_select";
pub const ACTION_TOOL_SELECT_VERTICAL: &str = "tool_select_vertical";
pub const ACTION_TOOL_PAN: &str = "tool_pan";
pub const ACTION_TOOL_PENCIL: &str = "tool_pencil";
pub const ACTION_TOOL_CURVE: &str = "tool_curve";
pub const ACTION_TOOL_SCISSORS: &str = "tool_scissors";
pub const ACTION_TOOL_ERASER: &str = "tool_eraser";

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
    ACTION_TOOL_SELECT,
    ACTION_TOOL_SELECT_VERTICAL,
    ACTION_TOOL_PAN,
    ACTION_TOOL_PENCIL,
    ACTION_TOOL_CURVE,
    ACTION_TOOL_SCISSORS,
    ACTION_TOOL_ERASER,
];

/// 快捷键表：动作 id → 快捷键列表（空列表 = 无快捷键）。
///
/// 一个动作可绑定多个快捷键；列表顺序即 UI 展示顺序。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keybindings {
    #[serde(deserialize_with = "de_combo_list")]
    pub map: BTreeMap<String, Vec<KeyCombo>>,
}

/// 反序列化兼容：新格式为数组 [combo, ...]；旧配置为单个 combo 或 null。
fn de_combo_list<'de, D>(d: D) -> Result<BTreeMap<String, Vec<KeyCombo>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Combos {
        List(Vec<KeyCombo>),
        Single(Option<KeyCombo>),
    }
    use serde::Deserialize as _;
    let raw = BTreeMap::<String, Combos>::deserialize(d)?;
    Ok(raw
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                match v {
                    Combos::List(list) => list,
                    Combos::Single(Some(c)) => vec![c],
                    Combos::Single(None) => Vec::new(),
                },
            )
        })
        .collect())
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
        // 工具切换默认用主键盘数字键 1-7（按工具栏显示顺序）。
        ACTION_TOOL_SELECT => combo(false, false, false, "1"),
        ACTION_TOOL_SELECT_VERTICAL => combo(false, false, false, "2"),
        ACTION_TOOL_PAN => combo(false, false, false, "3"),
        ACTION_TOOL_PENCIL => combo(false, false, false, "4"),
        ACTION_TOOL_CURVE => combo(false, false, false, "5"),
        ACTION_TOOL_SCISSORS => combo(false, false, false, "6"),
        ACTION_TOOL_ERASER => combo(false, false, false, "7"),
        _ => None,
    }
}

impl Default for Keybindings {
    fn default() -> Self {
        Self {
            map: ALL_ACTION_IDS
                .iter()
                .map(|&id| (id.to_string(), default_combo(id).into_iter().collect()))
                .collect(),
        }
    }
}

impl Keybindings {
    /// 查询动作的快捷键列表。表中有显式条目（含空列表）用条目值；
    /// 缺失（如旧配置升级）回退到该动作的默认值。
    pub fn get(&self, action_id: &str) -> Vec<KeyCombo> {
        match self.map.get(action_id) {
            Some(combos) => combos.clone(),
            None => default_combo(action_id).into_iter().collect(),
        }
    }

    /// 设置动作的快捷键列表（空列表 = 禁用该动作的快捷键）。
    pub fn set(&mut self, action_id: &str, combos: Vec<KeyCombo>) {
        self.map.insert(action_id.to_string(), combos);
    }

    /// 追加一个快捷键（若该动作已有相同组合则忽略）。
    pub fn add(&mut self, action_id: &str, combo: KeyCombo) {
        let combos = self.get(action_id);
        if combos.contains(&combo) {
            return;
        }
        let mut combos = combos;
        combos.push(combo);
        self.set(action_id, combos);
    }

    /// 移除指定快捷键；列表清空等价于禁用。
    pub fn remove(&mut self, action_id: &str, combo: &KeyCombo) {
        let combos = self.get(action_id);
        let combos: Vec<_> = combos.into_iter().filter(|c| c != combo).collect();
        self.set(action_id, combos);
    }

    /// 恢复全部动作到平台默认值。
    pub fn reset_to_defaults(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo(command: bool, shift: bool, key: &str) -> KeyCombo {
        KeyCombo {
            command,
            shift,
            alt: false,
            key: key.to_string(),
        }
    }

    #[test]
    fn serde_roundtrip() {
        let kb = Keybindings::default();
        let json = serde_json::to_string(&kb).unwrap();
        let back: Keybindings = serde_json::from_str(&json).unwrap();
        assert_eq!(kb, back);
    }

    #[test]
    fn serde_reads_old_single_combo_config() {
        // 旧格式：单个 combo 对象（非数组）
        let json = r#"{"map":{"save":{"command":true,"shift":false,"alt":false,"key":"S"}}}"#;
        let kb: Keybindings = serde_json::from_str(json).unwrap();
        assert_eq!(kb.get(ACTION_SAVE), vec![combo(true, false, "S")]);

        // 旧格式：null = 禁用
        let json = r#"{"map":{"save":null}}"#;
        let kb: Keybindings = serde_json::from_str(json).unwrap();
        assert_eq!(kb.get(ACTION_SAVE), Vec::<KeyCombo>::new());
    }

    #[test]
    fn defaults_cover_all_actions() {
        // 每个动作要么有默认快捷键、要么显式默认禁用；
        // 缺失的 id 必须能从 default_combo 回退。
        for &id in ALL_ACTION_IDS {
            let kb = Keybindings::default();
            assert!(kb.map.contains_key(id), "默认表缺少动作 {id}");
            assert_eq!(
                kb.get(id),
                default_combo(id).into_iter().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn get_falls_back_for_missing_id() {
        let kb = Keybindings::default();
        assert_eq!(
            kb.get(ACTION_SAVE),
            default_combo(ACTION_SAVE).into_iter().collect::<Vec<_>>()
        );
        // 从 map 删除后仍回退默认
        let mut kb2 = kb.clone();
        kb2.map.remove(ACTION_SAVE);
        assert_eq!(
            kb2.get(ACTION_SAVE),
            default_combo(ACTION_SAVE).into_iter().collect::<Vec<_>>()
        );
    }

    #[test]
    fn empty_list_disables_shortcut() {
        let mut kb = Keybindings::default();
        kb.set(ACTION_SAVE, Vec::new());
        assert!(kb.get(ACTION_SAVE).is_empty());
    }

    #[test]
    fn multiple_combos_per_action() {
        let mut kb = Keybindings::default();
        kb.add(ACTION_SAVE, combo(true, false, "S"));
        kb.add(ACTION_SAVE, combo(false, true, "F2"));
        assert_eq!(
            kb.get(ACTION_SAVE),
            vec![combo(true, false, "S"), combo(false, true, "F2")]
        );

        // 重复添加被忽略
        kb.add(ACTION_SAVE, combo(true, false, "S"));
        assert_eq!(kb.get(ACTION_SAVE).len(), 2);

        // 移除其中一个，另一个保留
        kb.remove(ACTION_SAVE, &combo(true, false, "S"));
        assert_eq!(kb.get(ACTION_SAVE), vec![combo(false, true, "F2")]);
    }
}
