use std::sync::{Mutex, mpsc};

use objc2::runtime::{AnyClass, AnyObject, Sel};

use super::{cls, cstr};

static OPEN_FILES_SENDER: Mutex<Option<mpsc::Sender<String>>> = Mutex::new(None);

pub(crate) fn register_open_files_handler() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    *OPEN_FILES_SENDER.lock().unwrap() = Some(tx);
    install_open_files_methods();
    rx
}

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

extern "C-unwind" fn handle_open_files(
    _this: &AnyObject,
    _sel: Sel,
    _sender: &AnyObject,
    files: *mut AnyObject,
) {
    if files.is_null() {
        return;
    }
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

extern "C-unwind" fn handle_open_urls(
    _this: &AnyObject,
    _sel: Sel,
    _sender: &AnyObject,
    urls: *mut AnyObject,
) {
    if urls.is_null() {
        return;
    }
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

fn send_paths(paths: Vec<String>) {
    if let Ok(sender_guard) = OPEN_FILES_SENDER.lock()
        && let Some(tx) = sender_guard.as_ref()
    {
        for path in paths {
            let _ = tx.send(path);
        }
    }
}
