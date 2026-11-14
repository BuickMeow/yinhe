use super::*;

/// 标尺标签（marker）JSON（堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_markers_json() -> *mut std::ffi::c_char {
    to_c_string(scene::markers_json())
}

/// 画布逻辑 x → 当前视图 tick（量化吸附；标尺交互用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_tick_at_x(x: f32) -> f64 {
    match scene::scene().lock() {
        Ok(s) => scene::tick_at_x(&s, x),
        Err(_) => 0.0,
    }
}

/// 在 tick 处插入标签。成功 1。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_marker_insert(tick: f64, text: *const std::ffi::c_char) -> i32 {
    if text.is_null() {
        return 0;
    }
    let text = unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned();
    scene::marker_insert(tick, text)
}

/// 重命名标签。成功 1。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_marker_rename(tick: f64, text: *const std::ffi::c_char) -> i32 {
    if text.is_null() {
        return 0;
    }
    let text = unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned();
    scene::marker_rename(tick, text)
}

/// 删除标签。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_marker_delete(tick: f64) -> i32 {
    scene::marker_delete(tick)
}

/// 移动标签（画布逻辑 x 决定目标 tick）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_marker_move(from: f64, x: f32) -> i32 {
    scene::marker_move(from, x)
}

/// 走带循环状态 JSON：`{"enabled","start","end"}`。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_loop_json() -> *mut std::ffi::c_char {
    to_c_string(scene::loop_json())
}

/// 切换循环；返回 1 = 开启。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_loop_toggle() -> i32 {
    scene::loop_toggle()
}

/// 直接设置循环开关；返回 1 = 成功。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_loop_set_enabled(enabled: i32) -> i32 {
    scene::loop_set_enabled(enabled != 0)
}

/// 设置循环区间；返回 1 = 成功。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_loop_set_range(start: f64, end: f64) -> i32 {
    scene::loop_set_range(start, end)
}

/// 全局事件行 JSON（markers/meter/key/chord/lyrics）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_event_rows_json() -> *mut std::ffi::c_char {
    to_c_string(scene::event_rows_json())
}

/// 播放光标跳到 tick（标尺标签点击）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_jump_to_tick(tick: f64) {
    scene::set_play_cursor(tick);
}

/// 事件列表跳转：光标精确落点 + PR 视口居中。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_jump_to_event(tick: f64) {
    scene::jump_to_event(tick);
}

/// 光标 tick 的 `bar.beat.tick` 文本（Rust 算，拍号变化时按段连续计数）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_bar_beat_text(tick: f64) -> *mut std::ffi::c_char {
    to_c_string(scene::bar_beat_text(tick))
}

/// 当前拍号段的小节长/拍长（tick）：`[measure_ticks, beat_ticks]`。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_bar_beat_ticks_json(at_tick: f64) -> *mut std::ffi::c_char {
    let (measure, beat) = scene::bar_beat_ticks(at_tick);
    to_c_string(format!("[{measure},{beat}]"))
}

/// 时长文本（从 0 开始）：`小节.拍.tick`。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_bar_beat_duration_text(ticks: f64, at_tick: f64) -> *mut std::ffi::c_char {
    to_c_string(scene::bar_beat_duration_text(ticks, at_tick))
}

/// 解析时长输入（`4` / `4.2` / `4.2.120`）；无效返回 -1。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_parse_bar_beat_duration(
    text: *const std::ffi::c_char,
    at_tick: f64,
) -> f64 {
    if text.is_null() {
        return -1.0;
    }
    // SAFETY: dart:ffi 传入以 NUL 结尾的 UTF-8 字符串，调用期间有效。
    let text = unsafe { CStr::from_ptr(text) };
    match text.to_str() {
        Ok(v) => scene::parse_bar_beat_duration(v, at_tick),
        Err(_) => -1.0,
    }
}

/// 时间标尺标签 JSON（Dart 只负责画）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ruler_labels_json() -> *mut std::ffi::c_char {
    to_c_string(scene::ruler_labels_json())
}

/// 时间网格线 JSON（Dart 只负责画）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_time_grid_json() -> *mut std::ffi::c_char {
    to_c_string(scene::time_grid_json())
}

/// 在 conductor 指定 tick 插入拍号（分子 + 分母 2 次幂）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_insert_time_sig(tick: f64, numerator: i32, denominator_power: i32) -> i32 {
    scene::insert_time_sig(
        tick,
        numerator.clamp(1, 32) as u8,
        denominator_power.clamp(0, 6) as u8,
    )
}

/// 在 conductor 指定 tick 插入调号（root 0=C..11=B；scale 0=大调 1=自然小调 …）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_insert_key_sig(tick: f64, root: i32, scale: i32) -> i32 {
    scene::insert_key_sig(tick, root.clamp(0, 11) as u8, scale)
}

/// 事件列表 JSON（从 from_tick 起，最多 limit 条）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_events_json(
    from_tick: f64,
    limit: i32,
    track: i32,
    skip: u32,
) -> *mut std::ffi::c_char {
    to_c_string(scene::events_json(from_tick, limit, track, skip))
}

/// 删除事件行（JSON）。成功 1。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_event_delete(json: *const std::ffi::c_char) -> i32 {
    if json.is_null() {
        return 0;
    }
    let json = unsafe { CStr::from_ptr(json) }
        .to_string_lossy()
        .into_owned();
    scene::event_delete(&json)
}

/// 修改事件行：音符 = (力度, 结束 tick)；自动化 = 显示值。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_event_update(json: *const std::ffi::c_char, value: f64, gate: f64) -> i32 {
    if json.is_null() {
        return 0;
    }
    let json = unsafe { CStr::from_ptr(json) }
        .to_string_lossy()
        .into_owned();
    scene::event_update(&json, value, gate)
}

/// 修改标签文本。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_event_set_text(
    json: *const std::ffi::c_char,
    text: *const std::ffi::c_char,
) -> i32 {
    if json.is_null() || text.is_null() {
        return 0;
    }
    let json = unsafe { CStr::from_ptr(json) }
        .to_string_lossy()
        .into_owned();
    let text = unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned();
    scene::event_set_text(&json, text)
}

/// 粘贴事件行：source 的内容粘贴到 target 的时间位置/音轨。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_event_paste(
    source: *const std::ffi::c_char,
    target: *const std::ffi::c_char,
) -> i32 {
    if source.is_null() || target.is_null() {
        return 0;
    }
    let source = unsafe { CStr::from_ptr(source) }
        .to_string_lossy()
        .into_owned();
    let target = unsafe { CStr::from_ptr(target) }
        .to_string_lossy()
        .into_owned();
    scene::event_paste(&source, &target)
}
