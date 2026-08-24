//! 钢琴卷帘交互逻辑：工具分发、hover 光标、键盘把手。
//!
//! 从 `piano_view.rs` 抽取，覆盖原 153-408 行的交互块。

use std::collections::HashSet;

use eframe::egui;

use yinhe_editor_core::audio_settings::QuickDeleteMode;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::{PianoRollView, TimeSigEvent};

use crate::widgets::tools_panel::Tool;

use super::types::{PianoViewEvent, PianoViewFeedback};

pub(crate) struct InteractionOutput {
    pub(crate) effective_tool: Tool,
    pub(crate) ghost_notes: Vec<(u32, u32, u8, u16)>,
    pub(crate) hidden_notes: HashSet<(u16, u32, u8)>,
    pub(crate) pencil_event: Option<PianoViewEvent>,
    pub(crate) eraser_event: Option<PianoViewEvent>,
    pub(crate) quick_delete_event: Option<PianoViewEvent>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch(
    ui: &mut egui::Ui,
    view: &mut PianoRollView,
    _rect: egui::Rect,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    midi: Option<&dyn yinhe_types::NoteSource>,
    selected: &mut yinhe_core::Selection,
    track_visible: &[bool],
    track_colors: &[[f32; 4]],
    cursor_tick: &mut Option<f64>,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    track_selected: &HashSet<u16>,
    write_track: Option<u16>,
    conductor_idx: Option<u16>,
    active_tool: &Tool,
    quick_delete_mode: QuickDeleteMode,
    feedback: &mut PianoViewFeedback<'_>,
    default_gate: Option<u32>,
) -> InteractionOutput {
    let mut ghost_notes: Vec<(u32, u32, u8, u16)> = Vec::new();
    let mut hidden_notes: HashSet<(u16, u32, u8)> = HashSet::new();
    let effective_tool = super::tool::effective_tool(
        ui,
        *active_tool,
        midi,
        view,
        content_rect,
        music_rect,
        track_visible,
        track_selected,
        sel_rect,
        write_track,
        conductor_idx,
    );
    let mut quick_delete_event: Option<PianoViewEvent> = None;
    let mut pencil_event: Option<PianoViewEvent> = None;
    let mut eraser_event: Option<PianoViewEvent> = None;
    if effective_tool == Tool::Select || effective_tool == Tool::SelectVertical {
        let vertical = effective_tool == Tool::SelectVertical;
        let (sel_ghosts, sel_hidden, sel_previews, sel_note_event, sel_pencil_drag, sel_quick) =
            super::drag::sel_drag_frame(
                ui,
                content_rect,
                music_rect,
                view,
                midi,
                selected,
                quantize,
                ppq,
                bar_line_data,
                total_ticks,
                cursor_tick,
                feedback.note_drag_delta,
                feedback.note_resize_delta,
                sel_rect,
                track_colors,
                track_visible,
                track_selected,
                write_track,
                conductor_idx,
                vertical,
                quick_delete_mode,
                default_gate,
            );
        ghost_notes = sel_ghosts;
        hidden_notes = sel_hidden.into_iter().collect();
        feedback.preview_reqs.extend(sel_previews);
        if let Some((track, start_tick, key)) = sel_quick {
            quick_delete_event = Some(PianoViewEvent::QuickDelete {
                track,
                start_tick,
                key,
            });
        }
        if let Some((note, track)) = sel_note_event {
            pencil_event = Some(PianoViewEvent::AddNote { track, note });
        }
        *feedback.pencil_note_drag = sel_pencil_drag;
    } else if effective_tool == Tool::Pencil {
        let (note_event, ghost, hidden, pencil_drag, preview, pencil_quick) =
            super::pencil::pencil_frame(
                ui,
                content_rect,
                music_rect,
                view,
                quantize,
                ppq,
                bar_line_data,
                write_track,
                track_visible,
                conductor_idx,
                midi,
                track_colors,
                total_ticks,
                quick_delete_mode,
                default_gate,
            );
        ghost_notes = ghost;
        hidden_notes.extend(hidden);
        *feedback.pencil_note_drag = pencil_drag;
        if let Some(p) = preview {
            feedback.preview_reqs.push(p);
        }
        if let Some((track, start_tick, key)) = pencil_quick {
            quick_delete_event = Some(PianoViewEvent::QuickDelete {
                track,
                start_tick,
                key,
            });
        }
        if let Some(note) = note_event
            && let Some(track) =
                super::pencil::valid_pencil_track(write_track, track_visible, conductor_idx)
        {
            pencil_event = Some(PianoViewEvent::AddNote { track, note });
        }
    } else if effective_tool == Tool::Eraser {
        eraser_event = super::marquee::eraser_drag_frame(
            ui,
            content_rect,
            music_rect,
            view,
            quantize,
            ppq,
            bar_line_data,
            total_ticks,
            track_selected,
        );
    }
    InteractionOutput {
        effective_tool,
        ghost_notes,
        hidden_notes,
        pencil_event,
        eraser_event,
        quick_delete_event,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_hover_cursor(
    ui: &egui::Ui,
    view: &PianoRollView,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    midi: Option<&dyn yinhe_types::NoteSource>,
    track_visible: &[bool],
    track_selected: &HashSet<u16>,
    sel_rect: &yinhe_editor_core::edit_state::SelRectState,
    write_track: Option<u16>,
    conductor_idx: Option<u16>,
    effective_tool: Tool,
) {
    if (effective_tool == Tool::Select || effective_tool == Tool::SelectVertical)
        && !crate::view_interaction::pointer_over_popup(ui.ctx())
        && let Some(pos) = ui.input(|i| i.pointer.hover_pos())
        && music_rect.contains(pos)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let eff_rects = sel_rect.effective_rects();
        let can_hover_edit =
            super::pencil::valid_pencil_track(write_track, track_visible, conductor_idx).is_some();
        let mut hit_note = false;
        let has_selection = !sel_rect.is_empty();
        if can_hover_edit
            && !has_selection
            && let Some((mode, _, _, _, _)) =
                super::drag::hit_test_note(midi, view, local, track_visible, track_selected)
        {
            use super::pencil::HitMode;
            match mode {
                HitMode::ResizeLeft => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeWest),
                HitMode::ResizeRight => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeEast),
                HitMode::Move => ui.ctx().set_cursor_icon(egui::CursorIcon::Move),
            }
            hit_note = true;
        }
        if !hit_note {
            if let Some((side, _, _)) = super::drag::hit_test_sel_edge(&eff_rects, view, local) {
                match side {
                    yinhe_editor_core::ResizeSide::Left => {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeWest)
                    }
                    yinhe_editor_core::ResizeSide::Right => {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeEast)
                    }
                }
            } else {
                let in_sel_rect = eff_rects.iter().any(|&(t_start, t_end, key_lo, key_hi)| {
                    let pixel_rect = crate::selection::drag::music_sel_to_pixel_rect(
                        view, t_start, t_end, key_lo, key_hi,
                    );
                    pixel_rect.contains(local)
                });
                if in_sel_rect {
                    let icon =
                        if effective_tool == Tool::SelectVertical || sel_rect.has_auto_vertical() {
                            egui::CursorIcon::ResizeHorizontal
                        } else {
                            egui::CursorIcon::Move
                        };
                    ui.ctx().set_cursor_icon(icon);
                }
            }
        }
    }
}

