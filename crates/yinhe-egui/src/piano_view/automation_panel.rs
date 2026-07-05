use std::sync::Arc;

use eframe::egui;
use egui_material_icons::icons::*;

use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::{AutomationLane, AutomationTarget, SegmentShape, TimeSigEvent};

use yinhe_automation::{AutomationGhost, AutomationPanelView, prepare_automation};
use yinhe_wgpu::InstanceRenderer;

use crate::widgets::tools_panel::Tool;

/// Curated list of known automation targets shown in the dropdown.
const AUTOMATION_TARGETS: &[AutomationTarget] = &[
    AutomationTarget::PitchBend,
    AutomationTarget::CC { controller: 7 },  // Volume
    AutomationTarget::CC { controller: 10 }, // Pan
    AutomationTarget::CC { controller: 11 }, // Expression
    AutomationTarget::CC { controller: 64 }, // Sustain
    AutomationTarget::CC { controller: 71 }, // Resonance
    AutomationTarget::CC { controller: 72 }, // Release
    AutomationTarget::CC { controller: 73 }, // Attack
    AutomationTarget::CC { controller: 74 }, // Cutoff
    AutomationTarget::Rpn { parameter: 0 },  // PB Sensitivity
    AutomationTarget::Rpn { parameter: 1 },  // Fine Tune
    AutomationTarget::Rpn { parameter: 2 },  // Coarse Tune
];

/// 锚点命中半径（像素）。鼠标在此半径内点击视为选中该锚点。
const ANCHOR_HIT_PX: f32 = 6.0;

/// 用户在 automation 面板上的编辑操作。
///
/// 由 `show_panels` 返回，由上层（ui_helpers）应用到 Document。
#[derive(Clone, Debug)]
pub enum AutomationEdit {
    /// 添加新事件。如果 lane 不存在会自动创建。
    /// `shape` = 新事件的 shape。
    Add {
        track_idx: u16,
        target: AutomationTarget,
        tick: u32,
        value: u16,
        shape: SegmentShape,
    },
    /// 移动已有事件。
    Move {
        track_idx: u16,
        lane_idx: usize,
        old_tick: u32,
        new_tick: u32,
        new_value: u16,
    },
    /// 切换已有事件的 shape（双击）。
    CycleShape {
        track_idx: u16,
        lane_idx: usize,
        tick: u32,
    },
}

/// 交互上下文：打包 `show_panels` 处理编辑所需的全部外部信息。
///
/// `None` 时（如未选中唯一 track）跳过所有编辑交互，仅渲染。
pub struct AutomationEditCtx<'a> {
    pub active_tool: Tool,
    pub active_track: Option<u16>,
    pub quantize: QuantizePreset,
    pub ppq: u32,
    pub bar_line_data: Option<(u32, u8, u8, &'a [TimeSigEvent])>,
}

use crate::render_context::RenderContext;
use crate::theme;

/// Height of the split/handle between automation panels.
pub(crate) const SPLIT_H: f32 = theme::AUTO_PANEL_SPLIT_H;

/// Ensure `renderers` has the same count as `panels`, creating/destroying as needed.
fn sync_renderer_count(
    renderers: &mut Vec<(InstanceRenderer, RenderContext)>,
    panels: &[AutomationPanelView],
    wgpu_state: &Arc<eframe::egui_wgpu::RenderState>,
    default_w: u32,
    default_h: u32,
) {
    while renderers.len() < panels.len() {
        let renderer = InstanceRenderer::new(
            wgpu_state.device.clone(),
            wgpu_state.queue.clone(),
            wgpu_state.target_format,
        );
        let ctx = RenderContext::from_render_state(Arc::clone(wgpu_state), default_w, default_h);
        renderers.push((renderer, ctx));
    }
    while renderers.len() > panels.len() {
        renderers.pop();
    }
}

