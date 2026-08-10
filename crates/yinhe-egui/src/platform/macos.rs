//! macOS-specific platform integrations:
//! - `setDocumentEdited:` for the traffic-light dot
//! - Native `NSMenu` menu bar with File / Edit menus

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, mpsc};

use muda::{
    IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use objc2::runtime::{AnyClass, AnyObject};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use rust_i18n::t;

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

// ── Finder 打开文件（预留通道）────────────────────────────────────────

/// Finder/桌面"打开方式"传入的文件路径发送端。
/// 目前 Finder 打开功能在实验分支（feat/finder-open-experiment）开发中，
/// 这里只保留通道，避免残留任何会在启动期执行的 ObjC 注册。
static OPEN_FILES_SENDER: Mutex<Option<mpsc::Sender<String>>> = Mutex::new(None);

/// 创建 Finder 打开文件的通道。返回接收端，由调用方在主线程轮询。
fn register_open_files_handler() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    *OPEN_FILES_SENDER.lock().unwrap() = Some(tx);
    rx
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

// ── App Nap（播放时阻止系统降频）──────────────────────────────────────

// 当前 App Nap 阻止令牌（beginActivityWithOptions: 的返回值），null 表示未阻止。
// 只在主线程访问（播放状态检查发生在 UI 帧内）。
thread_local! {
    static APP_NAP_TOKEN: std::cell::Cell<*mut AnyObject> =
        const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// 播放时阻止 App Nap（NSActivityUserInitiatedAllowingIdleSystemSleep），
/// 避免窗口遮挡/后台时系统降低定时器精度导致播放卡顿；停止播放时恢复。
pub(crate) fn set_app_nap_enabled(enabled: bool) {
    let Some(pi_class) = cls(cstr!("NSProcessInfo")) else {
        return;
    };
    let Some(string_class) = cls(cstr!("NSString")) else {
        return;
    };
    unsafe {
        let pi: *mut AnyObject = objc2::msg_send![pi_class, processInfo];
        APP_NAP_TOKEN.with(|cell| {
            let token = cell.get();
            if enabled && token.is_null() {
                let reason: *mut AnyObject = objc2::msg_send![
                    string_class,
                    stringWithUTF8String: c"Yinhe playback".as_ptr()
                ];
                let t: *mut AnyObject = objc2::msg_send![
                    pi,
                    beginActivityWithOptions: 0x00FFFFFFu64,
                    reason: reason
                ];
                if !t.is_null() {
                    // `beginActivityWithOptions:` 按 ObjC 内存规则返回 +0（autoreleased）
                    // 对象：裸 msg_send 存下的指针会在下一个 autorelease pool 排干时失效，
                    // 之后 `endActivity:` 就是 use-after-free（macOS 26 Swift Foundation
                    // 实测崩溃：SIGTRAP / objc_opt_isKindOfClass PAC 陷阱）。显式 retain
                    // 保证 token 在我们持有期间存活；endActivity 后不再 release——endActivity
                    // 是否接管所有权因系统版本而异，宁可泄漏这个小对象也不 double-free。
                    let _: *mut AnyObject = objc2::msg_send![t, retain];
                    cell.set(t);
                }
            } else if !enabled && !token.is_null() {
                let _: () = objc2::msg_send![pi, endActivity: token];
                cell.set(std::ptr::null_mut());
            }
        });
    }
}

// ── App 菜单系统级动作 ────────────────────────────────────────────────

/// NSApplication 单例（进程生命周期内恒定有效）。
fn ns_app() -> Option<&'static AnyObject> {
    let class = cls(cstr!("NSApplication"))?;
    Some(unsafe { objc2::msg_send![class, sharedApplication] })
}

/// 弹出系统「关于」面板（显示 bundle 中的应用名/版本/版权）。
fn show_about_panel() {
    let Some(app) = ns_app() else { return };
    let _: () = unsafe {
        objc2::msg_send![app, orderFrontStandardAboutPanel: std::ptr::null_mut::<AnyObject>()]
    };
}

/// 隐藏应用（⌘H）。
fn hide_app() {
    let Some(app) = ns_app() else { return };
    let _: () = unsafe { objc2::msg_send![app, hide: std::ptr::null_mut::<AnyObject>()] };
}

/// 隐藏其他应用（⌥⌘H）。
fn hide_others() {
    let Some(app) = ns_app() else { return };
    let _: () =
        unsafe { objc2::msg_send![app, hideOtherApplications: std::ptr::null_mut::<AnyObject>()] };
}

/// 显示全部应用。
fn show_all_apps() {
    let Some(app) = ns_app() else { return };
    let _: () = unsafe { objc2::msg_send![app, unhideAllApplications] };
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

/// 可在运行时更新文本的原生菜单项。
/// `muda::IsMenuItem` trait 上没有 `set_text`（那是具体类型的方法），
/// 这里扩展统一入口，语言切换时可以逐个刷新 NSMenuItem 标题。
trait MenuText: IsMenuItem {
    fn set_text(&self, text: &str);
}

impl MenuText for MenuItem {
    fn set_text(&self, text: &str) {
        // 全限定调用固有方法，避免与 trait 方法同名递归
        MenuItem::set_text(self, text);
    }
}

impl MenuText for Submenu {
    fn set_text(&self, text: &str) {
        Submenu::set_text(self, text);
    }
}

/// 持有菜单栏所有 Rust 对象，保持底层 NSMenu/NSMenuItem 存活。
struct NativeMenu {
    _menu: Menu,
    /// 按创建顺序保存 (翻译 key, 菜单项)：key 用于语言切换时重新取文本，
    /// 对象本身同时承担保活职责（drop 会导致底层 NSMenuItem 被释放）。
    _items: Vec<(&'static str, Box<dyn MenuText>)>,
}

/// 初始化原生 macOS 菜单栏，使用 `muda` crate。
/// 在 `MenuBarInner::new()` 中调用，此时 NSApplication 已就绪。
fn init_native_menu() -> muda::Result<()> {
    let mut map = HashMap::new();
    let mut items: Vec<(&'static str, Box<dyn MenuText>)> = Vec::new();
    let cmd = Modifiers::SUPER;

    // ── App 菜单（第一个菜单，标题由系统显示为应用名）──
    // macOS 惯例：About / 设置… / 隐藏类 / 退出 都放在这里。
    // About/Hide 等系统级动作由事件处理器就地执行，不经过主线程通道。
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

    // ── 文件菜单 ──
    let new_item = Box::new(MenuItem::new(
        t!("menu.new"),
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyN)),
    ));
    map.insert(new_item.id().clone(), MenuAction::NewProject);

    let open_item = Box::new(MenuItem::new(
        t!("menu.open"),
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyO)),
    ));
    map.insert(open_item.id().clone(), MenuAction::Open);

    let save_item = Box::new(MenuItem::new(
        t!("menu.save"),
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyS)),
    ));
    map.insert(save_item.id().clone(), MenuAction::Save);

    let save_as_item = Box::new(MenuItem::new(
        t!("menu.save_as"),
        true,
        Some(Accelerator::new(Some(cmd | Modifiers::SHIFT), Code::KeyS)),
    ));
    map.insert(save_as_item.id().clone(), MenuAction::SaveAs);

    let close_item = Box::new(MenuItem::new(
        t!("menu.close"),
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyW)),
    ));
    map.insert(close_item.id().clone(), MenuAction::CloseDocument);

    // `&dyn MenuText` 通过 trait upcasting 协变到 `&dyn IsMenuItem`（rustc 1.86+）
    let sep = PredefinedMenuItem::separator();
    let file_items: Vec<&dyn IsMenuItem> = vec![
        new_item.as_ref(),
        open_item.as_ref(),
        &sep,
        save_item.as_ref(),
        save_as_item.as_ref(),
        &sep,
        close_item.as_ref(),
    ];
    let file_menu = Submenu::with_items(t!("menu.file"), true, &file_items)?;

    // ── 编辑菜单 ──
    let undo_item = Box::new(MenuItem::new(
        t!("menu.undo"),
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyZ)),
    ));
    map.insert(undo_item.id().clone(), MenuAction::Undo);

    let redo_item = Box::new(MenuItem::new(
        t!("menu.redo"),
        true,
        Some(Accelerator::new(Some(cmd | Modifiers::SHIFT), Code::KeyZ)),
    ));
    map.insert(redo_item.id().clone(), MenuAction::Redo);

    let cut_item = Box::new(MenuItem::new(
        t!("menu.cut"),
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyX)),
    ));
    map.insert(cut_item.id().clone(), MenuAction::Cut);

    let copy_item = Box::new(MenuItem::new(
        t!("menu.copy"),
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyC)),
    ));
    map.insert(copy_item.id().clone(), MenuAction::Copy);

    let paste_item = Box::new(MenuItem::new(
        t!("menu.paste"),
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyV)),
    ));
    map.insert(paste_item.id().clone(), MenuAction::Paste);

    let select_all_item = Box::new(MenuItem::new(
        t!("menu.select_all"),
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyA)),
    ));
    map.insert(select_all_item.id().clone(), MenuAction::SelectAll);

    let duplicate_item = Box::new(MenuItem::new(
        t!("menu.duplicate"),
        true,
        Some(Accelerator::new(Some(cmd), Code::KeyD)),
    ));
    map.insert(duplicate_item.id().clone(), MenuAction::Duplicate);

    let delete_item = Box::new(MenuItem::new(
        t!("menu.delete"),
        true,
        Some(Accelerator::new(None, Code::Delete)),
    ));
    map.insert(delete_item.id().clone(), MenuAction::Delete);

    let transpose_up_item = Box::new(MenuItem::new(
        t!("menu.octave_up"),
        true,
        Some(Accelerator::new(Some(Modifiers::SHIFT), Code::ArrowUp)),
    ));
    map.insert(transpose_up_item.id().clone(), MenuAction::TransposeUp);

    let transpose_down_item = Box::new(MenuItem::new(
        t!("menu.octave_down"),
        true,
        Some(Accelerator::new(Some(Modifiers::SHIFT), Code::ArrowDown)),
    ));
    map.insert(transpose_down_item.id().clone(), MenuAction::TransposeDown);

    let sep = PredefinedMenuItem::separator();
    let edit_items: Vec<&dyn IsMenuItem> = vec![
        undo_item.as_ref(),
        redo_item.as_ref(),
        &sep,
        cut_item.as_ref(),
        copy_item.as_ref(),
        paste_item.as_ref(),
        &sep,
        select_all_item.as_ref(),
        duplicate_item.as_ref(),
        delete_item.as_ref(),
        &sep,
        transpose_up_item.as_ref(),
        transpose_down_item.as_ref(),
    ];
    let edit_menu = Submenu::with_items(t!("menu.edit"), true, &edit_items)?;

    let menu_items: Vec<&dyn IsMenuItem> = vec![&app_menu, &file_menu, &edit_menu];
    let menu = Menu::with_items(&menu_items)?;
    menu.init_for_nsapp();

    // 收集所有 items 保持存活（翻译 key 用于语言切换时刷新文本）
    items.push(("menu.new", new_item));
    items.push(("menu.open", open_item));
    items.push(("menu.save", save_item));
    items.push(("menu.save_as", save_as_item));
    items.push(("menu.close", close_item));
    items.push(("menu.undo", undo_item));
    items.push(("menu.redo", redo_item));
    items.push(("menu.cut", cut_item));
    items.push(("menu.copy", copy_item));
    items.push(("menu.paste", paste_item));
    items.push(("menu.select_all", select_all_item));
    items.push(("menu.duplicate", duplicate_item));
    items.push(("menu.delete", delete_item));
    items.push(("menu.octave_up", transpose_up_item));
    items.push(("menu.octave_down", transpose_down_item));
    items.push(("menu.about", about_item));
    items.push(("menu.settings", settings_item));
    items.push(("menu.hide", hide_item));
    items.push(("menu.hide_others", hide_others_item));
    items.push(("menu.show_all", show_all_item));
    items.push(("menu.quit", quit_item));
    items.push(("menu.file", Box::new(file_menu)));
    items.push(("menu.edit", Box::new(edit_menu)));
    items.push(("menu.app", Box::new(app_menu)));

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
                // App 菜单的系统级动作就地执行（主线程回调，无需经过通道）
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

