//! macOS-specific platform integrations:
//! - `setDocumentEdited:` for the traffic-light dot
//! - Native `NSMenu` menu bar with File / Edit menus

use std::ffi::c_void;
use std::sync::{mpsc, Mutex};

use objc2::runtime::{AnyClass, AnyObject, Sel};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::MenuAction;

/// Helper to look up an Objective-C class by name at runtime.
fn cls(name: &std::ffi::CStr) -> &'static AnyClass {
    AnyClass::get(name).expect("ObjC class not found")
}

/// Helper to create a CStr from a string literal at compile time.
macro_rules! cstr {
    ($s:literal) => {
        std::ffi::CStr::from_bytes_with_nul(concat!($s, "\0").as_bytes())
            .unwrap()
    };
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

/// Global channel for menu actions. The Objective-C callbacks write here.
static MENU_SENDER: Mutex<Option<mpsc::Sender<MenuAction>>> = Mutex::new(None);

pub(crate) struct MenuBarInner {
    rx: mpsc::Receiver<MenuAction>,
}

impl MenuBarInner {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        *MENU_SENDER.lock().unwrap() = Some(tx);
        unsafe { setup_menu_bar() };
        Self { rx }
    }

    pub fn poll(&self) -> Vec<MenuAction> {
        std::iter::from_fn(|| self.rx.try_recv().ok()).collect()
    }
}

/// Menu item tag values — used in the Objective-C callback to identify which
/// item was clicked.
const TAG_NEW: i32 = 1;
const TAG_OPEN: i32 = 2;
const TAG_SAVE: i32 = 3;
const TAG_SAVE_AS: i32 = 4;
const TAG_CLOSE: i32 = 5;
const TAG_UNDO: i32 = 6;
const TAG_REDO: i32 = 7;

/// Build and install the native macOS menu bar.
///
/// # Safety
/// Must be called on the main thread.
unsafe fn setup_menu_bar() {
    let ns_app: &AnyObject =
        unsafe { objc2::msg_send![cls(cstr!("NSApplication")), sharedApplication] };

    // Create main menu bar
    let main_menu: &AnyObject = unsafe { objc2::msg_send![cls(cstr!("NSMenu")), new] };

    // ── File menu ──
    let file_menu = unsafe { create_submenu("文件") };
    unsafe { add_item(file_menu, "新建", TAG_NEW, "n") };
    unsafe { add_item(file_menu, "打开…", TAG_OPEN, "o") };
    unsafe { add_separator(file_menu) };
    unsafe { add_item(file_menu, "保存", TAG_SAVE, "s") };
    unsafe { add_item(file_menu, "另存为…", TAG_SAVE_AS, "S") };
    unsafe { add_separator(file_menu) };
    unsafe { add_item(file_menu, "关闭", TAG_CLOSE, "w") };

    // ── Edit menu ──
    let edit_menu = unsafe { create_submenu("编辑") };
    unsafe { add_item(edit_menu, "撤销", TAG_UNDO, "z") };
    unsafe { add_item(edit_menu, "重做", TAG_REDO, "Z") };

    // Add submenus to main menu
    unsafe {
        let _: () = objc2::msg_send![main_menu, addItem: file_menu];
        let _: () = objc2::msg_send![main_menu, addItem: edit_menu];
    }

    // Install as the application main menu
    unsafe {
        let _: () = objc2::msg_send![ns_app, setMainMenu: main_menu];
    }

    // Create the target object that will receive menu item actions
    let target_class = create_target_class();
    let target: &AnyObject = unsafe { objc2::msg_send![target_class, new] };
    // Store target in a global so it's never freed.
    // SAFETY: the pointer is only stored as a usize, never dereferenced from Rust.
    // The Objective-C runtime manages the object's lifetime.
    static TARGET: Mutex<Option<usize>> = Mutex::new(None);
    *TARGET.lock().unwrap() = Some(target as *const AnyObject as usize);

    // Wire up all menu items to the target
    unsafe { wire_menu_items(main_menu, target) };
}

