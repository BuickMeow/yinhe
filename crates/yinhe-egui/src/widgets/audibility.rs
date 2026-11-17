//! 听感（M/S）三态按钮的统一视觉与交互。
//!
//! 用户口径：**单按钮**，按钮始终显示 `M`；亮 = 开、灭 = 关。
//! 左键切换静音（`Normal ↔ Muted`），右键切换独奏（`Normal ↔ Soloed`）。
//! 两种状态互斥，不会同时亮；颜色区分：静音用 `mute_active`，独奏用
//! `solo_active`。不再显示 `S` 字符，也不再有第二个按钮。

use eframe::egui;
use yinhe_types::Audibility;

/// 三态按钮的视觉：`(label, active, active_color)`。
///
/// label 恒为 `"M"`；`active` 表示当前有任一状态开启。
pub(crate) fn visual(audibility: Audibility) -> (&'static str, bool, egui::Color32) {
    match audibility {
        Audibility::Normal => ("M", false, crate::theme::mute_active()),
        Audibility::Muted => ("M", true, crate::theme::mute_active()),
        Audibility::Soloed => ("M", true, crate::theme::solo_active()),
    }
}

/// 左键：`Normal ↔ Muted`（从独奏按左键直接切到静音，保持互斥）。
pub(crate) fn next_on_primary(audibility: Audibility) -> Audibility {
    match audibility {
        Audibility::Muted => Audibility::Normal,
        Audibility::Normal | Audibility::Soloed => Audibility::Muted,
    }
}

/// 右键：`Normal ↔ Soloed`（从静音按右键直接切到独奏，保持互斥）。
pub(crate) fn next_on_secondary(audibility: Audibility) -> Audibility {
    match audibility {
        Audibility::Soloed => Audibility::Normal,
        Audibility::Normal | Audibility::Muted => Audibility::Soloed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_always_shows_m() {
        for a in [Audibility::Normal, Audibility::Muted, Audibility::Soloed] {
            assert_eq!(visual(a).0, "M", "按钮文字不应随状态变化");
        }
        assert!(!visual(Audibility::Normal).1);
        assert!(visual(Audibility::Muted).1);
        assert!(visual(Audibility::Soloed).1);
    }

    #[test]
    fn transitions_are_mutually_exclusive() {
        assert_eq!(next_on_primary(Audibility::Normal), Audibility::Muted);
        assert_eq!(next_on_primary(Audibility::Muted), Audibility::Normal);
        assert_eq!(next_on_primary(Audibility::Soloed), Audibility::Muted);
        assert_eq!(next_on_secondary(Audibility::Normal), Audibility::Soloed);
        assert_eq!(next_on_secondary(Audibility::Soloed), Audibility::Normal);
        assert_eq!(next_on_secondary(Audibility::Muted), Audibility::Soloed);
    }
}
