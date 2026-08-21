//! macOS-specific platform integrations:
//! - `setDocumentEdited:` for the traffic-light dot
//! - Native `NSMenu` menu bar with File / Edit menus

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, mpsc};

use muda::{
    CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use objc2::runtime::{AnyClass, AnyObject, Sel};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use rust_i18n::t;

use super::MenuAction;
use yinhe_editor_core::follow::FollowMode;
use yinhe_editor_core::shortcuts::{self, Keybindings};

/// 原生菜单项（按 i18n key 标识）与快捷键配置动作的对应关系，
/// 菜单项翻译 key → 快捷键动作 id。
/// 文件/编辑菜单由 `FileAction`/`EditAction` 驱动（单一来源），
/// 播放菜单与 App 菜单的系统项单独列出。
fn menu_action_id(label_key: &str) -> Option<&'static str> {
    use crate::chrome::transport_bar::{EditAction, FileAction};
    if let Some(a) = FileAction::ALL.iter().find(|a| a.label_key() == label_key) {
        return Some(a.action_id());
    }
    if let Some(a) = EditAction::ALL.iter().find(|a| a.label_key() == label_key) {
        return Some(a.action_id());
    }
    match label_key {
        "shortcuts.play_toggle" => Some(shortcuts::ACTION_TOGGLE_PLAY),
        "shortcuts.stop" => Some(shortcuts::ACTION_STOP),
        "menu.settings" => Some(shortcuts::ACTION_SETTINGS),
        "menu.quit" => Some(shortcuts::ACTION_EXIT),
        _ => None,
    }
}

/// 键名（`Keybindings` 中 `KeyCombo.key`）→ macOS 加速键码。
fn str_to_muda_code(s: &str) -> Option<Code> {
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

/// `KeyCombo` → 原生加速键；无修饰键或键名无法映射时返回 None（不设置加速键）。
///
/// AppKit 的菜单加速键只对带修饰键的组合可靠拦截；无修饰键的快捷键
/// （如 Space/Esc）由 egui 统一处理（见 `shortcuts::native_menu_handles`），
/// 不设原生加速键，否则按键会两头落空。
fn combo_to_accelerator(combo: &yinhe_editor_core::shortcuts::KeyCombo) -> Option<Accelerator> {
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

    /// 无修饰键的快捷键不设原生加速键（AppKit 拦不住，由 egui 处理），
    /// 否则按空格等会两头落空。
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

    /// 带修饰键的快捷键设置原生加速键。
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

// ── Finder 打开文件（application:openFiles: / openURLs:）──────────────

/// Finder/桌面"打开方式"传入的文件路径发送端。
static OPEN_FILES_SENDER: Mutex<Option<mpsc::Sender<String>>> = Mutex::new(None);

/// 创建 Finder 打开文件的通道，并把文档打开方法装到 winit 的 delegate 类上。
/// 返回接收端，由调用方在主线程轮询。
fn register_open_files_handler() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    *OPEN_FILES_SENDER.lock().unwrap() = Some(tx);
    install_open_files_methods();
    rx
}

/// 给 winit 的 NSApplicationDelegate 类动态添加 `application:openFiles:` 和
/// `application:openURLs:`（winit 未实现这两个方法）。
/// AppKit 收到 Finder 的 odoc 事件后默认路由到 delegate 的这两个方法；
/// 用 class_addMethod 直接加到既有类上，不需要子类化或替换 isa。
/// 时序：本函数在 App::new（winit 的 applicationDidFinishLaunching: 内）调用，
/// 而 odoc 事件在 didFinishLaunching 返回后才派发，所以冷启动也来得及。
fn install_open_files_methods() {
    let Some(ns_app_class) = cls(cstr!("NSApplication")) else {
        return;
    };
    unsafe {
        let app: *mut AnyObject = objc2::msg_send![ns_app_class, sharedApplication];
        let delegate: *mut AnyObject = objc2::msg_send![app, delegate];
        if delegate.is_null() {
            return;
        }
        let delegate_class: *mut AnyClass = objc2::msg_send![delegate, class];
        if delegate_class.is_null() {
            return;
        }
        // v@:@@ = void (id self, SEL _cmd, id sender, id files/urls)
        let types = c"v@:@@".as_ptr();
        let open_files: objc2::runtime::Imp = std::mem::transmute(
            handle_open_files
                as unsafe extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, *mut AnyObject),
        );
        let open_urls: objc2::runtime::Imp = std::mem::transmute(
            handle_open_urls
                as unsafe extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, *mut AnyObject),
        );
        objc2::ffi::class_addMethod(
            delegate_class,
            objc2::sel!(application:openFiles:),
            open_files,
            types,
        );
        objc2::ffi::class_addMethod(
            delegate_class,
            objc2::sel!(application:openURLs:),
            open_urls,
            types,
        );
    }
}

