use super::*;

// ── OpenHarmony 入口（ArkTS → NAPI → C ABI）──
//
// 与 Android 的 JNI 入口一一对应：ArkTS 侧从 Flutter-OH 的外接纹理拿到
// `OHNativeWindow*`（bigint），经 NAPI 胶水转成指针传进来。

/// 初始化日志、私有目录与占位文档（等价于 Android 的 `nativeInitContext`）。
///
/// `files_dir` 是应用沙箱 files 目录：最近文件、设置、音色库列表都落在这里。
#[cfg(target_env = "ohos")]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ohos_init(files_dir: *const std::ffi::c_char) {
    init_logging();
    log::info!("yinhe_ohos_init: 引擎已加载");
    if !files_dir.is_null() {
        // SAFETY: ArkTS 侧传入以 NUL 结尾的 UTF-8 路径，调用期间有效。
        if let Ok(dir) = unsafe { std::ffi::CStr::from_ptr(files_dir) }.to_str() {
            scene::set_files_dir(dir.to_string());
        }
    }
    scene::ensure_starter_document();
}

/// 设置（或替换）主画布 surface。
#[cfg(target_env = "ohos")]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ohos_set_surface(window: *mut std::ffi::c_void) {
    init_logging();
    if window.is_null() {
        log::warn!("yinhe_ohos_set_surface 收到空窗口");
        return;
    }
    if cmd_tx().send(Cmd::SetSurface(OwnedWindow(window))).is_err() {
        log::error!("渲染线程不可用，丢弃 surface");
    }
}

/// 主画布 surface 销毁。
#[cfg(target_env = "ohos")]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ohos_clear_surface() {
    let _ = cmd_tx().send(Cmd::ClearSurface);
}

/// 设置（或替换）AM 面板 surface。
#[cfg(target_env = "ohos")]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ohos_set_automation_surface(window: *mut std::ffi::c_void) {
    init_logging();
    if window.is_null() {
        log::warn!("yinhe_ohos_set_automation_surface 收到空窗口");
        return;
    }
    if cmd_tx()
        .send(Cmd::SetAutomationSurface(OwnedWindow(window)))
        .is_err()
    {
        log::error!("渲染线程不可用，丢弃 AM surface");
    }
}

/// AM 面板 surface 销毁。
#[cfg(target_env = "ohos")]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ohos_clear_automation_surface() {
    let _ = cmd_tx().send(Cmd::ClearAutomationSurface);
}

/// 主画布像素尺寸。
#[cfg(target_env = "ohos")]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ohos_set_size(width: u32, height: u32) {
    PIXEL_W.store(width.max(1), Ordering::Relaxed);
    PIXEL_H.store(height.max(1), Ordering::Relaxed);
}

/// AM 面板像素尺寸。
#[cfg(target_env = "ohos")]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ohos_set_automation_size(width: u32, height: u32) {
    AM_PIXEL_W.store(width.max(1), Ordering::Relaxed);
    AM_PIXEL_H.store(height.max(1), Ordering::Relaxed);
}

/// 画布逻辑尺寸（Flutter 逻辑像素 = Rust 的视图/手势坐标系）。
#[cfg(target_env = "ohos")]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ohos_set_viewport(width: f32, height: f32) {
    set_logical_size(width, height);
}
