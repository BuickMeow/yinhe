use std::ffi::{CStr, CString};

use super::*;

// ── dart:ffi 入口（Dart → Rust）──

/// 拖拽边缘自动滚动（Flutter 每帧调用）。返回 1 = 已滚动。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_drag_tick() -> i32 {
    // 编辑边缘自动滚动 + 松手惯性（两者都由帧驱动）。
    edit::tick_scroll() | inertia::tick()
}

/// 单指按下：按当前工具开始编辑。返回 1 = 进入编辑，0 = 交给平移。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_edit_begin(x: f32, y: f32) -> i32 {
    edit::begin(x, y)
}

/// 以选框语义起手（不修改全局工具；长按拖选框用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_begin_marquee(x: f32, y: f32) -> i32 {
    edit::begin_marquee(x, y)
}

/// 单指落在音符右端：进入改长度（返回 0 则按普通平移处理）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_begin_note_resize(x: f32, y: f32) -> i32 {
    edit::begin_note_resize(x, y)
}

/// 单指落在音符中部：进入移动（返回 0 则按普通平移处理）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_begin_note_move(x: f32, y: f32) -> i32 {
    edit::begin_note_move(x, y)
}

/// 命中音符的像素矩形 JSON（删除淡出动画用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_note_rect_at(x: f32, y: f32) -> *mut std::ffi::c_char {
    to_c_string(edit::note_rect_at(x, y))
}

/// 单指落在选框控制点：进入选框缩放（返回 0 则继续后续命中）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_begin_selection_handle(x: f32, y: f32) -> i32 {
    edit::begin_selection_handle(x, y)
}

/// 单指落在已有选框内：直接移动选框（返回 0 则继续后续命中）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_begin_selection_move(x: f32, y: f32) -> i32 {
    edit::begin_selection_move(x, y)
}

/// 时间标尺长按：以标尺位置起手插入/粘贴定位。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_begin_ruler_insert(x: f32, y: f32) -> i32 {
    edit::begin_ruler_insert(x, y)
}

/// 本次长按是否来自时间标尺。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_insert_from_ruler() -> i32 {
    edit::insert_from_ruler()
}

/// 长按点是否落在已有选框内。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_selection_box_at(x: f32, y: f32) -> i32 {
    edit::selection_box_at(x, y)
}

/// 单指移动：更新编辑拖拽。返回 1 = 仍在编辑，0 = 已取消（应转为平移）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_edit_update(x: f32, y: f32) -> i32 {
    edit::update(x, y)
}

/// 单指释放：提交编辑。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_edit_end(x: f32, y: f32) {
    edit::end(x, y);
}

/// 取消当前编辑拖拽（双指介入/模态窗口/切后台）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_edit_cancel() -> i32 {
    edit::cancel();
    1
}

/// Flutter 长按识别完成：把待定选框/空白态推进到移动态/粘贴态。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_edit_long_press() -> i32 {
    edit::long_press()
}

/// Flutter 手势应用：先平移，再围绕当前焦点缩放，最后统一 clamp。
///
/// `mode`：0=只平移 1=PR 时间轴 2=PR 音高轴 3=AR 时间轴 4=AR 行高。
/// PR 单击：命中音符删除，否则量化位置插入。返回 1 = 有改动。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_tap_note(x: f32, y: f32) -> i32 {
    edit::tap_note(x, y)
}

/// 长按位置上下文 JSON（只用于弹菜单，不进入拖拽态）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_long_press_context_at(x: f32, y: f32) -> *mut std::ffi::c_char {
    to_c_string(edit::context_at(x, y))
}

#[unsafe(no_mangle)]
pub extern "C" fn yinhe_viewport_gesture(
    mode: i32,
    pan_x: f32,
    pan_y: f32,
    scale: f32,
    focus_x: f32,
    focus_y: f32,
) -> i32 {
    scene::viewport_gesture(mode, pan_x, pan_y, scale, focus_x, focus_y)
}

/// 按逻辑像素平移视口（鼠标右键拖拽等）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_pan_by(dx: f32, dy: f32) -> i32 {
    scene::pan_by(dx, dy)
}

/// 以 Flutter 手势结束速度启动物理惯性（逻辑像素/秒）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_start_fling(vx: f32, vy: f32) {
    inertia::start(vx, vy);
}

