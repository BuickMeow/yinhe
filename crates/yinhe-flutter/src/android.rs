use std::ffi::CStr;

use super::*;

// ── JNI 入口（Kotlin → Rust）──

/// `MainActivity.nativeSetSurface(surface)`：替换生产端 surface。
// JVM 保证 env/surface 句柄有效；JNI 导出函数无法用 Rust 安全类型表达入参。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_jieneng_yinhe_MainActivity_nativeSetSurface(
    env: *mut jni::sys::JNIEnv,
    _this: jni::sys::jobject,
    surface: jni::sys::jobject,
) {
    init_logging();
    if surface.is_null() {
        log::warn!("nativeSetSurface 收到空 surface");
        return;
    }
    // 安全性：本函数只由 JVM 调用，env/surface 句柄有效；空指针已检查。
    let raw = native_window::from_surface(env, surface);
    if raw.is_null() {
        log::error!("ANativeWindow_fromSurface 返回空指针");
        return;
    }
    // 发送失败时 OwnedWindow 随错误值 drop，引用计数正确释放。
    if cmd_tx().send(Cmd::SetSurface(OwnedWindow(raw))).is_err() {
        log::error!("渲染线程不可用，丢弃 surface");
    }
}

/// `MainActivity.nativeClearSurface()`：surface 销毁。
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_jieneng_yinhe_MainActivity_nativeClearSurface(
    _env: *mut jni::sys::JNIEnv,
    _this: jni::sys::jobject,
) {
    init_logging();
    if cmd_tx().send(Cmd::ClearSurface).is_err() {
        log::error!("渲染线程不可用，忽略 ClearSurface");
    }
}

/// `MainActivity.nativeSetAutomationSurface(surface)`：AM 面板独立 surface。
// JVM 保证 env/surface 句柄有效；JNI 导出函数无法用 Rust 安全类型表达入参。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_jieneng_yinhe_MainActivity_nativeSetAutomationSurface(
    env: *mut jni::sys::JNIEnv,
    _this: jni::sys::jobject,
    surface: jni::sys::jobject,
) {
    init_logging();
    if surface.is_null() {
        log::warn!("nativeSetAutomationSurface 收到空 surface");
        return;
    }
    // 安全性：本函数只由 JVM 调用，env/surface 句柄有效；空指针已检查。
    let raw = native_window::from_surface(env, surface);
    if raw.is_null() {
        log::error!("AM ANativeWindow_fromSurface 返回空指针");
        return;
    }
    if cmd_tx()
        .send(Cmd::SetAutomationSurface(OwnedWindow(raw)))
        .is_err()
    {
        log::error!("渲染线程不可用，丢弃 AM surface");
    }
}

/// `MainActivity.nativeClearAutomationSurface()`：AM surface 销毁。
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_jieneng_yinhe_MainActivity_nativeClearAutomationSurface(
    _env: *mut jni::sys::JNIEnv,
    _this: jni::sys::jobject,
) {
    init_logging();
    if cmd_tx().send(Cmd::ClearAutomationSurface).is_err() {
        log::error!("渲染线程不可用，忽略 ClearAutomationSurface");
    }
}

/// `MainActivity.nativeSetSize(w, h)`：surface 像素尺寸（resize 只重配置交换链）。
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_jieneng_yinhe_MainActivity_nativeSetSize(
    _env: *mut jni::sys::JNIEnv,
    _this: jni::sys::jobject,
    width: jni::sys::jint,
    height: jni::sys::jint,
) {
    PIXEL_W.store(width.max(1) as u32, Ordering::Relaxed);
    PIXEL_H.store(height.max(1) as u32, Ordering::Relaxed);
}

/// `MainActivity.nativeSetAutomationSize(w, h)`：AM surface 像素尺寸。
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_jieneng_yinhe_MainActivity_nativeSetAutomationSize(
    _env: *mut jni::sys::JNIEnv,
    _this: jni::sys::jobject,
    width: jni::sys::jint,
    height: jni::sys::jint,
) {
    AM_PIXEL_W.store(width.max(1) as u32, Ordering::Relaxed);
    AM_PIXEL_H.store(height.max(1) as u32, Ordering::Relaxed);
}

