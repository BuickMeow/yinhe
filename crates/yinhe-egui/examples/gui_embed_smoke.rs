//! 手动验证：嵌入式插件原生界面（宿主自建 NSWindow + 插件 view 嵌入，
//! macOS）。会真实弹出插件窗口停留 3 秒。
//! 用法：cargo run -p yinhe-egui --example gui_embed_smoke -- '/Library/Audio/Plug-Ins/CLAP/KV-Element-FX.clap'
//!
//! 诊断工具：为在无 GUI 主程序下跑通 NSApplication 事件循环，内联最小
//! ObjC 窗口代码（不进 yinhe-clap 库代码，仅本 example 使用）。

#![cfg(target_os = "macos")]

use std::ffi::c_void;

use objc2::encode::{Encode, Encoding};
use objc2::msg_send;
use objc2::rc::autoreleasepool;
use objc2::runtime::{AnyClass, AnyObject};
use yinhe_clap::{ClapPluginInstance, HostInfo};

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

unsafe impl Encode for NSPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for NSSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for NSRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[NSPoint::ENCODING, NSSize::ENCODING]);
}

const STYLE_TITLED: u64 = 1 << 0;
const STYLE_CLOSABLE: u64 = 1 << 1;
const STYLE_RESIZABLE: u64 = 1 << 3;
const BACKING_BUFFERED: u64 = 2;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: gui_embed_smoke <path>");
    let infos = yinhe_clap::scan::scan_path(std::path::Path::new(&path)).expect("scan");
    let info = infos.first().expect("no plugins");
    let host = HostInfo::new("yinhe-test", "yinhe", "", "0.1").expect("host info");

    let (win, view) = make_window(&info.name, 800, 600);
    if win.is_null() {
        eprintln!("[x] 无法创建 NSWindow");
        return;
    }

    eprintln!("[1] load + create_gui ...");
    let mut inst = match ClapPluginInstance::load(info, &host) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("[x] load: {e}");
            return;
        }
    };
    let (w, h) = match inst.create_gui() {
        Ok(sz) => {
            eprintln!("[2] create_gui ok, size={}x{}", sz.0, sz.1);
            sz
        }
        Err(e) => {
            eprintln!("[x] create_gui: {e}");
            return;
        }
    };
    set_content_size(win, w, h);
    match inst.attach_and_show_gui(view.cast::<c_void>()) {
        Ok(()) => eprintln!("[3] attach_and_show ok"),
        Err(e) => {
            eprintln!("[x] attach_and_show: {e}");
            return;
        }
    }
    show_window(win);
    eprintln!("[4] window shown, pumping 3s（请观察是否弹出插件原生界面）");
    pump(3.0);
    eprintln!("[5] close_gui");
    inst.close_gui();
    eprintln!("[6] done, dropping");
}

fn pump(seconds: f64) {
    let pool_cls = AnyClass::get(c"NSAutoreleasePool").unwrap();
    let loop_cls = AnyClass::get(c"NSRunLoop").unwrap();
    let date_cls = AnyClass::get(c"NSDate").unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds);
    while std::time::Instant::now() < end {
        unsafe {
            let pool: *mut AnyObject = msg_send![pool_cls, new];
            let run_loop: *mut AnyObject = msg_send![loop_cls, currentRunLoop];
            let date: *mut AnyObject = msg_send![date_cls, dateWithTimeIntervalSinceNow: 0.05];
            let _: () = msg_send![run_loop, runUntilDate: date];
            let _: () = msg_send![pool, drain];
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn make_window(title: &str, width: u32, height: u32) -> (*mut AnyObject, *mut AnyObject) {
    autoreleasepool(|_| unsafe {
        let cls = AnyClass::get(c"NSWindow").expect("NSWindow class");
        let ns_string_cls = AnyClass::get(c"NSString").expect("NSString class");
        let win: *mut AnyObject = msg_send![cls, alloc];
        let rect = NSRect {
            origin: NSPoint { x: 200.0, y: 200.0 },
            size: NSSize {
                width: width as f64,
                height: height as f64,
            },
        };
        let style = STYLE_TITLED | STYLE_CLOSABLE | STYLE_RESIZABLE;
        let win: *mut AnyObject = msg_send![
            win,
            initWithContentRect: rect,
            styleMask: style,
            backing: BACKING_BUFFERED,
            defer: false
        ];
        if win.is_null() {
            return (std::ptr::null_mut(), std::ptr::null_mut());
        }
        let _: () = msg_send![win, setReleasedWhenClosed: false];
        let s: *mut AnyObject = msg_send![ns_string_cls, alloc];
        let ctitle = std::ffi::CString::new(title).unwrap_or_default();
        let s: *mut AnyObject = msg_send![s, initWithUTF8String: ctitle.as_ptr()];
        let _: () = msg_send![win, setTitle: s];
        let view: *mut AnyObject = msg_send![win, contentView];
        (win, view)
    })
}

fn set_content_size(win: *mut AnyObject, width: u32, height: u32) {
    let size = NSSize {
        width: width as f64,
        height: height as f64,
    };
    unsafe {
        let _: () = msg_send![win, setContentSize: size];
    }
}

fn show_window(win: *mut AnyObject) {
    unsafe {
        let _: () = msg_send![win, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
    }
}
