use super::*;

/// 常用量化档位 JSON（按 PPQ 推导，堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_quantize_presets_json(ppq: f64) -> *mut std::ffi::c_char {
    to_c_string(midi_names::quantize_presets_json(ppq))
}

/// 轨道面板 JSON（堆分配，调用方用 `yinhe_free_string` 释放）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_tracks_json() -> *mut std::ffi::c_char {
    to_c_string(scene::tracks_json())
}

/// PR 力度条二进制数据：`[tick, length, velocity01, track, ...]`，f64。
///
/// 写入 `out_len`（f64 元素数）并返回内部缓冲区指针；Flutter 必须在下一次
/// 调用前复制数据。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_velocity_bars(out_len: *mut u32) -> *const f64 {
    if out_len.is_null() {
        return std::ptr::null();
    }
    static BUFFER: OnceLock<Mutex<Vec<f64>>> = OnceLock::new();
    let bars = scene::velocity_bars();
    let Ok(mut slot) = BUFFER.get_or_init(|| Mutex::new(Vec::new())).lock() else {
        unsafe {
            *out_len = 0;
        }
        return std::ptr::null();
    };
    *slot = bars;
    let len = slot.len();
    unsafe {
        *out_len = len as u32;
    }
    if len == 0 {
        std::ptr::null()
    } else {
        slot.as_ptr()
    }
}

/// 力度条数据是否已就绪（摘要尚未追上当前文档时返回 0，Dart 稍后重试）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_velocity_ready() -> i32 {
    scene::velocity_ready()
}

/// PR 力度笔划：`points` 为扁平的 `[tick, velocity, ...]`（`len` 是 f64 元素数，
/// 不是点对数），长度必须为偶数。
///
/// 一笔只产生一个 undo；成功返回 1。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_velocity_stroke(points: *const f64, len: u32) -> i32 {
    if points.is_null() || len < 2 || !len.is_multiple_of(2) {
        return 0;
    }
    let flat = unsafe { std::slice::from_raw_parts(points, len as usize) };
    scene::velocity_stroke(&parse_velocity_stroke(flat))
}

/// 选框整体移动增量：`out_dt` tick 差、`out_dk` 半音差、`out_dtr` 轨道差。
/// 返回 1 = 正在移动态。PR 用 dt/dk，AR 用 dt/dtr。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_drag_delta(out_dt: *mut i64, out_dk: *mut i32, out_dtr: *mut i32) -> i32 {
    edit::drag_delta(out_dt, out_dk, out_dtr)
}

/// 选框拖拽状态：0=无、1=待定（长按中）、2=移动态、3=空白处长按完成。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_selection_state() -> i32 {
    edit::selection_state()
}

/// 复制当前选中音符到应用内剪贴板。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_copy_selection() -> i32 {
    scene::copy_selection()
}

/// 剪贴板是否为空：1 = 空。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_clipboard_empty() -> i32 {
    i32::from(scene::clipboard_empty())
}

/// 在长按位置（画布逻辑 x）粘贴剪贴板。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_paste_selection(x: f32) -> i32 {
    scene::paste_selection(x)
}

/// 通道级默认乐器 JSON（16 项：`[bank, program]` 或 null，堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_channel_instruments_json() -> *mut std::ffi::c_char {
    to_c_string(scene::channel_instruments_json())
}

/// 切换轨道 mute。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_toggle_track_mute(track: i32) -> i32 {
    if track < 0 {
        return 0;
    }
    scene::toggle_track_mute(track as usize)
}

/// 切换轨道 solo。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_toggle_track_solo(track: i32) -> i32 {
    if track < 0 {
        return 0;
    }
    scene::toggle_track_solo(track as usize)
}

/// 新建 MIDI 轨：count / port / start_channel / cycle(1=同 port 内循环)。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_add_tracks(count: i32, port: i32, channel: i32, cycle: i32) -> i32 {
    scene::add_tracks(
        count.max(1) as usize,
        port.clamp(0, 15) as u8,
        channel.clamp(0, 15) as u8,
        cycle != 0,
    )
}

/// 编辑音轨信息：名称/端口/通道 + 通道默认乐器（Bank MSB / Bank LSB / Program /
/// 鼓通道）。成功 1。
#[allow(clippy::not_unsafe_ptr_arg_deref, clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_track_info(
    track: i32,
    name: *const std::ffi::c_char,
    port: i32,
    channel: i32,
    bank_msb: i32,
    bank_lsb: i32,
    program: i32,
    is_drum: i32,
    use_instrument: i32,
) -> i32 {
    if track < 0 || name.is_null() {
        return 0;
    }
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    scene::set_track_info(
        track as usize,
        name,
        port.clamp(0, 15) as u8,
        channel.clamp(0, 15) as u8,
        bank_msb.clamp(0, 127) as u8,
        bank_lsb.clamp(0, 127) as u8,
        program.clamp(0, 127) as u8,
        is_drum != 0,
        use_instrument != 0,
    )
}

