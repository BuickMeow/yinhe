//! 手动验证：VST3 插件原生 GUI（macOS NSWindow 嵌入）。
//!
//! 用法：`cargo run -p yinhe-vst3 --example vst3_gui_probe -- <bundle> <class_id>`
//! 加载插件 → create_view → 宿主 NSWindow → attach_view → 跑 5 秒。

#[cfg(target_os = "macos")]
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[cfg(target_os = "macos")]
fn main() {
    use std::time::{Duration, Instant};

    use objc2::encode::{Encode, Encoding};
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use yinhe_vst3::Vst3PluginInstance;

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

    let args: Vec<String> = std::env::args().collect();
    let (Some(bundle), Some(class_id)) = (args.get(1), args.get(2)) else {
        eprintln!("用法: vst3_gui_probe <bundle.vst3> <class_id>");
        std::process::exit(2);
    };

    unsafe {
        let app: *mut AnyObject = msg_send![
            AnyClass::get(c"NSApplication").expect("NSApplication"),
            sharedApplication
        ];
        let _: objc2::runtime::Bool = msg_send![app, setActivationPolicy: 0u64];
        let _: () = msg_send![app, activateIgnoringOtherApps: true];
    }

    let mut instance = match Vst3PluginInstance::load(std::path::Path::new(bundle), class_id) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("加载失败: {e}");
            std::process::exit(1);
        }
    };
    // 先激活音频（Serum 2 等插件的编辑器依赖激活状态，未激活 attach 会段错误）。
    let _processor = match instance.activate_audio(48_000.0, 512) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("激活音频失败: {e}");
            std::process::exit(1);
        }
    };
    let (w, h) = match instance.create_view() {
        Ok(size) => {
            println!("create_view OK: {}x{}", size.0, size.1);
            size
        }
        Err(e) => {
            eprintln!("create_view 失败: {e}");
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

    // 先显示窗口再 attach：部分插件（Serum 2）在 attached 时需要窗口已在屏幕上。
    unsafe {
        let _: () = msg_send![win, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
    }
    if let Err(e) = unsafe { instance.attach_view(view.cast::<std::ffi::c_void>()) } {
        eprintln!("attach_view 失败: {e}");
        std::process::exit(1);
    }
    println!("GUI 已嵌入，跑 5 秒事件循环…");

    unsafe {
        let run_loop: *mut AnyObject = msg_send![
            AnyClass::get(c"NSRunLoop").expect("NSRunLoop"),
            currentRunLoop
        ];
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let date: *mut AnyObject = msg_send![
                AnyClass::get(c"NSDate").expect("NSDate"),
                dateWithTimeIntervalSinceNow: 0.05f64
            ];
            let _: () = msg_send![run_loop, runUntilDate: date];
        }
    }

    instance.close_view();
    unsafe {
        let _: () = msg_send![win, close];
    }
    println!("VST3 GUI 探测完成");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("vst3_gui_probe 仅支持 macOS");
}
