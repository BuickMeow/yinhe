use super::*;

/// 自动保存心跳：Dart 每约 1 秒调用一次，内部按间隔触发并回收后台结果。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_autosave_tick() {
    autosave::tick();
}

/// 启动时检查自动保存残留：`{"path","name","saved_at"}` 或 `{}`。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_autosave_pending_json() -> *mut std::ffi::c_char {
    to_c_string(autosave::pending_json())
}

/// 丢弃自动保存备份（用户选择“不恢复”）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_autosave_discard() {
    autosave::clear();
}

/// 自动保存开关。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_auto_save_enabled(on: i32) -> i32 {
    scene::set_auto_save_enabled(on != 0);
    1
}

/// 导出音频时写入歌曲信息。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_export_audio_metadata(on: i32) -> i32 {
    crate::audio::set_export_audio_metadata(on != 0);
    scene::save_audio_settings();
    1
}

/// 歌曲标题（空串 = 未设置，堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_song_title() -> *mut std::ffi::c_char {
    to_c_string(scene::song_title())
}

/// 界面语言设置（`system` / `zh` / `en`，堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ui_language() -> *mut std::ffi::c_char {
    to_c_string(scene::ui_language())
}

/// 设置界面语言（`system` / `zh` / `en`）并持久化。成功 1。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_set_ui_language(lang: *const std::ffi::c_char) -> i32 {
    if lang.is_null() {
        return 0;
    }
    let lang = unsafe { CStr::from_ptr(lang) }
        .to_string_lossy()
        .into_owned();
    let ok = scene::set_ui_language(lang);
    if ok == 1 {
        scene::save_audio_settings();
    }
    ok
}

/// 界面主题（light/dark，堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ui_theme() -> *mut std::ffi::c_char {
    to_c_string(scene::ui_theme())
}

/// 设置界面主题偏好（auto/light/dark）并持久化。
///
/// 实际明暗由 Flutter 解析系统亮度后调用 [`yinhe_set_theme`] 应用。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_set_ui_theme(theme: *const std::ffi::c_char) -> i32 {
    if theme.is_null() {
        return 0;
    }
    let theme = unsafe { CStr::from_ptr(theme) }
        .to_string_lossy()
        .into_owned();
    let ok = scene::set_ui_theme(theme);
    if ok == 1 {
        scene::save_audio_settings();
    }
    ok
}

/// 应用实际明暗（1=dark）：刷新 GPU 主题、Conductor 色带与场景快照。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_theme(dark: i32) -> i32 {
    let dark = dark != 0;
    yinhe_theme::set_current_gpu_theme(yinhe_theme::GpuTheme::from_base(if dark {
        yinhe_theme::base::BaseColors::DARK
    } else {
        yinhe_theme::base::BaseColors::LIGHT
    }));
    scene::apply_theme_colors();
    1
}

/// 当前工程累计前台工作时间（秒）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_work_time_secs() -> i64 {
    scene::work_time_secs().min(i64::MAX as u64) as i64
}

/// 前台计时器每秒累加；返回新的累计秒数。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_add_work_time(secs: i64) -> i64 {
    scene::add_work_time(secs.max(0) as u64).min(i64::MAX as u64) as i64
}

/// 本工程默认 BPM / 拍号 JSON（堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_project_defaults_json() -> *mut std::ffi::c_char {
    to_c_string(scene::project_defaults_json())
}

/// 设置本工程默认 BPM（f32）/ 拍号（分子 + 分母 2 次幂）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_project_defaults(bpm: f32, num: i32, den_pow: i32) -> i32 {
    scene::set_project_defaults(bpm, num.clamp(1, 32) as u8, den_pow.clamp(0, 6) as u8)
}

/// 音频设置 JSON：`{"max_voices":..,"buffer_size":..,"ignore_velocity":..}`。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_audio_settings_json() -> *mut std::ffi::c_char {
    to_c_string(audio::settings_json())
}

/// 设置音频参数（max_voices=0 自动；buffer_size=0 设备默认，下次初始化生效）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_audio_settings(
    max_voices: u32,
    buffer_size: u32,
    ignore_velocity: u32,
) -> i32 {
    audio::set_settings(max_voices, buffer_size, ignore_velocity.min(127) as u8);
    scene::save_audio_settings();
    1
}

/// 是否允许新音符/移动后的音符与其他音符重叠。关闭后复用桌面端拦截策略。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_allow_overlapping_notes(allow: i32) -> i32 {
    scene::set_allow_overlapping_notes(allow != 0);
    1
}

/// 卷帘背景是否显示 C 音名。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_show_keyboard_labels(show: i32) -> i32 {
    scene::set_show_keyboard_labels(show != 0);
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn yinhe_show_keyboard_labels() -> i32 {
    i32::from(scene::show_keyboard_labels())
}

/// 播放时视口跟随播放头（居中）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_follow_playhead(on: i32) -> i32 {
    scene::set_follow_playhead(on != 0);
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn yinhe_follow_playhead() -> i32 {
    i32::from(scene::follow_playhead())
}

#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_pause_on_home(on: i32) -> i32 {
    scene::set_pause_on_home(on != 0);
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn yinhe_pause_on_home() -> i32 {
    i32::from(scene::pause_on_home())
}

#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_pause_on_exit(on: i32) -> i32 {
    scene::set_pause_on_exit(on != 0);
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn yinhe_pause_on_exit() -> i32 {
    i32::from(scene::pause_on_exit())
}
