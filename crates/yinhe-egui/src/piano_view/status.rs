//! 状态栏与性能探针段（从 `piano_view.rs` 492-553 行抽取）。
//!
//! 覆盖：`perf::submit` 与 `feedback.status_hint` 悬停提示（`panels_status_hint` 优先）。

use std::time::Instant;

use eframe::egui;
use rust_i18n::t;
use yinhe_types::TimeSigEvent;
use yinhe_types::time_format::format_tick_bar_beat_with_time_sig;

use yinhe_editor_core::edit_state::SelRectState;

use crate::app::layout::SelHintInfo;
use crate::view_interaction::FollowMode;

use super::perf;
use super::types::PianoViewFeedback;

/// 更新性能探针与状态栏悬停提示（原 `piano_view.rs` 492-553 段）。
///
/// 参数覆盖任务要求的全集：`perf::submit` 时间戳、`panels_status_hint`、`view`、
/// `midi`、`rect/music_rect/content_rect/keyboard_rect`、`sel_hint/sel_rect`、
/// `bar_line_data`、`feedback` 等。
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_status(
    ui: &egui::Ui,
    rect: egui::Rect,
    music_rect: egui::Rect,
    content_rect: egui::Rect,
    view: &yinhe_types::PianoRollView,
    midi: Option<&dyn yinhe_types::NoteSource>,
    follow_mode: &FollowMode,
    t_show_start: Option<Instant>,
    t_input_end: Option<Instant>,
    t_prepare_end: Option<Instant>,
    t_paint_end: Option<Instant>,
    w: f32,
    panels_status_hint: Option<String>,
    sel_hint: Option<&SelHintInfo>,
    sel_rect: &SelRectState,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    feedback: &mut PianoViewFeedback<'_>,
) {
    perf::submit(perf::PerfCtx {
        t_show_start,
        t_input_end,
        t_prepare_end,
        t_paint_end,
        follow_mode,
        midi,
        view,
        width: w,
    });
    if let Some(pos) = ui.input(|i| i.pointer.hover_pos())
        && rect.contains(pos)
    {
        let hint = if let Some(h) = panels_status_hint {
            Some(h)
        } else if music_rect.contains(pos) {
            let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
            let (main_px, cross_px) =
                crate::selection::drag::main_cross_x_y(view, (local.x, local.y));
            let tick = crate::selection::drag::main_px_to_tick_dir(view, main_px).max(0.0);
            let key = view.cross_px_to_key(cross_px);
            let sel_text = if !sel_rect.effective_rects().is_empty()
                && let Some(sh) = sel_hint
            {
                Some(t!("hint.sel_notes", n = sh.count, span = &sh.span).to_string())
            } else {
                None
            };
            if let Some(s) = sel_text {
                Some(s)
            } else {
                let pos_str = match bar_line_data {
                    Some((ppq, num, den, events)) => {
                        format_tick_bar_beat_with_time_sig(tick, ppq, events, num, den)
                    }
                    None => format!("{}", tick as u32),
                };
                Some(format!("{} {}", pos_str, key))
            }
        } else {
            let kb_w = view.keyboard_width();
            let content_right_x = rect.max.x - crate::widgets::scrollbar::SCROLLBAR_W;
            let kb_bottom = rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H;
            let keyboard_rect = if view.is_vertical() {
                egui::Rect::from_min_max(
                    egui::pos2(content_rect.min.x, kb_bottom - kb_w),
                    egui::pos2(content_right_x, kb_bottom),
                )
            } else {
                egui::Rect::from_min_max(
                    egui::pos2(content_rect.min.x, content_rect.min.y),
                    egui::pos2(content_rect.min.x + kb_w, content_rect.max.y),
                )
            };
            if if view.is_vertical() {
                keyboard_rect.contains(pos)
            } else {
                content_rect.contains(pos)
            } {
                let key = if view.is_vertical() {
                    view.cross_px_to_key(pos.x - content_rect.min.x)
                } else {
                    view.y_to_key(pos.y - content_rect.min.y)
                };
                Some(format!("{}", key))
            } else {
                None
            }
        };
        *feedback.status_hint = hint;
    }
}
