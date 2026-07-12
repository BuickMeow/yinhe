use std::sync::Arc;

use eframe::egui;
use egui_material_icons::icons::*;

use yinhe_editor_core::quantize::QuantizePreset;
pub use yinhe_types::AutomationEdit;
use yinhe_types::{AutomationLane, AutomationTarget, SegmentShape, TimeSigEvent};
use yinhe_types::time_format::format_tick_bar_beat_with_time_sig;

use yinhe_automation::{AutomationGhost, build_lane_override, prepare_automation};
use yinhe_types::AutomationPanelView;
use yinhe_wgpu::InstanceRenderer;

use crate::widgets::tools_panel::Tool;

mod interaction;

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
const ANCHOR_HIT_PX: f32 = 10.0;

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

/// Tempo 的绝对上限（BPM）。来自 `bpm_from_mpq`：mpq=1 时 BPM=60_000_000。
const TEMPO_UPPER_BOUND: f32 = 60_000_000.0;

/// automation 面板交互产生的 pianoroll 联动反馈。
///
/// `show_panels` 返回，由 `piano_view::show` 应用到 pianoroll view。
#[derive(Clone, Copy)]
pub struct PanelPianorollFeedback {
    /// 水平滚动 delta（像素）。非零时 piano_view 会调整 `scroll_x`。
    pub scroll_x_delta: f32,
    /// 水平缩放因子（1.0 = 无缩放）。
    pub zoom_factor: f32,
    /// 缩放中心（pianoroll content 局部 x 坐标，已减去 rect.min.x）。
    pub zoom_center_x: f32,
}

impl Default for PanelPianorollFeedback {
    fn default() -> Self {
        Self {
            scroll_x_delta: 0.0,
            zoom_factor: 1.0, // 1.0 = 无缩放
            zoom_center_x: 0.0,
        }
    }
}

/// 计算 target 的值上限。达到此上限时不可再缩小 value_zoom。
/// - Tempo: 60_000_000 BPM
/// - CC/PB/RPN/NRPN: max_value()
fn value_upper_bound(panel: &AutomationPanelView) -> f32 {
    if panel.show_tempo {
        TEMPO_UPPER_BOUND
    } else if panel.show_velocity {
        127.0
    } else {
        panel.selected_target.max_value() as f32
    }
}

