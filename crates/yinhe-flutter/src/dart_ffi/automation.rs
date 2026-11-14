use super::*;

/// PR 自动化栏数据（仅 PB/CC/RPN）JSON（堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_automation_json(track: i32) -> *mut std::ffi::c_char {
    to_c_string(scene::automation_json(track.max(0) as usize))
}

/// 长按"插入"事件：kind 0=Tempo、1=PB、2=CC、3=RPN；
/// `number` = CC 号/RPN 号，`value` = 显示值（PB/CC/RPN 原始值域，Tempo 为 BPM）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_insert_automation(
    kind: i32,
    number: i32,
    tick: f64,
    track: i32,
    value: f64,
) -> i32 {
    scene::insert_automation(kind, number, tick, track, value)
}

/// 复制当前 AM lane 选中的锚点。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_automation_copy() -> i32 {
    scene::automation_copy()
}

/// 当前 AM lane 的锚点插值状态 + 预设列表（JSON）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_automation_shape_json() -> *mut std::ffi::c_char {
    to_c_string(scene::automation_shape_json())
}

/// 应用锚点插值形状（0=Step，1=线性，2..=25=缓动预设）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_automation_apply_shape(index: i32) -> i32 {
    scene::automation_apply_shape(index)
}

/// AM 锚点剪贴板是否为空：1 = 空。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_automation_clipboard_empty() -> i32 {
    i32::from(scene::automation_clipboard_empty())
}

/// 把 AM 锚点剪贴板粘贴到当前 lane（首个锚点对齐 `tick`）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_automation_paste(tick: f64) -> i32 {
    scene::automation_paste(tick)
}

/// 该位置当前显示值（无历史值返回 NaN）。
/// AM 直线：按当前 PR 量化在起止点之间插入自动化锚点（一次 undo）。
/// `v0/v1` 为归一化 0..1（Tempo 按最大 BPM 换算）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_automation_line(
    kind: i32,
    number: i32,
    track: i32,
    t0: f64,
    t1: f64,
    v0: f64,
    v1: f64,
) -> i32 {
    scene::automation_line(kind, number, track, t0, t1, v0, v1)
}

/// AM 直线力度：按起止点线性插值影响力度条（一次 undo）。`v0/v1` 归一化 0..1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_velocity_line(t0: f64, v0: f64, t1: f64, v1: f64) -> i32 {
    scene::velocity_line(t0, v0, t1, v1)
}

#[unsafe(no_mangle)]
pub extern "C" fn yinhe_automation_display_value(
    kind: i32,
    number: i32,
    tick: f64,
    track: i32,
) -> f64 {
    scene::automation_display_value(kind, number, tick, track)
}

/// 批量移动自动化锚点：`moves` 为扁平 `[old_tick, new_tick, value, ...]`
/// （`len` 是 f64 元素数，必须能被 3 整除），一笔一个 undo。成功 1。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_move_automation_anchors(
    track: i32,
    lane_index: i32,
    moves: *const f64,
    len: u32,
) -> i32 {
    // lane_index 是可视 lane 的 0 基下标：0 就是第一条自动化 lane，
    // 不能再按旧的一基约定拒绝 0（否则第一条 lane 的锚点永远拖不动）。
    if track < 0 || lane_index < 0 || moves.is_null() || len < 3 || !len.is_multiple_of(3) {
        return 0;
    }
    let flat = unsafe { std::slice::from_raw_parts(moves, len as usize) };
    let parsed = parse_automation_moves(flat);
    scene::move_automation_anchors(track as usize, lane_index as usize, &parsed)
}

/// 删除指定 tick 的自动化锚点（单击删除），一笔一个 undo。成功 1。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub extern "C" fn yinhe_delete_automation_anchors(
    track: i32,
    lane_index: i32,
    ticks: *const u32,
    len: u32,
) -> i32 {
    if track < 0 || lane_index < 0 || ticks.is_null() || len == 0 {
        return 0;
    }
    let ticks = unsafe { std::slice::from_raw_parts(ticks, len as usize) };
    scene::delete_automation_anchors(track as usize, lane_index as usize, ticks)
}

/// 确保自动化 lane 存在（kind/number 与插入自动化一致），成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ensure_automation_lane(track: i32, kind: i32, number: i32) -> i32 {
    if track < 0 {
        return 0;
    }
    scene::ensure_automation_lane(track as usize, kind, number)
}

/// Dart 布局后同步 AM 面板在主纹理中的绘制区域与当前 lane。`lane < 0` = 力度。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_automation_panel(
    w: f32,
    h: f32,
    visible: i32,
    active_track: i32,
    lane: i32,
    show_anchors: i32,
) {
    scene::set_automation_panel(
        w,
        h,
        visible != 0,
        active_track.max(0) as usize,
        if lane < 0 { None } else { Some(lane as usize) },
        show_anchors != 0,
    );
}

/// 自动化拖动 ghost：扁平 `[old_tick, new_tick, new_value, ...]`，len=0 清除。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_automation_ghost(flat: *const f64, len: u32) {
    if flat.is_null() || len == 0 {
        scene::set_automation_ghost(&[]);
        return;
    }
    let slice = unsafe { std::slice::from_raw_parts(flat, len as usize) };
    scene::set_automation_ghost(slice);
}

/// AM 选区同步：扁平 tick 数组（f64），空 = 清除。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_automation_selection(ticks: *const f64, len: u32) {
    if ticks.is_null() || len == 0 {
        scene::set_automation_selection(&[]);
        return;
    }
    let slice = unsafe { std::slice::from_raw_parts(ticks, len as usize) };
    let ticks: Vec<u32> = slice
        .iter()
        .filter(|v| v.is_finite() && **v >= 0.0)
        .map(|v| v.round() as u32)
        .collect();
    scene::set_automation_selection(&ticks);
}

/// 批量处理（AM 当前 lane）：值与位置各一个表达式，空串/无效 = 跳过。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_automation_batch_edit(
    value_expr: *const std::ffi::c_char,
    tick_expr: *const std::ffi::c_char,
) -> i32 {
    scene::automation_batch_edit(cstr_arg(value_expr), cstr_arg(tick_expr))
}