/// Render all automation panels between the pianoroll content and the scrollbar.
///
/// The first panel sits flush against the content above. Each subsequent panel
/// has a `SPLIT_H` drag handle at its top edge.
///
/// Returns the total height consumed by all panels (including split handles
/// between them, but no leading handle for the first panel).
pub fn show_panels(
    ui: &mut egui::Ui,
    panels: &mut Vec<AutomationPanelView>,
    renderers: &mut Vec<(InstanceRenderer, RenderContext)>,
    automation_lanes: &[AutomationLane],
    show_panels: &mut bool,
    wgpu_state: &Arc<eframe::egui_wgpu::RenderState>,
    combo_width: f32,
    pianoroll_scroll_x: f32,
    pianoroll_ppt: f32,
    content_rect_right: f32,
    content_top_y: f32,
    panels_visible_h: f32,
    tpb: Option<u32>,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[yinhe_types::TimeSigEvent],
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    scroll_mode: u32,
    min_border_width: f32,
    midi: Option<&dyn yinhe_automation::NoteSource>,
    velocity_display_mode: &mut u32,
    edit_ctx: Option<&AutomationEditCtx<'_>>,
    tempo_events: &[(u32, f64)],
) -> (f32, Vec<AutomationEdit>) {
    let mut edits = Vec::new();
    if !*show_panels || panels.is_empty() {
        return (0.0, edits);
    }

    // 派生 show_anchors：Pencil 或 Curve 工具下都显示锚点
    let active_tool = edit_ctx.map(|c| c.active_tool).unwrap_or(Tool::Select);
    let show_anchors = active_tool == Tool::Pencil || active_tool == Tool::Curve;

    // Sync scroll state from pianoroll
    for panel in panels.iter_mut() {
        panel.sync_from_pianoroll(pianoroll_scroll_x, pianoroll_ppt, combo_width);
    }

    // Ensure renderer count matches panel count
    sync_renderer_count(renderers, panels, wgpu_state, 640, 200);

    // Snapshot pre-drag heights so rendering stays consistent with the
    // pre-computed panels_total_h layout. Drag writes to panel_height for
    // the next frame instead of mid-frame, avoiding one-frame overlap jitter.
    let orig_heights: Vec<f32> = panels.iter().map(|p| p.panel_height).collect();

    // ── Scroll state for overflow ──
    let panels_natural_h: f32 =
        orig_heights.iter().sum::<f32>() + (panels.len() as f32 * SPLIT_H);
    let max_scroll = (panels_natural_h - panels_visible_h).max(0.0);

    let scroll_id = ui.id().with("auto_panel_scroll_y");
    let mut scroll_y: f32 = ui.data_mut(|d| d.get_persisted(scroll_id)).unwrap_or(0.0);
    scroll_y = scroll_y.clamp(0.0, max_scroll);

    // Panels area rect (visible portion only)
    let panels_area_rect = egui::Rect::from_min_max(
        egui::pos2(0.0, content_top_y),
        egui::pos2(content_rect_right, content_top_y + panels_visible_h),
    );

    // Handle mouse wheel / trackpad scroll in the panels area
    let pointer_in_panels = ui.input(|i| {
        i.pointer
            .hover_pos()
            .is_some_and(|p| panels_area_rect.contains(p))
    });
    if pointer_in_panels && max_scroll > 0.0 {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        scroll_y = (scroll_y - scroll_delta.y).clamp(0.0, max_scroll);
    }
    ui.data_mut(|d| d.insert_persisted(scroll_id, scroll_y));

    // Clip all painting to the panels area
    let old_clip = ui.clip_rect();
    ui.set_clip_rect(panels_area_rect.intersect(old_clip));

    let mut y_offset = content_top_y - scroll_y;
    let visible_top = content_top_y;
    let visible_bottom = content_top_y + panels_visible_h;

    for (i, panel) in panels.iter_mut().enumerate() {
        // Split handle before every panel (first = divider from pianoroll)
        let handle_rect = egui::Rect::from_min_max(
            egui::pos2(0.0, y_offset),
            egui::pos2(content_rect_right, y_offset + SPLIT_H),
        );
        let handle_resp =
            crate::widgets::split_handle::horizontal(ui, format!("auto_handle_{}", i), handle_rect);
        if handle_resp.dragged() {
            let delta = handle_resp.drag_delta().y;
            let new_h = (panel.panel_height - delta).clamp(
                yinhe_automation::automation_view::MIN_PANEL_HEIGHT,
                yinhe_automation::automation_view::MAX_PANEL_HEIGHT,
            );
            panel.panel_height = new_h;
            panel.dirty = true;
            ui.ctx().request_repaint();
        }
        y_offset += SPLIT_H;

        // Render at original height (consistent with pre-computed layout)
        let panel_h = orig_heights[i];
        let panel_top = y_offset;
        let panel_bottom = y_offset + panel_h;
        let panel_rect = egui::Rect::from_min_max(
            egui::pos2(0.0, panel_top),
            egui::pos2(content_rect_right, panel_bottom),
        );

        // Skip heavy rendering for panels entirely outside the visible area
        let is_visible = panel_bottom >= visible_top && panel_top <= visible_bottom;
        if !is_visible {
            y_offset += panel_h;
            continue;
        }

        // ── wgpu automation content (full width, from x=0) ──
        let grid_rect = egui::Rect::from_min_max(panel_rect.min, panel_rect.max);

        let gw = grid_rect.width() as u32;
        let gh = grid_rect.height() as u32;

        // 先处理交互，得到 ghost（传给 wgpu Layer 3 绘制）+ edits。
        // 必须在 prepare_automation 之前，这样 ghost 能当帧渲染。
        let mut panel_ghost: Option<AutomationGhost> = None;
        if let Some(ctx) = edit_ctx {
            if !panel.show_velocity && !panel.show_tempo {
                if let Some(track) = ctx.active_track {
                    let grid_area = egui::Rect::from_min_max(
                        egui::pos2(panel_rect.min.x + combo_width, panel_rect.min.y),
                        egui::pos2(panel_rect.max.x, panel_rect.max.y),
                    );
                    let (panel_edits, ghost) = handle_automation_interaction(
                        ui,
                        grid_area,
                        panel_rect,
                        panel,
                        automation_lanes,
                        track,
                        ctx,
                        i,
                    );
                    edits.extend(panel_edits);
                    panel_ghost = ghost;
                }
            }
        }

        if gw > 0 && gh > 0 {
            if let Some((renderer, render_ctx)) = renderers.get_mut(i) {
                render_ctx.ensure_size(gw, gh);

                let lanes: Vec<&AutomationLane> = automation_lanes
                    .iter()
                    .filter(|l| l.target == panel.selected_target)
                    .collect();

                let gpu_dirty = prepare_automation(
                    renderer,
                    gw,
                    gh,
                    panel,
                    &lanes,
                    midi,
                    tpb,
                    default_num,
                    default_den,
                    time_sig_events,
                    track_visible,
                    track_colors,
                    scroll_mode,
                    min_border_width,
                    *velocity_display_mode,
                    show_anchors,
                    tempo_events,
                    panel_ghost,
                );

                let content_changed = panel.dirty || gpu_dirty;
                panel.dirty = false;

                let painter = ui.painter();
                render_ctx.paint(
                    renderer,
                    gw,
                    gh,
                    &format!("auto_panel_{}", i),
                    painter,
                    grid_rect,
                    content_changed,
                );
            }
        }

        // ── Left side: target selector + display mode buttons ──
        let combo_rect = egui::Rect::from_min_max(
            panel_rect.min,
            egui::pos2(panel_rect.min.x + combo_width, panel_rect.max.y),
        );

        // Draw left panel background (covers the grid underneath)
        ui.painter().rect_filled(combo_rect, 0.0, theme::APP_BG);

        let combo_inner = combo_rect.shrink(4.0);

        ui.scope_builder(egui::UiBuilder::new().max_rect(combo_inner), |ui| {
            ui.set_clip_rect(combo_inner.intersect(panels_area_rect));
            let layout = egui::Layout::top_down(egui::Align::Center);
            ui.with_layout(layout, |ui| {
                // ── Target selector button (tools panel style) ──
                let target_resp = ui.add(
                    egui::Label::new(ICON_AUTOMATION.rich_text().size(14.0).color(egui::Color32::GRAY))
                        .sense(egui::Sense::click())
                        .selectable(false),
                );
                crate::widgets::hover::hover_highlight(
                    ui,
                    &target_resp,
                    ICON_AUTOMATION.codepoint,
                    egui::FontId::new(14.0, ICON_AUTOMATION.font_family()),
                    false,
                );

                // ── Popup menu (manually managed Area to support DragValue interaction) ──
                let popup_id = ui.id().with("auto_target_popup");
                let is_open = ui.data_mut(|d| d.get_persisted::<bool>(popup_id)).unwrap_or(false);

                if target_resp.clicked() {
                    ui.data_mut(|d| d.insert_persisted(popup_id, !is_open));
                }

                if is_open {
                    let popup_pos = egui::pos2(target_resp.rect.left(), target_resp.rect.bottom());
                    let area_resp = egui::Area::new(popup_id)
                        .order(egui::Order::Foreground)
                        .fixed_pos(popup_pos)
                        .show(ui.ctx(), |ui| {
                            egui::Frame::menu(ui.style()).show(ui, |ui| {
                                ui.set_min_width(120.0);
                                // Velocity (special: not an AutomationTarget, renders from notes)
                                let vel_selected = panel.show_velocity;
                                if ui.add(egui::Button::selectable(vel_selected, "Velocity")).clicked() {
                                    panel.show_velocity = true;
                                    panel.show_tempo = false;
                                    panel.dirty = true;
                                    ui.ctx().data_mut(|d| d.insert_persisted(popup_id, false));
                                }
                                // Tempo (special: renders from conductor tempo events)
                                let tempo_selected = panel.show_tempo;
                                if ui.add(egui::Button::selectable(tempo_selected, "Tempo")).clicked() {
                                    panel.show_tempo = true;
                                    panel.show_velocity = false;
                                    panel.dirty = true;
                                    ui.ctx().data_mut(|d| d.insert_persisted(popup_id, false));
                                }
                                ui.separator();
                                for target in AUTOMATION_TARGETS {
                                    let name = target.display_name();
                                    let selected = !panel.show_velocity && !panel.show_tempo && panel.selected_target == *target;
                                    if ui.add(egui::Button::selectable(selected, &name)).clicked() {
                                        panel.selected_target = target.clone();
                                        panel.show_velocity = false;
                                        panel.show_tempo = false;
                                        panel.dirty = true;
                                        ui.ctx().data_mut(|d| d.insert_persisted(popup_id, false));
                                    }
                                }
                                ui.separator();
                                ui.label("自定义 CC:");
                                let mut cc_input = match &panel.selected_target {
                                    AutomationTarget::CC { controller } => *controller as i32,
                                    _ => 0,
                                };
                                let old_cc = cc_input;
                                ui.add(egui::DragValue::new(&mut cc_input).range(0..=127).speed(1));
                                if cc_input != old_cc {
                                    panel.selected_target = AutomationTarget::CC { controller: cc_input as u8 };
                                    panel.show_velocity = false;
                                    panel.show_tempo = false;
                                    panel.dirty = true;
                                }
                            });
                        });

                    // Close only when clicking outside the popup area (not on any interactive element)
                    if ui.input(|i| i.pointer.any_pressed()) {
                        if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                            if !area_resp.response.rect.contains(pos) && !target_resp.rect.contains(pos) {
                                ui.data_mut(|d| d.insert_persisted(popup_id, false));
                            }
                        }
                    }
                }

                ui.add_space(4.0);

                // 自动化渲染模式按钮已删除：默认使用折线图，
                // 锚点显示由当前工具决定（铅笔工具下显示）。
            });
        });

        // ── Grid overlay: value labels + target name ──
        let name = if panel.show_velocity {
            "Velocity".to_string()
        } else if panel.show_tempo {
            "Tempo".to_string()
        } else {
            panel.selected_target.display_name()
        };
        let label_color = theme::MEASURE_LABEL;
        let font_id = egui::FontId::proportional(10.0);
        let pad_x = 4.0;

        let (top_val, mid_val, bot_val) = if panel.show_velocity {
            ("127".to_string(), "64".to_string(), "0".to_string())
        } else if panel.show_tempo {
            let max_bpm = tempo_events
                .iter()
                .map(|(_, bpm)| *bpm)
                .fold(0.0f64, f64::max);
            (format!("{:.1}", max_bpm), format!("{:.1}", max_bpm / 2.0), "0.0".into())
        } else {
            let target = &panel.selected_target;
            let max = target.max_value();
            let def = target.default_value();
            match target {
                AutomationTarget::PitchBend => {
                    let half = max - def; // 8191
                    (half.to_string(), "0".into(), (-(half as i32)).to_string())
                }
                _ if target.has_center_line() => {
                    (max.to_string(), def.to_string(), "0".into())
                }
                _ => {
                    (max.to_string(), (max / 2).to_string(), "0".into())
                }
            }
        };

        let text_x = panel_rect.min.x + combo_width + pad_x;
        let top_y = panel_rect.min.y + 4.0;
        let mid_y = panel_rect.center().y;
        let bot_y = panel_rect.max.y - 4.0;

        let painter = ui.painter();
        painter.text(
            egui::pos2(text_x, top_y),
            egui::Align2::LEFT_TOP,
            top_val,
            font_id.clone(),
            label_color,
        );
        painter.text(
            egui::pos2(text_x, mid_y),
            egui::Align2::LEFT_CENTER,
            mid_val,
            font_id.clone(),
            label_color,
        );
        painter.text(
            egui::pos2(text_x, bot_y),
            egui::Align2::LEFT_BOTTOM,
            bot_val,
            font_id.clone(),
            label_color,
        );

        // Target name: bottom-left, 100px from grid left edge, same row as bottom value
        let name_x = panel_rect.min.x + combo_width + 40.0;
        painter.text(
            egui::pos2(name_x, bot_y),
            egui::Align2::LEFT_BOTTOM,
            &name,
            font_id.clone(),
            label_color,
        );

        y_offset += panel_h;
    }

    // Restore clip rect
    ui.set_clip_rect(old_clip);

    (panels_visible_h, edits)
}

