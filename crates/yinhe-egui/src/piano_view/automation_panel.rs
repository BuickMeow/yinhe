use eframe::egui;
use rust_i18n::t;

pub use yinhe_types::AutomationEdit;
use yinhe_types::time_format::format_tick_bar_beat_with_time_sig;
use yinhe_types::{AutomationLane, AutomationTarget};

use yinhe_types::AutomationPanelView;

use crate::right_panel::{InfoContent, RightTab};
use crate::widgets::tools_panel::Tool;

pub(crate) mod interaction;
mod velocity;

mod constants;
mod layout;
mod render;
mod scroll_zoom;
mod types;
mod value;
mod widgets;

pub use constants::*;
pub use types::*;
use value::{min_value_zoom, panel_max_val, value_upper_bound};

pub use widgets::show_toggle_buttons;

/// Render all automation panels between the pianoroll content and the scrollbar.
///
/// The first panel sits flush against the content above. Each subsequent panel
/// has a `SPLIT_H` drag handle at its top edge.
///
/// Returns the total height consumed by all panels (including split handles
/// between them, but no leading handle for the first panel).
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub fn show_panels(
    ui: &mut egui::Ui,
    state: &mut PanelsState<'_>,
    data: &PanelsData<'_>,
    lay: PanelsLayout,
    cfg: PanelsCfg<'_>,
    edit: &mut PanelsEdit<'_>,
    edit_ctx: Option<&AutomationEditCtx<'_>>,
    pr_sel_rect: &yinhe_editor_core::edit_state::SelRectState,
    pr_track_selected: &std::collections::HashSet<u16>,
) -> PanelsOutput {
    let mut edits = Vec::new();
    let mut velocity_edits = Vec::new();
    let mut feedback = PanelPianorollFeedback::default();
    let mut all_drag_info: Option<(u32, f32)> = None;
    if !*state.show_panels || state.panels.is_empty() {
        return (0.0, edits, velocity_edits, feedback, None);
    }

    let active_tool = edit_ctx.map(|c| c.active_tool).unwrap_or(Tool::Select);
    let show_anchors = matches!(
        active_tool,
        Tool::Pencil | Tool::Curve | Tool::Select | Tool::SelectVertical
    );

    for panel in state.panels.iter_mut() {
        panel.sync_from_pianoroll(cfg.pianoroll_scroll_x, cfg.pianoroll_ppt, lay.combo_width);
    }

    layout::sync_renderer_count(state.renderers, state.panels, state.wgpu_state, 640, 200);

    let frame = layout::begin_frame(ui, state.panels, lay);
    let orig_heights = frame.orig_heights;
    let max_scroll = frame.max_scroll;
    let panels_area_rect = frame.panels_area_rect;
    let old_clip = frame.old_clip;
    let mut y_offset = frame.y_offset;
    let visible_top = frame.visible_top;
    let visible_bottom = frame.visible_bottom;

    for (i, panel) in state.panels.iter_mut().enumerate() {
        let handle_rect = egui::Rect::from_min_max(
            egui::pos2(0.0, y_offset),
            egui::pos2(lay.content_rect_right, y_offset + SPLIT_H),
        );
        widgets::handle_split_drag(ui, panel, handle_rect, i);
        y_offset += SPLIT_H;

        let panel_h = orig_heights[i];
        let panel_top = y_offset;
        let panel_bottom = y_offset + panel_h;
        let panel_right = lay.content_rect_right - crate::widgets::scrollbar::SCROLLBAR_W;
        let panel_rect = egui::Rect::from_min_max(
            egui::pos2(0.0, panel_top),
            egui::pos2(panel_right, panel_bottom),
        );

        let is_visible = panel_bottom >= visible_top && panel_top <= visible_bottom;
        if !is_visible {
            y_offset += panel_h;
            continue;
        }

        let grid_rect = egui::Rect::from_min_max(panel_rect.min, panel_rect.max);

        let ppp = ui.ctx().pixels_per_point();
        let gw = grid_rect.width() as u32;
        let gh = grid_rect.height() as u32;
        let gpw = (gw as f32 * ppp) as u32;
        let gph = (gh as f32 * ppp) as u32;

        let grid_area = egui::Rect::from_min_max(
            egui::pos2(panel_rect.min.x + lay.combo_width, panel_rect.min.y),
            egui::pos2(panel_rect.max.x, panel_rect.max.y),
        );
        let combo_area = egui::Rect::from_min_max(
            panel_rect.min,
            egui::pos2(panel_rect.min.x + lay.combo_width, panel_rect.max.y),
        );
        let upper_bound = value_upper_bound(panel);
        let mut max_val_f = panel_max_val(panel, data.tempo_lane);
        let zoom_min = min_value_zoom(max_val_f, upper_bound);

        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            if grid_area.contains(pos) {
                let x_in_grid = pos.x - grid_area.min.x;
                let raw_tick =
                    ((x_in_grid + panel.base.scroll_x) / panel.base.pixels_per_tick).max(0.0);
                let y_in_panel = (pos.y - panel_rect.min.y).clamp(0.0, panel_rect.height());
                let value = panel
                    .y_to_value(y_in_panel, max_val_f)
                    .clamp(0.0, max_val_f);
                let pos_str = match cfg.bar_line_data {
                    Some((ppq, num, den, events)) => {
                        format_tick_bar_beat_with_time_sig(raw_tick as f64, ppq, events, num, den)
                    }
                    None => format!("{}", raw_tick as u32),
                };
                let val_str = if panel.show_velocity {
                    format!("{}", value.round() as i32)
                } else if panel.selected_target == AutomationTarget::Tempo {
                    format!("{:.2} BPM", value)
                } else {
                    format!("{}", value.round() as i32)
                };
                let sel_text = if !panel.anchor_sel_rects.is_empty()
                    && let Some(sh) = cfg.sel_hint
                {
                    Some(t!("hint.sel_events", n = sh.count, span = &sh.span).to_string())
                } else {
                    None
                };
                feedback.status_hint = Some(if let Some(s) = sel_text {
                    s
                } else {
                    format!("{} {}", pos_str, val_str)
                });
            } else if panel_rect.contains(pos) {
                feedback.status_hint = None;
            }
        }
        scroll_zoom::handle_panel_scroll_zoom(
            ui,
            panel,
            grid_area,
            combo_area,
            panel_rect,
            max_val_f,
            zoom_min,
            max_scroll,
            &mut feedback,
        );

        let out = dispatch_edit_interaction(
            ui,
            grid_area,
            panel_rect,
            panel,
            data.automation_lanes,
            data.tempo_lane,
            data.midi,
            edit_ctx,
            i,
            data.track_colors,
            edit.info_content,
            edit.right_tab,
            pr_sel_rect,
            pr_track_selected,
        );
        edits.extend(out.automation_edits);
        velocity_edits.extend(out.velocity_edits);
        if out.anchor_drag.is_some() {
            all_drag_info = out.anchor_drag;
        }
        if let Some((_, v)) = out.anchor_drag {
            max_val_f = max_val_f.max(v);
        }
        let panel_ghost = out.ghost;
        let velocity_preview = out.preview;
        let marquee_rect = out.marquee_rect;
        if let Some(op) = out.sel_op {
            use interaction::{SelOp, SelRectOp};
            match op {
                SelOp::Set(rect_op) => {
                    match rect_op {
                        SelRectOp::Set(r) => panel.anchor_sel_rects = vec![r],
                        SelRectOp::Append(r) => panel.anchor_sel_rects.push(r),
                        SelRectOp::ReplaceAll(rects) => panel.anchor_sel_rects = rects,
                        SelRectOp::Keep => {}
                    }
                    panel.dirty = true;
                }
                SelOp::Clear => {
                    panel.anchor_sel_rects.clear();
                    panel.dirty = true;
                }
                SelOp::ClearNoteSelection => {
                    edit.selected.clear();
                }
            }
        }

        if gw > 0
            && gh > 0
            && let Some((renderer, render_ctx)) = state.renderers.get_mut(i)
        {
            render::render_panel_content(
                ui,
                renderer,
                render_ctx,
                panel,
                grid_rect,
                gpw,
                gph,
                data.render_lanes,
                data.tempo_lane,
                data.midi,
                data.track_visible,
                data.track_colors,
                cfg.scroll_mode,
                cfg.min_border_width,
                show_anchors,
                max_val_f,
                panel_ghost,
                cfg.revision,
                edit.info_content,
                i,
                lay.combo_width,
            );
            render::draw_panel_overlay(
                ui,
                panel,
                panel_rect,
                grid_area,
                max_val_f,
                i,
                &PanelOverlayData {
                    marquee_rect,
                    velocity_preview,
                },
            );
        }

        let vsb_rect = egui::Rect::from_min_max(
            egui::pos2(panel_right, panel_top),
            egui::pos2(lay.content_rect_right, panel_bottom),
        );
        ui.push_id(format!("auto_vscroll_{}", i), |ui| {
            crate::widgets::scrollbar::show_vertical_value(
                ui,
                vsb_rect,
                panel.panel_height,
                &mut panel.value_scroll,
                &mut panel.value_zoom,
                upper_bound,
                zoom_min,
                8.0,
                &mut panel.dirty,
            );
        });

        let combo_rect = egui::Rect::from_min_max(
            panel_rect.min,
            egui::pos2(panel_rect.min.x + lay.combo_width, panel_rect.max.y),
        );
        widgets::show_target_combo(
            ui,
            panel,
            combo_rect,
            panels_area_rect,
            cfg.editing_is_conductor,
        );

        render::draw_value_labels(ui, panel, panel_rect, lay.combo_width, max_val_f);

        y_offset += panel_h;
    }

    ui.set_clip_rect(old_clip);

    widgets::apply_right_click_anchor(
        ui,
        state.panels.len(),
        data.automation_lanes,
        edit.info_content,
        edit.right_tab,
    );

    (
        lay.panels_visible_h,
        edits,
        velocity_edits,
        feedback,
        all_drag_info,
    )
}

