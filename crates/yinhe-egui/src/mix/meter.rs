use super::*;

/// 读某 dense 通道电平并做 UI 侧弹道处理，返回 ((level L,R), (hold L,R))。
pub(crate) fn smoothed_peak(
    handle: Option<&yinhe_audio::CpalAudioHandle>,
    smoothed: &mut [MeterState],
    dense: usize,
    dt: f32,
) -> ((f32, f32), (f32, f32)) {
    let raw = handle
        .and_then(|a| a.handle.channel_meter_read(dense))
        .unwrap_or((0.0, 0.0));
    let Some(s) = smoothed.get_mut(dense) else {
        return (raw, raw);
    };
    s.update(raw, dt);
    (s.level(), s.hold())
}

/// 更新总输出电平弹道（每帧一次，main_loop 在走带栏之前调用）。结果存入
/// [`MixUiState::master_display`]，供混音台 master 条与走带栏时间码表复用。
pub(crate) fn update_master_meter(app: &mut App, dt: f32) -> ((f32, f32), (f32, f32)) {
    let raw = app
        .audio_state
        .handle
        .as_ref()
        .map(|a| a.handle.master_meter_read())
        .unwrap_or((0.0, 0.0));
    let s = &mut app.mix.smoothed_master;
    s.update(raw, dt);
    app.mix.master_display = (s.level(), s.hold());
    app.mix.master_display
}

/// 更新每轨电平表弹道（每帧一次，main_loop 在走带栏/编排视图之前调用）。
/// 读数来自引擎（仅播放时非零），结果存入 [`MixUiState::track_display`]。
pub(crate) fn update_track_meters(app: &mut App, dt: f32) {
    let n = app
        .workspace
        .active_doc
        .map(|idx| app.workspace.documents[idx].data.model.tracks.len())
        .unwrap_or(0);
    if app.mix.smoothed_tracks.len() != n {
        app.mix.smoothed_tracks = vec![MeterState::default(); n];
    }
    app.mix.track_display.clear();
    for i in 0..n {
        let raw = app
            .audio_state
            .handle
            .as_ref()
            .map(|a| a.handle.track_meter_read(i))
            .unwrap_or((0.0, 0.0));
        let s = &mut app.mix.smoothed_tracks[i];
        s.update(raw, dt);
        app.mix.track_display.push((s.level(), s.hold()));
    }
}

/// 混音台自动高度：基础布局 + 发送区按总线数扩展。
pub(crate) fn auto_panel_height() -> f32 {
    strip::auto_panel_height()
}