/// 拖拽状态（ghost）。存在 egui data 中，跨帧保持。
#[derive(Clone, Copy, Debug)]
enum AutoDrag {
    /// Pencil 拖拽锚点：`old_tick` 是原始位置，`(cur_tick, cur_value)` 是 ghost 位置
    MoveAnchor { old_tick: u32 },
    /// Curve 拖拽：起点已固定
    CurveDraw { start_tick: u32, start_value: u16 },
}

/// 处理 automation 面板上的鼠标交互。
///
/// **Ghost 模式**：拖拽中不写模型，只返回 ghost 几何（由 wgpu Layer 3 绘制），
/// 释放时才提交编辑。
fn handle_automation_interaction(
    ui: &mut egui::Ui,
    grid_area: egui::Rect,
    panel_rect: egui::Rect,
    panel: &AutomationPanelView,
    automation_lanes: &[AutomationLane],
    track_idx: u16,
    ctx: &AutomationEditCtx<'_>,
    panel_index: usize,
) -> (Vec<AutomationEdit>, Option<AutomationGhost>) {
    let mut edits = Vec::new();
    let target = &panel.selected_target;
    let max_val = target.max_value();
    if max_val == 0 {
        return (edits, None);
    }

    let ppu = panel.base.pixels_per_tick;
    let scroll_x = panel.base.scroll_x;
    let drag_id = ui.id().with("auto_drag").with(panel_index);

    // 读取当前拖拽状态
    let drag_state = ui.ctx().data(|d| d.get_temp::<AutoDrag>(drag_id));

    // 不用 ui.interact——piano_view 的 handle_input 已用 Sense::click_and_drag()
    // 占用了整个 music_rect，自动化面板的 grid_area 是子区域，事件已被父级消费。
    // 改用 ui.input() 直接检测指针状态。
    let pointer_hover_pos = ui.input(|i| i.pointer.hover_pos());
    let pointer_pressed = ui.input(|i| i.pointer.primary_pressed());
    let pointer_released = ui.input(|i| i.pointer.primary_released());
    let pointer_clicked = ui.input(|i| i.pointer.primary_clicked());
    let pointer_double_clicked = ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));

    // 鼠标位置 → tick/value。tick clamp 到 >= 0 防止 as u32 溢出。
    let pos = pointer_hover_pos;
    let mouse_info = pos.map(|p| {
        let x_in_grid = p.x - grid_area.min.x;
        let raw_tick = ((x_in_grid + scroll_x) / ppu).max(0.0);
        let snapped_tick = crate::view_interaction::snap_tick(
            raw_tick as f64,
            ctx.quantize,
            ctx.ppq,
            ctx.bar_line_data,
        ).max(0.0) as u32;
        let y_in_panel = (p.y - panel_rect.min.y).clamp(0.0, panel_rect.height());
        let value = ((1.0 - y_in_panel / panel_rect.height()) * max_val as f32)
            .round()
            .clamp(0.0, max_val as f32) as u16;
        (p, snapped_tick, value)
    });

    // 鼠标是否在 grid 区域内
    let in_grid = pos.is_some_and(|p| grid_area.contains(p));

    // 找当前 lane
    let lane_idx = automation_lanes.iter().position(|l| l.target == *target);
    let lane = lane_idx.and_then(|i| automation_lanes.get(i));

    // 命中检测：找距离鼠标最近的锚点
    let hit_anchor = lane.and_then(|l| {
        let (_, snapped_tick, _) = mouse_info?;
        l.events
            .iter()
            .enumerate()
            .min_by_key(|(_, e)| (e.tick as i64 - snapped_tick as i64).unsigned_abs())
            .and_then(|(i, e)| {
                let (p, _, _) = mouse_info?;
                let ex = grid_area.min.x + (e.tick as f32) * ppu - scroll_x;
                let ey = panel_rect.min.y + (1.0 - e.value as f32 / max_val as f32) * panel_rect.height();
                let dist = ((ex - p.x).powi(2) + (ey - p.y).powi(2)).sqrt();
                if dist <= ANCHOR_HIT_PX {
                    Some((i, e.tick))
                } else {
                    None
                }
            })
    });

    match ctx.active_tool {
        Tool::Pencil => {
            // 双击已有锚点：切换 shape
            if pointer_double_clicked && in_grid {
                if let Some((_, tick)) = hit_anchor {
                    if let Some(lidx) = lane_idx {
                        edits.push(AutomationEdit::CycleShape {
                            track_idx,
                            lane_idx: lidx,
                            tick,
                        });
                    }
                }
                return (edits, None);
            }

            // 拖拽锚点：press 记录，release 提交
            if pointer_pressed && in_grid {
                if let Some((_, tick)) = hit_anchor {
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(drag_id, AutoDrag::MoveAnchor { old_tick: tick });
                    });
                }
            }
            if pointer_released && in_grid {
                let drag = ui.ctx().data(|d| d.get_temp::<AutoDrag>(drag_id));
                ui.ctx().data_mut(|d| d.remove::<AutoDrag>(drag_id));
                if let Some(AutoDrag::MoveAnchor { old_tick }) = drag {
                    if let Some((_, new_tick, new_value)) = mouse_info {
                        if let Some(lidx) = lane_idx {
                            edits.push(AutomationEdit::Move {
                                track_idx,
                                lane_idx: lidx,
                                old_tick,
                                new_tick,
                                new_value,
                            });
                        }
                    }
                }
                return (edits, None);
            }

            // 点击空白（非拖拽）：添加新锚点（shape = Step）
            if pointer_clicked && in_grid && hit_anchor.is_none() && drag_state.is_none() {
                if let Some((_, tick, value)) = mouse_info {
                    edits.push(AutomationEdit::Add {
                        track_idx,
                        target: target.clone(),
                        tick,
                        value,
                        shape: SegmentShape::Step,
                    });
                }
                return (edits, None);
            }
        }
        Tool::Curve => {
            // 拖拽起点 → 终点：press 记录起点，release 提交 2 个锚点
            if pointer_pressed && in_grid {
                if let Some((_, tick, value)) = mouse_info {
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(drag_id, AutoDrag::CurveDraw { start_tick: tick, start_value: value });
                    });
                }
            }
            if pointer_released && in_grid {
                let drag = ui.ctx().data(|d| d.get_temp::<AutoDrag>(drag_id));
                ui.ctx().data_mut(|d| d.remove::<AutoDrag>(drag_id));
                if let Some(AutoDrag::CurveDraw { start_tick: t1, start_value: v1 }) = drag {
                    if let Some((_, t2, v2)) = mouse_info {
                        if t1 != t2 {
                            // 两个锚点：起点 Curve{tension:0}，终点 Step
                            edits.push(AutomationEdit::Add {
                                track_idx,
                                target: target.clone(),
                                tick: t1.min(t2),
                                value: v1,
                                shape: SegmentShape::Curve { tension: 0 },
                            });
                            edits.push(AutomationEdit::Add {
                                track_idx,
                                target: target.clone(),
                                tick: t1.max(t2),
                                value: v2,
                                shape: SegmentShape::Step,
                            });
                        } else {
                            // 单击：只加一个 Curve{tension:0} 锚点
                            edits.push(AutomationEdit::Add {
                                track_idx,
                                target: target.clone(),
                                tick: t2,
                                value: v2,
                                shape: SegmentShape::Curve { tension: 0 },
                            });
                        }
                    }
                }
                return (edits, None);
            }
        }
        _ => {}
    }

    // ── Ghost 计算（panel 局部坐标，传给 wgpu Layer 3 绘制）──
    // 重新读取 drag_state：press 分支可能刚设置过，release 分支已 return。
    let drag_now = ui.ctx().data(|d| d.get_temp::<AutoDrag>(drag_id));
    let ghost = if let Some(drag) = drag_now
        && let Some((_, cur_tick, cur_value)) = mouse_info
    {
        // panel 局部坐标，与 build_data_lines 一致：x = x_offset + tick*ppu
        let x_offset = panel.base.left_panel_width - scroll_x;
        let h = panel_rect.height();
        let cur_x = x_offset + cur_tick as f32 * ppu;
        let cur_y = h - (cur_value as f32 / max_val as f32) * h;
        match drag {
            AutoDrag::MoveAnchor { old_tick } => {
                let old_value = lane
                    .and_then(|l| l.events.iter().find(|e| e.tick == old_tick))
                    .map(|e| e.value)
                    .unwrap_or(cur_value);
                let old_x = x_offset + old_tick as f32 * ppu;
                let old_y = h - (old_value as f32 / max_val as f32) * h;
                Some(AutomationGhost::Move { old_x, old_y, cur_x, cur_y })
            }
            AutoDrag::CurveDraw { start_tick, start_value } => {
                let start_x = x_offset + start_tick as f32 * ppu;
                let start_y = h - (start_value as f32 / max_val as f32) * h;
                Some(AutomationGhost::Curve { start_x, start_y, cur_x, cur_y })
            }
        }
    } else {
        None
    };

    (edits, ghost)
}

