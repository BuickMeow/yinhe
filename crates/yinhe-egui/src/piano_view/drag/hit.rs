use eframe::egui;

use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::TimeSigEvent;

use crate::selection::drag::{
    main_cross_x_y, main_px_to_tick_dir, orient_rect, tick_to_main_px_dir,
};

/// 双击写音符：write_track 有效且点击位置无音符时创建新音符，长度优先取该轨 gate 记忆。
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
    default_gate: Option<u32>,
) -> Option<(yinhe_core::NoteEvent, u16)> {
    let track =
        crate::piano_view::pencil::valid_pencil_track(write_track, track_visible, conductor_idx)?;
    let (main_px, cross_px) = main_cross_x_y(view, (local.x, local.y));
    let raw_tick = main_px_to_tick_dir(view, main_px);
    let key = view.cross_px_to_key(cross_px);
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
    let gate = default_gate.unwrap_or(quantize.tick_interval(ppq)) as f64;
    Some((
        yinhe_core::NoteEvent {
            id: 0,
            start_tick: tick as u32,
            end_tick: (tick + gate) as u32,
            key,
            velocity: 100,
        },
        track,
    ))
}

/// 音符 hit-test：边缘→伸缩，中部→移动；作用域 = track_selected(空=全部)∩track_visible。
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
        let in_scope = (track_selected.is_empty() || track_selected.contains(&note.track))
            && track_visible
                .get(note.track as usize)
                .copied()
                .unwrap_or(true);
        if !in_scope {
            continue;
        }
        let a = tick_to_main_px_dir(view, note.start_tick as f64);
        let b = tick_to_main_px_dir(view, note.end_tick as f64);
        let c = view.key_to_cross_px(key);
        let note_rect = orient_rect(view, a, b, c, c + view.key_height);
        if !note_rect.contains(local) {
            continue;
        }
        let dist_start = (main_px - a).abs();
        let dist_end = (main_px - b).abs();
        let mode = if dist_start <= EDGE_THRESHOLD_PX {
            crate::piano_view::pencil::HitMode::ResizeLeft
        } else if dist_end <= EDGE_THRESHOLD_PX {
            crate::piano_view::pencil::HitMode::ResizeRight
        } else {
            crate::piano_view::pencil::HitMode::Move
        };
        return Some((mode, note.track, note.start_tick, note.end_tick, key));
    }
    None
}

fn is_on_bar(
    pos: egui::Pos2,
    music_rect: egui::Rect,
    view: &yinhe_types::PianoRollView,
    eff_rects: &[(f64, f64, u8, u8)],
) -> bool {
    eff_rects.iter().any(|&(t_start, t_end, key_lo, key_hi)| {
        let pixel_rect =
            crate::selection::drag::music_sel_to_pixel_rect(view, t_start, t_end, key_lo, key_hi);
        crate::widgets::selection_actions::compute_bar_rect(music_rect, pixel_rect)
            .is_some_and(|bar| bar.contains(pos))
    })
}

/// 选择工具的早期快速删除检测（右键 + 双击）：命中音符时返回待删除项。
#[allow(clippy::too_many_arguments)]
pub(crate) fn quick_delete_early(
    ui: &egui::Ui,
    pointer: &egui::PointerState,
    music_rect: egui::Rect,
    content_rect: egui::Rect,
    view: &yinhe_types::PianoRollView,
    eff_rects: &[(f64, f64, u8, u8)],
    midi: Option<&dyn yinhe_types::NoteSource>,
    track_visible: &[bool],
    track_selected: &std::collections::HashSet<u16>,
    quick_delete_mode: yinhe_editor_core::audio_settings::QuickDeleteMode,
) -> Option<(u16, u32, u8)> {
    if quick_delete_mode.allows_right_click()
        && ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary))
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
        && !is_on_bar(pos, music_rect, view, eff_rects)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        if let Some((_, track, start_tick, _, key)) =
            hit_test_note(midi, view, local, track_visible, track_selected)
        {
            return Some((track, start_tick, key));
        }
    }
    if quick_delete_mode.allows_double_click()
        && ui.input(|i| {
            i.pointer
                .button_double_clicked(egui::PointerButton::Primary)
        })
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
        && !is_on_bar(pos, music_rect, view, eff_rects)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        if let Some((_, track, start_tick, _, key)) =
            hit_test_note(midi, view, local, track_visible, track_selected)
        {
            return Some((track, start_tick, key));
        }
    }
    None
}

/// 双击空白处创建音符（第二击 release 帧触发），已处理快速删除则跳过。
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_double_click_create(
    ui: &egui::Ui,
    pointer: &egui::PointerState,
    state: &mut super::state::SelDragFrameState,
    quick_delete: &Option<(u16, u32, u8)>,
    music_rect: egui::Rect,
    content_rect: egui::Rect,
    view: &yinhe_types::PianoRollView,
    eff_rects: &[(f64, f64, u8, u8)],
    midi: Option<&dyn yinhe_types::NoteSource>,
    write_track: Option<u16>,
    track_visible: &[bool],
    conductor_idx: Option<u16>,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    default_gate: Option<u32>,
) -> Option<(yinhe_core::NoteEvent, u16)> {
    if quick_delete.is_some() {
        return None;
    }
    if !ui.input(|i| {
        i.pointer
            .button_double_clicked(egui::PointerButton::Primary)
    }) {
        return None;
    }
    if state.note_drag_origin.is_some()
        || state.sel_resize_state.is_some()
        || state.sel_note_resize.is_some()
        || state.sel_note_move.is_some()
    {
        return None;
    }
    let pos = pointer.hover_pos()?;
    if !music_rect.contains(pos) || is_on_bar(pos, music_rect, view, eff_rects) {
        return None;
    }
    let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
    let (note, track) = double_click_note(
        midi,
        write_track,
        track_visible,
        conductor_idx,
        view,
        local,
        quantize,
        ppq,
        bar_line_data,
        default_gate,
    )?;
    state.preview_reqs.push(crate::piano_view::PreviewReq::Note(
        crate::piano_view::NotePreview {
            track,
            key: note.key,
            velocity: None,
            target_tick: note.start_tick,
            duration_ticks: note.end_tick - note.start_tick,
        },
    ));
    Some((note, track))
}

/// 选框区域内是否至少有一个音符（数据层面，track 范围限定）。
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
