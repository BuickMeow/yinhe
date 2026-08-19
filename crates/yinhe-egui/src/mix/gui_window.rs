//! 插件原生 GUI 的宿主窗口（macOS：NSWindow + 空白 content view 供插件嵌入）。
//!
//! 背景：JUCE 系插件的 CLAP 包装层拒绝浮动窗口（is_floating=true 直接 false），
//! 只支持 set_parent 嵌入。这里由宿主自建顶层 NSWindow，插件把 NSView 嵌进
//! content view——对插件是嵌入式，对用户是独立浮动窗口。
//!
//! 生命周期约定：所有方法必须在主线程调用；drop 前必须先调
//! `ClapPluginInstance::close_gui()`（插件 view 从父 view 移除后窗口才能释放，
//! 机架靠 SlotRuntime 字段声明顺序保证：instance 先于 gui_window drop）。

#![cfg(target_os = "macos")]

use std::ffi::c_void;

use objc2::encode::{Encode, Encoding};
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};

#[repr(C)]
struct NSPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
struct NSSize {
    width: f64,
    height: f64,
}

#[repr(C)]
struct NSRect {
    origin: NSPoint,
    size: NSSize,
}

// msg_send! 传结构体参数需要 Encode（C ABI 按值传递）。
unsafe impl Encode for NSPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for NSSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for NSRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[NSPoint::ENCODING, NSSize::ENCODING]);
}

// NSWindowStyleMask
const STYLE_TITLED: u64 = 1 << 0;
const STYLE_CLOSABLE: u64 = 1 << 1;
const STYLE_RESIZABLE: u64 = 1 << 3;
// NSBackingStoreBuffered
const BACKING_BUFFERED: u64 = 2;

pub(crate) struct PluginGuiWindow {
    /// NSWindow（+1 持有，drop 时 release）。
    window: *mut AnyObject,
    /// content view（窗口持有，不额外 retain）。
    view: *mut AnyObject,
}

impl PluginGuiWindow {
    /// 创建窗口（内容区尺寸 = 插件首选尺寸）。NSApplication 未就绪时返回 None。
    pub(crate) fn new(title: &str, width: u32, height: u32) -> Option<Self> {
        objc2::rc::autoreleasepool(|_| unsafe {
            let cls = AnyClass::get(c"NSWindow")?;
            let rect = NSRect {
                origin: NSPoint { x: 200.0, y: 200.0 },
                size: NSSize {
                    width: width as f64,
                    height: height as f64,
                },
            };
            let style = STYLE_TITLED | STYLE_CLOSABLE | STYLE_RESIZABLE;
            // alloc + initWithContentRect:styleMask:backing:defer:
            let win: *mut AnyObject = msg_send![cls, alloc];
            if win.is_null() {
                return None;
            }
            let win: *mut AnyObject = msg_send![
                win,
                initWithContentRect: rect,
                styleMask: style,
                backing: BACKING_BUFFERED,
                defer: false
            ];
            if win.is_null() {
                return None;
            }
            // 用户关窗时窗口对象不释放（我们要轮询 isVisible 感知关闭）。
            let _: () = msg_send![win, setReleasedWhenClosed: false];
            // 标题（NSString）
            if let Some(ns_string_cls) = AnyClass::get(c"NSString") {
                let c_title = std::ffi::CString::new(title).unwrap_or_default();
                let s: *mut AnyObject = msg_send![ns_string_cls, alloc];
                let s: *mut AnyObject = msg_send![s, initWithUTF8String: c_title.as_ptr()];
                if !s.is_null() {
                    let _: () = msg_send![win, setTitle: s];
                }
            }
            let view: *mut AnyObject = msg_send![win, contentView];
            if view.is_null() {
                let _: () = msg_send![win, release];
                return None;
            }
            Some(Self { window: win, view })
        })
    }

    /// 插件 set_parent 的目标 view 指针。
    pub(crate) fn view_ptr(&self) -> *mut c_void {
        self.view.cast()
    }

    pub(crate) fn show(&self) {
        unsafe {
            let _: () = msg_send![self.window, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
        }
    }

    pub(crate) fn is_visible(&self) -> bool {
        let visible: bool = unsafe { msg_send![self.window, isVisible] };
        visible
    }

    /// 插件请求的尺寸调整（改内容区大小）。
    pub(crate) fn set_content_size(&self, width: u32, height: u32) {
        let size = NSSize {
            width: width as f64,
            height: height as f64,
        };
        unsafe {
            let _: () = msg_send![self.window, setContentSize: size];
        }
    }
}

impl Drop for PluginGuiWindow {
    fn drop(&mut self) {
        // 调用方保证此时插件 GUI 已 destroy（插件 view 已从本 view 移除）。
        unsafe {
            let _: () = msg_send![self.window, close];
            let _: () = msg_send![self.window, release];
        }
    }
}
