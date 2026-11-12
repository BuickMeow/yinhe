//! ANSI 104 物理键盘布局表（快捷键设置的键盘视图用）。
//!
//! 只描述「键在键盘上的位置与外观」，不含快捷键逻辑；每个键通过 `key()`
//! 映射回 `egui::Key`，再交给 `Keybindings` 查绑定。
//!
//! 尺寸遵循 ANSI 104 的国际标准键位单位（1u = 普通字母键宽）：
//! Tab 1.5u、Caps 1.75u、Enter 2.25u、Backspace 2u、左右 Shift 2.25u/2.75u、
//! Ctrl/Alt/Cmd 各 1.25u、Space 6.25u。
//!
//! 修饰键的**称呼随平台变化**（macOS 是 ⌘ Command / ⌥ Option / ⌃ Control，
//! Windows/Linux 是 Ctrl / Alt / Win），见 [`Modifier::label`]。

use eframe::egui::Key;

/// 纯修饰键（不参与绑定，只占位并按真实状态高亮）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Modifier {
    Shift,
    Ctrl,
    Alt,
    /// macOS 的 ⌘ / Windows 的 Win 键。
    Cmd,
}

impl Modifier {
    /// 该修饰键在当前输入状态下是否按下。
    pub(crate) fn active(self, mods: egui::Modifiers) -> bool {
        match self {
            Modifier::Shift => mods.shift,
            Modifier::Ctrl => mods.ctrl,
            Modifier::Alt => mods.alt,
            Modifier::Cmd => mods.command || mods.mac_cmd,
        }
    }

    /// 平台相关称呼。
    pub(crate) fn label(self) -> &'static str {
        let mac = cfg!(target_os = "macos");
        match self {
            Modifier::Shift => "Shift",
            Modifier::Ctrl => "Ctrl",
            Modifier::Alt => {
                if mac {
                    "Option"
                } else {
                    "Alt"
                }
            }
            Modifier::Cmd => {
                if mac {
                    "Command"
                } else {
                    "Win"
                }
            }
        }
    }
}

/// 一个物理按键。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct KeyCap {
    /// 键位对应的 egui 键；`None` = 纯修饰键或占位。
    pub key: Option<Key>,
    /// 纯修饰键（不参与绑定）。
    pub modifier: Option<Modifier>,
    /// 键帽主标签（修饰键用 `Modifier::label()`，这里留空）。
    pub label: &'static str,
    /// 相对宽度（1u = 普通键）。
    pub width: f32,
}

impl KeyCap {
    const fn plain(key: Key, label: &'static str) -> Self {
        Self {
            key: Some(key),
            modifier: None,
            label,
            width: 1.0,
        }
    }
    const fn wide(key: Key, label: &'static str, width: f32) -> Self {
        Self {
            key: Some(key),
            modifier: None,
            label,
            width,
        }
    }
    const fn modifier(m: Modifier, width: f32) -> Self {
        Self {
            key: None,
            modifier: Some(m),
            label: "",
            width,
        }
    }
    /// 空占位（布局对齐用）。
    const fn gap(width: f32) -> Self {
        Self {
            key: None,
            modifier: None,
            label: "",
            width,
        }
    }
}

/// 键盘一行。
#[derive(Clone, Copy, Debug)]
pub(crate) struct KeyRow {
    pub caps: &'static [KeyCap],
    /// 该行左侧缩进（单位 u）。
    pub indent: f32,
}

const fn row(indent: f32, caps: &'static [KeyCap]) -> KeyRow {
    KeyRow { indent, caps }
}

/// ANSI 104 布局（美式全键盘）。
pub(crate) fn ansi_layout() -> &'static [KeyRow] {
    &ANSI
}