/// Create an `NSMenu` with a title (used as a submenu).
unsafe fn create_submenu(title: &str) -> *mut AnyObject {
    let ns_string: &AnyObject = unsafe {
        objc2::msg_send![
            cls(cstr!("NSString")),
            stringWithUTF8String: title.as_ptr().cast::<c_void>()
        ]
    };
    let menu: &AnyObject = unsafe { objc2::msg_send![cls(cstr!("NSMenu")), new] };
    unsafe {
        let _: () = objc2::msg_send![menu, setTitle: ns_string];
    }
    // Create a "root" item to hold this submenu
    let item: &AnyObject = unsafe { objc2::msg_send![cls(cstr!("NSMenuItem")), new] };
    let _: () = unsafe { objc2::msg_send![item, setTitle: ns_string] };
    let _: () = unsafe { objc2::msg_send![item, setSubmenu: menu] };
    item as *const AnyObject as *mut AnyObject
}

/// Add an `NSMenuItem` with a title, tag, and key equivalent to a menu.
unsafe fn add_item(menu: *mut AnyObject, title: &str, tag: i32, key_eq: &str) {
    let ns_title: &AnyObject = unsafe {
        objc2::msg_send![
            cls(cstr!("NSString")),
            stringWithUTF8String: title.as_ptr().cast::<c_void>()
        ]
    };
    let ns_key: &AnyObject = unsafe {
        objc2::msg_send![
            cls(cstr!("NSString")),
            stringWithUTF8String: key_eq.as_ptr().cast::<c_void>()
        ]
    };
    let item: &AnyObject = unsafe { objc2::msg_send![cls(cstr!("NSMenuItem")), alloc] };
    let action_sel = Sel::register(cstr!("menuItemAction:"));
    let item: &AnyObject = unsafe {
        objc2::msg_send![item, initWithTitle: ns_title, action: action_sel, keyEquivalent: ns_key]
    };
    let _: () = unsafe { objc2::msg_send![item, setTag: tag] };
    let _: () = unsafe { objc2::msg_send![menu, addItem: item] };
}

/// Add a separator item to a menu.
unsafe fn add_separator(menu: *mut AnyObject) {
    let item: &AnyObject = unsafe { objc2::msg_send![cls(cstr!("NSMenuItem")), separatorItem] };
    let _: () = unsafe { objc2::msg_send![menu, addItem: item] };
}

/// Recursively wire all menu items to the target object.
unsafe fn wire_menu_items(menu: &AnyObject, target: &AnyObject) {
    let count: usize = unsafe { objc2::msg_send![menu, numberOfItems] };
    for i in 0..count {
        let item: &AnyObject = unsafe { objc2::msg_send![menu, itemAtIndex: i] };
        let has_submenu: bool = unsafe { objc2::msg_send![item, hasSubmenu] };
        if has_submenu {
            let sub: &AnyObject = unsafe { objc2::msg_send![item, submenu] };
            unsafe { wire_menu_items(sub, target) };
        } else {
            let is_sep: bool = unsafe { objc2::msg_send![item, isSeparatorItem] };
            if !is_sep {
                let _: () = unsafe { objc2::msg_send![item, setTarget: target] };
            }
        }
    }
}

/// Dynamically create an Objective-C class `YinheMenuTarget` that handles
/// menu item clicks and forwards them through the global `MENU_SENDER`.
fn create_target_class() -> &'static AnyClass {
    let class_name = cstr!("YinheMenuTarget");

    // Check if already registered (e.g. hot-reload scenarios)
    if let Some(cls) = AnyClass::get(class_name) {
        return cls;
    }

    let superclass = cls(cstr!("NSObject"));
    let mut builder = objc2::declare::ClassBuilder::new(class_name, superclass)
        .expect("YinheMenuTarget creation failed");

    extern "C" fn handle_menu_item(_this: *mut AnyObject, _cmd: Sel, sender: *mut AnyObject) {
        let tag: i32 = unsafe { objc2::msg_send![sender, tag] };
        let action = match tag {
            TAG_NEW => MenuAction::NewProject,
            TAG_OPEN => MenuAction::Open,
            TAG_SAVE => MenuAction::Save,
            TAG_SAVE_AS => MenuAction::SaveAs,
            TAG_CLOSE => MenuAction::CloseDocument,
            TAG_UNDO => MenuAction::Undo,
            TAG_REDO => MenuAction::Redo,
            _ => return,
        };
        if let Ok(guard) = MENU_SENDER.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(action);
            }
        }
    }

    let action_sel = cstr!("menuItemAction:");
    unsafe {
        builder.add_method(
            Sel::register(action_sel),
            handle_menu_item as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
    }

    builder.register()
}