pub(crate) fn handle_kb_resize(
    ui: &mut egui::Ui,
    view: &mut PianoRollView,
    rect: egui::Rect,
    content_rect: egui::Rect,
    w: u32,
    h: u32,
) {
    ui.push_id("kb_handle", |ui| {
        let vertical = view.is_vertical();
        let content_right_x = rect.max.x - crate::widgets::scrollbar::SCROLLBAR_W;
        let handle_rect = if vertical {
            let hy = rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H - view.keyboard_width();
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x, hy - 2.0),
                egui::pos2(content_right_x, hy + 2.0),
            )
        } else {
            let handle_x = rect.min.x + view.keyboard_width();
            egui::Rect::from_min_max(
                egui::pos2(handle_x - 2.0, rect.min.y),
                egui::pos2(handle_x + 2.0, content_rect.max.y),
            )
        };
        let handle_resp = ui.interact(handle_rect, ui.id(), egui::Sense::click_and_drag());
        let on_handle = ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| handle_rect.contains(p));
        let press_on_handle = ui
            .input(|i| i.pointer.press_origin())
            .is_some_and(|p| handle_rect.contains(p));
        if on_handle && (handle_resp.hovered() || handle_resp.dragged()) {
            ui.ctx().set_cursor_icon(if vertical {
                egui::CursorIcon::ResizeVertical
            } else {
                egui::CursorIcon::ResizeHorizontal
            });
        }
        if press_on_handle && handle_resp.dragged() {
            let delta = if vertical {
                handle_resp.drag_delta().y
            } else {
                handle_resp.drag_delta().x
            };
            let old_kb = view.keyboard_width();
            let new_kb = (old_kb + delta).clamp(
                crate::theme::MIN_KEYBOARD_WIDTH,
                rect.width() * crate::theme::MAX_KEYBOARD_RATIO,
            );
            let old_main = (if vertical { h as f32 } else { w as f32 }) - old_kb;
            let new_main = (if vertical { h as f32 } else { w as f32 }) - new_kb;
            if old_main > 0.0 && new_main > 0.0 {
                let start_tick = view.main_scroll_val() / view.base.pixels_per_tick;
                let new_start_tick = start_tick * old_main / new_main;
                *view.main_scroll() = new_start_tick * view.base.pixels_per_tick;
            }
            view.base.left_panel_width = new_kb;
            view.base.dirty = true;
            ui.ctx().request_repaint();
        }
    });
}