/// 停止惯性。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_stop_fling() {
    inertia::stop();
}

/// 文档/选区世代号（变化时 Dart 刷新轨道/自动化元数据）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_document_generation() -> u32 {
    scene::document_generation()
}

/// 取消编辑并停止惯性（模态窗口打开/App 切后台时调用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_reset_interaction() {
    edit::cancel();
    inertia::stop();
}

/// 切换视图：0=AR 走带，1=PR 卷帘。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_view_mode(mode: i32) {
    if let Ok(mut s) = scene::scene().lock() {
        s.view_mode = if mode == 0 {
            scene::ViewMode::Arrangement
        } else {
            scene::ViewMode::Pianoroll
        };
        // 切视图后选框数据换了一套：强制 Flutter 重取覆盖层（避免残留）。
        s.overlay_rev = s.overlay_rev.wrapping_add(1);
        // 清掉上一视图的拖动 ghost（AR 自动化竖线不能残留到 PR）。
        s.preview.clear();
    }
}

/// 走带控制：0=播放，1=暂停，2=停止。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_transport(cmd: i32) {
    match cmd {
        0 => audio::play(),
        1 => {
            let tick = audio::pause();
            scene::set_cursor_exact(tick);
        }
        2 => {
            audio::stop();
            scene::reset_cursor();
        }
        _ => {}
    }
}

/// 是否正在播放（0/1）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_is_playing() -> i32 {
    i32::from(audio::is_playing())
}

/// 检查音频流是否已失效并重建（1 = 本次执行了重建）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_audio_recover() -> i32 {
    i32::from(audio::recover_stream_error())
}

/// 播放到工程末尾且 release 尾音已结束：暂停在末尾并同步 UI 光标。
/// 返回 1 表示本次调用触发了暂停。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_auto_pause_at_end() -> i32 {
    match audio::auto_pause_at_end() {
        Some(end) => {
            scene::set_cursor_exact(end);
            1
        }
        None => 0,
    }
}

/// 当前播放位置（秒，UI 时间显示用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_playback_seconds() -> f64 {
    audio::playback_seconds()
}

/// 播放光标在当前视图下的 x（逻辑像素；-1 = 无文档）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_playhead_x() -> f32 {
    scene::playhead_x()
}

/// 播放位置（tick，时间码显示用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_playhead_tick() -> f64 {
    audio::playback_tick()
}

/// AR 长按排序：把 `from` 轨移动到 `to` 位置，成功返回 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_move_track(from: i32, to: i32) -> i32 {
    if from < 0 || to < 0 {
        return 0;
    }
    scene::move_track(from as usize, to as usize)
}

/// 视图快照（Dart 文本浮层用）：写入 15 个 f32，成功返回 1。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_view_snapshot(out: *mut f32) -> i32 {
    if out.is_null() {
        return 0;
    }
    let snap = scene::view_snapshot();
    unsafe {
        std::ptr::copy_nonoverlapping(snap.as_ptr(), out, snap.len());
    }
    1
}

/// 设置 PR 音域键数（88/128/256）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_pr_set_key_count(count: i32) -> i32 {
    if !(0..=u16::MAX as i32).contains(&count) {
        return 0;
    }
    scene::set_pr_key_count(count as u16)
}

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

/// 常用量化档位 JSON（按 PPQ 推导，堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_quantize_presets_json(ppq: f64) -> *mut std::ffi::c_char {
    to_c_string(midi_names::quantize_presets_json(ppq))
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

/// 轨道面板 JSON（堆分配，调用方用 `yinhe_free_string` 释放）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_tracks_json() -> *mut std::ffi::c_char {
    to_c_string(scene::tracks_json())
}

/// PR 自动化栏数据（仅 PB/CC/RPN）JSON（堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_automation_json(track: i32) -> *mut std::ffi::c_char {
    to_c_string(scene::automation_json(track.max(0) as usize))
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

