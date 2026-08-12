//! 平台相关的应用目录解析。
//!
//! - 桌面端：用户配置目录（`dirs::config_dir()`）。
//! - 安卓端：应用内部存储 `getFilesDir()`（无需任何存储权限）。

use std::path::PathBuf;

/// 应用配置目录（`<config>/yinhe`，安卓上为 `<files>/yinhe`）。
#[cfg(not(target_os = "android"))]
pub fn app_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("yinhe")
}

/// 应用配置目录（安卓：内部存储的 files 目录，应用卸载时一并清除）。
#[cfg(target_os = "android")]
pub fn app_config_dir() -> PathBuf {
    let ctx = ndk_context::android_context();
    let fallback = || PathBuf::from(".");
    // ndk-context 的 JavaVM/Context 只在 android_main 之后有效。
    if ctx.vm().is_null() {
        return fallback();
    }
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) };
    let raw_context = ctx.context() as jni::sys::jobject;
    let dir = vm.attach_current_thread(|env: &mut jni::Env<'_>| {
        let context_obj = unsafe { jni::objects::JObject::from_raw(env, raw_context) };
        let files_dir = env
            .call_method(
                &context_obj,
                jni::strings::JNIString::new("getFilesDir"),
                jni::jni_sig!("()Ljava/io/File;"),
                &[],
            )?
            .l()?;
        let abs = env
            .call_method(
                &files_dir,
                jni::strings::JNIString::new("getAbsolutePath"),
                jni::jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let abs = unsafe { jni::objects::JString::from_raw(env, abs.into_raw()) };
        let s = abs.mutf8_chars(env)?;
        let s: String = s.into();
        Ok::<PathBuf, jni::errors::Error>(PathBuf::from(s).join("yinhe"))
    });
    dir.unwrap_or_else(|_| fallback())
}

/// 应用配置文件的完整路径（`<app_config_dir>/yinhe_settings.json`）。
pub fn app_config_file() -> PathBuf {
    let dir = app_config_dir();
    std::fs::create_dir_all(&dir).ok();
    dir.join("yinhe_settings.json")
}
