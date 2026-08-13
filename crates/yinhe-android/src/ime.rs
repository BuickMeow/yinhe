//! 软键盘（IME）显示/隐藏桥。
//!
//! egui 的 TextEdit 聚焦时本应经 egui-winit → winit → android-activity 自动弹出
//! 软键盘，但 android-activity 0.6 的 showSoftInput 在 Android 11+ 上不可靠
//!（GameActivity 无 EditText 焦点，InputMethodManager 拒绝弹出），因此直接
//! JNI 调 `MainActivity.showIme/hideIme`（InputMethodManager 显式调用）。
//! 触发时机：工程设置弹窗的 TextEdit `gained_focus → show`、`lost_focus → hide`。
//! 非安卓平台（桌面调试）为空操作。

/// 显示软键盘。
#[cfg(target_os = "android")]
pub fn show() {
    call_activity("showIme");
}

/// 隐藏软键盘。
#[cfg(target_os = "android")]
pub fn hide() {
    call_activity("hideIme");
}

/// 通过 JNI 调 MainActivity 的无参方法（UI 线程执行）。
#[cfg(target_os = "android")]
fn call_activity(method: &str) {
    let Some(app) = crate::file_picker::android_app() else {
        log::warn!("ime: AndroidApp 未初始化");
        return;
    };
    let app_for_closure = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        let jvm = unsafe { jni::JavaVM::from_raw(app_for_closure.vm_as_ptr() as _) };
        if let Err(err) = jvm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let raw: jni::sys::jobject = app_for_closure.activity_as_ptr() as _;
            let activity = unsafe { jni::objects::JObject::from_raw(env, raw) };
            let name = jni::strings::JNIString::from(method);
            let sig = jni::signature::RuntimeMethodSignature::from_str("()V")?;
            env.call_method(activity, name, sig.method_signature(), &[])?;
            Ok(())
        }) {
            log::error!("ime: 调用 {method} 失败: {err:?}");
        }
    }));
}

#[cfg(not(target_os = "android"))]
pub fn show() {}

#[cfg(not(target_os = "android"))]
pub fn hide() {}
