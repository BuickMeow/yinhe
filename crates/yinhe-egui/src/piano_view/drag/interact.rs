use eframe::egui;

use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::TimeSigEvent;

use crate::selection::drag::{main_cross_x_y, main_px_to_tick_dir};

/// 指针是否在选框浮动工具条（selection_actions bar）上。
pub(crate) fn on_action_bar(
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

/// 简单点击（无 marquee）时的播放指示器定位。
///
/// 点在浮动工具条（selection_actions bar）上或 music_rect 外时返回 `None`——
/// 这是防穿透的关键：点击工具条按钮不能让 playhead 跳转（曾复发两次）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn cursor_tick_from_click(
    pos: egui::Pos2,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &yinhe_types::PianoRollView,
    eff_rects: &[(f64, f64, u8, u8)],
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> Option<f64> {
    if !music_rect.contains(pos) || on_action_bar(pos, music_rect, view, eff_rects) {
        return None;
    }
    let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
    let (main_px, _) = main_cross_x_y(view, (local.x, local.y));
    let tick = main_px_to_tick_dir(view, main_px);
    let snapped = crate::view_interaction::snap_tick(tick, quantize, ppq, bar_line_data);
    Some(snapped.max(0.0))
}

/// 拖拽推出屏幕时的 auto-scroll + 视口 clamp（4 个状态机共用）。
pub(crate) fn drag_scroll_and_clamp(
    ui: &mut egui::Ui,
    view: &mut yinhe_types::PianoRollView,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    total_ticks: f64,
    pos: egui::Pos2,
) {
    // auto-scroll：拖拽能推出屏幕（pos 未 clamp）。方向感知：clamp 按主轴/副轴
    // 拆分（纵向 scroll_x = 音高、scroll_y = 时间），由 view.clamp_scroll 统一处理。
    crate::selection::drag::auto_scroll_on_drag_dir(ui, view, music_rect, pos, |view, w, h| {
        view.clamp_scroll(w, h, total_ticks);
    });
    view.clamp_scroll(content_rect.width(), content_rect.height(), total_ticks);
}

/// 位置 clamp 到 music_rect（避免鼠标飞出后产生异常值）并换算 local 坐标。
pub(crate) fn clamped_local(
    pos: egui::Pos2,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
) -> (f32, f32) {
    let clamped = pos.clamp(music_rect.min, music_rect.max);
    (
        clamped.x - content_rect.min.x,
        clamped.y - content_rect.min.y,
    )
}

/// Marquee 框选与简单点击的后续处理（选择工具）。
///
/// 有 note 拖拽/缩放时仅清理 marquee 持久化状态；否则尝试 marquee 框选，
/// 无框选且为简单点击时更新 `cursor_tick`。
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_sel_marquee(
    ui: &mut egui::Ui,
    state: &super::state::SelDragFrameState,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &mut yinhe_types::PianoRollView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    cursor_tick: &mut Option<f64>,
    note_drag_delta: &Option<(i64, i32, bool)>,
    note_resize_delta: &Option<(yinhe_editor_core::ResizeSide, i64)>,
    pencil_note_drag: &Option<yinhe_types::PencilNoteDrag>,
    selected: &mut yinhe_core::Selection,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    midi: Option<&dyn yinhe_types::NoteSource>,
    track_selected: &std::collections::HashSet<u16>,
    vertical: bool,
    press_on_bar: bool,
    eff_rects: &[(f64, f64, u8, u8)],
) {
    if state.note_drag_origin.is_some()
        || state.sel_resize_state.is_some()
        || state.sel_note_resize.is_some()
        || state.sel_note_move.is_some()
    {
        let sel_id = ui.id().with("sel_drag");
        ui.data_mut(|d| {
            d.insert_persisted(sel_id, Option::<((f64, f32), egui::Pos2, egui::Pos2)>::None)
        });
        return;
    }
    let release_was_drag =
        note_drag_delta.is_some() || note_resize_delta.is_some() || pencil_note_drag.is_some();
    if let Some(result) = crate::piano_view::marquee::marquee_drag_frame(
        ui,
        content_rect,
        music_rect,
        view,
        quantize,
        ppq,
        bar_line_data,
        total_ticks,
        "sel_drag",
        press_on_bar,
    ) {
        let (track_lo, track_hi) = crate::selection::drag::pr_track_range(track_selected);
        let auto_vertical = !vertical
            && !super::hit::rect_has_notes(
                midi,
                result.t_start as u32,
                result.t_end as u32,
                result.key_lo,
                result.key_hi,
                track_lo,
                track_hi,
            );
        let (key_lo, key_hi) = if vertical || auto_vertical {
            (0, 127)
        } else {
            (result.key_lo, result.key_hi)
        };
        crate::selection::drag::add_pr_selection_rect(
            selected,
            result.t_start as u32,
            result.t_end as u32,
            key_lo,
            key_hi,
            track_selected,
        );
        sel_rect.push_rect(
            (result.t_start, result.t_end, key_lo, key_hi),
            auto_vertical,
        );
    } else if ui.input(|i| i.pointer.primary_released())
        && !release_was_drag
        && let Some(pos) = ui.input(|i| i.pointer.hover_pos())
        && let Some(tick) = cursor_tick_from_click(
            pos,
            content_rect,
            music_rect,
            view,
            eff_rects,
            quantize,
            ppq,
            bar_line_data,
        )
    {
        *cursor_tick = Some(tick);
    }
}
