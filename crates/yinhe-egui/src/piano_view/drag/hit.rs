use eframe::egui;

use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::TimeSigEvent;

use crate::selection::drag::{
    main_cross_x_y, main_px_to_tick_dir, orient_rect, tick_to_main_px_dir,
};

/// 双击写音符：write_track 有效且点击位置无音符时创建新音符。
///
/// 音符长度 = 一个量化间隔（与铅笔点击一致）。返回 `(note, track)`。
/// 命中已有音符（write_track 上）时返回 `None`——双击保持选中/拖拽行为。
#[allow(clippy::too_many_arguments)]
pub(crate) fn double_click_note(
    midi: Option<&dyn yinhe_types::NoteSource>,
    write_track: Option<u16>,
    track_visible: &[bool],
    conductor_idx: Option<u16>,
    view: &yinhe_types::PianoRollView,
    local: egui::Pos2,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> Option<(yinhe_core::NoteEvent, u16)> {
    let track =
        crate::piano_view::pencil::valid_pencil_track(write_track, track_visible, conductor_idx)?;
    let (main_px, cross_px) = main_cross_x_y(view, (local.x, local.y));
    let raw_tick = main_px_to_tick_dir(view, main_px);
    let key = view.cross_px_to_key(cross_px);
    // 点击位置已有音符（write_track 上）→ 不创建。
    // key_notes_in_range 左边界保守（tick - max_note_len），右边界精确，
    // 任何覆盖该像素点的音符都会被包含；像素判定过滤跨边界长音符。
    if let Some(midi) = midi {
        let hit = midi
            .key_notes_in_range(key, raw_tick as u32, (raw_tick + 1.0) as u32)
            .any(|n| {
                n.track == track
                    && tick_to_main_px_dir(view, n.start_tick as f64) <= main_px
                    && main_px <= tick_to_main_px_dir(view, n.end_tick as f64)
            });
        if hit {
            return None;
        }
    }
    let tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data).max(0.0);
    let interval = quantize.tick_interval(ppq) as f64;
    Some((
        yinhe_core::NoteEvent {
            id: 0, // 由 Document::add_note 分配
            start_tick: tick as u32,
            end_tick: (tick + interval) as u32,
            key,
            velocity: 100, // App 层替换为 default_velocity
        },
        track,
    ))
}

/// 音符 hit-test：返回 `(mode, track, start_tick, end_tick, key)`。
///
/// 不需要先选中：边缘 → 单音符伸缩；中部 → 单音符移动（与铅笔一致）。
/// 轨道作用域 = track_selected（空 = 全部）∩ track_visible。
/// 只查可能覆盖鼠标点的音符：key_notes_in_range 左边界保守（tick - max_note_len），
/// 右边界精确，每帧 hover 开销与铅笔 hit-test 同级。
pub(crate) fn hit_test_note(
    midi: Option<&dyn yinhe_types::NoteSource>,
    view: &yinhe_types::PianoRollView,
    local: egui::Pos2,
    track_visible: &[bool],
    track_selected: &std::collections::HashSet<u16>,
) -> Option<(crate::piano_view::pencil::HitMode, u16, u32, u32, u8)> {
    const EDGE_THRESHOLD_PX: f32 = 6.0;
    let (main_px, cross_px) = main_cross_x_y(view, (local.x, local.y));
    let (midi, key) = (midi?, view.cross_px_to_key(cross_px));
    let raw_tick = main_px_to_tick_dir(view, main_px);
    let notes = midi.key_notes_in_range(key, raw_tick as u32, (raw_tick + 1.0) as u32);
    for note in notes {
        // 轨道作用域：track_selected（空 = 全部）∩ track_visible。
        let in_scope = (track_selected.is_empty() || track_selected.contains(&note.track))
            && track_visible
                .get(note.track as usize)
                .copied()
                .unwrap_or(true);
        if !in_scope {
            continue;
        }
        // 方向感知的像素矩形：横向 x = tick、y = key；纵向 x = key、y = tick。
        let a = tick_to_main_px_dir(view, note.start_tick as f64);
        let b = tick_to_main_px_dir(view, note.end_tick as f64);
        let c = view.key_to_cross_px(key);
        let note_rect = orient_rect(view, a, b, c, c + view.key_height);
        if !note_rect.contains(local) {
            continue;
        }
        // 主轴上到两端距离：起点 = 伸缩左缘，终点 = 伸缩右缘。
        let dist_start = (main_px - a).abs();
        let dist_end = (main_px - b).abs();
        let mode = if dist_start <= EDGE_THRESHOLD_PX {
            crate::piano_view::pencil::HitMode::ResizeLeft
        } else if dist_end <= EDGE_THRESHOLD_PX {
            crate::piano_view::pencil::HitMode::ResizeRight
        } else {
            crate::piano_view::pencil::HitMode::Move // 音符中部：直接拖动移动该音符
        };
        return Some((mode, note.track, note.start_tick, note.end_tick, key));
    }
    None
}

/// 选框区域内是否至少有一个音符（数据层面，track 范围限定）。
///
/// 框选松手时判断：区域内无音符 → 自动变为垂直选框（全 128 键）。
pub(crate) fn rect_has_notes(
    midi: Option<&dyn yinhe_types::NoteSource>,
    t_start: u32,
    t_end: u32,
    key_lo: u8,
    key_hi: u8,
    track_lo: u16,
    track_hi: u16,
) -> bool {
    let Some(midi) = midi else { return false };
    (key_lo..=key_hi).any(|key| {
        midi.key_notes_in_range(key, t_start, t_end)
            .any(|n| n.track >= track_lo && n.track <= track_hi && n.start_tick >= t_start)
    })
}
