//! macOS-specific platform integrations:
//! - `setDocumentEdited:` for the traffic-light dot
//! - Native `NSMenu` menu bar with File / Edit menus

use std::sync::{mpsc, Mutex};

use objc2::runtime::{AnyClass, AnyObject, Sel};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::MenuAction;

/// Helper to look up an Objective-C class by name at runtime.
/// Returns None if the class is not found (instead of panicking).
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

// ── setDocumentEdited ──────────────────────────────────────────────────────

/// Set the document-edited indicator (dot in the red traffic-light button).
pub(crate) fn set_document_edited(frame: &eframe::Frame, edited: bool) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let Ok(handle) = frame.window_handle() else { return };
        let raw = handle.as_raw();
        let RawWindowHandle::AppKit(appkit) = raw else { return };
        let ns_view: &AnyObject = unsafe { &*appkit.ns_view.as_ptr().cast() };
        let ns_window: Option<&AnyObject> = unsafe { objc2::msg_send![ns_view, window] };
        let Some(ns_window) = ns_window else { return };
        unsafe {
            let _: () = objc2::msg_send![ns_window, setDocumentEdited: edited];
        }
    }));
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
        // Catch any panic during menu bar setup (e.g. ObjC classes not found)
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            unsafe { setup_menu_bar() };
        }));
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
    // Verify all required ObjC classes exist
    let ns_app_class = match cls(cstr!("NSApplication")) {
        Some(c) => c,
        None => return,
    };
    let ns_menu_class = match cls(cstr!("NSMenu")) {
        Some(c) => c,
        None => return,
    };
    let ns_string_class = match cls(cstr!("NSString")) {
        Some(c) => c,
        None => return,
    };
    let ns_menu_item_class = match cls(cstr!("NSMenuItem")) {
        Some(c) => c,
        None => return,
    };
    let ns_object_class = match cls(cstr!("NSObject")) {
        Some(c) => c,
        None => return,
    };

    let ns_app: &AnyObject =
        unsafe { objc2::msg_send![ns_app_class, sharedApplication] };

    // Create main menu bar
    let main_menu: &AnyObject = unsafe { objc2::msg_send![ns_menu_class, new] };

    // ── File menu ──
    let file_menu = unsafe { create_submenu(ns_string_class, ns_menu_class, "文件") };
    unsafe { add_submenu_item(ns_string_class, ns_menu_item_class, main_menu, file_menu, "文件") };
    let file_menu_ref: &AnyObject = unsafe { &*file_menu };
    unsafe { add_item(ns_string_class, ns_menu_item_class, file_menu_ref, "新建", TAG_NEW, "n") };
    unsafe { add_item(ns_string_class, ns_menu_item_class, file_menu_ref, "打开…", TAG_OPEN, "o") };
    unsafe { add_separator(ns_menu_item_class, file_menu_ref) };
    unsafe { add_item(ns_string_class, ns_menu_item_class, file_menu_ref, "保存", TAG_SAVE, "s") };
    unsafe { add_item(ns_string_class, ns_menu_item_class, file_menu_ref, "另存为…", TAG_SAVE_AS, "S") };
    unsafe { add_separator(ns_menu_item_class, file_menu_ref) };
    unsafe { add_item(ns_string_class, ns_menu_item_class, file_menu_ref, "关闭", TAG_CLOSE, "w") };

    // ── Edit menu ──
    let edit_menu = unsafe { create_submenu(ns_string_class, ns_menu_class, "编辑") };
    unsafe { add_submenu_item(ns_string_class, ns_menu_item_class, main_menu, edit_menu, "编辑") };
    let edit_menu_ref: &AnyObject = unsafe { &*edit_menu };
    unsafe { add_item(ns_string_class, ns_menu_item_class, edit_menu_ref, "撤销", TAG_UNDO, "z") };
    unsafe { add_item(ns_string_class, ns_menu_item_class, edit_menu_ref, "重做", TAG_REDO, "Z") };

    // Install as the application main menu
    unsafe {
        let _: () = objc2::msg_send![ns_app, setMainMenu: main_menu];
    }

    // Create the target object that will receive menu item actions
    let target_class = create_target_class(ns_object_class);
    let target: &AnyObject = unsafe { objc2::msg_send![target_class, new] };
    // Store target in a global so it's never freed.
    // SAFETY: the pointer is only stored as a usize, never dereferenced from Rust.
    // The Objective-C runtime manages the object's lifetime.
    static TARGET: Mutex<Option<usize>> = Mutex::new(None);
    *TARGET.lock().unwrap() = Some(target as *const AnyObject as usize);

    // Wire up all menu items to the target
    unsafe { wire_menu_items(main_menu, target) };
}

