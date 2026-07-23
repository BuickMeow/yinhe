//! macOS-specific platform integrations:
//! - `setDocumentEdited:` for the traffic-light dot
//! - Native `NSMenu` menu bar with File / Edit menus

use std::collections::HashMap;
use std::sync::{mpsc, Mutex, OnceLock};

use muda::{
    accelerator::{Accelerator, Code, Modifiers},
    IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
};
use objc2::runtime::{AnyClass, AnyObject};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::MenuAction;

/// Helper to look up an Objective-C class by name at runtime.
fn cls(name: &std::ffi::CStr) -> Option<&'static AnyClass> {
    AnyClass::get(name)
}

/// Helper to create a CStr from a string literal at compile time.
macro_rules! cstr {
    ($s:literal) => {
        std::ffi::CStr::from_bytes_with_nul(concat!($s, "\0").as_bytes())
            .unwrap()
    };
}

// ── Dock icon bounce ───────────────────────────────────────────────────────

/// 让 Dock 栏图标跳动，提示用户注意（例如关闭未保存文档时）。
pub(crate) fn request_user_attention() {
    let ns_app_class = match cls(cstr!("NSApplication")) {
        Some(c) => c,
        None => return,
    };
    let ns_app: &AnyObject = unsafe { objc2::msg_send![ns_app_class, sharedApplication] };
    // NSInformationalRequest = 10，让 Dock 图标跳动一次
    let _: () = unsafe { objc2::msg_send![ns_app, requestUserAttention: 10i64] };
}

// ── setDocumentEdited ──────────────────────────────────────────────────────

/// Set the document-edited indicator (dot in the red traffic-light button).
pub(crate) fn set_document_edited(frame: &eframe::Frame, edited: bool) {
    let Ok(handle) = frame.window_handle() else { return };
    let raw = handle.as_raw();
    let RawWindowHandle::AppKit(appkit) = raw else { return };
    let ns_view: &AnyObject = unsafe { &*appkit.ns_view.as_ptr().cast() };
    let ns_window: Option<&AnyObject> = unsafe { objc2::msg_send![ns_view, window] };
    let Some(ns_window) = ns_window else { return };
    unsafe {
        let _: () = objc2::msg_send![ns_window, setDocumentEdited: edited];
    }
}

// ── Menu Bar ───────────────────────────────────────────────────────────────

/// Global channel for menu actions. The muda event handler writes here.
static MENU_SENDER: Mutex<Option<mpsc::Sender<MenuAction>>> = Mutex::new(None);

/// Maps `muda::MenuId` to `MenuAction` for dispatching menu events.
static MENU_MAP: OnceLock<HashMap<MenuId, MenuAction>> = OnceLock::new();

// 持有 `Menu` 及所有子菜单/菜单项，防止它们被 drop 后底层 NSMenuItem 被释放。
// 菜单栏生命周期与应用相同，永不释放。
thread_local! {
    static NATIVE_MENU: OnceLock<NativeMenu> = const { OnceLock::new() };
}

/// 持有菜单栏所有 Rust 对象，保持底层 NSMenu/NSMenuItem 存活。
struct NativeMenu {
    _menu: Menu,
    _items: Vec<Box<dyn IsMenuItem>>,
}