static ANSI: [KeyRow; 6] = [
    // ── 功能键行 ──
    row(
        0.0,
        &[
            KeyCap::plain(Key::Escape, "Esc"),
            KeyCap::gap(1.0),
            KeyCap::plain(Key::F1, "F1"),
            KeyCap::plain(Key::F2, "F2"),
            KeyCap::plain(Key::F3, "F3"),
            KeyCap::plain(Key::F4, "F4"),
            KeyCap::gap(0.5),
            KeyCap::plain(Key::F5, "F5"),
            KeyCap::plain(Key::F6, "F6"),
            KeyCap::plain(Key::F7, "F7"),
            KeyCap::plain(Key::F8, "F8"),
            KeyCap::gap(0.5),
            KeyCap::plain(Key::F9, "F9"),
            KeyCap::plain(Key::F10, "F10"),
            KeyCap::plain(Key::F11, "F11"),
            KeyCap::plain(Key::F12, "F12"),
        ],
    ),
    // ── 数字行（右侧为导航区）──
    row(
        0.0,
        &[
            KeyCap::plain(Key::Backtick, "`"),
            KeyCap::plain(Key::Num1, "1"),
            KeyCap::plain(Key::Num2, "2"),
            KeyCap::plain(Key::Num3, "3"),
            KeyCap::plain(Key::Num4, "4"),
            KeyCap::plain(Key::Num5, "5"),
            KeyCap::plain(Key::Num6, "6"),
            KeyCap::plain(Key::Num7, "7"),
            KeyCap::plain(Key::Num8, "8"),
            KeyCap::plain(Key::Num9, "9"),
            KeyCap::plain(Key::Num0, "0"),
            KeyCap::plain(Key::Minus, "-"),
            KeyCap::plain(Key::Equals, "="),
            KeyCap::wide(Key::Backspace, "Backspace", 2.0),
            KeyCap::gap(0.5),
            KeyCap::plain(Key::Insert, "Ins"),
            KeyCap::plain(Key::Home, "Home"),
            KeyCap::plain(Key::PageUp, "PgUp"),
        ],
    ),
    // ── QWERTY 行 ──
    row(
        0.0,
        &[
            KeyCap::wide(Key::Tab, "Tab", 1.5),
            KeyCap::plain(Key::Q, "Q"),
            KeyCap::plain(Key::W, "W"),
            KeyCap::plain(Key::E, "E"),
            KeyCap::plain(Key::R, "R"),
            KeyCap::plain(Key::T, "T"),
            KeyCap::plain(Key::Y, "Y"),
            KeyCap::plain(Key::U, "U"),
            KeyCap::plain(Key::I, "I"),
            KeyCap::plain(Key::O, "O"),
            KeyCap::plain(Key::P, "P"),
            KeyCap::plain(Key::OpenBracket, "["),
            KeyCap::plain(Key::CloseBracket, "]"),
            KeyCap::wide(Key::Backslash, "\\", 1.5),
            KeyCap::gap(0.5),
            KeyCap::plain(Key::Delete, "Del"),
            KeyCap::plain(Key::End, "End"),
            KeyCap::plain(Key::PageDown, "PgDn"),
        ],
    ),
    // ── ASDF 行 ──
    row(
        0.0,
        &[
            KeyCap::modifier(Modifier::Shift, 2.25),
            KeyCap::plain(Key::A, "A"),
            KeyCap::plain(Key::S, "S"),
            KeyCap::plain(Key::D, "D"),
            KeyCap::plain(Key::F, "F"),
            KeyCap::plain(Key::G, "G"),
            KeyCap::plain(Key::H, "H"),
            KeyCap::plain(Key::J, "J"),
            KeyCap::plain(Key::K, "K"),
            KeyCap::plain(Key::L, "L"),
            KeyCap::plain(Key::Semicolon, ";"),
            KeyCap::plain(Key::Quote, "'"),
            KeyCap::wide(Key::Enter, "Enter", 2.25),
        ],
    ),
    // ── ZXCV 行 ──
    row(
        0.0,
        &[
            KeyCap::modifier(Modifier::Ctrl, 1.25),
            KeyCap::modifier(Modifier::Alt, 1.25),
            KeyCap::modifier(Modifier::Cmd, 1.25),
            KeyCap::wide(Key::Space, "Space", 6.25),
            KeyCap::modifier(Modifier::Cmd, 1.25),
            KeyCap::modifier(Modifier::Alt, 1.25),
            KeyCap::plain(Key::ArrowLeft, "←"),
            KeyCap::plain(Key::ArrowUp, "↑"),
            KeyCap::plain(Key::ArrowDown, "↓"),
            KeyCap::plain(Key::ArrowRight, "→"),
        ],
    ),
    // ── 字母区最后一行（Z 行）──
    row(
        1.5,
        &[
            KeyCap::plain(Key::Z, "Z"),
            KeyCap::plain(Key::X, "X"),
            KeyCap::plain(Key::C, "C"),
            KeyCap::plain(Key::V, "V"),
            KeyCap::plain(Key::B, "B"),
            KeyCap::plain(Key::N, "N"),
            KeyCap::plain(Key::M, "M"),
            KeyCap::plain(Key::Comma, ","),
            KeyCap::plain(Key::Period, "."),
            KeyCap::plain(Key::Slash, "/"),
        ],
    ),
];

