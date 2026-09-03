use std::sync::OnceLock;

use objc2::runtime::{AnyClass, AnyObject, Sel};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{cls, cstr};

pub(crate) fn request_user_attention() {
    let ns_app_class = match cls(cstr!("NSApplication")) {
        Some(c) => c,
        None => return,
    };
    let ns_app: &AnyObject = unsafe { objc2::msg_send![ns_app_class, sharedApplication] };
    let _: () = unsafe { objc2::msg_send![ns_app, requestUserAttention: 10i64] };
}

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

extern "C-unwind" fn no_background_drag(_this: &AnyObject, _sel: Sel) -> i8 {
    0
}

static BG_DRAG_DISABLED: OnceLock<()> = OnceLock::new();

pub(crate) fn disable_background_window_drag(frame: &eframe::Frame) {
    if BG_DRAG_DISABLED.get().is_some() {
        return;
    }
    let Ok(handle) = frame.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    let ns_view: &AnyObject = unsafe { &*appkit.ns_view.as_ptr().cast() };
    let view_class: *mut AnyClass = unsafe { objc2::msg_send![ns_view, class] };
    if view_class.is_null() {
        return;
    }
    let imp: objc2::runtime::Imp = unsafe {
        std::mem::transmute(
            no_background_drag as unsafe extern "C-unwind" fn(&AnyObject, Sel) -> i8,
        )
    };
    let added = unsafe {
        objc2::ffi::class_addMethod(
            view_class,
            objc2::sel!(mouseDownCanMoveWindow),
            imp,
            c"B@:".as_ptr(),
        )
    };
    let _ = BG_DRAG_DISABLED.set(());
    if !bool::from(added) {
        tracing::warn!("Failed to override mouseDownCanMoveWindow on content view");
    }
}

thread_local! {
    static APP_NAP_TOKEN: std::cell::Cell<*mut AnyObject> =
        const { std::cell::Cell::new(std::ptr::null_mut()) };
}

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

pub(super) fn ns_app() -> Option<&'static AnyObject> {
    let class = cls(cstr!("NSApplication"))?;
    Some(unsafe { objc2::msg_send![class, sharedApplication] })
}

pub(super) fn show_about_panel() {
    let Some(app) = ns_app() else { return };
    let _: () = unsafe {
        objc2::msg_send![app, orderFrontStandardAboutPanel: std::ptr::null_mut::<AnyObject>()]
    };
}

pub(super) fn hide_app() {
    let Some(app) = ns_app() else { return };
    let _: () = unsafe { objc2::msg_send![app, hide: std::ptr::null_mut::<AnyObject>()] };
}

pub(super) fn hide_others() {
    let Some(app) = ns_app() else { return };
    let _: () =
        unsafe { objc2::msg_send![app, hideOtherApplications: std::ptr::null_mut::<AnyObject>()] };
}

pub(super) fn show_all_apps() {
    let Some(app) = ns_app() else { return };
    let _: () = unsafe { objc2::msg_send![app, unhideAllApplications] };
}