/// 计算 value_zoom 的下限，使得 visible_range 不超过 upper_bound。
fn min_value_zoom(max_val: f32, upper_bound: f32) -> f32 {
    if upper_bound <= 0.0 {
        return 1.0;
    }
    (max_val / upper_bound).max(0.01)
}

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
    midi_version: u64,
) -> (f32, Vec<AutomationEdit>, PanelPianorollFeedback) {
    let mut edits = Vec::new();
    let mut feedback = PanelPianorollFeedback::default();
    if !*show_panels || panels.is_empty() {
        return (0.0, edits, feedback);
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
                yinhe_types::automation_panel_view::MIN_PANEL_HEIGHT,
                yinhe_types::automation_panel_view::MAX_PANEL_HEIGHT,
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

        let ppp = ui.ctx().pixels_per_point();
        let gw = grid_rect.width() as u32;
        let gh = grid_rect.height() as u32;
        let gpw = (gw as f32 * ppp) as u32;
        let gph = (gh as f32 * ppp) as u32;

        // ── 垂直 zoom/scroll + 水平联动交互 ──
        // 内容区（grid_area）：
        //   触控板双指滑动 x → pianoroll 水平滚动（feedback）
        //   触控板双指滑动 y → value_scroll（仅单面板时；多面板时面板间滚动已在上方处理）
        //   触控板捏合 (zoom_delta) → pianoroll 水平缩放（feedback）
        //   Cmd+滚轮 → pianoroll 水平缩放（feedback）
        //   中键拖拽 → 水平 pan (feedback) + value_scroll
        // 左侧面板（combo_area）：
        //   触控板捏合 (zoom_delta) → 垂直缩放
        //   Cmd+滚轮 → 垂直缩放
        //   普通滚轮 → 不操作
        let grid_area = egui::Rect::from_min_max(
            egui::pos2(panel_rect.min.x + combo_width, panel_rect.min.y),
            egui::pos2(panel_rect.max.x, panel_rect.max.y),
        );
        let combo_area = egui::Rect::from_min_max(
            panel_rect.min,
            egui::pos2(panel_rect.min.x + combo_width, panel_rect.max.y),
        );
        let pointer_pos = ui.input(|i| i.pointer.hover_pos());
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        let zoom_delta = ui.input(|i| i.zoom_delta());
        let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        let upper_bound = value_upper_bound(panel);
        let max_val_f = if panel.show_tempo {
            tempo_events.iter().map(|(_, b)| *b as f32).fold(0.0_f32, f32::max).max(1.0)
        } else if panel.show_velocity {
            127.0
        } else {
            panel.selected_target.max_value() as f32
        };
        let zoom_min = min_value_zoom(max_val_f, upper_bound);

        // 垂直缩放辅助闭包
        let apply_vertical_zoom = |panel: &mut AutomationPanelView, factor: f32| {
            panel.value_zoom = (panel.value_zoom * factor).clamp(zoom_min, 8.0);
            panel.clamp_value_scroll(max_val_f);
            panel.dirty = true;
            ui.ctx().request_repaint();
        };

        if let Some(p) = pointer_pos {
            let in_grid = grid_area.contains(p);
            let in_combo = combo_area.contains(p);
            if in_grid {
                // 触控板捏合 → 水平缩放（联动 pianoroll）
                if (zoom_delta - 1.0).abs() > 0.001 {
                    feedback.zoom_factor = zoom_delta;
                    feedback.zoom_center_x = p.x - panel_rect.min.x;
                }
                // Cmd+滚轮 → 水平缩放（联动 pianoroll）
                if cmd && scroll_delta.y.abs() > 0.5 {
                    let factor = if scroll_delta.y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                    feedback.zoom_factor = factor;
                    feedback.zoom_center_x = p.x - panel_rect.min.x;
                }
                // 触控板水平滑动 → pianoroll 水平滚动
                if !cmd && scroll_delta.x.abs() > 0.5 {
                    feedback.scroll_x_delta += scroll_delta.x;
                }
                // 触控板垂直滑动 → value_scroll（仅单面板时）
                if !cmd && scroll_delta.y.abs() > 0.5 && max_scroll <= 0.0 {
                    let visible_range = max_val_f / panel.value_zoom;
                    let scroll_amount = (scroll_delta.y / 100.0) * visible_range * 0.2;
                    let max_scroll_val = (max_val_f - visible_range).max(0.0);
                    panel.value_scroll = (panel.value_scroll + scroll_amount).clamp(0.0, max_scroll_val);
                    panel.dirty = true;
                    ui.ctx().request_repaint();
                }
                // 中键拖拽 → 水平 pan + value_scroll
                if ui.input(|i| i.pointer.middle_down()) {
                    let delta = ui.input(|i| i.pointer.delta());
                    feedback.scroll_x_delta += delta.x;
                    let visible_range = max_val_f / panel.value_zoom;
                    let scroll_amount = -delta.y / panel_rect.height() * visible_range;
                    let max_scroll_val = (max_val_f - visible_range).max(0.0);
                    panel.value_scroll = (panel.value_scroll + scroll_amount).clamp(0.0, max_scroll_val);
                    panel.dirty = true;
                    ui.ctx().request_repaint();
                }
            } else if in_combo {
                // 左侧面板：触控板捏合 → 垂直缩放
                if (zoom_delta - 1.0).abs() > 0.001 {
                    apply_vertical_zoom(panel, zoom_delta);
                }
                // Cmd+滚轮 → 垂直缩放
                if cmd && scroll_delta.y.abs() > 0.5 {
                    let factor = if scroll_delta.y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                    apply_vertical_zoom(panel, factor);
                }
            }
        }

        // 先处理交互，得到 ghost（传给 wgpu Layer 3 绘制）+ edits。
        // 必须在 prepare_automation 之前，这样 ghost 能当帧渲染。
        let mut panel_ghost: Option<AutomationGhost> = None;
        if let Some(ctx) = edit_ctx {
            if !panel.show_velocity && !panel.show_tempo {
                if let Some(track) = ctx.active_track {
                    let (panel_edits, ghost, drag_info) = interaction::handle_automation_interaction(
                        ui,
                        grid_area,
                        panel_rect,
                        panel,
                        automation_lanes,
                        track,
                        ctx,
                        i,
                        track_colors,
                    );
                    edits.extend(panel_edits);
                    panel_ghost = ghost;

                    // 拖拽 tooltip：鼠标指针右侧显示位置和值
                    if let Some((tick, value)) = drag_info {
                        if let Some(pos) = pointer_pos {
                            let pos_str = if let Some((ppq, num, den, ts_events)) = ctx.bar_line_data {
                                format_tick_bar_beat_with_time_sig(tick as f64, ppq, ts_events, num, den)
                            } else {
                                format!("{}", tick)
                            };
                            let val_str = if panel.show_tempo {
                                format!("{:.2} BPM", value as f32)
                            } else {
                                format!("{}", value)
                            };
                            let painter = ui.ctx().debug_painter();
                            let font_id = egui::FontId::monospace(12.0);
                            let gap = 8.0;
                            let tooltip_x = pos.x + gap;
                            let tooltip_y = pos.y - 24.0;
                            // 两行文本
                            let lines = [pos_str.as_str(), val_str.as_str()];
                            let mut max_w = 0.0_f32;
                            let mut total_h = 0.0;
                            let line_h = 16.0;
                            for line in &lines {
                                let galley = painter.layout_no_wrap(line.to_string(), font_id.clone(), egui::Color32::WHITE);
                                max_w = max_w.max(galley.rect.width());
                                total_h += line_h;
                            }
                            let pad = 6.0;
                            let bg_rect = egui::Rect::from_min_size(
                                egui::pos2(tooltip_x - pad, tooltip_y - pad),
                                egui::vec2(max_w + pad * 2.0, total_h + pad * 2.0),
                            );
                            painter.rect_filled(bg_rect, 4.0, egui::Color32::from_black_alpha(180));
                            let mut ly = tooltip_y;
                            for line in &lines {
                                painter.text(
                                    egui::pos2(tooltip_x, ly),
                                    egui::Align2::LEFT_TOP,
                                    *line,
                                    font_id.clone(),
                                    egui::Color32::WHITE,
                                );
                                ly += line_h;
                            }
                        }
                    }
                }
            }
        }

        if gw > 0 && gh > 0 {
            if let Some((renderer, render_ctx)) = renderers.get_mut(i) {
                render_ctx.ensure_size(gpw, gph);

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
                    midi_version,
                );

                let content_changed = panel.dirty || gpu_dirty;
                panel.dirty = false;

                let painter = ui.painter();
                render_ctx.paint(
                    renderer,
                    gpw,
                    gph,
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

        let (top_val, mid_val, bot_val) = if panel.show_tempo || panel.show_velocity {
            // Velocity / Tempo: 根据垂直 zoom/scroll 计算实际显示范围
            let h = panel_rect.height();
            let top_f = panel.y_to_value(0.0, max_val_f).round() as u32;
            let mid_f = panel.y_to_value(h * 0.5, max_val_f).round() as u32;
            let bot_f = panel.y_to_value(h, max_val_f).round() as u32;
            (top_f.to_string(), mid_f.to_string(), bot_f.to_string())
        } else {
            let target = &panel.selected_target;
            let max = target.max_value();
            let max_f = max as f32;
            // 根据垂直 zoom/scroll 计算面板顶部、中部、底部的实际值
            let h = panel_rect.height();
            let top_val_f = panel.y_to_value(0.0, max_f).round() as u32;
            let mid_val_f = panel.y_to_value(h * 0.5, max_f).round() as u32;
            let bot_val_f = panel.y_to_value(h, max_f).round() as u32;
            (top_val_f.to_string(), mid_val_f.to_string(), bot_val_f.to_string())
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

    // ── 右键锚点编辑弹窗 ──
    for i in 0..panels.len() {
        let right_click_id = ui.id().with("auto_right_click").with(i);
        if let Some(anchor) = ui.ctx().data(|d| d.get_temp::<interaction::RightClickAnchor>(right_click_id)) {
            let max_val = anchor.target.max_value();
            let edit_tick_id = ui.id().with("auto_right_tick").with(i);
            let edit_value_id = ui.id().with("auto_right_value").with(i);
            let mut new_tick = ui.ctx().data(|d| d.get_temp::<f64>(edit_tick_id)).unwrap_or(anchor.old_tick as f64);
            let mut new_value = ui.ctx().data(|d| d.get_temp::<f64>(edit_value_id)).unwrap_or(anchor.old_value as f64);
            let mut submitted = false;

            let area_id = ui.id().with("auto_right_edit_area").with(i);
            let area_pos = anchor.anchor_pos + egui::Vec2::new(16.0, -8.0);
            let area_response = egui::Area::new(area_id)
                .order(egui::Order::Foreground)
                .fixed_pos(area_pos)
                .show(ui.ctx(), |ui| {
                    let frame = egui::Frame::popup(ui.style());
                    frame.show(ui, |ui| {
                        ui.set_min_width(160.0);
                        // 只读位置显示
                        if let Some(ctx) = edit_ctx {
                            if let Some((ppq, num, den, ts_events)) = ctx.bar_line_data {
                                let pos_str = format_tick_bar_beat_with_time_sig(anchor.old_tick as f64, ppq, ts_events, num, den);
                                ui.label(egui::RichText::new(pos_str).size(11.0).color(egui::Color32::GRAY));
                            }
                        }
                        ui.add_space(4.0);
                        // Tick（只有 lost_focus 即按 Enter 才提交，拖拽不触发 undo）
                        ui.horizontal(|ui| {
                            ui.label("Tick:");
                            let resp = ui.add(egui::DragValue::new(&mut new_tick).range(0..=u32::MAX as i64).speed(1.0));
                            if resp.lost_focus() {
                                submitted = true;
                            }
                        });
                        // Value（同上）
                        ui.horizontal(|ui| {
                            ui.label("Value:");
                            let resp = ui.add(egui::DragValue::new(&mut new_value).range(0..=max_val as i64).speed(1.0));
                            if resp.lost_focus() {
                                submitted = true;
                            }
                        });
                    });
                });

            // 保存编辑中的值，跨帧保持
            ui.ctx().data_mut(|d| {
                d.insert_temp(edit_tick_id, new_tick);
                d.insert_temp(edit_value_id, new_value);
            });

            if submitted {
                // 提交编辑
                println!("[auto_debug] popup submit: old_tick={}, new_tick={}, new_value={}", anchor.old_tick, new_tick as u32, new_value as u16);
                edits.push(AutomationEdit::Move {
                    track_idx: anchor.track_idx,
                    lane_idx: anchor.lane_idx,
                    old_tick: anchor.old_tick,
                    new_tick: new_tick as u32,
                    new_value: new_value as u16,
                });
                // 更新 interaction::RightClickAnchor 的 old_tick/old_value，使后续 edit 使用新位置
                ui.ctx().data_mut(|d| {
                    if let Some(mut a) = d.get_temp::<interaction::RightClickAnchor>(right_click_id) {
                        a.old_tick = new_tick as u32;
                        a.old_value = new_value as u16;
                        d.insert_temp(right_click_id, a);
                    }
                });
            }

            // 弹窗外点击 → 提交最终值然后关闭弹窗（跳过首帧，避免右键打开弹窗时立即关闭）
            let was_open_id = ui.id().with("auto_right_was_open").with(i);
            let was_open = ui.ctx().data(|d| d.get_temp::<bool>(was_open_id)).unwrap_or(false);
            ui.ctx().data_mut(|d| d.insert_temp(was_open_id, true));
            if was_open && area_response.response.clicked_elsewhere() {
                // 仅值有变化时提交（ghost 模式：最终值一次性提交，不产生中间 undo）
                if new_tick as u32 != anchor.old_tick || new_value as u16 != anchor.old_value {
                    edits.push(AutomationEdit::Move {
                        track_idx: anchor.track_idx,
                        lane_idx: anchor.lane_idx,
                        old_tick: anchor.old_tick,
                        new_tick: new_tick as u32,
                        new_value: new_value as u16,
                    });
                }
                ui.ctx().data_mut(|d| {
                    d.remove::<interaction::RightClickAnchor>(right_click_id);
                    d.remove::<f64>(edit_tick_id);
                    d.remove::<f64>(edit_value_id);
                    d.remove::<bool>(was_open_id);
                });
            }
        }
    }

    (panels_visible_h, edits, feedback)
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
