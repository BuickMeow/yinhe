//! 本地打开文件（SAF 系统文件选择器）桥。
//!
//! 菜单"本地打开"→ Rust 通过 JNI 调 `MainActivity.openFilePicker`（UI 线程）；
//! 用户选中后 MainActivity 把文件复制到私有目录（SAF uri 授权期短），再回调
//! [`Java_com_jieneng_yinhe_MainActivity_onFilePicked`] 把路径写入全局状态，
//! UI 每帧消费（[`take_picked_path`]）。
//! 非安卓平台（桌面调试）为空操作。

use std::sync::Mutex;

/// 最近一次选中的文件路径（UI 每帧消费后清空）。
static PICKED: Mutex<Option<String>> = Mutex::new(None);

/// 消费最近一次选中的文件路径（None = 无新选择）。
pub fn take_picked_path() -> Option<String> {
    PICKED.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// AndroidApp 引用（android_main 里初始化），供 JNI 调用 MainActivity 方法。
#[cfg(target_os = "android")]
static ANDROID_APP: Mutex<Option<winit::platform::android::activity::AndroidApp>> =
    Mutex::new(None);

/// 保存 AndroidApp（android_main 启动时调用一次）。
#[cfg(target_os = "android")]
pub fn init(app: winit::platform::android::activity::AndroidApp) {
    *ANDROID_APP.lock().unwrap_or_else(|e| e.into_inner()) = Some(app);
}

/// 取回 AndroidApp 引用（ime 等模块复用，避免重复存储）。
#[cfg(target_os = "android")]
pub(crate) fn android_app() -> Option<winit::platform::android::activity::AndroidApp> {
    ANDROID_APP
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// 打开系统文件选择器。非安卓平台为空操作。
#[cfg(target_os = "android")]
pub fn open_file_picker() {
    let app = ANDROID_APP
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(app) = app else {
        log::warn!("file_picker: AndroidApp 未初始化");
        return;
    };
    // run_on_java_main_thread 借用 app，闭包内需要 owned 引用，单独 clone。
    let app_for_closure = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        let jvm = unsafe { jni::JavaVM::from_raw(app_for_closure.vm_as_ptr() as _) };
        if let Err(err) = jvm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let raw: jni::sys::jobject = app_for_closure.activity_as_ptr() as _;
            let activity = unsafe { jni::objects::JObject::from_raw(env, raw) };
            let name = jni::strings::JNIString::from("openFilePicker");
            let sig = jni::signature::RuntimeMethodSignature::from_str("()V")?;
            env.call_method(activity, name, sig.method_signature(), &[])?;
            Ok(())
        }) {
            log::error!("file_picker: 调用 openFilePicker 失败: {err:?}");
        }
    }));
}

/// JNI 回调：MainActivity 已把所选文件复制到私有目录，path 为绝对路径。
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_jieneng_yinhe_MainActivity_onFilePicked<'local>(
    mut env: jni::EnvUnowned<'local>,
    _this: jni::objects::JObject<'local>,
    path: jni::objects::JString<'local>,
) {
    use jni::Outcome;
    let outcome = env
        .with_env(|env| -> jni::errors::Result<String> { Ok(path.mutf8_chars(env)?.to_string()) });
    let path = match outcome.into_outcome() {
        Outcome::Ok(s) => s,
        Outcome::Err(e) => {
            log::error!("file_picker: 读取路径失败: {e}");
            String::new()
        }
        Outcome::Panic(p) => {
            log::error!("file_picker: 回调 panic: {p:?}");
            String::new()
        }
    };
    if !path.is_empty() {
        log::info!("file_picker: 选中文件 {path}");
        *PICKED.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
    }
}

/// 桌面端占位（无文件选择器，菜单里不显示"本地打开"）。
#[cfg(not(target_os = "android"))]
pub fn open_file_picker() {}
