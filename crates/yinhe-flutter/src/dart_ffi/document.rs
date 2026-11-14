use super::*;

/// 文档/选区世代号（变化时 Dart 刷新轨道/自动化元数据）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_document_generation() -> u32 {
    scene::document_generation()
}

/// 最近文件 JSON（堆分配，调用方用 `yinhe_free_string` 释放）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_recent_json() -> *mut std::ffi::c_char {
    to_c_string(scene::recent_json())
}

/// 从最近列表移除一项（不删文件）。成功 1。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_recent_remove(path: *const std::ffi::c_char) -> i32 {
    if path.is_null() {
        return 0;
    }
    let path = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    scene::recent_remove(&path)
}

/// 重命名最近工程文件。成功 1。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_recent_rename(
    path: *const std::ffi::c_char,
    new_name: *const std::ffi::c_char,
) -> i32 {
    if path.is_null() || new_name.is_null() {
        return 0;
    }
    let path = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    let new_name = unsafe { CStr::from_ptr(new_name) }
        .to_string_lossy()
        .into_owned();
    scene::recent_rename(&path, &new_name)
}

/// 压缩包状态：0=无 1=打开中 2=就绪 3=需要密码 4=失败。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_archive_status() -> i32 {
    scene::archive_status()
}

/// 压缩包信息 JSON（条目列表；堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_archive_json() -> *mut std::ffi::c_char {
    to_c_string(scene::archive_json())
}

/// 打开压缩包（无密码）；后台线程，Dart 轮询 yinhe_archive_status。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_archive_open(path: *const std::ffi::c_char) -> i32 {
    if path.is_null() {
        return 0;
    }
    let path = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    scene::open_archive(path, None)
}

/// 带密码打开压缩包。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_archive_open_password(
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
    scene::open_archive(path, Some(password))
}

/// 打开压缩包内第 `index` 个 MIDI 条目。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_archive_open_entry(index: i32) -> i32 {
    scene::open_archive_entry(index.max(0) as usize)
}

/// 新建空工程。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_new_project() -> i32 {
    init_logging();
    scene::new_project()
}

/// 工程元数据编辑（歌名/艺术家/专辑/流派/年份/编号/版权/简介/PPQ）。JSON 参数，成功 1。
#[allow(clippy::not_unsafe_ptr_arg_deref)] // JNI/dart:ffi 入口，指针有效性由调用方保证
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_project_meta_json(json: *const std::ffi::c_char) -> i32 {
    if json.is_null() {
        return 0;
    }
    let json = unsafe { CStr::from_ptr(json) }.to_string_lossy();
    scene::set_project_meta_json(&json)
}

/// 设置工程封面（图片路径）。成功 1。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn yinhe_set_project_cover(path: *const std::ffi::c_char) -> i32 {
    if path.is_null() {
        return 0;
    }
    let path = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    scene::set_project_cover(path)
}

/// 当前封面的预览文件路径（无封面返回空串）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_cover_preview_path() -> *mut std::ffi::c_char {
    to_c_string(scene::cover_preview_path())
}

/// 清除工程封面。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_clear_project_cover() -> i32 {
    scene::clear_project_cover()
}

/// 工程元数据 JSON（堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_project_meta_json() -> *mut std::ffi::c_char {
    to_c_string(scene::project_meta_json())
}

/// 开始后台保存 `.yin` 工程到指定路径。成功 1。
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_save_project_start(path: *const std::ffi::c_char) -> i32 {
    if path.is_null() {
        return 0;
    }
    // 安全性：同上，调用方保证指针有效。
    let p = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    scene::save_project_start(p)
}

/// 工程保存进度 JSON（堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_save_project_status_json() -> *mut std::ffi::c_char {
    to_c_string(scene::save_status_json())
}

/// 保存完成后标记工程状态（清除 dirty）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_mark_saved() -> i32 {
    scene::mark_saved()
}

/// 工程状态写入 `out`：`[can_undo, can_redo, dirty]`。成功 1。
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_doc_state(out: *mut i32) -> i32 {
    if out.is_null() {
        return 0;
    }
    let st = scene::doc_state();
    // 安全性：调用方保证 out 至少可写 3 个 i32（dart:ffi 分配）。
    unsafe {
        *out = st[0];
        *out.add(1) = st[1];
        *out.add(2) = st[2];
    }
    1
}

/// 打开指定路径的 MIDI（最近文件列表点击用）。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_open_path(path: *const std::ffi::c_char) {
    if path.is_null() {
        return;
    }
    let path = unsafe { CStr::from_ptr(path) };
    match path.to_str() {
        Ok(p) => scene::open_midi(p.to_string()),
        Err(e) => log::error!("打开路径无效: {e}"),
    }
}

/// 文件加载状态 JSON（堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_load_status_json() -> *mut std::ffi::c_char {
    to_c_string(scene::load_status_json())
}

/// 当前文档来源路径（无文档时为空串，堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_current_path() -> *mut std::ffi::c_char {
    to_c_string(scene::current_path())
}
