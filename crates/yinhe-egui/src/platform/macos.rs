//! macOS-specific platform integrations:
//! - `setDocumentEdited:` for the traffic-light dot
//! - Native `NSMenu` menu bar with File / Edit menus

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, mpsc};

use muda::{
    IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::MenuAction;

/// Helper to look up an Objective-C class by name at runtime.
fn cls(name: &std::ffi::CStr) -> Option<&'static AnyClass> {
    AnyClass::get(name)
}

/// Helper to create a CStr from a string literal at compile time.
macro_rules! cstr {
    ($s:literal) => {
        std::ffi::CStr::from_bytes_with_nul(concat!($s, "\0").as_bytes()).unwrap()
    };
}

// ── Finder 打开文件（NSAppleEventManager kAEOpenDocuments）──────────────

/// Finder/桌面"打开方式"传入的文件路径发送端。
static OPEN_FILES_SENDER: Mutex<Option<mpsc::Sender<String>>> = Mutex::new(None);

/// Apple Events 四字符代码。
/// keyDirectObject ('----')：事件直对象；kCoreEventClass ('core')：核心事件类；
/// kAEOpenDocuments ('odoc')：打开文档事件（Finder 双击文件时发送）。
const KEY_DIRECT_OBJECT: u32 = 0x2D2D_2D2D;
const KEY_ERROR_NUMBER: u32 = 0x6572_7221; // 'err!'
const CORE_EVENT_CLASS: u32 = 0x636F_7265;
const AE_OPEN_DOCUMENTS: u32 = 0x6F64_6F63;

/// 接收 Finder 双击/右键"打开方式"打开的文件（kAEOpenDocuments Apple 事件）。
/// 通过 NSAppleEventManager 注册独立 handler，不替换 winit 的 NSApplication delegate。
/// 返回接收端，由调用方在主线程轮询。
fn register_open_files_handler() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    *OPEN_FILES_SENDER.lock().unwrap() = Some(tx);

    let Some(superclass) = cls(cstr!("NSObject")) else {
        return rx;
    };
    let Some(handler_class) = ClassBuilder::new(cstr!("YinheOpenFilesHandler"), superclass) else {
        return rx;
    };
    let mut handler_class = handler_class;
    unsafe {
        handler_class.add_method(
            objc2::sel!(handleOpenEvent:withReplyEvent:),
            handle_open_event as extern "C-unwind" fn(_, _, _, _),
        );
    }
    let handler_class = handler_class.register();

    // 持有 handler 实例，避免被释放（AppKit 不会 retain AppleEventManager 的 handler）。
    // 只能在主线程创建，与 NSAppleEventManager 的派发线程一致。
    let handler: objc2::rc::Retained<AnyObject> = unsafe { objc2::msg_send![handler_class, new] };
    let handler_ptr = &*handler as *const AnyObject as *mut AnyObject;
    thread_local! {
        static HANDLER: OnceLock<*mut AnyObject> = const { OnceLock::new() };
    }
    HANDLER.with(|cell| {
        let _ = cell.set(handler_ptr);
    });

    let Some(mgr_class) = cls(cstr!("NSAppleEventManager")) else {
        return rx;
    };
    unsafe {
        let mgr: *mut AnyObject = objc2::msg_send![mgr_class, sharedManager];
        let _: () = objc2::msg_send![
            mgr,
            setEventHandler: handler_ptr,
            selector: objc2::sel!(handleOpenEvent:withReplyEvent:),
            forEventClass: CORE_EVENT_CLASS,
            andEventID: AE_OPEN_DOCUMENTS
        ];
    }

    rx
}

/// kAEOpenDocuments 事件回调：提取文件路径并发送到 UI 线程通道。
extern "C-unwind" fn handle_open_event(
    _this: &AnyObject,
    _sel: Sel,
    event: *mut AnyObject,
    reply: *mut AnyObject,
) {
    if event.is_null() {
        return;
    }
    // 回调中不 panic（ObjC 边界），用 catch_unwind 包一层。
    let paths = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut paths = Vec::new();
        unsafe {
            // 直对象是文件 URL 的 AEDesc 列表
            let desc: *mut AnyObject =
                objc2::msg_send![event, eventDescriptorForKeyword: KEY_DIRECT_OBJECT];
            if desc.is_null() {
                return paths;
            }
            let count: isize = objc2::msg_send![desc, numberOfItems];
            for i in 1..=count {
                let item: *mut AnyObject = objc2::msg_send![desc, descriptorAtIndex: i];
                if item.is_null() {
                    continue;
                }
                let url: *mut AnyObject = objc2::msg_send![item, fileURLValue];
                if url.is_null() {
                    continue;
                }
                let ns_path: *mut AnyObject = objc2::msg_send![url, path];
                if ns_path.is_null() {
                    continue;
                }
                let cstr_ptr: *const std::ffi::c_char = objc2::msg_send![ns_path, UTF8String];
                if cstr_ptr.is_null() {
                    continue;
                }
                paths.push(
                    std::ffi::CStr::from_ptr(cstr_ptr)
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
        paths
    }));
    if let Ok(paths) = paths
        && let Ok(sender_guard) = OPEN_FILES_SENDER.lock()
        && let Some(tx) = sender_guard.as_ref()
    {
        for path in paths {
            let _ = tx.send(path);
        }
    }
    // 给发送方（osascript 等脚本）一个明确回复，避免其挂起等待。
    // 未设置回复时发送方会一直等待直到超时。
    if !reply.is_null() {
        let Some(desc_class) = cls(cstr!("NSAppleEventDescriptor")) else {
            return;
        };
        unsafe {
            let err_desc: *mut AnyObject =
                objc2::msg_send![desc_class, descriptorWithInt32: -1708i32];
            if !err_desc.is_null() {
                let _: () = objc2::msg_send![
                    reply,
                    setDescriptor: err_desc,
                    forKeyword: KEY_ERROR_NUMBER
                ];
            }
        }
    }
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
    let Ok(handle) = frame.window_handle() else {
        return;
    };
    let raw = handle.as_raw();
    let RawWindowHandle::AppKit(appkit) = raw else {
        return;
    };
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

    let edit_menu = Submenu::with_items(
        "编辑",
        true,
        &[
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
        ],
    )?;

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
                && let Ok(sender_guard) = MENU_SENDER.lock()
                && let Some(tx) = sender_guard.as_ref()
            {
                let _ = tx.send(action.clone());
            }
        }));
    }));

    Ok(())
}

pub(crate) struct MenuBarInner {
    rx: mpsc::Receiver<MenuAction>,
    open_files_rx: mpsc::Receiver<String>,
}

impl MenuBarInner {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        *MENU_SENDER.lock().unwrap() = Some(tx);
        let open_files_rx = register_open_files_handler();
        if let Err(e) = init_native_menu() {
            tracing::error!("Failed to init macOS menu bar: {e:?}");
        }
        Self { rx, open_files_rx }
    }

    pub fn poll(&mut self) -> Vec<MenuAction> {
        std::iter::from_fn(|| self.rx.try_recv().ok()).collect()
    }

    /// 轮询 Finder/桌面"打开方式"传入的文件路径。
    pub fn poll_open_files(&mut self) -> Vec<String> {
        std::iter::from_fn(|| self.open_files_rx.try_recv().ok()).collect()
    }
}