/// `MainActivity.nativeSetViewport(w, h)`：画布逻辑尺寸（Flutter 逻辑像素）。
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_jieneng_yinhe_MainActivity_nativeSetViewport(
    _env: *mut jni::sys::JNIEnv,
    _this: jni::sys::jobject,
    width: jni::sys::jfloat,
    height: jni::sys::jfloat,
) {
    init_logging();
    set_logical_size(width, height);
}

/// 从 JNI 字符串读取 Rust String（失败返回 None）。
#[cfg(target_os = "android")]
fn read_jstring(env: &mut jni::EnvUnowned<'_>, s: &jni::objects::JString<'_>) -> Option<String> {
    use jni::Outcome;
    let outcome =
        env.with_env(|env| -> jni::errors::Result<String> { Ok(s.mutf8_chars(env)?.to_string()) });
    match outcome.into_outcome() {
        Outcome::Ok(text) => Some(text),
        Outcome::Err(e) => {
            log::error!("JNI 字符串读取失败: {e}");
            None
        }
        Outcome::Panic(p) => {
            log::error!("JNI 字符串读取 panic: {p:?}");
            None
        }
    }
}

/// `MainActivity.nativeInitContext(context)`：初始化 `ndk-context`。
///
/// cpal 的 Android 后端（AAudio）需要 JavaVM + `android.content.Context`；
/// 传入 `applicationContext`（Application 单例由系统持有，无需额外全局引用）。
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_jieneng_yinhe_MainActivity_nativeInitContext<'local>(
    mut env: jni::EnvUnowned<'local>,
    _this: jni::objects::JObject<'local>,
    context: jni::objects::JObject<'local>,
    files_dir: jni::objects::JString<'local>,
) {
    init_logging();
    use jni::Outcome;
    // 私有目录：最近文件持久化用。
    if let Some(dir) = read_jstring(&mut env, &files_dir)
        && !dir.is_empty()
    {
        scene::set_files_dir(dir);
    }
    let outcome = env.with_env(|env| -> jni::errors::Result<usize> {
        Ok(env.get_java_vm()?.get_raw() as usize)
    });
    match outcome.into_outcome() {
        Outcome::Ok(vm_ptr) => {
            let ctx_ptr = context.as_raw() as usize;
            static INIT: std::sync::Once = std::sync::Once::new();
            INIT.call_once(|| {
                // 安全性：vm/context 由 JVM 提供；applicationContext 在进程
                // 生命周期内常驻，不会被子 GC 回收。
                unsafe {
                    ndk_context::initialize_android_context(
                        vm_ptr as *mut std::ffi::c_void,
                        ctx_ptr as *mut std::ffi::c_void,
                    );
                }
                log::info!("ndk-context 已初始化");
            });
            // 必须等 AAudio 依赖的 ndk-context 就绪后再创建占位文档：
            // apply_document 会初始化音频引擎，提前调用会触发 ndk-context panic。
            scene::ensure_starter_document();
        }
        Outcome::Err(e) => log::error!("ndk-context 初始化失败: {e}"),
        Outcome::Panic(p) => log::error!("ndk-context 初始化 panic: {p:?}"),
    }
}

/// 音色库选择完成：入表并持久化。
///
/// Android/鸿蒙/iOS 共用：壳只把文件拷进沙箱并回路径，入表由 Dart 调这里。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_add_soundfont(path: *const std::ffi::c_char) {
    if path.is_null() {
        return;
    }
    // SAFETY: 调用方（Dart）传入以 NUL 结尾的 UTF-8 路径，调用期间有效。
    if let Ok(p) = unsafe { CStr::from_ptr(path) }.to_str()
        && !p.is_empty()
    {
        log::info!("音色库入表: {p}");
        audio::add_soundfont(p.to_string());
        scene::save_soundfont_settings();
    }
}
