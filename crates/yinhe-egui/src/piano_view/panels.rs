//! 自动化面板段（从 `piano_view.rs` 336-475 行抽取）。
//!
//! 覆盖：`automation_panel::show_panels` 组装、`scroll/zoom` 回写、`toggle_buttons`。

use std::collections::HashSet;

use eframe::egui;
use yinhe_types::{AutomationLane, PianoRollView, TimeSigEvent};

use yinhe_editor_core::edit_state::SelRectState;
use yinhe_editor_core::quantize::QuantizePreset;

use crate::app::layout::SelHintInfo;
use crate::widgets::tools_panel::Tool;

use super::automation_panel;
use super::types::{AutomationPanelsCtx, PianoViewFeedback};

/// 自动化面板渲染与交互（原 `piano_view.rs` 336-475 段）。
///
/// 参数覆盖任务要求的全集：`ui, view, rect, content_rect, panels_y,
/// panels_total_h, auto_ctx, midi, track_visible, track_colors, tempo_lane,
/// sel_rect, track_selected, active_tool, write_track, conductor_idx,
/// revision, feedback, theme`。
#[allow(clippy::too_many_arguments)]
pub(crate) fn show_panels(
    ui: &mut egui::Ui,
    view: &mut PianoRollView,
    rect: egui::Rect,
    content_rect: egui::Rect,
    panels_y: f32,
    panels_total_h: f32,
    mut auto_ctx: Option<AutomationPanelsCtx<'_>>,
    midi: Option<&dyn yinhe_types::NoteSource>,
    track_visible: &[bool],
    track_colors: &[[f32; 4]],
    tempo_lane: &AutomationLane,
    sel_rect: &SelRectState,
    track_selected: &HashSet<u16>,
    active_tool: &Tool,
    write_track: Option<u16>,
    conductor_idx: Option<u16>,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    sel_hint: Option<&SelHintInfo>,
    min_border_width: f32,
    revision: u64,
    feedback: &mut PianoViewFeedback<'_>,
    theme: &yinhe_theme::GpuTheme,
    selected: &mut yinhe_core::Selection,
) -> Option<String> {
    let _ = theme;
    let _ = panels_y;
    let mut panels_status_hint: Option<String> = None;
    if let Some(ctx) = auto_ctx.as_mut() {
        let kb_w = view.keyboard_width();
        let combo_w = kb_w * crate::theme::AUTO_PANEL_COMBO_WIDTH_RATIO;
        let active_track = write_track
            .filter(|&t| track_visible.get(t as usize).copied().unwrap_or(false))
            .filter(|&t| Some(t) != conductor_idx);
        let edit_ctx = if matches!(
            *active_tool,
            Tool::Pencil | Tool::Curve | Tool::Select | Tool::SelectVertical
        ) {
            Some(automation_panel::AutomationEditCtx {
                active_tool: *active_tool,
                active_track,
                quantize,
                ppq,
                bar_line_data,
            })
        } else {
            None
        };
        let mut panels_state = automation_panel::PanelsState {
            panels: ctx.panels,
            renderers: ctx.renderers,
            wgpu_state: ctx.wgpu_state,
            show_panels: ctx.show,
        };
        let panels_data = automation_panel::PanelsData {
            automation_lanes: ctx.lanes,
            render_lanes: ctx.render_lanes,
            tempo_lane,
            midi,
            track_visible,
            track_colors,
        };
        let panels_layout = automation_panel::PanelsLayout {
            combo_width: combo_w,
            content_rect_right: rect.max.x,
            content_top_y: panels_y,
            panels_visible_h: panels_total_h,
        };
        let panels_cfg = automation_panel::PanelsCfg {
            pianoroll_scroll_x: if view.is_vertical() {
                view.base.scroll_y
            } else {
                view.base.scroll_x
            },
            pianoroll_ppt: view.base.pixels_per_tick,
            min_border_width,
            revision,
            bar_line_data,
            sel_hint,
            editing_is_conductor: write_track == conductor_idx,
        };
        let mut panels_edit = automation_panel::PanelsEdit {
            selected,
            info_content: feedback.info_content,
            right_tab: feedback.right_tab,
        };
        let (_h, auto_edits, velocity_edits, auto_feedback, auto_drag_info) =
            automation_panel::show_panels(
                ui,
                &mut panels_state,
                &panels_data,
                panels_layout,
                panels_cfg,
                &mut panels_edit,
                edit_ctx.as_ref(),
                sel_rect,
                track_selected,
            );
        panels_status_hint = auto_feedback.status_hint.clone();
        for edit in auto_edits {
            feedback.auto_edit_events.push(edit);
        }
        feedback.velocity_edits.extend(velocity_edits);
        if auto_feedback.scroll_x_delta != 0.0 {
            if view.is_vertical() {
                view.base.scroll_y -= auto_feedback.scroll_x_delta;
                view.base.scroll_y = view.base.scroll_y.max(0.0);
            } else {
                view.base.scroll_x -= auto_feedback.scroll_x_delta;
                view.base.scroll_x = view.base.scroll_x.max(0.0);
            }
            view.base.dirty = true;
        }
        if (auto_feedback.zoom_factor - 1.0).abs() > 0.001 {
            if view.is_vertical() {
                view.zoom_around_y(
                    auto_feedback.zoom_center_x,
                    auto_feedback.zoom_factor,
                    content_rect.height(),
                );
            } else {
                view.zoom_around_x(auto_feedback.zoom_center_x, auto_feedback.zoom_factor);
            }
        }
        *feedback.automation_drag_ghost = auto_drag_info;
        if midi.is_some() {
            let sb_y = rect.min.y + rect.height() - crate::widgets::scrollbar::SCROLLBAR_H;
            let sb_left_blank = egui::Rect::from_min_max(
                egui::pos2(rect.min.x, sb_y),
                egui::pos2(
                    rect.min.x + kb_w,
                    sb_y + crate::widgets::scrollbar::SCROLLBAR_H,
                ),
            );
            ui.painter()
                .rect_filled(sb_left_blank, 0.0, crate::theme::track_bg());
            ui.scope_builder(egui::UiBuilder::new().max_rect(sb_left_blank), |ui| {
                ui.horizontal_centered(|ui| {
                    let mut count = ctx.panels.len();
                    automation_panel::show_toggle_buttons(ui, ctx.show, &mut count);
                    while ctx.panels.len() < count {
                        ctx.panels.push(yinhe_types::AutomationPanelView::default());
                    }
                    while ctx.panels.len() > count {
                        ctx.panels.pop();
                    }
                });
            });
        }
    }
    panels_status_hint
}
