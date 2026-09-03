use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, mpsc};

use muda::{
    CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use rust_i18n::t;

use super::MenuAction;
use super::accelerator::combo_to_accelerator;
use super::window::{hide_app, hide_others, show_about_panel, show_all_apps};
use yinhe_editor_core::follow::FollowMode;
use yinhe_editor_core::shortcuts::Keybindings;

static MENU_SENDER: Mutex<Option<mpsc::Sender<MenuAction>>> = Mutex::new(None);
static MENU_MAP: OnceLock<Mutex<HashMap<MenuId, MenuAction>>> = OnceLock::new();

thread_local! {
    pub(super) static NATIVE_MENU: OnceLock<NativeMenu> = const { OnceLock::new() };
}

pub(crate) trait MenuText: IsMenuItem {
    fn set_text(&self, text: &str);
    fn update_accelerator(&self, accelerator: Option<Accelerator>);
}

impl MenuText for MenuItem {
    fn set_text(&self, text: &str) {
        MenuItem::set_text(self, text);
    }
    fn update_accelerator(&self, accelerator: Option<Accelerator>) {
        if let Err(e) = MenuItem::set_accelerator(self, accelerator) {
            tracing::warn!("Failed to update menu accelerator: {e:?}");
        }
    }
}

impl MenuText for Submenu {
    fn set_text(&self, text: &str) {
        Submenu::set_text(self, text);
    }
    fn update_accelerator(&self, _accelerator: Option<Accelerator>) {}
}

impl MenuText for PredefinedMenuItem {
    fn set_text(&self, _text: &str) {}
    fn update_accelerator(&self, _accelerator: Option<Accelerator>) {}
}

pub(super) struct NativeMenu {
    pub(super) _menu: Menu,
    pub(super) _items: Vec<(&'static str, Box<dyn MenuText>)>,
    pub(super) recent_submenu: Submenu,
    pub(super) recent_items: std::cell::RefCell<Vec<MenuItem>>,
    pub(super) follow_checks: Vec<(FollowMode, &'static str, CheckMenuItem)>,
}

trait MenuActionFrom {
    fn to_menu_action(self) -> MenuAction;
}

impl MenuActionFrom for crate::chrome::transport_bar::FileAction {
    fn to_menu_action(self) -> MenuAction {
        use crate::chrome::transport_bar::FileAction;
        match self {
            FileAction::NewProject => MenuAction::NewProject,
            FileAction::Open => MenuAction::Open,
            FileAction::Save => MenuAction::Save,
            FileAction::SaveAs => MenuAction::SaveAs,
            FileAction::CloseDocument => MenuAction::CloseDocument,
            FileAction::ExportAudio => MenuAction::ExportAudio,
            FileAction::ExportMidi => MenuAction::ExportMidi,
            FileAction::ProjectSettings => MenuAction::ProjectSettings,
            FileAction::Settings => MenuAction::Settings,
            FileAction::Exit => MenuAction::Exit,
        }
    }
}

impl MenuActionFrom for crate::chrome::transport_bar::EditAction {
    fn to_menu_action(self) -> MenuAction {
        use crate::chrome::transport_bar::EditAction;
        match self {
            EditAction::Undo => MenuAction::Undo,
            EditAction::Redo => MenuAction::Redo,
            EditAction::Cut => MenuAction::Cut,
            EditAction::Copy => MenuAction::Copy,
            EditAction::Paste => MenuAction::Paste,
            EditAction::SelectAll => MenuAction::SelectAll,
            EditAction::Duplicate => MenuAction::Duplicate,
            EditAction::Delete => MenuAction::Delete,
            EditAction::TransposeUp => MenuAction::TransposeUp,
            EditAction::TransposeDown => MenuAction::TransposeDown,
            EditAction::DedupWithinTrack => MenuAction::DedupWithinTrack,
            EditAction::DedupAcrossTracks => MenuAction::DedupAcrossTracks,
        }
    }
}

pub(super) fn build_action_submenu<A>(
    map: &mut HashMap<muda::MenuId, MenuAction>,
    items: &mut Vec<(&'static str, Box<dyn MenuText>)>,
    title_key: &'static str,
    groups: &[&[A]],
    default_kb: &Keybindings,
) -> muda::Result<Submenu>
where
    A: Copy + MenuActionFrom + crate::chrome::transport_bar::PopupRow,
{
    let mut rows: Vec<(&'static str, Box<dyn MenuText>)> = Vec::new();
    for (gi, group) in groups.iter().enumerate() {
        if gi > 0 {
            rows.push((title_key, Box::new(PredefinedMenuItem::separator())));
        }
        for &action in *group {
            let accel = default_kb
                .get(action.action_id())
                .first()
                .and_then(combo_to_accelerator);
            let item = Box::new(MenuItem::new(t!(action.label_key()), true, accel));
            map.insert(item.id().clone(), action.to_menu_action());
            rows.push((action.label_key(), item));
        }
    }
    let refs: Vec<&dyn IsMenuItem> = rows
        .iter()
        .map(|(_, b)| b.as_ref() as &dyn IsMenuItem)
        .collect();
    let submenu = Submenu::with_items(t!(title_key), true, &refs)?;
    items.extend(rows);
    Ok(submenu)
}

pub(super) fn init_native_menu() -> muda::Result<()> {
    let mut map = HashMap::new();
    let mut items: Vec<(&'static str, Box<dyn MenuText>)> = Vec::new();
    let cmd = Modifiers::SUPER;
    let default_kb = Keybindings::default();

    let about_item = Box::new(MenuItem::new(t!("menu.about"), true, None));
    map.insert(about_item.id().clone(), MenuAction::About);
    let settings_item = Box::new(MenuItem::new(
        t!("menu.settings"),
        true,
        Some(Accelerator::new(Some(cmd), Code::Comma)),
    ));
    map.insert(settings_item.id().clone(), MenuAction::Settings);
    let hide_item = Box::new(MenuItem::new(
        t!("menu.hide"),
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyH)),
    ));
    map.insert(hide_item.id().clone(), MenuAction::Hide);
    let hide_others_item = Box::new(MenuItem::new(
        t!("menu.hide_others"),
        true,
        Some(Accelerator::new(Some(cmd | Modifiers::SHIFT), Code::KeyH)),
    ));
    map.insert(hide_others_item.id().clone(), MenuAction::HideOthers);
    let show_all_item = Box::new(MenuItem::new(t!("menu.show_all"), true, None));
    map.insert(show_all_item.id().clone(), MenuAction::ShowAll);
    let quit_item = Box::new(MenuItem::new(
        t!("menu.quit"),
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyQ)),
    ));
    map.insert(quit_item.id().clone(), MenuAction::Exit);
    let sep = PredefinedMenuItem::separator();
    let app_items: Vec<&dyn IsMenuItem> = vec![
        about_item.as_ref(),
        &sep,
        settings_item.as_ref(),
        &sep,
        hide_item.as_ref(),
        hide_others_item.as_ref(),
        show_all_item.as_ref(),
        &sep,
        quit_item.as_ref(),
    ];
    let app_menu = Submenu::with_items("Yinhe", true, &app_items)?;

    let file_menu = build_action_submenu(
        &mut map,
        &mut items,
        "menu.file",
        &crate::chrome::transport_bar::FILE_GROUPS[..4],
        &default_kb,
    )?;
    let recent_submenu = Submenu::new(t!("menu.recent_files"), false);
    file_menu.insert(&recent_submenu, 2)?;

    let edit_menu = build_action_submenu(
        &mut map,
        &mut items,
        "menu.edit",
        &crate::chrome::transport_bar::EDIT_GROUPS,
        &default_kb,
    )?;

    let play_item = Box::new(MenuItem::new(
        t!("shortcuts.play_toggle"),
        true,
        default_kb
            .get(yinhe_editor_core::shortcuts::ACTION_TOGGLE_PLAY)
            .first()
            .and_then(combo_to_accelerator),
    ));
    map.insert(play_item.id().clone(), MenuAction::TogglePlay);
    let stop_item = Box::new(MenuItem::new(
        t!("shortcuts.stop"),
        true,
        default_kb
            .get(yinhe_editor_core::shortcuts::ACTION_STOP)
            .first()
            .and_then(combo_to_accelerator),
    ));
    map.insert(stop_item.id().clone(), MenuAction::Stop);
    let record_item = Box::new(MenuItem::new(t!("menu.record"), true, None));
    map.insert(record_item.id().clone(), MenuAction::ToggleRecord);
    let step_item = Box::new(MenuItem::new(t!("menu.step_input"), true, None));
    map.insert(step_item.id().clone(), MenuAction::ToggleStepInput);

    const FOLLOW_MODES: [(FollowMode, &str); 4] = [
        (FollowMode::None, "follow.none"),
        (FollowMode::Centered, "follow.centered"),
        (FollowMode::Page, "follow.page"),
        (FollowMode::Continuous, "follow.continuous"),
    ];
    let mut follow_checks: Vec<(FollowMode, &'static str, CheckMenuItem)> = Vec::new();
    for (mode, key) in FOLLOW_MODES {
        let item = CheckMenuItem::new(t!(key), true, false, None);
        map.insert(item.id().clone(), MenuAction::SetFollowMode(mode));
        follow_checks.push((mode, key, item));
    }
    let sep_play = PredefinedMenuItem::separator();
    let sep_follow = PredefinedMenuItem::separator();
    let mut play_items: Vec<&dyn IsMenuItem> = vec![
        play_item.as_ref(),
        stop_item.as_ref(),
        &sep_play,
        record_item.as_ref(),
        step_item.as_ref(),
        &sep_follow,
    ];
    for (_, _, item) in &follow_checks {
        play_items.push(item);
    }
    let play_menu = Submenu::with_items(t!("menu.playback"), true, &play_items)?;

    let menu_items: Vec<&dyn IsMenuItem> = vec![&app_menu, &file_menu, &edit_menu, &play_menu];
    let menu = Menu::with_items(&menu_items)?;
    menu.init_for_nsapp();

    items.push(("shortcuts.play_toggle", play_item));
    items.push(("shortcuts.stop", stop_item));
    items.push(("menu.record", record_item));
    items.push(("menu.step_input", step_item));
    items.push(("menu.about", about_item));
    items.push(("menu.settings", settings_item));
    items.push(("menu.hide", hide_item));
    items.push(("menu.hide_others", hide_others_item));
    items.push(("menu.show_all", show_all_item));
    items.push(("menu.quit", quit_item));
    items.push(("menu.file", Box::new(file_menu)));
    items.push(("menu.edit", Box::new(edit_menu)));
    items.push(("menu.playback", Box::new(play_menu)));
    items.push(("menu.app", Box::new(app_menu)));

    let _ = MENU_MAP.set(Mutex::new(map));
    NATIVE_MENU.with(|cell| {
        let _ = cell.set(NativeMenu {
            _menu: menu,
            _items: items,
            recent_submenu,
            recent_items: std::cell::RefCell::new(Vec::new()),
            follow_checks,
        });
    });

    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Some(map_lock) = MENU_MAP.get()
                && let Ok(map) = map_lock.lock()
                && let Some(action) = map.get(event.id())
            {
                match action {
                    MenuAction::About => show_about_panel(),
                    MenuAction::Hide => hide_app(),
                    MenuAction::HideOthers => hide_others(),
                    MenuAction::ShowAll => show_all_apps(),
                    other => {
                        if let Ok(sender_guard) = MENU_SENDER.lock()
                            && let Some(tx) = sender_guard.as_ref()
                        {
                            let _ = tx.send(other.clone());
                        }
                    }
                }
            }
        }));
    }));
    Ok(())
}

pub(super) fn menu_sender() -> &'static Mutex<Option<mpsc::Sender<MenuAction>>> {
    &MENU_SENDER
}
pub(super) fn menu_map() -> &'static OnceLock<Mutex<HashMap<MenuId, MenuAction>>> {
    &MENU_MAP
}
