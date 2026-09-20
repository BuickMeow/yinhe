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

use std::cell::Cell;
use std::ffi::c_void;

use objc2::encode::{Encode, Encoding};
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};

use super::plugin_instance::PluginInstance;

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
    /// 当前窗口外观是否深色（同步主题用，见 [`Self::sync_theme`]）。
    dark: Cell<bool>,
    /// 上一次已知的 content 尺寸：区分"用户拖窗口缩放"与"插件请求改尺寸"。
    last_size: Cell<(u32, u32)>,
}

impl PluginGuiWindow {
    /// 创建窗口（内容区尺寸 = 插件首选尺寸）。NSApplication 未就绪时返回 None。
    ///
    /// `resizable` 来自插件能力（CLAP `can_resize` / VST3 `canResize`）：
    /// 不支持缩放的插件窗口去掉 RESIZABLE 样式，避免用户拖大后插件 GUI
    /// 不跟随、露出宿主背景的空白区域。
    pub(crate) fn new(title: &str, width: u32, height: u32, resizable: bool) -> Option<Self> {
        objc2::rc::autoreleasepool(|_| unsafe {
            let cls = AnyClass::get(c"NSWindow")?;
            let rect = NSRect {
                origin: NSPoint { x: 200.0, y: 200.0 },
                size: NSSize {
                    width: width as f64,
                    height: height as f64,
                },
            };
            let mut style = STYLE_TITLED | STYLE_CLOSABLE;
            if resizable {
                style |= STYLE_RESIZABLE;
            }
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
            let dark = crate::theme::dark_mode();
            apply_appearance(win, dark);
            // 以创建后的实际 content 尺寸为基准：把窗口管理器对尺寸的微调
            // 排除在"用户缩放"之外（避免首次轮询误报一次 resize）。
            let actual: NSRect = msg_send![view, frame];
            let actual = (actual.size.width as u32, actual.size.height as u32);
            Some(Self {
                window: win,
                view,
                dark: Cell::new(dark),
                last_size: Cell::new(actual),
            })
        })
    }

    /// 跟随应用主题同步窗口外观（标题栏深浅）。主题切换后已打开的插件
    /// 窗口也跟随，无需系统切换深色模式。
    pub(crate) fn sync_theme(&self) {
        let dark = crate::theme::dark_mode();
        if self.dark.get() == dark {
            return;
        }
        self.dark.set(dark);
        apply_appearance(self.window, dark);
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

    /// 当前 content view 尺寸（像素）。
    fn content_size(&self) -> (u32, u32) {
        let rect: NSRect = unsafe { msg_send![self.view, frame] };
        (rect.size.width as u32, rect.size.height as u32)
    }

    /// 用户拖窗口缩放：content 尺寸相对上次已知值变化则返回新尺寸
    /// （插件自己请求的调整会同步 `last_size`，不会误判）。
    pub(crate) fn take_user_resize(&self) -> Option<(u32, u32)> {
        let size = self.content_size();
        if size == self.last_size.get() || size.0 == 0 || size.1 == 0 {
            return None;
        }
        self.last_size.set(size);
        Some(size)
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
        // 记录应用后的实际尺寸：插件请求触发的变化不算用户缩放。
        self.last_size.set(self.content_size());
    }
}

/// 给窗口设置 NSAppearance（`NSAppearanceNameDarkAqua` / `NSAppearanceNameAqua`），
/// 让原生标题栏跟随应用主题，而不依赖系统深色模式。
fn apply_appearance(win: *mut AnyObject, dark: bool) {
    unsafe {
        let Some(appearance_cls) = AnyClass::get(c"NSAppearance") else {
            return;
        };
        let Some(ns_string_cls) = AnyClass::get(c"NSString") else {
            return;
        };
        let name = if dark {
            c"NSAppearanceNameDarkAqua"
        } else {
            c"NSAppearanceNameAqua"
        };
        let s: *mut AnyObject = msg_send![ns_string_cls, alloc];
        let s: *mut AnyObject = msg_send![s, initWithUTF8String: name.as_ptr()];
        if s.is_null() {
            return;
        }
        let appearance: *mut AnyObject = msg_send![appearance_cls, appearanceNamed: s];
        let _: () = msg_send![s, release];
        if !appearance.is_null() {
            let _: () = msg_send![win, setAppearance: appearance];
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

/// 每帧轮询插件原生 GUI（CLAP/VST3 共用）：主题同步、用户缩放通知、
/// 插件主动关窗/改尺寸请求。效果器机架与乐器机架都用它。
///
/// 返回 `false` = 用户点了宿主窗口关闭按钮（调用方负责关闭插件 GUI 并清窗口）。
#[must_use]
pub(crate) fn poll_plugin_gui(
    instance: Option<&mut PluginInstance>,
    window: &mut Option<PluginGuiWindow>,
) -> bool {
    let Some(win) = window.as_ref() else {
        return true;
    };
    if !win.is_visible() {
        return false;
    }
    // 应用主题变化：标题栏跟随（不依赖系统深色模式）。
    win.sync_theme();
    let mut instance = instance;
    // 用户拖动窗口缩放：交给插件适配（CLAP adjust_size + set_size /
    // VST3 checkSizeConstraint + onSize），再把插件修正后的尺寸回设窗口。
    if let Some((w, h)) = win.take_user_resize() {
        match instance.as_deref_mut() {
            Some(PluginInstance::Clap(inst)) => {
                let (w, h) = inst.gui_set_size(w, h);
                win.set_content_size(w, h);
            }
            Some(PluginInstance::Vst3 { instance, .. }) => {
                let (w, h) = instance.adjust_view_size(w, h);
                win.set_content_size(w, h);
                instance.notify_view_resize(w, h);
            }
            Some(PluginInstance::Builtin { .. }) | None => {}
        }
    }
    match instance {
        Some(PluginInstance::Clap(inst)) => {
            // 插件侧主动断开（closed 回调）：host destroy 确认。
            if inst.take_gui_closed() {
                inst.on_gui_closed();
                return false;
            }
            // 插件请求调整尺寸（如编辑器内部布局变化）。
            if let Some((w, h)) = inst.take_gui_resize() {
                win.set_content_size(w, h);
            }
        }
        Some(PluginInstance::Vst3 { instance, .. }) => {
            // 插件请求调整尺寸：调窗口 + 回调 onSize（VST3 规范）。
            if let Some((w, h)) = instance.take_view_resize() {
                win.set_content_size(w, h);
                instance.notify_view_resize(w, h);
            }
        }
        Some(PluginInstance::Builtin { .. }) | None => {}
    }
    true
}