/// `=` 键帽同时代表 `Plus`（Shift+`=`）：两个 Key 共用一个物理键位。
pub(crate) fn alias_keys(key: Key) -> &'static [Key] {
    match key {
        Key::Equals => &[Key::Plus],
        _ => &[],
    }
}

/// 布局里出现过的全部可绑定键（去重，供一致性测试与遍历）。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn layout_keys() -> Vec<Key> {
    let mut out: Vec<Key> = Vec::new();
    for row in ansi_layout() {
        for cap in row.caps {
            if let Some(key) = cap.key
                && !out.contains(&key)
            {
                out.push(key);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shortcuts::RECORDABLE_KEYS;

    /// 布局里的可绑定键必须都在 `RECORDABLE_KEYS` 里。
    #[test]
    fn layout_keys_are_all_recordable() {
        for key in layout_keys() {
            assert!(
                RECORDABLE_KEYS.contains(&key),
                "布局里有 {key:?}，但它不在 RECORDABLE_KEYS 里"
            );
        }
    }

    /// 反向：可录制的键都应该能在键盘上找到（别名键也算）。
    #[test]
    fn recordable_keys_are_all_on_layout() {
        let mut on_layout = layout_keys();
        for &key in &on_layout.clone() {
            on_layout.extend_from_slice(alias_keys(key));
        }
        for &key in RECORDABLE_KEYS {
            assert!(
                on_layout.contains(&key),
                "RECORDABLE_KEYS 里的 {key:?} 没有出现在布局上"
            );
        }
    }

    /// 同一个键不能出现两次（左右修饰键是纯占位，不参与绑定）。
    #[test]
    fn layout_has_no_duplicate_bindable_keys() {
        let mut seen: Vec<Key> = Vec::new();
        for row in ansi_layout() {
            for cap in row.caps {
                let Some(key) = cap.key else { continue };
                assert!(!seen.contains(&key), "{key:?} 在布局里出现了两次");
                seen.push(key);
            }
        }
    }

    /// 修饰键称呼随平台变化：macOS 用 Command / Option。
    #[test]
    fn modifier_labels_are_platform_specific() {
        let mac = cfg!(target_os = "macos");
        assert_eq!(Modifier::Cmd.label(), if mac { "Command" } else { "Win" });
        assert_eq!(Modifier::Alt.label(), if mac { "Option" } else { "Alt" });
        // Shift 两个平台同名。
        assert_eq!(Modifier::Shift.label(), "Shift");
    }

    /// Space 必须是国际标准的 6.25u（用户反馈键盘错位的主要来源）。
    #[test]
    fn space_is_six_and_quarter_units() {
        let space = ansi_layout()
            .iter()
            .flat_map(|r| r.caps.iter())
            .find(|c| c.key == Some(Key::Space))
            .expect("布局里应有空格键");
        assert!((space.width - 6.25).abs() < 1e-6, "Space 应为 6.25u");
    }
}