/// 长按落点 tick（每帧更新指示线用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_long_press_tick() -> f64 {
    edit::long_press_tick()
}

/// 长按落点轨道（-1 = 无）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_long_press_track() -> i32 {
    edit::long_press_track()
}

/// 长按空白处上下文 JSON（插入事件用，堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_long_press_context_json() -> *mut std::ffi::c_char {
    to_c_string(edit::long_press_context_json())
}

/// 重命名音轨。成功 1。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_track_name(track: i32, name: *const std::ffi::c_char) -> i32 {
    if track < 0 || name.is_null() {
        return 0;
    }
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    scene::set_track_name(track as usize, name)
}

/// 删除音轨（连同其音符；一步一 undo）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_delete_track(idx: i32) -> i32 {
    scene::delete_track(idx.max(0) as usize)
}

/// 全曲音符数（顶栏标题展示用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_note_count() -> i64 {
    scene::note_count()
}

/// 撤销一步；成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_undo() -> i32 {
    scene::undo()
}

/// 重做一步；成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_redo() -> i32 {
    scene::redo()
}

/// 设置当前工具（0=选框 1=手形 2=铅笔 3=橡皮 4=刷子）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_tool(tool: i32) -> i32 {
    scene::set_tool(tool)
}

/// 设置量化吸附档位：view 0=AR 1=PR；kind 0=Off 1=Auto 2=Fraction(num/den) 3=Absolute(ticks)。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_quantize(view: i32, kind: i32, num: u32, den: u32) -> i32 {
    scene::set_quantize(view, kind, num, den)
}

/// 按标签设置量化（`1/8`、`120 tick`、`Auto`、`Off`）。成功 1。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_set_quantize_label(view: i32, label: *const std::ffi::c_char) -> i32 {
    if label.is_null() {
        return 0;
    }
    // SAFETY: dart:ffi 传入以 NUL 结尾的 UTF-8 字符串，调用期间有效。
    let label = unsafe { CStr::from_ptr(label) };
    match label.to_str() {
        Ok(v) => scene::set_quantize_label(view, v),
        Err(_) => 0,
    }
}

/// 当前视图量化档位的分数：`[num, den]`（非分数返回 `[0,0]`）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_quantize_fraction_json(view: i32) -> *mut std::ffi::c_char {
    let (num, den) = scene::quantize_fraction(view);
    to_c_string(format!("[{num},{den}]"))
}

/// 当前视图量化档位的 tick 间隔（AM 选框按量化吸附用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_quantize_tick_interval() -> i64 {
    i64::from(edit::quantize_tick_interval())
}

/// 设置当前编辑轨（-1 = 无）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_active_track(track: i32) -> i32 {
    scene::set_active_track(track)
}

/// 框选批量操作。`scope`: 0 = 音符（PR/AR），1 = AM 锚点。
///
/// `action` 编号与桌面 transport bar「操作」菜单一致。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_selection_action(scope: i32, action: i32) -> i32 {
    scene::selection_action(scope, action)
}

/// 全选。`scope`: 0 = PR，1 = AR；`track < 0` = 全工程。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_select_all(scope: i32, track: i32) -> i32 {
    scene::select_all(scope, track)
}

/// 清空音符/AR 选区。返回 1 = 原本有选区。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_clear_selection() -> i32 {
    scene::clear_selection()
}

/// 当前音符选区 tick 范围；有选区时写入 out 并返回 1。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_selection_range(start: *mut f64, end: *mut f64) -> i32 {
    if start.is_null() || end.is_null() {
        return 0;
    }
    let Some((t0, t1)) = scene::selection_tick_range() else {
        return 0;
    };
    unsafe {
        *start = t0;
        *end = t1;
    }
    1
}

/// 人性化：时间/力度/时值三路随机偏差幅度。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_humanize(timing: u32, velocity: u32, gate: u32) -> i32 {
    scene::humanize_selection(timing, velocity, gate)
}

/// 批量处理（音符）：每个字段一个表达式，空串/无效 = 跳过。
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_batch_edit(
    velocity_expr: *const std::ffi::c_char,
    gate_expr: *const std::ffi::c_char,
    key_expr: *const std::ffi::c_char,
    tick_expr: *const std::ffi::c_char,
    stretch_expr: *const std::ffi::c_char,
) -> i32 {
    scene::batch_edit(
        cstr_arg(velocity_expr),
        cstr_arg(gate_expr),
        cstr_arg(key_expr),
        cstr_arg(tick_expr),
        cstr_arg(stretch_expr),
    )
}
