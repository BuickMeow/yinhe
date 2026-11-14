use std::ffi::CStr;

use super::*;

// ── iOS 入口（Swift → C ABI）──
//
// 与 Android 的 JNI 入口一一对应；宿主回调（acquire/submit）在 `ios.rs`。

/// 初始化日志、私有目录与占位文档（等价于 Android 的 `nativeInitContext`）。
///
/// `files_dir` 是应用沙箱的 Documents 目录：最近文件、设置、音色库列表都落在这里。
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ios_init(files_dir: *const std::ffi::c_char) {
    init_logging();
    log::info!("yinhe_ios_init: 引擎已加载");
    if !files_dir.is_null() {
        // SAFETY: Swift 侧传入以 NUL 结尾的 UTF-8 路径，调用期间有效。
        if let Ok(dir) = unsafe { CStr::from_ptr(files_dir) }.to_str() {
            scene::set_files_dir(dir.to_string());
        }
    }
    scene::ensure_starter_document();
}

/// 设置 iOS 用户可见 Documents（音色库/最近文件持久化的路径基准）。
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ios_set_documents_dir(dir: *const std::ffi::c_char) {
    if dir.is_null() {
        return;
    }
    // SAFETY: Swift 侧传入以 NUL 结尾的 UTF-8 路径，调用期间有效。
    if let Ok(dir) = unsafe { CStr::from_ptr(dir) }.to_str() {
        scene::set_documents_dir(dir.to_string());
    }
}

/// 登记外部纹理呈现端（0 = 主画布，1 = AM 面板）。
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ios_ensure(kind: i32) {
    init_logging();
    if cmd_tx().send(Cmd::EnsureExternal(kind)).is_err() {
        log::error!("渲染线程不可用，忽略 EnsureExternal({kind})");
    }
}

/// surface 像素尺寸（Swift 侧从 Flutter 纹理池上报）。
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ios_set_size(kind: i32, width: i32, height: i32) {
    let (w, h) = (width.max(1) as u32, height.max(1) as u32);
    if kind == 0 {
        PIXEL_W.store(w, Ordering::Relaxed);
        PIXEL_H.store(h, Ordering::Relaxed);
    } else {
        AM_PIXEL_W.store(w, Ordering::Relaxed);
        AM_PIXEL_H.store(h, Ordering::Relaxed);
    }
}

/// 画布逻辑尺寸（Flutter 逻辑像素）：视图/手势坐标系。
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ios_set_viewport(width: f32, height: f32) {
    init_logging();
    set_logical_size(width, height);
}
