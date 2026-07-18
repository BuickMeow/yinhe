//! Pencil-tool input handling for the piano-roll view.

use eframe::egui;

use yinhe_types::{key_notes_in_range, TimeSigEvent};
use yinhe_editor_core::quantize::QuantizePreset;
use super::PencilNoteDrag;

/// Internal pencil-tool drag mode persisted across frames.
#[derive(Clone)]
pub(crate) enum PencilDrag {
    /// Creating a new note: (start_tick, key)
    Create(f64, u8),
    /// Moving an existing note: (track, original_start_tick, original_key, original_end, press_snapped_tick)
    Move(u16, u32, u8, u32, f64),
    /// Resizing right edge: (track, start_tick, end_tick, key)
    ResizeRight(u16, u32, u32, u8),
    /// Resizing left edge: (track, start_tick, end_tick, key)
    ResizeLeft(u16, u32, u32, u8),
}

/// Result of hit-testing the cursor against existing notes.
pub(crate) struct HitNote {
    pub track: u16,
    pub start_tick: u32,
    pub end_tick: u32,
    pub key: u8,
    pub mode: HitMode,
}

#[derive(Clone)]
pub(crate) enum HitMode {
    Move,
    ResizeLeft,
    ResizeRight,
}

/// Returns the single valid target track for the Pencil tool, if any.
pub(crate) fn valid_pencil_track(
    track_selected: &std::collections::HashSet<u16>,
    conductor_idx: Option<u16>,
) -> Option<u16> {
    if track_selected.len() != 1 {
        return None;
    }
    let &track = track_selected.iter().next()?;
    if Some(track) == conductor_idx {
        return None;
    }
    Some(track)
}

