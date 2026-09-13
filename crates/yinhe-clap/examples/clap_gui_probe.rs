//! 手动验证：CLAP 插件原生界面（macOS 嵌入式 NSWindow）。
//!
//! 用法：`cargo run -p yinhe-clap --example clap_gui_probe -- /path/to/plugin.clap`
//! 加载插件 → 创建宿主 NSWindow → 嵌入插件 view → 跑 5 秒事件循环 → 关闭。
//! 每步打印结果，用于定位原生 GUI 打不开的失败点。

#[cfg(target_os = "macos")]
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[cfg(target_os = "macos")]
fn main() {
    use std::ffi::c_void;
    use std::time::{Duration, Instant};

    use objc2::encode::{Encode, Encoding};
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use yinhe_clap::{ClapPluginInstance, HostInfo, scan};

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
        const ENCODING: Encoding =
            Encoding::Struct("CGRect", &[NSPoint::ENCODING, NSSize::ENCODING]);
    }

    const STYLE_TITLED: u64 = 1 << 0;
    const STYLE_CLOSABLE: u64 = 1 << 1;
    const STYLE_RESIZABLE: u64 = 1 << 3;
    const BACKING_BUFFERED: u64 = 2;

    let Some(path) = std::env::args().nth(1) else {
        eprintln!("用法: clap_gui_probe <plugin.clap>");
        std::process::exit(2);
    };

    unsafe {
        let app: *mut AnyObject =
            msg_send![AnyClass::get(c"NSApplication").expect("NSApplication"), sharedApplication];
        let _: objc2::runtime::Bool = msg_send![app, setActivationPolicy: 0u64]; // Regular
        let _: () = msg_send![app, activateIgnoringOtherApps: true];
    }

    let infos = scan::scan_path(std::path::Path::new(&path)).expect("扫描插件失败");
    for i in &infos {
        println!("发现插件: {} ({})", i.name, i.id);
    }
    let info = infos.first().expect("插件无描述");
    let host_info =
        HostInfo::new("yinhe", "yinhe", "", env!("CARGO_PKG_VERSION")).expect("host info");
    let mut instance = ClapPluginInstance::load(info, &host_info).expect("创建实例失败");
    println!("[1/4] 实例已创建");

    let (w, h) = match instance.create_gui() {
        Ok(size) => {
            println!("[2/4] create_gui OK: {w}x{h}", w = size.0, h = size.1);
            size
        }
        Err(e) => {
            eprintln!("[2/4] create_gui 失败: {e}");
            std::process::exit(1);
        }
    };

    let win: *mut AnyObject = unsafe {
        let cls = AnyClass::get(c"NSWindow").expect("NSWindow");
        let rect = NSRect {
            origin: NSPoint { x: 200.0, y: 200.0 },
            size: NSSize {
                width: w as f64,
                height: h as f64,
            },
        };
        let style = STYLE_TITLED | STYLE_CLOSABLE | STYLE_RESIZABLE;
        let win: *mut AnyObject = msg_send![cls, alloc];
        let win: *mut AnyObject = msg_send![
            win,
            initWithContentRect: rect,
            styleMask: style,
            backing: BACKING_BUFFERED,
            defer: false
        ];
        assert!(!win.is_null(), "NSWindow 创建失败");
        let _: () = msg_send![win, setReleasedWhenClosed: false];
        win
    };
    let view: *mut AnyObject = unsafe { msg_send![win, contentView] };
    println!("[3/4] 宿主窗口已创建");

    match instance.attach_and_show_gui(view.cast::<c_void>()) {
        Ok(()) => println!("[4/4] 插件 view 已嵌入"),
        Err(e) => {
            eprintln!("[4/4] attach_and_show_gui 失败: {e}");
            std::process::exit(1);
        }
    }
    unsafe {
        let _: () = msg_send![win, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
    }

    // 跑 5 秒事件循环，让窗口实际绘制出来。
    unsafe {
        let run_loop: *mut AnyObject =
            msg_send![AnyClass::get(c"NSRunLoop").expect("NSRunLoop"), currentRunLoop];
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let date: *mut AnyObject =
                msg_send![AnyClass::get(c"NSDate").expect("NSDate"), dateWithTimeIntervalSinceNow: 0.05f64];
            let _: () = msg_send![run_loop, runUntilDate: date];
        }
    }

    instance.close_gui();
    unsafe {
        let _: () = msg_send![win, close];
    }
    println!("GUI 探测完成（5 秒后自动关闭）");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("clap_gui_probe 仅支持 macOS");
}
