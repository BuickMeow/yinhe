//! macOS 平台集成：文档脏点、原生菜单、Finder 打开、App Nap 等

use std::sync::mpsc;

use objc2::runtime::AnyClass;
use rust_i18n::t;

use yinhe_editor_core::follow::FollowMode;
use yinhe_editor_core::shortcuts::Keybindings;

use super::MenuAction;

pub(crate) mod accelerator;
pub(crate) mod menu;
pub(crate) mod menu_refresh;
pub(crate) mod open_files;
pub(crate) mod window;

use menu::{menu_map, menu_sender};

pub(crate) use window::{
    disable_background_window_drag, request_user_attention, set_app_nap_enabled,
    set_document_edited,
};

pub(crate) use open_files::register_open_files_handler;

pub(super) fn cls(name: &std::ffi::CStr) -> Option<&'static AnyClass> {
    AnyClass::get(name)
}

macro_rules! cstr {
    ($s:literal) => {
        std::ffi::CStr::from_bytes_with_nul(concat!($s, "\0").as_bytes()).unwrap()
    };
}
pub(super) use cstr;

pub(crate) struct MenuBarInner {
    rx: mpsc::Receiver<MenuAction>,
    open_files_rx: mpsc::Receiver<String>,
    last_locale: String,
    last_keybindings: Keybindings,
    accelerators_suspended: bool,
    last_recent_files: Vec<String>,
    last_follow_mode: Option<FollowMode>,
}

impl MenuBarInner {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        *menu_sender().lock().unwrap() = Some(tx);
        let open_files_rx = register_open_files_handler();
        if let Err(e) = menu::init_native_menu() {
            tracing::error!("Failed to init macOS menu bar: {e:?}");
        }
        Self {
            rx,
            open_files_rx,
            last_locale: rust_i18n::locale().to_string(),
            last_keybindings: Keybindings::default(),
            accelerators_suspended: false,
            last_recent_files: Vec::new(),
            last_follow_mode: None,
        }
    }

    pub fn poll(
        &mut self,
        keybindings: &Keybindings,
        suspend: bool,
        recent_files: &[String],
        follow_mode: FollowMode,
    ) -> Vec<MenuAction> {
        if recent_files != self.last_recent_files {
            self.last_recent_files = recent_files.to_vec();
            menu_refresh::refresh_recent_submenu(recent_files);
        }
        if self.last_follow_mode != Some(follow_mode) {
            self.last_follow_mode = Some(follow_mode);
            menu_refresh::refresh_follow_checks(follow_mode);
        }
        let locale = rust_i18n::locale();
        if *locale != self.last_locale {
            self.last_locale = locale.to_string();
            menu_refresh::refresh_native_menu_texts();
        }
        if suspend {
            if !self.accelerators_suspended {
                self.accelerators_suspended = true;
                menu::NATIVE_MENU.with(|cell| {
                    if let Some(native) = cell.get() {
                        accelerator::clear_accelerators(&native._items);
                    }
                });
            }
        } else if self.accelerators_suspended || keybindings != &self.last_keybindings {
            self.accelerators_suspended = false;
            self.last_keybindings = keybindings.clone();
            menu::NATIVE_MENU.with(|cell| {
                if let Some(native) = cell.get() {
                    accelerator::refresh_accelerators(&native._items, keybindings);
                }
            });
        }
        std::iter::from_fn(|| self.rx.try_recv().ok()).collect()
    }

    pub fn poll_open_files(&mut self) -> Vec<String> {
        std::iter::from_fn(|| self.open_files_rx.try_recv().ok()).collect()
    }
}