/// `application:openFiles:` 回调：files 是文件路径字符串数组（NSString*）。
extern "C-unwind" fn handle_open_files(
    _this: &AnyObject,
    _sel: Sel,
    _sender: &AnyObject,
    files: *mut AnyObject,
) {
    if files.is_null() {
        return;
    }
    // 回调中不 panic（ObjC 边界），用 catch_unwind 包一层。
    let paths = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut paths = Vec::new();
        unsafe {
            let count: isize = objc2::msg_send![files, count];
            for i in 0..count {
                let item: *mut AnyObject = objc2::msg_send![files, objectAtIndex: i];
                if item.is_null() {
                    continue;
                }
                let cstr_ptr: *const std::ffi::c_char = objc2::msg_send![item, UTF8String];
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
    if let Ok(paths) = paths {
        send_paths(paths);
    }
}

/// `application:openURLs:` 回调：urls 是 NSArray<NSURL>，提取每个 URL 的路径。
extern "C-unwind" fn handle_open_urls(
    _this: &AnyObject,
    _sel: Sel,
    _sender: &AnyObject,
    urls: *mut AnyObject,
) {
    if urls.is_null() {
        return;
    }
    // 回调中不 panic（ObjC 边界），用 catch_unwind 包一层。
    let paths = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut paths = Vec::new();
        unsafe {
            let count: isize = objc2::msg_send![urls, count];
            for i in 0..count {
                let item: *mut AnyObject = objc2::msg_send![urls, objectAtIndex: i];
                if item.is_null() {
                    continue;
                }
                let ns_path: *mut AnyObject = objc2::msg_send![item, path];
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
    if let Ok(paths) = paths {
        send_paths(paths);
    }
}

/// 把提取到的路径发送到 UI 线程通道。
fn send_paths(paths: Vec<String>) {
    if let Ok(sender_guard) = OPEN_FILES_SENDER.lock()
        && let Some(tx) = sender_guard.as_ref()
    {
        for path in paths {
            let _ = tx.send(path);
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
/// Mutex：「最近修改的文件」子菜单在运行时重建内容，需要增删映射。
static MENU_MAP: OnceLock<Mutex<HashMap<MenuId, MenuAction>>> = OnceLock::new();

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
    /// 更新菜单项加速键（快捷键配置变化时调用）。
    fn update_accelerator(&self, accelerator: Option<Accelerator>);
}

impl MenuText for MenuItem {
    fn set_text(&self, text: &str) {
        // 全限定调用固有方法，避免与 trait 方法同名递归
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

    fn update_accelerator(&self, _accelerator: Option<Accelerator>) {
        // 子菜单没有加速键
    }
}

impl MenuText for PredefinedMenuItem {
    fn set_text(&self, _text: &str) {
        // 分隔符无文本
    }

    fn update_accelerator(&self, _accelerator: Option<Accelerator>) {
        // 分隔符无加速键
    }
}

/// 持有菜单栏所有 Rust 对象，保持底层 NSMenu/NSMenuItem 存活。
struct NativeMenu {
    _menu: Menu,
    /// 按创建顺序保存 (翻译 key, 菜单项)：key 用于语言切换时重新取文本，
    /// 对象本身同时承担保活职责（drop 会导致底层 NSMenuItem 被释放）。
    _items: Vec<(&'static str, Box<dyn MenuText>)>,
    /// 「最近修改的文件」子菜单（内容随 recent_files 在运行时重建）。
    recent_submenu: Submenu,
    /// 子菜单项保活 + 重建时移除（thread_local OnceLock 内只能不可变访问，用 RefCell）。
    recent_items: std::cell::RefCell<Vec<MenuItem>>,
    /// 播放跟随四档勾选项（模式, 翻译 key, 句柄）：勾选态/文本的运行时刷新入口。
    follow_checks: Vec<(FollowMode, &'static str, CheckMenuItem)>,
}

/// FileAction/EditAction → 原生菜单动作的桥接。
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

/// 按动作分组构建原生菜单子菜单。
/// 菜单项文本与 transport bar popup 共用 label_key（单一来源），
/// 加速键取自默认快捷键表（后续由 poll 按用户配置刷新），
/// 保证 macOS 菜单栏与弹窗的动作集合、快捷键始终同步。
fn build_action_submenu<A>(
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
    // &dyn MenuText 通过 trait upcasting 协变到 &dyn IsMenuItem（rustc 1.86+）
    let refs: Vec<&dyn IsMenuItem> = rows
        .iter()
        .map(|(_, b)| b.as_ref() as &dyn IsMenuItem)
        .collect();
    let submenu = Submenu::with_items(t!(title_key), true, &refs)?;
    items.extend(rows);
    Ok(submenu)
}

/// 初始化原生 macOS 菜单栏，使用 muda crate。
/// 在 MenuBarInner::new() 中调用，此时 NSApplication 已就绪。
fn init_native_menu() -> muda::Result<()> {
    let mut map = HashMap::new();
    let mut items: Vec<(&'static str, Box<dyn MenuText>)> = Vec::new();
    let cmd = Modifiers::SUPER;
    let default_kb = Keybindings::default();

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

    // ── 文件菜单（动作/分组与 transport bar 文件 popup 同步；
    //    设置/退出在 App 菜单，故取前 3 组）──
    let file_menu = build_action_submenu(
        &mut map,
        &mut items,
        "menu.file",
        &crate::chrome::transport_bar::FILE_GROUPS[..4],
        &default_kb,
    )?;

    // ── 「最近修改的文件」子菜单（跟在「打开」之后，即 popup 里的位置；
    //    初始为空禁用，由 poll 按 recent_files 重建内容）──
    let recent_submenu = Submenu::new(t!("menu.recent_files"), false);
    file_menu.insert(&recent_submenu, 2)?;

    // ── 编辑菜单（与 transport bar 编辑 popup 同步）──
    let edit_menu = build_action_submenu(
        &mut map,
        &mut items,
        "menu.edit",
        &crate::chrome::transport_bar::EDIT_GROUPS,
        &default_kb,
    )?;

    // ── 播放菜单 ──
    let play_item = Box::new(MenuItem::new(
        t!("shortcuts.play_toggle"),
        true,
        default_kb
            .get(shortcuts::ACTION_TOGGLE_PLAY)
            .first()
            .and_then(combo_to_accelerator),
    ));
    map.insert(play_item.id().clone(), MenuAction::TogglePlay);

    let stop_item = Box::new(MenuItem::new(
        t!("shortcuts.stop"),
        true,
        default_kb
            .get(shortcuts::ACTION_STOP)
            .first()
            .and_then(combo_to_accelerator),
    ));
    map.insert(stop_item.id().clone(), MenuAction::Stop);

    // 录音 / 步进输入（与播放 popup 同步的普通动作项）
    let record_item = Box::new(MenuItem::new(t!("menu.record"), true, None));
    map.insert(record_item.id().clone(), MenuAction::ToggleRecord);
    let step_item = Box::new(MenuItem::new(t!("menu.step_input"), true, None));
    map.insert(step_item.id().clone(), MenuAction::ToggleStepInput);

    // 播放跟随四档（CheckMenuItem 单选勾选；勾选态由 poll 按当前 follow_mode 同步）
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

    // 收集所有 items 保持存活（翻译 key 用于语言切换时刷新文本）
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
    // 此前遗漏：播放菜单标题也要随语言切换刷新
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
            // recent 子菜单标题与跟随四档由专门字段持有（不进 _items），单独刷新
            native.recent_submenu.set_text(&t!("menu.recent_files"));
            for (_, key, item) in &native.follow_checks {
                item.set_text(&t!(*key));
            }
        }
    });
}

/// 重建「最近修改的文件」子菜单内容（recent_files 变化时由 poll 调用）。
/// 与 transport bar 文件 popup 的子菜单保持同一数据源（AudioSettings::recent_files）。
fn refresh_recent_submenu(files: &[String]) {
    NATIVE_MENU.with(|cell| {
        let Some(native) = cell.get() else { return };
        let mut items = native.recent_items.borrow_mut();
        // 移除旧项并清掉事件映射（handle drop 前先从菜单摘除）
        for item in items.drain(..) {
            let _ = native.recent_submenu.remove(&item);
            if let Some(lock) = MENU_MAP.get()
                && let Ok(mut map) = lock.lock()
            {
                map.remove(item.id());
            }
        }
        // 空列表时禁用子菜单（macOS 惯例）
        native.recent_submenu.set_enabled(!files.is_empty());
        for path in files {
            let item = MenuItem::new(
                crate::chrome::transport_bar::recent_display_name(path),
                true,
                None,
            );
            if let Some(lock) = MENU_MAP.get()
                && let Ok(mut map) = lock.lock()
            {
                map.insert(item.id().clone(), MenuAction::OpenRecent(path.clone()));
            }
            let _ = native.recent_submenu.append(&item);
            items.push(item);
        }
    });
}

/// 按当前跟随模式刷新播放菜单四档勾选（popup 内切换后原生菜单同步）。
fn refresh_follow_checks(mode: FollowMode) {
    NATIVE_MENU.with(|cell| {
        if let Some(native) = cell.get() {
            for (m, _, item) in &native.follow_checks {
                item.set_checked(*m == mode);
            }
        }
    });
}

/// 把用户自定义快捷键同步到原生菜单加速键。
/// 由 MenuBarInner::poll 在检测到快捷键配置变化后调用（主线程）。
fn refresh_native_menu_accelerators(keybindings: &Keybindings) {
    NATIVE_MENU.with(|cell| {
        if let Some(native) = cell.get() {
            for (key, item) in &native._items {
                let key = *key;
                if let Some(action_id) = menu_action_id(key) {
                    // 原生菜单一个条目只能显示一个加速键，取第一个快捷键
                    let acc = keybindings
                        .get(action_id)
                        .first()
                        .and_then(combo_to_accelerator);
                    item.update_accelerator(acc);
                }
            }
        }
    });
}

/// 清空全部原生菜单加速键。
/// 设置窗口打开或快捷键录制期间调用：macOS 的菜单加速键由 AppKit 在
/// 系统层面拦截按键（不经过 egui），不清空的话设置页内按 Cmd+S 等组合
/// 会直接触发菜单动作，录制器收不到按键。
fn clear_native_menu_accelerators() {
    NATIVE_MENU.with(|cell| {
        if let Some(native) = cell.get() {
            for (key, item) in &native._items {
                let key = *key;
                if menu_action_id(key).is_some() {
                    item.update_accelerator(None);
                }
            }
        }
    });
}

pub(crate) struct MenuBarInner {
    rx: mpsc::Receiver<MenuAction>,
    open_files_rx: mpsc::Receiver<String>,
    /// 上次应用到原生菜单的 locale，变化时刷新菜单文本。
    last_locale: String,
    /// 上次应用到原生菜单加速键的配置，变化时刷新加速键。
    last_keybindings: Keybindings,
    /// 加速键是否处于暂停（设置窗口打开/录制中）。暂停期间全部清空，
    /// 避免 AppKit 系统级拦截按键导致设置页录不到组合键。
    accelerators_suspended: bool,
    /// 上次同步到原生菜单的最近文件列表，变化时重建子菜单。
    last_recent_files: Vec<String>,
    /// 上次同步到原生菜单的跟随模式（None = 未同步，首次强制刷新勾选）。
    last_follow_mode: Option<FollowMode>,
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
            last_keybindings: Keybindings::default(),
            accelerators_suspended: false,
            last_recent_files: Vec::new(),
            last_follow_mode: None,
        }
    }

    /// suspend 为 true 时清空全部菜单加速键（设置窗口打开或快捷键录制中），
    /// 防止系统在应用层面前拦截组合键；恢复 false 后按最新配置刷新。
    pub fn poll(
        &mut self,
        keybindings: &Keybindings,
        suspend: bool,
        recent_files: &[String],
        follow_mode: FollowMode,
    ) -> Vec<MenuAction> {
        // 动态内容与 transport bar popup 同步：最近文件子菜单、跟随档勾选
        if recent_files != self.last_recent_files {
            self.last_recent_files = recent_files.to_vec();
            refresh_recent_submenu(recent_files);
        }
        if self.last_follow_mode != Some(follow_mode) {
            self.last_follow_mode = Some(follow_mode);
            refresh_follow_checks(follow_mode);
        }
        // 应用内语言切换（设置对话框）时原生菜单文本不会自动更新，
        // 检测 locale 变化后逐个刷新标题（setText 走主线程，poll 在 UI 帧内调用）。
        let locale = rust_i18n::locale();
        if *locale != self.last_locale {
            self.last_locale = locale.to_string();
            refresh_native_menu_texts();
        }
        if suspend {
            if !self.accelerators_suspended {
                self.accelerators_suspended = true;
                clear_native_menu_accelerators();
            }
        } else if self.accelerators_suspended || keybindings != &self.last_keybindings {
            // 恢复或配置变化：按最新配置刷新（初始值相等时不刷新，init_native_menu 已按默认值构建）。
            self.accelerators_suspended = false;
            self.last_keybindings = keybindings.clone();
            refresh_native_menu_accelerators(keybindings);
        }
        std::iter::from_fn(|| self.rx.try_recv().ok()).collect()
    }

    /// 轮询 Finder/桌面"打开方式"传入的文件路径。
    pub fn poll_open_files(&mut self) -> Vec<String> {
        std::iter::from_fn(|| self.open_files_rx.try_recv().ok()).collect()
    }
}