/// 初始化原生 macOS 菜单栏，使用 `muda` crate。
/// 在 `MenuBarInner::new()` 中调用，此时 NSApplication 已就绪。
fn init_native_menu() -> muda::Result<()> {
    let mut map = HashMap::new();
    let mut items: Vec<Box<dyn IsMenuItem>> = Vec::new();
    let cmd = Modifiers::SUPER;

    // ── 文件菜单 ──
    let new_item = Box::new(MenuItem::new(
        "新建",
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyN)),
    ));
    map.insert(new_item.id().clone(), MenuAction::NewProject);

    let open_item = Box::new(MenuItem::new(
        "打开…",
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyO)),
    ));
    map.insert(open_item.id().clone(), MenuAction::Open);

    let save_item = Box::new(MenuItem::new(
        "保存",
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyS)),
    ));
    map.insert(save_item.id().clone(), MenuAction::Save);

    let save_as_item = Box::new(MenuItem::new(
        "另存为…",
        true,
        Some(Accelerator::new(Some(cmd | Modifiers::SHIFT), Code::KeyS)),
    ));
    map.insert(save_as_item.id().clone(), MenuAction::SaveAs);

    let close_item = Box::new(MenuItem::new(
        "关闭",
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyW)),
    ));
    map.insert(close_item.id().clone(), MenuAction::CloseDocument);

    let file_menu = Submenu::with_items(
        "文件",
        true,
        &[
            new_item.as_ref(),
            open_item.as_ref(),
            &PredefinedMenuItem::separator(),
            save_item.as_ref(),
            save_as_item.as_ref(),
            &PredefinedMenuItem::separator(),
            close_item.as_ref(),
        ],
    )?;

    // ── 编辑菜单 ──
    let undo_item = Box::new(MenuItem::new(
        "撤销",
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyZ)),
    ));
    map.insert(undo_item.id().clone(), MenuAction::Undo);

    let redo_item = Box::new(MenuItem::new(
        "重做",
        true,
        Some(Accelerator::new(Some(cmd | Modifiers::SHIFT), Code::KeyZ)),
    ));
    map.insert(redo_item.id().clone(), MenuAction::Redo);

    let cut_item = Box::new(MenuItem::new(
        "剪切",
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyX)),
    ));
    map.insert(cut_item.id().clone(), MenuAction::Cut);

    let copy_item = Box::new(MenuItem::new(
        "拷贝",
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyC)),
    ));
    map.insert(copy_item.id().clone(), MenuAction::Copy);

    let paste_item = Box::new(MenuItem::new(
        "粘贴",
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyV)),
    ));
    map.insert(paste_item.id().clone(), MenuAction::Paste);

    let select_all_item = Box::new(MenuItem::new(
        "全选",
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyA)),
    ));
    map.insert(select_all_item.id().clone(), MenuAction::SelectAll);

    let duplicate_item = Box::new(MenuItem::new(
        "重复",
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyD)),
    ));
    map.insert(duplicate_item.id().clone(), MenuAction::Duplicate);

    let delete_item = Box::new(MenuItem::new(
        "删除",
        true,
        Some(Accelerator::new(None, Code::Delete)),
    ));
    map.insert(delete_item.id().clone(), MenuAction::Delete);

    let transpose_up_item = Box::new(MenuItem::new(
        "升八度",
        true,
        Some(Accelerator::new(Some(Modifiers::SHIFT), Code::ArrowUp)),
    ));
    map.insert(transpose_up_item.id().clone(), MenuAction::TransposeUp);

    let transpose_down_item = Box::new(MenuItem::new(
        "降八度",
        true,
        Some(Accelerator::new(Some(Modifiers::SHIFT), Code::ArrowDown)),
    ));
    map.insert(transpose_down_item.id().clone(), MenuAction::TransposeDown);

    let edit_menu = Submenu::with_items("编辑", true, &[
        undo_item.as_ref(),
        redo_item.as_ref(),
        &PredefinedMenuItem::separator(),
        cut_item.as_ref(),
        copy_item.as_ref(),
        paste_item.as_ref(),
        &PredefinedMenuItem::separator(),
        select_all_item.as_ref(),
        duplicate_item.as_ref(),
        delete_item.as_ref(),
        &PredefinedMenuItem::separator(),
        transpose_up_item.as_ref(),
        transpose_down_item.as_ref(),
    ])?;

    let menu = Menu::with_items(&[&file_menu, &edit_menu])?;
    menu.init_for_nsapp();

    // 收集所有 items 保持存活
    items.push(new_item);
    items.push(open_item);
    items.push(save_item);
    items.push(save_as_item);
    items.push(close_item);
    items.push(undo_item);
    items.push(redo_item);
    items.push(cut_item);
    items.push(copy_item);
    items.push(paste_item);
    items.push(select_all_item);
    items.push(duplicate_item);
    items.push(delete_item);
    items.push(transpose_up_item);
    items.push(transpose_down_item);
    items.push(Box::new(file_menu));
    items.push(Box::new(edit_menu));

    let _ = MENU_MAP.set(map);

    NATIVE_MENU.with(|cell| {
        let _ = cell.set(NativeMenu {
            _menu: menu,
            _items: items,
        });
    });

    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Some(map) = MENU_MAP.get()
                && let Some(action) = map.get(event.id())
            {
                if let Ok(sender_guard) = MENU_SENDER.lock()
                    && let Some(tx) = sender_guard.as_ref()
                {
                    let _ = tx.send(action.clone());
                }
            }
        }));
    }));

    Ok(())
}

pub(crate) struct MenuBarInner {
    rx: mpsc::Receiver<MenuAction>,
}

impl MenuBarInner {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        *MENU_SENDER.lock().unwrap() = Some(tx);
        if let Err(e) = init_native_menu() {
            tracing::error!("Failed to init macOS menu bar: {e:?}");
        }
        Self { rx }
    }

    pub fn poll(&mut self) -> Vec<MenuAction> {
        std::iter::from_fn(|| self.rx.try_recv().ok()).collect()
    }
}
