use muda::{MenuId, MenuItem};
use rust_i18n::t;
use std::collections::HashMap;
use std::sync::Mutex;

use super::menu::{MenuText, NATIVE_MENU, menu_map};
use yinhe_editor_core::follow::FollowMode;

pub(super) fn refresh_native_menu_texts() {
    NATIVE_MENU.with(|cell| {
        if let Some(native) = cell.get() {
            for (key, item) in &native._items {
                item.set_text(&t!(*key));
            }
            native.recent_submenu.set_text(&t!("menu.recent_files"));
            for (_, key, item) in &native.follow_checks {
                item.set_text(&t!(*key));
            }
        }
    });
}

pub(super) fn refresh_recent_submenu(files: &[String]) {
    NATIVE_MENU.with(|cell| {
        let Some(native) = cell.get() else { return };
        let mut items = native.recent_items.borrow_mut();
        for item in items.drain(..) {
            let _ = native.recent_submenu.remove(&item);
            if let Some(lock) = menu_map().get()
                && let Ok(mut map) = lock.lock()
            {
                map.remove(item.id());
            }
        }
        native.recent_submenu.set_enabled(!files.is_empty());
        for path in files {
            let item = MenuItem::new(
                crate::chrome::transport_bar::recent_display_name(path),
                true,
                None,
            );
            if let Some(lock) = menu_map().get()
                && let Ok(mut map) = lock.lock()
            {
                map.insert(
                    item.id().clone(),
                    super::MenuAction::OpenRecent(path.clone()),
                );
            }
            let _ = native.recent_submenu.append(&item);
            items.push(item);
        }
    });
}

pub(super) fn refresh_follow_checks(mode: FollowMode) {
    NATIVE_MENU.with(|cell| {
        if let Some(native) = cell.get() {
            for (m, _, item) in &native.follow_checks {
                item.set_checked(*m == mode);
            }
        }
    });
}

/// 供 accelerator.rs 或 menu.rs 使用的辅助：检查 map
#[allow(dead_code)]
pub(super) fn menu_map_ref() -> Option<&'static Mutex<HashMap<MenuId, super::MenuAction>>> {
    menu_map().get()
}
