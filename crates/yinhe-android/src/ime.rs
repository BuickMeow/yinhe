//! 软键盘（IME）显示/隐藏桥 + 输入法文本回流。
//!
//! egui 的 TextEdit 聚焦时本应经 egui-winit → winit → android-activity 自动弹出
//! 软键盘，但 android-activity 0.6 的 showSoftInput 在 Android 11+ 上不可靠
//!（GameActivity 无 EditText 焦点，InputMethodManager 拒绝弹出）。且
//! `showSoftInput(decorView)` 同样会被拒绝——decorView 不是文本输入控件。
//! 因此 Kotlin 侧放一个 1x1 透明 EditText 作为 IME 目标：
//!
//! - 显示/隐藏：JNI 调 `MainActivity.showIme/hideIme`（InputMethodManager 显式调用）。
//! - 文本回流：EditText 的 TextWatcher 经 `onImeText` 回调把全量文本 + 光标
//!   写入 [`PENDING_EDIT`]，egui 每帧经 [`pump_into`] 消费：注入 Ctrl+A 全选 +
//!   `Event::Text` 全量替换（不依赖增量 diff，中文组合输入也稳），再把光标
//!   写回 `TextEditState`。
//! - 光标回推：用户点击输入框改动 egui 光标时，经 `set_selection` 同步
//!   EditText 选区，保证输入法在正确位置插入。
//!
//! 触发时机：工程设置弹窗的 TextEdit `gained_focus → show`、`lost_focus → hide`。
//! 非安卓平台（桌面调试）为空操作。

#[cfg(target_os = "android")]
use std::sync::Mutex;

/// 输入法最近一次提交的（全量文本, 光标码点位置），UI 帧消费后清空。
#[cfg(target_os = "android")]
static PENDING_EDIT: Mutex<Option<(String, usize)>> = Mutex::new(None);

/// 消费输入法文本（None = 无新输入）。
#[cfg(target_os = "android")]
pub fn take_pending_edit() -> Option<(String, usize)> {
    PENDING_EDIT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
}

/// 最近一次与 EditText 对齐的光标位置；焦点输入框共用（切换时自然更新）。
#[cfg(target_os = "android")]
static LAST_PUSHED: Mutex<Option<usize>> = Mutex::new(None);

/// 显示软键盘。
#[cfg(target_os = "android")]
pub fn show() {
    log::info!("ime: show");
    call_activity("showIme", "()V", ImeArgs::None);
}

/// 隐藏软键盘。
#[cfg(target_os = "android")]
pub fn hide() {
    log::info!("ime: hide");
    call_activity("hideIme", "()V", ImeArgs::None);
}

/// 把 egui 光标（码点位置）同步给 EditText 选区。
#[cfg(target_os = "android")]
pub fn set_selection(pos: usize) {
    log::debug!("ime: 光标同步 {pos}");
    call_activity(
        "setImeSelection",
        "(I)V",
        ImeArgs::Int(pos as jni::sys::jint),
    );
}

/// 每帧调用：消费输入法文本注入 egui，并把 egui 光标变化回推 EditText。
/// 注入只在有 TextEdit 聚焦时进行（无焦点说明输入框已失焦，文本丢弃）。
#[cfg(target_os = "android")]
pub fn pump_into(ctx: &egui::Context) {
    use egui::text::{CCursor, CCursorRange};

    let mut last = LAST_PUSHED.lock().unwrap_or_else(|e| e.into_inner());

    let focused = ctx.memory(|m| m.focused());
    if let Some((text, cursor)) = take_pending_edit() {
        let Some(id) = focused else {
            log::warn!("ime: 输入法文本到达但无焦点，丢弃");
            return;
        };
        // 只注入给 TextEdit（load_state 有值说明该 widget 是 TextEdit）。
        if egui::TextEdit::load_state(ctx, id).is_none() {
            log::debug!("ime: 焦点不在输入框，丢弃输入法文本");
            return;
        }
        // Ctrl+A 全选 + 文本全量替换：不依赖 egui 与 EditText 的增量对齐。
        let cmd = egui::Modifiers::COMMAND;
        ctx.input_mut(|i| {
            for pressed in [true, false] {
                i.events.push(egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: cmd,
                });
            }
            i.events.push(egui::Event::Text(text));
        });
        // 替换后光标在末尾，直接把输入法光标位置写回 TextEditState。
        if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
            state.cursor.set_char_range(Some(CCursorRange::two(
                CCursor::new(cursor),
                CCursor::new(cursor),
            )));
            state.store(ctx, id);
        }
        *last = Some(cursor);
    } else if let Some(id) = focused
        && let Some(state) = egui::TextEdit::load_state(ctx, id)
        && let Some(range) = state.cursor.char_range()
    {
        // 用户点击输入框改动了光标 → 回推给 EditText（UTF-16 换算在 Kotlin 侧）。
        let pos = usize::from(range.primary.index);
        if *last != Some(pos) {
            set_selection(pos);
            *last = Some(pos);
        }
    }
}

#[cfg(not(target_os = "android"))]
pub fn show() {}

#[cfg(not(target_os = "android"))]
pub fn hide() {}

#[cfg(not(target_os = "android"))]
// 桌面 pump_into 为空操作，此实现无人调用（仅保持 API 对称）。
#[allow(dead_code)]
pub fn set_selection(_pos: usize) {}

#[cfg(not(target_os = "android"))]
pub fn pump_into(_ctx: &egui::Context) {}

/// MainActivity 方法的参数（无 JNI 生命周期，闭包内构造 JValue）。
#[cfg(target_os = "android")]
enum ImeArgs {
    None,
    Int(jni::sys::jint),
}

/// 通过 JNI 调 MainActivity 的方法（UI 线程执行）。
#[cfg(target_os = "android")]
fn call_activity(method: &'static str, sig: &'static str, args: ImeArgs) {
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
            let signature = jni::signature::RuntimeMethodSignature::from_str(sig)?;
            let arg = match args {
                ImeArgs::None => None,
                ImeArgs::Int(v) => Some(jni::objects::JValue::Int(v)),
            };
            env.call_method(activity, name, signature.method_signature(), arg.as_slice())?;
            Ok(())
        }) {
            log::error!("ime: 调用 {method} 失败: {err:?}");
        }
    }));
}

/// JNI 回调：`MainActivity.onImeText`，输入法文本变化时由 TextWatcher 调用。
/// cursor 为 Unicode 码点位置（Kotlin 侧已从 UTF-16 换算）。
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_jieneng_yinhe_MainActivity_onImeText<'local>(
    mut env: jni::EnvUnowned<'local>,
    _this: jni::objects::JObject<'local>,
    text: jni::objects::JString<'local>,
    cursor: jni::sys::jint,
) {
    use jni::Outcome;
    let outcome = env.with_env(|env| -> jni::errors::Result<(String, usize)> {
        Ok((text.mutf8_chars(env)?.to_string(), cursor.max(0) as usize))
    });
    match outcome.into_outcome() {
        Outcome::Ok((text, cursor)) => {
            log::debug!("ime: 文本变化 len={} cursor={cursor}", text.chars().count());
            *PENDING_EDIT.lock().unwrap_or_else(|e| e.into_inner()) = Some((text, cursor));
        }
        Outcome::Err(e) => log::error!("ime: 读取输入法文本失败: {e}"),
        Outcome::Panic(p) => log::error!("ime: 回调 panic: {p:?}"),
    }
}
