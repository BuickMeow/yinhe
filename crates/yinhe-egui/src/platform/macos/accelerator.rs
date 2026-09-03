use muda::accelerator::{Accelerator, Code, Modifiers};

use yinhe_editor_core::shortcuts::{self as sc, Keybindings};

/// 菜单项翻译 key → 快捷键动作 id
pub(super) fn menu_action_id(label_key: &str) -> Option<&'static str> {
    use crate::chrome::transport_bar::{EditAction, FileAction};
    if let Some(a) = FileAction::ALL.iter().find(|a| a.label_key() == label_key) {
        return Some(a.action_id());
    }
    if let Some(a) = EditAction::ALL.iter().find(|a| a.label_key() == label_key) {
        return Some(a.action_id());
    }
    match label_key {
        "shortcuts.play_toggle" => Some(sc::ACTION_TOGGLE_PLAY),
        "shortcuts.stop" => Some(sc::ACTION_STOP),
        "menu.settings" => Some(sc::ACTION_SETTINGS),
        "menu.quit" => Some(sc::ACTION_EXIT),
        _ => None,
    }
}

/// 键名 → 加速键码
pub(super) fn str_to_muda_code(s: &str) -> Option<Code> {
    let alpha = |c: char| -> Option<Code> {
        Some(match c {
            'A' => Code::KeyA,
            'B' => Code::KeyB,
            'C' => Code::KeyC,
            'D' => Code::KeyD,
            'E' => Code::KeyE,
            'F' => Code::KeyF,
            'G' => Code::KeyG,
            'H' => Code::KeyH,
            'I' => Code::KeyI,
            'J' => Code::KeyJ,
            'K' => Code::KeyK,
            'L' => Code::KeyL,
            'M' => Code::KeyM,
            'N' => Code::KeyN,
            'O' => Code::KeyO,
            'P' => Code::KeyP,
            'Q' => Code::KeyQ,
            'R' => Code::KeyR,
            'S' => Code::KeyS,
            'T' => Code::KeyT,
            'U' => Code::KeyU,
            'V' => Code::KeyV,
            'W' => Code::KeyW,
            'X' => Code::KeyX,
            'Y' => Code::KeyY,
            'Z' => Code::KeyZ,
            _ => return None,
        })
    };
    let digit = |c: char| -> Option<Code> {
        Some(match c {
            '0' => Code::Digit0,
            '1' => Code::Digit1,
            '2' => Code::Digit2,
            '3' => Code::Digit3,
            '4' => Code::Digit4,
            '5' => Code::Digit5,
            '6' => Code::Digit6,
            '7' => Code::Digit7,
            '8' => Code::Digit8,
            '9' => Code::Digit9,
            _ => return None,
        })
    };
    let first = s.chars().next()?;
    if s.chars().count() == 1 {
        if first.is_ascii_alphabetic() {
            return alpha(first.to_ascii_uppercase());
        }
        if first.is_ascii_digit() {
            return digit(first);
        }
    }
    Some(match s {
        "F1" => Code::F1,
        "F2" => Code::F2,
        "F3" => Code::F3,
        "F4" => Code::F4,
        "F5" => Code::F5,
        "F6" => Code::F6,
        "F7" => Code::F7,
        "F8" => Code::F8,
        "F9" => Code::F9,
        "F10" => Code::F10,
        "F11" => Code::F11,
        "F12" => Code::F12,
        "Space" => Code::Space,
        "Enter" => Code::Enter,
        "Escape" => Code::Escape,
        "Tab" => Code::Tab,
        "Backspace" => Code::Backspace,
        "Delete" => Code::Delete,
        "Insert" => Code::Insert,
        "Home" => Code::Home,
        "End" => Code::End,
        "PageUp" => Code::PageUp,
        "PageDown" => Code::PageDown,
        "ArrowUp" => Code::ArrowUp,
        "ArrowDown" => Code::ArrowDown,
        "ArrowLeft" => Code::ArrowLeft,
        "ArrowRight" => Code::ArrowRight,
        "Comma" => Code::Comma,
        "Period" => Code::Period,
        "Minus" => Code::Minus,
        "Plus" => Code::Equal,
        "Semicolon" => Code::Semicolon,
        "Quote" => Code::Quote,
        "Backtick" => Code::Backquote,
        "Backslash" => Code::Backslash,
        "Slash" => Code::Slash,
        _ => return None,
    })
}

/// KeyCombo → 原生加速键（无修饰键返回 None，由 egui 处理）
pub(super) fn combo_to_accelerator(
    combo: &yinhe_editor_core::shortcuts::KeyCombo,
) -> Option<Accelerator> {
    if !crate::shortcuts::native_menu_handles(combo) {
        return None;
    }
    let mut mods = Modifiers::empty();
    if combo.command {
        mods |= Modifiers::SUPER;
    }
    if combo.shift {
        mods |= Modifiers::SHIFT;
    }
    if combo.alt {
        mods |= Modifiers::ALT;
    }
    Some(Accelerator::new(Some(mods), str_to_muda_code(&combo.key)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_editor_core::shortcuts::KeyCombo;

    #[test]
    fn bare_key_combo_gets_no_accelerator() {
        let bare = KeyCombo {
            command: false,
            shift: false,
            alt: false,
            key: "Space".to_string(),
        };
        assert!(combo_to_accelerator(&bare).is_none());
        let esc = KeyCombo {
            command: false,
            shift: false,
            alt: false,
            key: "Escape".to_string(),
        };
        assert!(combo_to_accelerator(&esc).is_none());
    }

    #[test]
    fn modified_combo_gets_accelerator() {
        let with_cmd = KeyCombo {
            command: true,
            shift: false,
            alt: false,
            key: "S".to_string(),
        };
        assert!(combo_to_accelerator(&with_cmd).is_some());
    }
}

/// 快捷键表变化时刷新原生菜单加速键
pub(super) fn refresh_accelerators(
    items: &[(&'static str, Box<dyn super::menu::MenuText>)],
    keybindings: &Keybindings,
) {
    for (key, item) in items {
        if let Some(action_id) = menu_action_id(key) {
            let acc = keybindings
                .get(action_id)
                .first()
                .and_then(combo_to_accelerator);
            item.update_accelerator(acc);
        }
    }
}

pub(super) fn clear_accelerators(items: &[(&'static str, Box<dyn super::menu::MenuText>)]) {
    for (key, item) in items {
        if menu_action_id(key).is_some() {
            item.update_accelerator(None);
        }
    }
}
