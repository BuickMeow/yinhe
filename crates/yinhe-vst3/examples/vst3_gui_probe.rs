//! 手动验证：VST3 插件原生 GUI（macOS NSWindow / Windows Win32 嵌入）。
//!
//! 用法：`cargo run -p yinhe-vst3 --example vst3_gui_probe -- <bundle> <class_id>`
//! 加载插件 → create_view → 宿主顶层窗口 → attach_view → 跑 5 秒消息循环。

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

#[cfg(target_os = "windows")]
fn main() {
    use std::ffi::c_void;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, IDC_ARROW,
        LoadCursorW, MSG, PM_REMOVE, PeekMessageW, PostQuitMessage, RegisterClassW, SW_SHOW,
        SetForegroundWindow, ShowWindow, TranslateMessage, WM_CLOSE, WM_QUIT, WNDCLASSW, WS_CHILD,
        WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };
    use yinhe_vst3::Vst3PluginInstance;

    /// 用户点关闭按钮：只退出消息循环。
    ///
    /// 绝不能在这里直接 `DestroyWindow`——插件 view 还挂在子窗口上，
    /// 先让主流程 `close_view()` 再从父窗口移除，最后才销毁窗口。
    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_CLOSE {
            unsafe { PostQuitMessage(0) };
            return 0;
        }
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    let args: Vec<String> = std::env::args().collect();
    let (Some(bundle), Some(class_id)) = (args.get(1), args.get(2)) else {
        eprintln!("用法: vst3_gui_probe <bundle.vst3> <class_id>");
        std::process::exit(2);
    };

    let mut instance = match Vst3PluginInstance::load(std::path::Path::new(bundle), class_id) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("加载失败: {e}");
            std::process::exit(1);
        }
    };
    // 先激活音频（部分插件的编辑器依赖激活状态，未激活 attach 可能崩溃）。
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

    // 顶层窗口（带标题栏）+ content 子窗口（插件 attach 到此，对应 macOS contentView）。
    let (win, content) = unsafe {
        let hinstance = GetModuleHandleW(std::ptr::null());
        let class_name: Vec<u16> = "YinheVst3GuiProbe\0".encode_utf16().collect();
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: std::ptr::null_mut(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        if RegisterClassW(&wc) == 0 {
            eprintln!("RegisterClassW 失败");
            std::process::exit(1);
        }

        // 插件尺寸是客户区像素，窗口外框要加标题栏/边框。
        let style = WS_OVERLAPPEDWINDOW;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: w as i32,
            bottom: h as i32,
        };
        if windows_sys::Win32::UI::WindowsAndMessaging::AdjustWindowRectEx(&mut rect, style, 0, 0)
            == 0
        {
            eprintln!("AdjustWindowRectEx 失败");
            std::process::exit(1);
        }
        let title: Vec<u16> = "VST3 GUI Probe\0".encode_utf16().collect();
        let win = CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            style | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            rect.right - rect.left,
            rect.bottom - rect.top,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        );
        if win.is_null() {
            eprintln!("顶层窗口创建失败");
            std::process::exit(1);
        }
        let content = CreateWindowExW(
            0,
            class_name.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            w as i32,
            h as i32,
            win,
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        );
        if content.is_null() {
            eprintln!("content 子窗口创建失败");
            std::process::exit(1);
        }
        // 先显示窗口再 attach：部分插件在 attached 时需要窗口已在屏幕上。
        ShowWindow(win, SW_SHOW);
        SetForegroundWindow(win);
        (win, content)
    };

    if let Err(e) = unsafe { instance.attach_view(content.cast::<c_void>()) } {
        eprintln!("attach_view 失败: {e}");
        std::process::exit(1);
    }
    println!("GUI 已嵌入，跑 5 秒消息循环…");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        let mut quit = false;
        unsafe {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_QUIT {
                    quit = true;
                    break;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if quit || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    instance.close_view();
    unsafe {
        DestroyWindow(win);
    }
    println!("VST3 GUI 探测完成");
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn main() {
    eprintln!("vst3_gui_probe 仅支持 macOS / Windows");
}