/// 按面板模式分派编辑交互：Tempo / CC / PB / RPN / NRPN 走 lane 编辑；
/// Velocity 走铅笔笔划（改音符力度）。同时负责绘制 hover/drag tooltip。
#[allow(clippy::too_many_arguments)]
fn dispatch_edit_interaction(
    ui: &mut egui::Ui,
    grid_area: egui::Rect,
    panel_rect: egui::Rect,
    panel: &mut AutomationPanelView,
    automation_lanes: &[AutomationLane],
    tempo_lane: &AutomationLane,
    midi: Option<&dyn yinhe_types::NoteSource>,
    edit_ctx: Option<&AutomationEditCtx<'_>>,
    panel_index: usize,
    track_colors: &[[f32; 4]],
    info_content: &mut Option<InfoContent>,
    right_tab: &mut Option<RightTab>,
    pr_sel_rect: &yinhe_editor_core::edit_state::SelRectState,
    pr_track_selected: &std::collections::HashSet<u16>,
) -> PanelInteractionOut {
    let mut out = PanelInteractionOut {
        automation_edits: Vec::new(),
        velocity_edits: Vec::new(),
        ghost: None,
        preview: None,
        anchor_drag: None,
        marquee_rect: None,
        sel_op: None,
    };
    let mut tooltip: Option<interaction::HoverTooltip> = None;
    if let Some(ctx) = edit_ctx {
        if panel.show_velocity {
            // Velocity：铅笔/选框笔划修改力度条（命中 noteon，只作用于 active_track）
            if matches!(
                ctx.active_tool,
                Tool::Pencil | Tool::Select | Tool::SelectVertical
            ) && let Some(track) = ctx.active_track
                && let Some(midi_src) = midi
            {
                let track_color = track_colors
                    .get(track as usize)
                    .copied()
                    .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR);
                let (vel_edits, preview, tip) = velocity::handle_velocity_interaction(
                    ui,
                    grid_area,
                    panel_rect,
                    panel,
                    midi_src,
                    track,
                    track_color,
                    panel_index,
                    pr_sel_rect,
                    pr_track_selected,
                );
                out.velocity_edits = vel_edits;
                out.preview = preview;
                tooltip = tip.map(|(tick, value, pos)| interaction::HoverTooltip::Anchor {
                    tick,
                    value,
                    pos,
                });
            }
        } else if panel.selected_target == AutomationTarget::Tempo {
            // Tempo 不依赖 active_track：无论编辑目标是哪个轨道（甚至没有编辑目标）
            // 都可编辑。document 层忽略 track_idx，直接操作 conductor.tempo，
            // 所以这里传 0。非 Tempo 事件绝不能落进 Conductor（曾导致弯音写入别的轨道）。
            let (panel_edits, ghost, drag_info, hover_info, marquee_rect, sel_op) =
                interaction::handle_automation_interaction(
                    ui,
                    grid_area,
                    panel_rect,
                    panel,
                    automation_lanes,
                    Some(tempo_lane),
                    0,
                    ctx,
                    ui.id().with(panel_index),
                    track_colors,
                    info_content,
                    right_tab,
                );
            out.automation_edits = panel_edits;
            out.ghost = ghost;
            out.marquee_rect = marquee_rect;
            out.sel_op = sel_op;
            // anchor_drag 只跟锚点拖拽（InfoPanel 用它显示实时 tick/value）
            if let Some(interaction::HoverTooltip::Anchor { tick, value, .. }) = drag_info {
                out.anchor_drag = Some((tick, value));
            }
            tooltip = drag_info.or(hover_info);
        } else if let Some(track) = ctx.active_track {
            let (panel_edits, ghost, drag_info, hover_info, marquee_rect, sel_op) =
                interaction::handle_automation_interaction(
                    ui,
                    grid_area,
                    panel_rect,
                    panel,
                    automation_lanes,
                    Some(tempo_lane),
                    track,
                    ctx,
                    ui.id().with(panel_index),
                    track_colors,
                    info_content,
                    right_tab,
                );
            out.automation_edits = panel_edits;
            out.ghost = ghost;
            out.marquee_rect = marquee_rect;
            out.sel_op = sel_op;
            // anchor_drag 只跟锚点拖拽（InfoPanel 用它显示实时 tick/value）
            if let Some(interaction::HoverTooltip::Anchor { tick, value, .. }) = drag_info {
                out.anchor_drag = Some((tick, value));
            }
            tooltip = drag_info.or(hover_info);
        }
    }

    // tooltip：拖拽中显示 drag_info，否则 hover 锚点/控制点超时显示 hover_info。
    if let (Some(tip), Some(ctx)) = (tooltip, edit_ctx) {
        let (lines, x, y): (Vec<String>, f32, f32) = match tip {
            interaction::HoverTooltip::Anchor { tick, value, pos } => {
                let pos_str = if let Some((ppq, num, den, ts_events)) = ctx.bar_line_data {
                    format_tick_bar_beat_with_time_sig(tick as f64, ppq, ts_events, num, den)
                } else {
                    format!("{}", tick)
                };
                let val_str = if panel.show_velocity {
                    format!("{}", value.round() as i32)
                } else if panel.selected_target == AutomationTarget::Tempo {
                    format!("{:.2} BPM", value)
                } else {
                    format!("{:.2}", value)
                };
                (vec![pos_str, val_str], pos.x, pos.y)
            }
            interaction::HoverTooltip::ControlPoint {
                x1,
                y1,
                x2,
                y2,
                pos,
            } => (
                vec![
                    format!("X1: {:.2}", x1),
                    format!("Y1: {:.2}", y1),
                    format!("X2: {:.2}", x2),
                    format!("Y2: {:.2}", y2),
                ],
                pos.x,
                pos.y,
            ),
        };
        crate::view_interaction::draw_hover_tooltip(ui.ctx(), &lines, x, y);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_editor_core::quantize::QuantizePreset;
    use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

    /// 测试用面板：Tempo target、1px/tick、面板高 80px、无滚动、无缩放。
    fn tempo_panel() -> AutomationPanelView {
        AutomationPanelView {
            selected_target: AutomationTarget::Tempo,
            show_velocity: false,
            panel_height: 80.0,
            value_zoom: 1.0,
            value_scroll: 0.0,
            base: yinhe_types::TimelineViewBase {
                pixels_per_tick: 1.0,
                scroll_x: 0.0,
                scroll_y: 0.0,
                left_panel_width: 0.0,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
                follow_target: None,
            },
            ..Default::default()
        }
    }

    fn tempo_lane(events: Vec<(u32, f32)>) -> AutomationLane {
        AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: events
                .into_iter()
                .map(|(tick, value)| AutomationEvent {
                    tick,
                    value,
                    shape: SegmentShape::Step,
                })
                .collect(),
        }
    }

    fn panel_rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 80.0))
    }

    fn press_event(pos: egui::Pos2) -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(pos));
        raw.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        });
        raw
    }

    fn drag_event(pos: egui::Pos2) -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(pos));
        raw
    }

    fn release_event(pos: egui::Pos2) -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(pos));
        raw.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        });
        raw
    }

    /// 跑一帧 dispatch_edit_interaction（Tempo 面板），返回 automation_edits。
    fn run_dispatch(
        ctx: &egui::Context,
        raw: egui::RawInput,
        panel: &mut AutomationPanelView,
        lane: &AutomationLane,
        active_track: Option<u16>,
    ) -> Vec<AutomationEdit> {
        let mut edits = Vec::new();
        ctx.run_ui(raw, |ui| {
            let edit_ctx = AutomationEditCtx {
                active_tool: Tool::Pencil,
                active_track,
                // 1/16 音符网格：interval = 480*4/16 = 120 tick。
                quantize: QuantizePreset::Fraction(1, 16),
                ppq: 480,
                bar_line_data: None,
            };
            let mut info: Option<InfoContent> = None;
            let mut right_tab: Option<RightTab> = None;
            let pr_sel = yinhe_editor_core::edit_state::SelRectState::default();
            let pr_tracks = std::collections::HashSet::new();
            edits = dispatch_edit_interaction(
                ui,
                panel_rect(),
                panel_rect(),
                panel,
                &[],
                lane,
                None,
                Some(&edit_ctx),
                0,
                &[[0.8, 0.8, 0.8, 1.0]],
                &mut info,
                &mut right_tab,
                &pr_sel,
                &pr_tracks,
            )
            .automation_edits;
        })
        .textures_delta
        .clear();
        edits
    }

    /// 回归测试：Tempo 编辑不依赖 active_track——没有任何写入目标轨
    /// （write_track=None）时 Tempo 锚点依然可拖拽（Conductor 下编辑的前提）。
    #[test]
    fn tempo_editable_without_active_track() {
        let ctx = egui::Context::default();
        let mut panel = tempo_panel();
        let lane = tempo_lane(vec![(0, 120.0)]);
        // 锚点 (tick=0, value=120) 位于面板顶部。
        let anchor = egui::pos2(0.0, 0.0);
        // 拖到面板上方 20px、tick 120（1/16 音符量化点）。
        let above = egui::pos2(120.0, -20.0);

        let _ = run_dispatch(&ctx, press_event(anchor), &mut panel, &lane, None);
        let _ = run_dispatch(&ctx, drag_event(above), &mut panel, &lane, None);
        let edits = run_dispatch(&ctx, release_event(above), &mut panel, &lane, None);

        let move_edit = edits
            .iter()
            .find(|e| matches!(e, AutomationEdit::Move { .. }))
            .expect("无 active_track 时 Tempo 拖拽也应提交 Move");
        match move_edit {
            AutomationEdit::Move { new_value, .. } => {
                assert_eq!(*new_value, 150.0, "BPM 应突破显示上限 120");
            }
            _ => unreachable!(),
        }
    }
}
