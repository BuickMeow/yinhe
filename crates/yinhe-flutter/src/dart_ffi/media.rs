use super::*;

/// 开始导出 WAV（路径 + 16/24/32 位深）。成功 1。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_export_audio_start(path: *const std::ffi::c_char, bit_depth: i32) -> i32 {
    if path.is_null() {
        return 0;
    }
    let path = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    audio::start_export(&path, bit_depth)
}

/// 音频导出进度 JSON（堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_export_audio_progress_json() -> *mut std::ffi::c_char {
    to_c_string(audio::export_progress_json())
}

/// 取消音频导出。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_export_audio_cancel() {
    audio::cancel_export();
}

/// 开始导出 MIDI（路径 + 选项 JSON）。成功 1。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_export_midi_start(
    path: *const std::ffi::c_char,
    opts: *const std::ffi::c_char,
) -> i32 {
    if path.is_null() {
        return 0;
    }
    let path = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    let opts = if opts.is_null() {
        "{}".to_string()
    } else {
        unsafe { CStr::from_ptr(opts) }
            .to_string_lossy()
            .into_owned()
    };
    midi_export::start(path, &opts)
}

/// 开始导出压缩 MIDI（.7z，路径 + 选项 JSON）。成功 1。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_export_midi_archive_start(
    path: *const std::ffi::c_char,
    opts: *const std::ffi::c_char,
    format: i32,
) -> i32 {
    if path.is_null() {
        return 0;
    }
    let path = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    let opts = if opts.is_null() {
        "{}".to_string()
    } else {
        unsafe { CStr::from_ptr(opts) }
            .to_string_lossy()
            .into_owned()
    };
    midi_export::start_archive(path, &opts, format)
}

/// MIDI 导出状态 JSON（堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_export_midi_status_json() -> *mut std::ffi::c_char {
    to_c_string(midi_export::status_json())
}

/// MIDI 静态名称表 JSON（GM/CC/RPN/音名/插入上限，堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_midi_names_json() -> *mut std::ffi::c_char {
    to_c_string(midi_names::midi_names_json())
}

/// 当前音色库表 JSON（顺序 = 优先级，堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_soundfonts_json() -> *mut std::ffi::c_char {
    to_c_string(audio::soundfonts_json())
}

/// 音色库切换进度 JSON：`{"loading":bool,"done":n,"total":n}`。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_sf_loading_json() -> *mut std::ffi::c_char {
    to_c_string(audio::sf_loading_json())
}

/// 设置音色库表 JSON（设置页排序/启停/删除后调用）并持久化。成功 1。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_set_soundfonts(json: *const std::ffi::c_char) -> i32 {
    if json.is_null() {
        return 0;
    }
    let json = unsafe { CStr::from_ptr(json) }
        .to_string_lossy()
        .into_owned();
    let ok = audio::set_soundfonts_json(&json);
    if ok == 1 {
        scene::save_soundfont_settings();
        soundfont_archive::cleanup();
    }
    ok
}

/// 打开音色库压缩包（无密码）。后台线程，Dart 轮询 yinhe_soundfont_archive_status。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn yinhe_soundfont_archive_open(path: *const std::ffi::c_char) -> i32 {
    if path.is_null() {
        return 0;
    }
    let path = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    soundfont_archive::open(path, None)
}

/// 带密码打开音色库压缩包。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn yinhe_soundfont_archive_open_password(
    path: *const std::ffi::c_char,
    password: *const std::ffi::c_char,
) -> i32 {
    if path.is_null() || password.is_null() {
        return 0;
    }
    let path = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    let password = unsafe { CStr::from_ptr(password) }
        .to_string_lossy()
        .into_owned();
    soundfont_archive::open(path, Some(password))
}

/// 音色库压缩包状态：0=无 1=打开中 2=就绪 3=需要密码 4=失败。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_soundfont_archive_status() -> i32 {
    soundfont_archive::status()
}

/// 音色库压缩包条目 JSON（堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_soundfont_archive_json() -> *mut std::ffi::c_char {
    to_c_string(soundfont_archive::json())
}

/// 导入第 index 个音色库条目（后台解压）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_soundfont_archive_import(index: i32) -> i32 {
    soundfont_archive::import(index.max(0) as usize)
}

/// 音色库导入进度 JSON（堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_soundfont_archive_import_json() -> *mut std::ffi::c_char {
    to_c_string(soundfont_archive::import_json())
}

/// 音乐库 JSON（只读 project.json，按专辑分组）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_library_json() -> *mut std::ffi::c_char {
    to_c_string(library::library_json())
}

/// 文档后台任务状态 JSON：`{"state","progress","status","error"}`。
///
/// `state`: 0=无任务 1=运行中 2=完成 3=失败。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_doc_job_status_json() -> *mut std::ffi::c_char {
    to_c_string(doc_job::status_json())
}

/// 取消当前文档任务（只有 PPQ 缩放在安全点响应）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_doc_job_cancel() -> i32 {
    doc_job::cancel()
}

/// 开始解析要导入的 MIDI；返回 1 = 已开始。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_midi_import_start(path: *const std::ffi::c_char) -> i32 {
    if path.is_null() {
        return 0;
    }
    let Ok(path) = unsafe { CStr::from_ptr(path) }.to_str() else {
        return 0;
    };
    midi_import::start(path.to_string())
}

/// 导入解析状态 JSON：`{"state","progress","error","summary"}`。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_midi_import_status_json() -> *mut std::ffi::c_char {
    to_c_string(midi_import::status_json())
}

/// 应用导入；`include_json` 是每源轨 bool 数组，`channel_mode` 0=保留源通道 1=分配空闲。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn yinhe_midi_import_apply(
    include_json: *const std::ffi::c_char,
    channel_mode: i32,
) -> i32 {
    if include_json.is_null() {
        return 0;
    }
    let Ok(include) = unsafe { CStr::from_ptr(include_json) }.to_str() else {
        return 0;
    };
    midi_import::apply(include, channel_mode)
}

/// 取消/丢弃当前导入解析结果。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_midi_import_cancel() -> i32 {
    midi_import::cancel()
}

/// 导入完成后把 AR 视口定位到导入起点。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_midi_import_focus() {
    midi_import::focus_selection();
}
