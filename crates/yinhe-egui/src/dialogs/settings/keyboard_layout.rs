//! ANSI 104 物理键盘布局表（快捷键设置的键盘视图用）。
//!
//! 这份表只描述「键在键盘上的位置与外观」，不含任何快捷键逻辑；
//! 每个键通过 `key()` 映射回 `egui::Key`，再交给 `Keybindings` 查绑定。
//!
//! 与 [`crate::shortcuts::RECORDABLE_KEYS`] 的一致性由测试保证：
//! 布局里出现的键必须全部可录制，反之可录制的键也不该在键盘上找不到
//! （否则用户能在录制器里录到、却永远看不到它的占用情况）。

use eframe::egui::Key;

/// 一个物理按键。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct KeyCap {
    /// 键位对应的 egui 键；`None` = 纯修饰键（Shift/Ctrl/Alt/Cmd），
    /// 它们不参与绑定，但要在图上占位并随真实状态高亮。
    pub key: Option<Key>,
    /// 键帽主标签（`A`、`1`、`F1`、`Esc`…）。
    pub label: &'static str,
    /// 相对宽度（单位 = 基础键宽）。普通键 1.0，`Backspace` 2.0，`Space` 6.25 等。
    pub width: f32,
}

impl KeyCap {
    const fn plain(key: Key, label: &'static str) -> Self {
        Self {
            key: Some(key),
            label,
            width: 1.0,
        }
    }
    const fn wide(key: Key, label: &'static str, width: f32) -> Self {
        Self {
            key: Some(key),
            label,
            width,
        }
    }
    /// 纯修饰键占位（不参与绑定）。
    const fn modifier(label: &'static str, width: f32) -> Self {
        Self {
            key: None,
            label,
            width,
        }
    }
    /// 空占位（对齐用，例如 ANSI 布局里 ISO 才有的键位置）。
    const fn gap(width: f32) -> Self {
        Self {
            key: None,
            label: "",
            width,
        }
    }
}

/// 键盘一行。
#[derive(Clone, Copy, Debug)]
pub(crate) struct KeyRow {
    pub caps: &'static [KeyCap],
    /// 该行在整块键盘里的左侧缩进（单位 = 基础键宽）。
    pub indent: f32,
}

const fn row(indent: f32, caps: &'static [KeyCap]) -> KeyRow {
    KeyRow { indent, caps }
}

/// ANSI 104 布局（美式），5 行主键区 + 功能键行。
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
    // ── 数字行 ──
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
            // `=` 与 `+` 共用一个物理键（Shift+`=`），键帽只画一次；
            // `plus_alias` 让它同时统计 `Plus` 的绑定。
            KeyCap::plain(Key::Equals, "="),
            KeyCap::wide(Key::Backspace, "Backspace", 1.5),
            // 导航区（与主键区之间留半键宽）
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
            KeyCap::modifier("Caps", 1.75),
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
            KeyCap::modifier("Shift", 2.25),
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
            KeyCap::modifier("Shift", 2.75),
        ],
    ),
    // ── 底行 ──
    row(
        0.0,
        &[
            KeyCap::modifier("Ctrl", 1.25),
            KeyCap::modifier("Alt", 1.25),
            KeyCap::modifier("Cmd", 1.25),
            KeyCap::wide(Key::Space, "Space", 6.25),
            KeyCap::modifier("Cmd", 1.25),
            KeyCap::modifier("Alt", 1.25),
            KeyCap::plain(Key::ArrowLeft, "←"),
            KeyCap::plain(Key::ArrowUp, "↑"),
            KeyCap::plain(Key::ArrowDown, "↓"),
            KeyCap::plain(Key::ArrowRight, "→"),
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

/// 布局里出现过的全部可绑定键（去重，供一致性测试与「全部按键」遍历）。
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

    /// 布局里的可绑定键必须都在 `RECORDABLE_KEYS` 里：
    /// 否则用户按这个键录制时会被判成「不可录制」，键盘图却画着它。
    #[test]
    fn layout_keys_are_all_recordable() {
        for key in layout_keys() {
            assert!(
                RECORDABLE_KEYS.contains(&key),
                "布局里有 {key:?}，但它不在 RECORDABLE_KEYS 里"
            );
        }
    }

    /// 反向：`RECORDABLE_KEYS` 里的键都应该能在键盘上找到，
    /// 否则它占用情况永远不可见（例如 F13 之类）。
    #[test]
    fn recordable_keys_are_all_on_layout() {
        let mut on_layout = layout_keys();
        // 别名键（如 Plus 借 `=` 的物理键位）也算"在布局上"。
        for &key in &on_layout.clone() {
            on_layout.extend_from_slice(alias_keys(key));
        }
        for &key in RECORDABLE_KEYS {
            assert!(
                on_layout.contains(&key),
                "RECORDABLE_KEYS 里的 {key:?} 没有出现在 ANSI 布局上"
            );
        }
    }

    /// 同一个键不能在布局里出现两次（左右修饰键是纯占位，不参与绑定）。
    #[test]
    fn layout_has_no_unexpected_duplicates() {
        let mut seen: Vec<Key> = Vec::new();
        for row in ansi_layout() {
            for cap in row.caps {
                let Some(key) = cap.key else { continue };
                assert!(!seen.contains(&key), "{key:?} 在布局里出现了两次");
                seen.push(key);
            }
        }
    }
}