/// 把扁平 `[tick, velocity, ...]` 解析为点对；非有限值丢弃。
fn parse_velocity_stroke(flat: &[f64]) -> Vec<(f64, f32)> {
    flat.as_chunks::<2>()
        .0
        .iter()
        .filter(|p| p[0].is_finite() && p[1].is_finite())
        .map(|p| (p[0], p[1] as f32))
        .collect()
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

/// 滚轮/触控板：ctrl=1 缩放（键盘/面板区副轴，其余主轴），否则平移。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_scroll_at(x: f32, y: f32, dx: f32, dy: f32, ctrl: i32) -> i32 {
    scene::scroll_at(x, y, dx, dy, ctrl != 0)
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

/// 解析扁平 `[old_tick, new_tick, value, ...]`；非有限值丢弃。
fn parse_automation_moves(flat: &[f64]) -> Vec<(u32, u32, f32)> {
    flat.as_chunks::<3>()
        .0
        .iter()
        .filter(|p| p.iter().all(|v| v.is_finite()))
        .map(|p| {
            (
                p[0].max(0.0).round() as u32,
                p[1].max(0.0).round() as u32,
                (p[2] as f32).clamp(0.0, 1.0),
            )
        })
        .collect()
}

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

/// 歌曲标题（空串 = 未设置，堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_song_title() -> *mut std::ffi::c_char {
    to_c_string(scene::song_title())
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

/// tick → 秒（时间码预显示用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_seconds_at_tick(tick: f64) -> f64 {
    scene::seconds_at_tick(tick)
}

/// 删除音轨（连同其音符；一步一 undo）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_delete_track(idx: i32) -> i32 {
    scene::delete_track(idx.max(0) as usize)
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

/// 全曲音符数（顶栏标题展示用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_note_count() -> i64 {
    scene::note_count()
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

/// 设置 AR 左侧轨道面板宽度（折叠/展开两档）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_arr_panel_width(width: f32) -> i32 {
    scene::set_arr_panel_width(width)
}

/// 设置“显示轨道”过滤（-1 = 全部轨道）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_display_track(track: i32) -> i32 {
    scene::set_display_track(track)
}

/// 设置 PR 其他轨道是否去色（0/1）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_gray_other_tracks(on: i32) -> i32 {
    scene::set_gray_other_tracks(on != 0)
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

/// 覆盖层修订号（Dart 仅在变化时重取 JSON）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_overlay_revision() -> u32 {
    scene::overlay_revision()
}

/// 覆盖层 JSON（选框/预览，堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_overlay_json() -> *mut std::ffi::c_char {
    to_c_string(scene::overlay_json())
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

/// 批量处理（AM 当前 lane）：值与位置各一个表达式，空串/无效 = 跳过。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_automation_batch_edit(
    value_expr: *const std::ffi::c_char,
    tick_expr: *const std::ffi::c_char,
) -> i32 {
    scene::automation_batch_edit(cstr_arg(value_expr), cstr_arg(tick_expr))
}

/// 读取可空 C 字符串；null/非法 UTF-8 当空串。
fn cstr_arg<'a>(p: *const std::ffi::c_char) -> &'a str {
    if p.is_null() {
        return "";
    }
    unsafe { std::ffi::CStr::from_ptr(p) }
        .to_str()
        .unwrap_or("")
}

/// Flutter 帧驱动：请求渲染一帧并同步等待 present 完成（纹理与浮层同帧产出）。
///
/// 返回 1 表示本帧已完成 present，0 表示发送失败或被合并/超时。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_render_frame() -> i32 {
    let (done, wait) = std::sync::mpsc::sync_channel::<()>(1);
    if cmd_tx().send(Cmd::RenderNow(done)).is_err() {
        return 0;
    }
    // 同步等待：Flutter 的 paint/raster 因此一定发生在 present 之后，纹理与浮层
    // 对应同一份 Rust 状态，不再依赖各平台（Android ImageReader / iOS
    // copyPixelBuffer / OHOS 外接纹理）各自的合成时序。
    // 超时是兜底：surface 重建或 GPU 驱动异常时不能让 UI 线程永久阻塞。
    match wait.recv_timeout(std::time::Duration::from_millis(500)) {
        Ok(()) => 1,
        Err(_) => 0,
    }
}

/// 释放本库返回的堆字符串。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_free_string(ptr: *mut std::ffi::c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(ptr));
    }
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

fn to_c_string(s: String) -> *mut std::ffi::c_char {
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests;