/// Pencil-tool input handling: hover preview, click to write a note, drag to lengthen,
/// or hover over / drag existing notes to move or resize them.
/// Returns `(note_event, ghost_notes, hidden_notes, pencil_note_drag)`.
/// ghost_notes are (start_tick, end_tick, key, track) — color fetched from uniform in shader.
/// hidden_notes are (track, start_tick, key) for notes being dragged.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pencil_frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &mut yinhe_types::PianoRollView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    track_selected: &std::collections::HashSet<u16>,
    conductor_idx: Option<u16>,
    midi: Option<&dyn yinhe_types::NoteSource>,
    _track_colors: &[[f32; 3]],
) -> (Option<yinhe_core::NoteEvent>, Vec<(f64, f64, u8, u16)>, Vec<(u16, u32, u8)>, Option<PencilNoteDrag>) {
    let pencil_id = ui.id().with("pencil_drag");
    let drag_state: Option<PencilDrag> =
        ui.data_mut(|d| d.get_persisted(pencil_id)).unwrap_or(None);

    let pointer = ui.input(|i| i.pointer.clone());

    // Clear stale drag state.
    if drag_state.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
    }

    // 弹窗打开时跳过所有 pointer 处理，避免点击穿透
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return (None, Vec::new(), Vec::new(), None);
    }

    let hover_pos = pointer.hover_pos();
    let can_write = valid_pencil_track(track_selected, conductor_idx).is_some();
    let track = valid_pencil_track(track_selected, conductor_idx);
    let track_idx = track.unwrap_or(0);

    // Hover / drag preview.
    let preview = if let Some(pos) = hover_pos {
        if music_rect.contains(pos) {
            let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
            let raw_tick = view.x_to_tick(local.x);
            let tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
            let key = view.y_to_key(local.y);
            Some((tick.max(0.0), key))
        } else {
            None
        }
    } else {
        None
    };

    // ── Hit-test existing notes (only when not dragging) ──
    // Returns the closest note under cursor with its hit mode.
    // This is independent of `preview` / `snap_tick` so that clicking
    // on a note always starts a drag, never accidentally creates a new note.
    const EDGE_THRESHOLD_PX: f32 = 6.0;
    let kb_w = music_rect.min.x - content_rect.min.x;

    let hit_note = if drag_state.is_none() && can_write {
        // Use a closure so `?` returns from the closure, not from pencil_frame
        (|| -> Option<HitNote> {
            let mouse_screen = hover_pos?;
            if !music_rect.contains(mouse_screen) {
                return None;
            }
            let mouse_local_x = mouse_screen.x - music_rect.min.x;
            let mouse_local_y = mouse_screen.y - music_rect.min.y;
            let key = view.y_to_key(mouse_local_y);
            let midi = midi?;
            let notes = key_notes_in_range(midi.key_notes(key), 0, u32::MAX);

            for note in notes {
                let note_left = view.tick_to_x(note.start_tick as f64) - kb_w;
                let note_right = view.tick_to_x(note.end_tick as f64) - kb_w;
                let note_top = view.key_to_y(key);
                let note_bottom = note_top + view.key_height;

                if mouse_local_x >= note_left && mouse_local_x <= note_right
                    && mouse_local_y >= note_top && mouse_local_y <= note_bottom
                {
                    let dist_left = (mouse_local_x - note_left).abs();
                    let dist_right = (mouse_local_x - note_right).abs();
                    let mode = if dist_left < EDGE_THRESHOLD_PX {
                        HitMode::ResizeLeft
                    } else if dist_right < EDGE_THRESHOLD_PX {
                        HitMode::ResizeRight
                    } else {
                        HitMode::Move
                    };
                    return Some(HitNote {
                        track: note.track,
                        start_tick: note.start_tick,
                        end_tick: note.end_tick,
                        key,
                        mode,
                    });
                }
            }
            None
        })()
    } else {
        None
    };

    // ── Set cursor based on hit test ──
    if let Some(ref hit) = hit_note {
        match hit.mode {
            HitMode::ResizeLeft => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeWest),
            HitMode::ResizeRight => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeEast),
            HitMode::Move => ui.ctx().set_cursor_icon(egui::CursorIcon::Move),
        }
    }

    // ── Ghost notes: only when not over an existing note ──
    let mut ghost_notes: Vec<(f64, f64, u8, u16)> = Vec::new();
    let mut hidden_notes: Vec<(u16, u32, u8)> = Vec::new();
    if can_write && drag_state.is_none() && hit_note.is_none() {
        if let Some((tick, key)) = preview {
            let interval = quantize.tick_interval(ppq) as f64;
            // Not dragging (drag_state is None due to the outer condition),
            // show preview at hover position
            ghost_notes.push((tick, tick + interval, key, track_idx));
        }
    }

    // ── Start drag ──
    if pointer.primary_pressed() {
        if let Some(hit) = hit_note {
            let new_drag = match hit.mode {
                HitMode::ResizeLeft => PencilDrag::ResizeLeft(hit.track, hit.start_tick, hit.end_tick, hit.key),
                HitMode::ResizeRight => PencilDrag::ResizeRight(hit.track, hit.start_tick, hit.end_tick, hit.key),
                HitMode::Move => {
                    let press_tick = preview.map(|(t, _)| t).unwrap_or(0.0);
                    PencilDrag::Move(hit.track, hit.start_tick, hit.key, hit.end_tick, press_tick)
                }
            };
            ui.data_mut(|d| d.insert_persisted(pencil_id, Some(new_drag)));
        } else if let Some((tick, key)) = preview {
            ui.data_mut(|d| d.insert_persisted(pencil_id, Some(PencilDrag::Create(tick, key))));
        }
    }

    // ── Compute drag output ──
    let mut result = None;
    let mut pencil_note_drag = None;

    match &drag_state {
        Some(PencilDrag::Create(s_tick, s_key)) => {
            // Show ghost while dragging (before release)
            if pointer.primary_down() && !pointer.primary_released() {
                if let Some((tick, _)) = preview {
                    let interval = quantize.tick_interval(ppq) as f64;
                    let current_end = tick.max(*s_tick + interval);
                    ghost_notes.push((*s_tick, current_end, *s_key, track_idx));
                }
            }
            // Release -> commit note.
            if pointer.primary_released() {
                if can_write {
                    let interval = quantize.tick_interval(ppq) as f64;
                    let end_tick = if let Some((tick, _)) = preview {
                        let current_end = tick.max(*s_tick + interval);
                        let snapped_end = crate::view_interaction::snap_tick_ceil(
                            current_end,
                            quantize,
                            ppq,
                            bar_line_data,
                        );
                        snapped_end.max(*s_tick + interval)
                    } else {
                        *s_tick + interval
                    };
                    result = Some(yinhe_core::NoteEvent {
                        id: 0, // 由 Document::add_note 分配
                        start_tick: *s_tick as u32,
                        end_tick: end_tick as u32,
                        key: *s_key,
                        velocity: 100,
                    });
                }
                ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
            }
        }
        Some(PencilDrag::Move(trk, orig_tick, orig_key, orig_end, press_tick)) => {
            if let Some((tick, key)) = preview {
                let dt = (tick as i64) - (*press_tick as i64);
                let dk = (key as i32) - (*orig_key as i32);

                // Show ghost at the dragged position for visual feedback.
                // The original note stays in place until release.
                let new_start = (*orig_tick as i64 + dt).max(0) as u32;
                let new_end = new_start + (*orig_end - *orig_tick);
                ghost_notes.push((new_start as f64, new_end as f64, key, *trk));
                hidden_notes.push((*trk, *orig_tick, *orig_key));

                // Only output drag on release — do NOT modify the model during drag.
                if pointer.primary_released() {
                    pencil_note_drag = Some(PencilNoteDrag::Move {
                        track: *trk,
                        start_tick: *orig_tick,
                        key: *orig_key,
                        delta_ticks: dt,
                        delta_keys: dk,
                    });
                    ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
                }
            } else {
                if pointer.primary_released() {
                    ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
                }
            }
        }
        Some(PencilDrag::ResizeRight(trk, orig_tick, _orig_end, orig_key)) => {
            if let Some((tick, _)) = preview {
                let interval = quantize.tick_interval(ppq) as f64;
                let snapped = crate::view_interaction::snap_tick_ceil(
                    tick.max(*orig_tick as f64 + interval),
                    quantize,
                    ppq,
                    bar_line_data,
                );
                let new_end = snapped.max(*orig_tick as f64 + interval).min(u32::MAX as f64) as u32;

                // Show ghost and hide original note
                ghost_notes.push((*orig_tick as f64, new_end as f64, *orig_key, *trk));
                hidden_notes.push((*trk, *orig_tick, *orig_key));

                // Only output on release
                if pointer.primary_released() {
                    pencil_note_drag = Some(PencilNoteDrag::ResizeRight {
                        track: *trk,
                        start_tick: *orig_tick,
                        key: *orig_key,
                        new_end_tick: new_end,
                    });
                    ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
                }
            } else {
                if pointer.primary_released() {
                    ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
                }
            }
        }
        Some(PencilDrag::ResizeLeft(trk, orig_tick, orig_end, orig_key)) => {
            if let Some((tick, _)) = preview {
                let interval = quantize.tick_interval(ppq) as f64;
                let snapped = crate::view_interaction::snap_tick_floor(
                    tick,
                    quantize,
                    ppq,
                    bar_line_data,
                );
                let new_start = (snapped as u32).min(*orig_end - 1);
                // Ensure minimum length: new_start must be <= orig_end - interval
                let max_start = (*orig_end as f64 - interval).max(0.0) as u32;
                let new_start = new_start.min(max_start);

                // Show ghost and hide original note
                ghost_notes.push((new_start as f64, *orig_end as f64, *orig_key, *trk));
                hidden_notes.push((*trk, *orig_tick, *orig_key));

                // Only output on release
                if pointer.primary_released() {
                    pencil_note_drag = Some(PencilNoteDrag::ResizeLeft {
                        track: *trk,
                        start_tick: *orig_tick,
                        key: *orig_key,
                        new_start_tick: new_start,
                    });
                    ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
                }
            } else {
                if pointer.primary_released() {
                    ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
                }
            }
        }
        None => {}
    }

    (result, ghost_notes, hidden_notes, pencil_note_drag)
}