/// Create an `NSMenu` with a title. Returns the NSMenu pointer.
unsafe fn create_submenu(
    ns_string_class: &AnyClass,
    ns_menu_class: &AnyClass,
    title: &str,
) -> *mut AnyObject {
    let ns_string: &AnyObject = unsafe {
        objc2::msg_send![
            ns_string_class,
            stringWithUTF8String: title.as_ptr().cast::<std::ffi::c_char>()
        ]
    };
    let menu: &AnyObject = unsafe { objc2::msg_send![ns_menu_class, new] };
    unsafe {
        let _: () = objc2::msg_send![menu, setTitle: ns_string];
    }
    menu as *const AnyObject as *mut AnyObject
}

/// Create an `NSMenuItem` that wraps a submenu and add it to the parent menu.
unsafe fn add_submenu_item(
    ns_string_class: &AnyClass,
    ns_menu_item_class: &AnyClass,
    parent_menu: &AnyObject,
    submenu: *mut AnyObject,
    title: &str,
) {
    let ns_title: &AnyObject = unsafe {
        objc2::msg_send![
            ns_string_class,
            stringWithUTF8String: title.as_ptr().cast::<std::ffi::c_char>()
        ]
    };
    let item: &AnyObject = unsafe { objc2::msg_send![ns_menu_item_class, new] };
    let _: () = unsafe { objc2::msg_send![item, setTitle: ns_title] };
    let _: () = unsafe { objc2::msg_send![item, setSubmenu: &*submenu] };
    let _: () = unsafe { objc2::msg_send![parent_menu, addItem: item] };
}

/// Add an `NSMenuItem` with a title, tag, and key equivalent to a menu.
unsafe fn add_item(
    ns_string_class: &AnyClass,
    ns_menu_item_class: &AnyClass,
    menu: &AnyObject,
    title: &str,
    tag: i32,
    key_eq: &str,
) {
    let ns_title: &AnyObject = unsafe {
        objc2::msg_send![
            ns_string_class,
            stringWithUTF8String: title.as_ptr().cast::<std::ffi::c_char>()
        ]
    };
    let ns_key: &AnyObject = unsafe {
        objc2::msg_send![
            ns_string_class,
            stringWithUTF8String: key_eq.as_ptr().cast::<std::ffi::c_char>()
        ]
    };
    let item: &AnyObject = unsafe { objc2::msg_send![ns_menu_item_class, alloc] };
    let action_sel = Sel::register(cstr!("menuItemAction:"));
    let item: &AnyObject = unsafe {
        objc2::msg_send![item, initWithTitle: ns_title, action: action_sel, keyEquivalent: ns_key]
    };
    let _: () = unsafe { objc2::msg_send![item, setTag: tag as i64] };
    let _: () = unsafe { objc2::msg_send![menu, addItem: item] };
}

/// Add a separator item to a menu.
unsafe fn add_separator(ns_menu_item_class: &AnyClass, menu: &AnyObject) {
    let item: &AnyObject = unsafe { objc2::msg_send![ns_menu_item_class, separatorItem] };
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
fn create_target_class(superclass: &AnyClass) -> &'static AnyClass {
    let class_name = cstr!("YinheMenuTarget");

    // Check if already registered (e.g. hot-reload scenarios)
    if let Some(cls) = AnyClass::get(class_name) {
        return cls;
    }

    let mut builder = objc2::declare::ClassBuilder::new(class_name, superclass)
        .expect("YinheMenuTarget creation failed");

    extern "C" fn handle_menu_item(_this: *mut AnyObject, _cmd: Sel, sender: *mut AnyObject) {
        // Must not panic across FFI boundary — catch any panic and swallow it.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if sender.is_null() {
                return;
            }
            let tag: i64 = unsafe { objc2::msg_send![sender, tag] };
            let action = match tag as i32 {
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
        }));
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