/// Show the toggle / add / remove buttons horizontally.
///
/// Designed to be called inside a `ui.horizontal()` or `ui.horizontal_centered()`
/// scope (e.g. inside the scrollbar left blank area).
pub fn show_toggle_buttons(ui: &mut egui::Ui, show_panels: &mut bool, panel_count: &mut usize) {
    ui.spacing_mut().item_spacing.x = 6.0;
    ui.add_space(6.0);

    // Toggle button
    let toggle_color = if *show_panels {
        theme::ACCENT_ACTIVE
    } else {
        egui::Color32::GRAY
    };
    let toggle_label = ICON_SIGNAL_CELLULAR_ALT
        .rich_text()
        .size(theme::MODE_LABEL_FONT + 2.0)
        .color(toggle_color);
    let toggle_resp = ui.add(
        egui::Label::new(toggle_label)
            .sense(egui::Sense::click())
            .selectable(false),
    );
    crate::widgets::hover::hover_highlight(
        ui,
        &toggle_resp,
        ICON_SIGNAL_CELLULAR_ALT.codepoint,
        egui::FontId::new(
            theme::MODE_LABEL_FONT + 2.0,
            ICON_SIGNAL_CELLULAR_ALT.font_family(),
        ),
        *show_panels,
    );
    if toggle_resp.clicked() {
        *show_panels = !*show_panels;
        if *show_panels && *panel_count == 0 {
            *panel_count = 1;
        }
    }

    if *show_panels {
        // + button (add panel)
        let plus_color = egui::Color32::GRAY;
        let plus_resp = ui.add(
            egui::Label::new(
                ICON_ADD
                    .rich_text()
                    .size(theme::MODE_LABEL_FONT + 2.0)
                    .color(plus_color),
            )
            .sense(egui::Sense::click())
            .selectable(false),
        );
        crate::widgets::hover::hover_highlight(
            ui,
            &plus_resp,
            ICON_ADD.codepoint,
            egui::FontId::new(theme::MODE_LABEL_FONT + 2.0, ICON_ADD.font_family()),
            false,
        );
        if plus_resp.clicked() {
            *panel_count += 1;
        }

        // - button (remove panel)
        let minus_resp = ui.add(
            egui::Label::new(
                ICON_REMOVE
                    .rich_text()
                    .size(theme::MODE_LABEL_FONT + 2.0)
                    .color(plus_color),
            )
            .sense(egui::Sense::click())
            .selectable(false),
        );
        crate::widgets::hover::hover_highlight(
            ui,
            &minus_resp,
            ICON_REMOVE.codepoint,
            egui::FontId::new(theme::MODE_LABEL_FONT + 2.0, ICON_REMOVE.font_family()),
            false,
        );
        if minus_resp.clicked() && *panel_count > 0 {
            *panel_count -= 1;
        }
    }
}