/// 用当前 locale 刷新全部原生菜单文本。
/// 由 `MenuBarInner::poll` 在检测到语言切换后调用。
fn refresh_native_menu_texts() {
    NATIVE_MENU.with(|cell| {
        if let Some(native) = cell.get() {
            for (key, item) in &native._items {
                let key = *key;
                item.set_text(&t!(key));
            }
        }
    });
}

pub(crate) struct MenuBarInner {
    rx: mpsc::Receiver<MenuAction>,
    open_files_rx: mpsc::Receiver<String>,
    /// 上次应用到原生菜单的 locale，变化时刷新菜单文本。
    last_locale: String,
}

impl MenuBarInner {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        *MENU_SENDER.lock().unwrap() = Some(tx);
        let open_files_rx = register_open_files_handler();
        if let Err(e) = init_native_menu() {
            tracing::error!("Failed to init macOS menu bar: {e:?}");
        }
        Self {
            rx,
            open_files_rx,
            last_locale: rust_i18n::locale().to_string(),
        }
    }

    pub fn poll(&mut self) -> Vec<MenuAction> {
        // 应用内语言切换（设置对话框）时原生菜单文本不会自动更新，
        // 检测 locale 变化后逐个刷新标题（setText 走主线程，poll 在 UI 帧内调用）。
        let locale = rust_i18n::locale();
        if *locale != self.last_locale {
            self.last_locale = locale.to_string();
            refresh_native_menu_texts();
        }
        std::iter::from_fn(|| self.rx.try_recv().ok()).collect()
    }

    /// 轮询 Finder/桌面"打开方式"传入的文件路径。
    pub fn poll_open_files(&mut self) -> Vec<String> {
        std::iter::from_fn(|| self.open_files_rx.try_recv().ok()).collect()
    }
}
