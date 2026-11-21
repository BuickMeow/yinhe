use super::*;

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

/// tick → 秒（时间码预显示用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_seconds_at_tick(tick: f64) -> f64 {
    scene::seconds_at_tick(tick)
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

/// Flutter 帧驱动：只投递渲染请求，不等待 present。
///
/// 返回 1 表示请求已投递，0 表示渲染线程不可用。UI 线程不再被 GPU/交换链
/// 阻塞；慢帧只表现为纹理更新变慢，不再冻结输入。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_render_frame() -> i32 {
    if cmd_tx().send(Cmd::RenderNow).is_err() {
        return 0;
    }
    1
}
