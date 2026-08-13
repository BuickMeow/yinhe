//! 系统安全区（挖孔/刘海/系统栏）insets 桥。
//!
//! Kotlin 侧（`MainActivity.onSystemInsetsChanged`）在 `WindowInsets` 变化时
//! 通过 JNI 回调写入 [`SAFE_INSETS`]（单位：物理像素 px）；egui 每帧读取并
//! 除以 `pixels_per_point` 换算成逻辑点，用于布局避让。
//! 非安卓平台（桌面调试）恒为 0——普通窗口没有安全区概念。

use std::sync::atomic::{AtomicI32, Ordering};

/// 安全区 insets（px）：[left, top, right, bottom]。
static SAFE_INSETS: [AtomicI32; 4] = [const { AtomicI32::new(0) }; 4];

/// 读取安全区 insets（px）。
pub fn safe_insets_px() -> [i32; 4] {
    [
        SAFE_INSETS[0].load(Ordering::Relaxed),
        SAFE_INSETS[1].load(Ordering::Relaxed),
        SAFE_INSETS[2].load(Ordering::Relaxed),
        SAFE_INSETS[3].load(Ordering::Relaxed),
    ]
}

/// JNI 回调：`MainActivity.onSystemInsetsChanged`，insets 变化时由 Kotlin 调用。
/// 参数为逻辑像素（px），布局侧再除以 pixels_per_point。
/// 类型按 jni-sys 的 ABI 定义（JNIEnv = `const JNINativeInterface_ *`，
/// jobject/jint 为标准 JNI 类型），不依赖 jni crate。
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_jieneng_yinhe_MainActivity_onSystemInsetsChanged(
    _env: JNIEnv,
    _this: JObject,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
) {
    log::debug!("safe insets: l={left} t={top} r={right} b={bottom}");
    SAFE_INSETS[0].store(left, Ordering::Relaxed);
    SAFE_INSETS[1].store(top, Ordering::Relaxed);
    SAFE_INSETS[2].store(right, Ordering::Relaxed);
    SAFE_INSETS[3].store(bottom, Ordering::Relaxed);
}

/// 最小 JNI FFI 类型（见上方导出函数）。
#[cfg(target_os = "android")]
#[repr(C)]
pub struct JNINativeInterface {
    _opaque: [u8; 0],
}

/// `JNIEnv *`：不透明指针，回调中不使用，只要求 ABI 兼容。
#[cfg(target_os = "android")]
pub type JNIEnv = *const JNINativeInterface;

/// `jobject`：不透明对象引用。
#[cfg(target_os = "android")]
pub type JObject = *mut core::ffi::c_void;
