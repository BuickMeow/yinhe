//! 选择工具编排器：`sel_drag_frame` 为 press → 各状态机 → 双击写音符 → marquee 的总调度。

use eframe::egui;

use yinhe_editor_core::ResizeSide;
use yinhe_editor_core::audio_settings::QuickDeleteMode;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::TimeSigEvent;

use super::group_move::note_drag_frame;
use super::group_resize::sel_resize_frame;
use super::hit::{handle_double_click_create, quick_delete_early};
use super::interact::{handle_sel_marquee, on_action_bar};
use super::press::sel_press;
use super::single_move::single_note_move_frame;
use super::single_resize::single_note_resize_frame;
use super::state::SelDragFrameState;
use super::types::{SelFrameOut, SelNoteEvent};

#[allow(clippy::too_many_arguments)]
pub(crate) fn sel_drag_frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &mut yinhe_types::PianoRollView,
    midi: Option<&dyn yinhe_types::NoteSource>,
    selected: &mut yinhe_core::Selection,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    cursor_tick: &mut Option<f64>,
    note_drag_delta: &mut Option<(i64, i32, bool)>,
    note_resize_delta: &mut Option<(ResizeSide, i64)>,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    _track_colors: &[[f32; 4]],
    track_visible: &[bool],
    track_selected: &std::collections::HashSet<u16>,
    write_track: Option<u16>,
    conductor_idx: Option<u16>,
    vertical: bool,
    quick_delete_mode: QuickDeleteMode,
) -> SelFrameOut {
    let mut pencil_note_drag: Option<yinhe_types::PencilNoteDrag> = None;
    let mut state = SelDragFrameState::load(ui);
    let pointer = ui.input(|i| i.pointer.clone());
    #[cfg(target_os = "macos")]
    let additive = ui.input(|i| i.modifiers.shift || i.modifiers.command);
    #[cfg(not(target_os = "macos"))]
    let additive = ui.input(|i| i.modifiers.shift || i.modifiers.command || i.modifiers.ctrl);
    state.clear_stale(sel_rect, &pointer);
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return (Vec::new(), Vec::new(), Vec::new(), None, None, None);
    }
    let eff_rects = sel_rect.effective_rects();
    let press_on_bar = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|pos| on_action_bar(pos, music_rect, view, &eff_rects));
    let can_edit =
        crate::piano_view::pencil::valid_pencil_track(write_track, track_visible, conductor_idx)
            .is_some();
    let quick_delete = quick_delete_early(
        ui,
        &pointer,
        music_rect,
        content_rect,
        view,
        &eff_rects,
        midi,
        track_visible,
        track_selected,
        quick_delete_mode,
    );
    sel_press(
        ui,
        &mut state,
        content_rect,
        music_rect,
        view,
        midi,
        selected,
        sel_rect,
        quantize,
        ppq,
        bar_line_data,
        &eff_rects,
        track_visible,
        track_selected,
        additive,
        press_on_bar,
        &pointer,
        can_edit,
    );
    note_drag_frame(
        ui,
        &mut state,
        view,
        content_rect,
        music_rect,
        quantize,
        ppq,
        bar_line_data,
        total_ticks,
        vertical,
        sel_rect,
        note_drag_delta,
        &pointer,
    );
    sel_resize_frame(
        ui,
        &mut state,
        view,
        content_rect,
        music_rect,
        quantize,
        ppq,
        bar_line_data,
        total_ticks,
        sel_rect,
        note_resize_delta,
        &pointer,
    );
    if can_edit {
        single_note_resize_frame(
            ui,
            &mut state,
            view,
            content_rect,
            music_rect,
            quantize,
            ppq,
            bar_line_data,
            total_ticks,
            &mut pencil_note_drag,
            &pointer,
        );
        single_note_move_frame(
            ui,
            &mut state,
            view,
            content_rect,
            music_rect,
            quantize,
            ppq,
            bar_line_data,
            total_ticks,
            midi,
            selected,
            vertical,
            note_drag_delta,
            &mut pencil_note_drag,
            &pointer,
        );
    }
    let note_event: SelNoteEvent = handle_double_click_create(
        ui,
        &pointer,
        &mut state,
        &quick_delete,
        music_rect,
        content_rect,
        view,
        &eff_rects,
        midi,
        write_track,
        track_visible,
        conductor_idx,
        quantize,
        ppq,
        bar_line_data,
    );
    handle_sel_marquee(
        ui,
        &state,
        content_rect,
        music_rect,
        view,
        quantize,
        ppq,
        bar_line_data,
        total_ticks,
        cursor_tick,
        note_drag_delta,
        note_resize_delta,
        &pencil_note_drag,
        selected,
        sel_rect,
        midi,
        track_selected,
        vertical,
        press_on_bar,
        &eff_rects,
    );
    state.save(ui);
    (
        state.ghost_notes,
        state.hidden_notes,
        state.preview_reqs,
        note_event,
        pencil_note_drag,
        quick_delete,
    )
}
