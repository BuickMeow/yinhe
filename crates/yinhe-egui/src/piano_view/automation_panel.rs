use std::sync::Arc;

use eframe::egui;
use egui_material_icons::icons::*;
use rust_i18n::t;

use yinhe_editor_core::quantize::QuantizePreset;
pub use yinhe_types::AutomationEdit;
use yinhe_types::time_format::format_tick_bar_beat_with_time_sig;
use yinhe_types::{AutomationLane, AutomationTarget, TimeSigEvent, VelocityEdit};

use yinhe_types::AutomationPanelView;
use yinhe_wgpu::InstanceRenderer;
use yinhe_wgpu::{AutomationGhost, prepare_automation};

use crate::right_panel::{InfoContent, RightTab};
use crate::widgets::tools_panel::Tool;

mod interaction;
mod velocity;
use velocity::VelocityPreview;

/// Curated list of known automation targets shown in the dropdown.
const AUTOMATION_TARGETS: &[AutomationTarget] = &[
    AutomationTarget::Tempo,
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

/// 面板集合渲染状态（panels 与 renderers 一一对应）。
pub(crate) struct PanelsState<'a> {
    pub panels: &'a mut Vec<AutomationPanelView>,
    pub renderers: &'a mut Vec<(InstanceRenderer, RenderContext)>,
    pub wgpu_state: &'a Arc<eframe::egui_wgpu::RenderState>,
    pub show_panels: &'a mut bool,
}

/// 面板布局几何。
#[derive(Clone, Copy)]
pub(crate) struct PanelsLayout {
    pub combo_width: f32,
    pub content_rect_right: f32,
    pub content_top_y: f32,
    pub panels_visible_h: f32,
}

/// 面板渲染/联动配置。
#[derive(Clone, Copy)]
pub(crate) struct PanelsCfg<'a> {
    pub pianoroll_scroll_x: f32,
    pub pianoroll_ppt: f32,
    pub scroll_mode: u32,
    pub min_border_width: f32,
    pub revision: u64,
    /// 状态栏讲解行格式化位置所需（拍号事件）。
    pub bar_line_data: Option<(u32, u8, u8, &'a [TimeSigEvent])>,
    /// 讲解行选框统计（AM 选框命中时显示）。
    pub sel_hint: Option<&'a crate::app::layout::SelHintInfo>,
    /// 当前编辑目标是否为 Conductor 轨。Conductor 下 AM 面板只可编辑 Tempo
    /// （下拉菜单仅显示 Tempo，非 Tempo 编辑被 dispatch 层禁用）。
    pub editing_is_conductor: bool,
}

/// 面板模型只读数据。
pub(crate) struct PanelsData<'a> {
    pub automation_lanes: &'a [AutomationLane],
    pub render_lanes: &'a [&'a AutomationLane],
    pub tempo_lane: &'a AutomationLane,
    pub midi: Option<&'a dyn yinhe_types::NoteSource>,
    pub track_visible: &'a [bool],
    pub track_colors: &'a [[f32; 4]],
}

/// 面板编辑状态。
pub(crate) struct PanelsEdit<'a> {
    pub selected: &'a mut yinhe_core::Selection,
    pub info_content: &'a mut Option<InfoContent>,
    pub right_tab: &'a mut Option<RightTab>,
}

/// `show_panels` 的返回：总高度 + 编辑动作列表 + 联动反馈 + (拖拽锚点 tick, value)。
pub(crate) type PanelsOutput = (
    f32,
    Vec<AutomationEdit>,
    Vec<VelocityEdit>,
    PanelPianorollFeedback,
    Option<(u32, f32)>,
);

/// automation 面板交互产生的 pianoroll 联动反馈。
///
/// `show_panels` 返回，由 `piano_view::show` 应用到 pianoroll view。
pub struct PanelPianorollFeedback {
    /// 水平滚动 delta（像素）。非零时 piano_view 会调整 `scroll_x`。
    pub scroll_x_delta: f32,
    /// 水平缩放因子（1.0 = 无缩放）。
    pub zoom_factor: f32,
    /// 缩放中心（pianoroll content 局部 x 坐标，已减去 rect.min.x）。
    pub zoom_center_x: f32,
    /// 状态栏讲解行：鼠标悬停在面板 grid 区时的提示（位置 + 值）。
    pub status_hint: Option<String>,
}

impl Default for PanelPianorollFeedback {
    fn default() -> Self {
        Self {
            scroll_x_delta: 0.0,
            zoom_factor: 1.0, // 1.0 = 无缩放
            zoom_center_x: 0.0,
            status_hint: None,
        }
    }
}

/// 计算 target 的值上限。达到此上限时不可再缩小 value_zoom。
/// - Tempo: `AutomationTarget::Tempo.max_value()`（BPM 理论上限 60_000_000）
/// - CC/PB/RPN/NRPN: max_value()
fn value_upper_bound(panel: &AutomationPanelView) -> f32 {
    if panel.show_velocity {
        127.0
    } else if panel.selected_target == AutomationTarget::Tempo {
        AutomationTarget::Tempo.max_value()
    } else {
        panel.selected_target.max_value()
    }
}

/// 面板当前 target 的值上限（velocity=127；Tempo 由实际事件动态计算；其他 max_value()）。
/// show_panels（zoom/scroll/标签）与 interaction（y↔value 换算）共用。
pub(crate) fn panel_max_val(panel: &AutomationPanelView, tempo_lane: &AutomationLane) -> f32 {
    if panel.show_velocity {
        127.0
    } else if panel.selected_target == AutomationTarget::Tempo {
        tempo_lane
            .events
            .iter()
            .map(|e| e.value)
            .fold(0.0_f32, f32::max)
            .max(1.0)
    } else {
        panel.selected_target.max_value()
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
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub fn show_panels(
    ui: &mut egui::Ui,
    state: &mut PanelsState<'_>,
    data: &PanelsData<'_>,
    layout: PanelsLayout,
    cfg: PanelsCfg<'_>,
    edit: &mut PanelsEdit<'_>,
    edit_ctx: Option<&AutomationEditCtx<'_>>,
) -> PanelsOutput {
    let mut edits = Vec::new();
    let mut velocity_edits = Vec::new();
    let mut feedback = PanelPianorollFeedback::default();
    let mut all_drag_info: Option<(u32, f32)> = None;
    if !*state.show_panels || state.panels.is_empty() {
        return (0.0, edits, velocity_edits, feedback, None);
    }

    // 派生 show_anchors：Pencil/Curve/Select/SelectVertical 工具下显示锚点
    let active_tool = edit_ctx.map(|c| c.active_tool).unwrap_or(Tool::Select);
    let show_anchors = matches!(
        active_tool,
        Tool::Pencil | Tool::Curve | Tool::Select | Tool::SelectVertical
    );

    // Sync scroll state from pianoroll
    for panel in state.panels.iter_mut() {
        panel.sync_from_pianoroll(
            cfg.pianoroll_scroll_x,
            cfg.pianoroll_ppt,
            layout.combo_width,
        );
    }

    // Ensure renderer count matches panel count
    sync_renderer_count(state.renderers, state.panels, state.wgpu_state, 640, 200);

    // Snapshot pre-drag heights so rendering stays consistent with the
    // pre-computed panels_total_h layout. Drag writes to panel_height for
    // the next frame instead of mid-frame, avoiding one-frame overlap jitter.
    let orig_heights: Vec<f32> = state.panels.iter().map(|p| p.panel_height).collect();

    // ── Scroll state for overflow ──
    let panels_natural_h: f32 =
        orig_heights.iter().sum::<f32>() + (state.panels.len() as f32 * SPLIT_H);
    let max_scroll = (panels_natural_h - layout.panels_visible_h).max(0.0);

    let scroll_id = ui.id().with("auto_panel_scroll_y");
    let mut scroll_y: f32 = ui.data_mut(|d| d.get_persisted(scroll_id)).unwrap_or(0.0);
    scroll_y = scroll_y.clamp(0.0, max_scroll);

    // Panels area rect (visible portion only)
    let panels_area_rect = egui::Rect::from_min_max(
        egui::pos2(0.0, layout.content_top_y),
        egui::pos2(
            layout.content_rect_right,
            layout.content_top_y + layout.panels_visible_h,
        ),
    );

    // Handle mouse wheel / trackpad scroll in the state.panels area
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

    // Clip all painting to the state.panels area
    let old_clip = ui.clip_rect();
    ui.set_clip_rect(panels_area_rect.intersect(old_clip));

    // 面板右侧竖条：panel 让出 SCROLLBAR_W 给垂直滚动条，这块区域补背景
    let vbar_rect = egui::Rect::from_min_max(
        egui::pos2(
            layout.content_rect_right - crate::widgets::scrollbar::SCROLLBAR_W,
            layout.content_top_y,
        ),
        egui::pos2(
            layout.content_rect_right,
            layout.content_top_y + layout.panels_visible_h,
        ),
    );
    ui.painter().rect_filled(vbar_rect, 0.0, theme::track_bg());

    let mut y_offset = layout.content_top_y - scroll_y;
    let visible_top = layout.content_top_y;
    let visible_bottom = layout.content_top_y + layout.panels_visible_h;

    for (i, panel) in state.panels.iter_mut().enumerate() {
        // Split handle before every panel (first = divider from pianoroll)
        let handle_rect = egui::Rect::from_min_max(
            egui::pos2(0.0, y_offset),
            egui::pos2(layout.content_rect_right, y_offset + SPLIT_H),
        );
        handle_split_drag(ui, panel, handle_rect, i);
        y_offset += SPLIT_H;

        // Render at original height (consistent with pre-computed layout)
        let panel_h = orig_heights[i];
        let panel_top = y_offset;
        let panel_bottom = y_offset + panel_h;
        // 右边界让出 SCROLLBAR_W 给垂直滚动条
        let panel_right = layout.content_rect_right - crate::widgets::scrollbar::SCROLLBAR_W;
        let panel_rect = egui::Rect::from_min_max(
            egui::pos2(0.0, panel_top),
            egui::pos2(panel_right, panel_bottom),
        );

        // Skip heavy rendering for state.panels entirely outside the visible area
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
            egui::pos2(panel_rect.min.x + layout.combo_width, panel_rect.min.y),
            egui::pos2(panel_rect.max.x, panel_rect.max.y),
        );
        let combo_area = egui::Rect::from_min_max(
            panel_rect.min,
            egui::pos2(panel_rect.min.x + layout.combo_width, panel_rect.max.y),
        );
        let upper_bound = value_upper_bound(panel);
        let mut max_val_f = panel_max_val(panel, data.tempo_lane);
        let zoom_min = min_value_zoom(max_val_f, upper_bound);

        // ── 状态栏讲解行：面板 grid 区悬停提示（位置 + 值）──
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
                // 本面板有选框 → 讲解行显示选框统计（参考 info panel）
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
                // 面板内非 grid 区（combo 选择器/滚动条）→ 清空
                feedback.status_hint = None;
            }
        }
        handle_panel_scroll_zoom(
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

        // 先处理交互，得到 ghost（传给 wgpu Layer 3 绘制）+ edits。
        // 必须在 prepare_automation 之前，这样 ghost 能当帧渲染。
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
        );
        edits.extend(out.automation_edits);
        velocity_edits.extend(out.velocity_edits);
        if out.anchor_drag.is_some() {
            all_drag_info = out.anchor_drag;
        }
        // 拖拽锚点突破当前显示上限时扩展显示范围（Tempo 可拖出 120 以上的 BPM）：
        // 渲染/标签/滚动条用扩展后的 max_val_f，锚点保持在面板内、整条曲线实时拉开。
        if let Some((_, v)) = out.anchor_drag {
            max_val_f = max_val_f.max(v);
        }
        let panel_ghost = out.ghost;
        let velocity_preview = out.preview;
        let marquee_rect = out.marquee_rect;
        // 应用 Select 工具的选区变更 + 持续化选框
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
                    // 三视图选框互斥：开始新框选时清空共享音符选区，
                    // App 层检测到选区被清空后清除其他视图的选框。
                    edit.selected.clear();
                }
            }
        }

        if gw > 0
            && gh > 0
            && let Some((renderer, render_ctx)) = state.renderers.get_mut(i)
        {
            render_panel_content(
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
                layout.combo_width,
            );
            draw_panel_overlay(
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

        // ── 垂直滚动条（值空间） ──
        // 占用面板右侧 SCROLLBAR_W 宽度。仅在 visible_range < upper_bound 时显示。
        // tempo 模式下 upper_bound 是 `AutomationTarget::Tempo.max_value()`；其他模式用 max_value()。
        let vsb_rect = egui::Rect::from_min_max(
            egui::pos2(panel_right, panel_top),
            egui::pos2(layout.content_rect_right, panel_bottom),
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

        // ── Left side: target selector + display mode buttons ──
        let combo_rect = egui::Rect::from_min_max(
            panel_rect.min,
            egui::pos2(panel_rect.min.x + layout.combo_width, panel_rect.max.y),
        );
        show_target_combo(
            ui,
            panel,
            combo_rect,
            panels_area_rect,
            cfg.editing_is_conductor,
        );

        draw_value_labels(ui, panel, panel_rect, layout.combo_width, max_val_f);

        y_offset += panel_h;
    }

    // Restore clip rect
    ui.set_clip_rect(old_clip);

    // ── 右键锚点：设置 edit.info_content 打开信息面板 ──
    apply_right_click_anchor(
        ui,
        state.panels.len(),
        data.automation_lanes,
        edit.info_content,
        edit.right_tab,
    );

    (
        layout.panels_visible_h,
        edits,
        velocity_edits,
        feedback,
        all_drag_info,
    )
}

/// 当帧交互产生的临时 overlay 数据。
struct PanelOverlayData {
    marquee_rect: Option<egui::Rect>,
    velocity_preview: Option<VelocityPreview>,
}

/// 绘制 wgpu 纹理之上的面板 overlay：velocity 笔划预览、持续化选框、拖拽中 marquee。
fn draw_panel_overlay(
    ui: &mut egui::Ui,
    panel: &AutomationPanelView,
    panel_rect: egui::Rect,
    grid_area: egui::Rect,
    max_val_f: f32,
    panel_idx: usize,
    overlay: &PanelOverlayData,
) {
    // ── velocity 笔划预览 ──
    if let Some(preview) = &overlay.velocity_preview {
        let painter = ui.painter();
        for bar in &preview.bars {
            painter.rect_filled(*bar, 0.0, preview.color.gamma_multiply(0.85));
            // 顶部亮线标示新高度
            painter.line_segment(
                [bar.left_top(), bar.right_top()],
                egui::Stroke::new(1.0, crate::theme::contrast_fg()),
            );
        }
    }
    // ── 持续化选框（框选完成后持续显示，画在 wgpu 纹理之上）──
    if overlay.marquee_rect.is_none() {
        let x_offset = grid_area.min.x - panel.base.scroll_x;
        let ppu = panel.base.pixels_per_tick;
        // MoveAnchors 拖拽中偏移选框（跟随锚点移动）
        let move_offset_id = ui.id().with("auto_move_offset").with(panel_idx);
        let (d_tick, d_value) = ui
            .ctx()
            .data(|d| d.get_temp::<(i64, f32)>(move_offset_id))
            .unwrap_or((0, 0.0));
        let painter = ui.painter();
        for sel_rect in &panel.anchor_sel_rects {
            let ts = (sel_rect.tick_start.min(sel_rect.tick_end) + d_tick as f64).max(0.0);
            let te = (sel_rect.tick_start.max(sel_rect.tick_end) + d_tick as f64).max(0.0);
            let x1 = x_offset + (ts as f32) * ppu;
            let x2 = x_offset + (te as f32) * ppu;
            let (y1, y2) = match sel_rect.value_range {
                None => (grid_area.min.y, grid_area.max.y),
                Some((vmin, vmax)) => {
                    let v1 = (vmin + d_value).clamp(0.0, max_val_f);
                    let v2 = (vmax + d_value).clamp(0.0, max_val_f);
                    let ya = panel_rect.min.y + panel.value_to_y(v2, max_val_f);
                    let yb = panel_rect.min.y + panel.value_to_y(v1, max_val_f);
                    (ya.min(yb), ya.max(yb))
                }
            };
            let rect = egui::Rect::from_min_max(egui::pos2(x1, y1), egui::pos2(x2, y2))
                .intersect(grid_area);
            // 选框颜色与 PR/AR 一致：白色 + gamma_multiply
            painter.rect_filled(
                rect,
                0.0,
                crate::theme::contrast_fg().gamma_multiply(crate::theme::marquee_fill_alpha()),
            );
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(
                    1.0,
                    crate::theme::contrast_fg()
                        .gamma_multiply(crate::theme::marquee_stroke_alpha()),
                ),
                egui::StrokeKind::Inside,
            );
        }
    }
    // ── Select 工具框选矩形（拖拽中的临时选框，画在最上层）──
    if let Some(rect) = overlay.marquee_rect {
        let painter = ui.painter();
        // 选框颜色与 PR/AR 一致：白色 + gamma_multiply（拖拽中与已提交同透明度）
        painter.rect_filled(
            rect,
            0.0,
            crate::theme::contrast_fg().gamma_multiply(crate::theme::marquee_fill_alpha()),
        );
        painter.rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(
                1.0,
                crate::theme::contrast_fg().gamma_multiply(crate::theme::marquee_stroke_alpha()),
            ),
            egui::StrokeKind::Inside,
        );
    }
}

/// 绘制面板左侧的值标签（顶部/中部/底部）与 target 名。
fn draw_value_labels(
    ui: &mut egui::Ui,
    panel: &AutomationPanelView,
    panel_rect: egui::Rect,
    combo_width: f32,
    max_val_f: f32,
) {
    let name = if panel.show_velocity {
        t!("automation.velocity").to_string()
    } else {
        panel.selected_target.display_name()
    };
    let label_color = theme::measure_label();
    let font_id = egui::FontId::proportional(crate::theme::SMALL_LABEL_FONT);
    let pad_x = 4.0;

    // Velocity / Tempo 用面板级 max_val_f；其他用 target 固定 max_value()
    let label_max = if panel.show_velocity || panel.selected_target == AutomationTarget::Tempo {
        max_val_f
    } else {
        panel.selected_target.max_value()
    };
    // 根据垂直 zoom/scroll 计算面板顶部、中部、底部的实际值
    let h = panel_rect.height();
    let (top_val, mid_val, bot_val) = (
        panel.y_to_value(0.0, label_max).round() as u32,
        panel.y_to_value(h * 0.5, label_max).round() as u32,
        panel.y_to_value(h, label_max).round() as u32,
    );
    let (top_val, mid_val, bot_val) = (
        top_val.to_string(),
        mid_val.to_string(),
        bot_val.to_string(),
    );

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
}

/// 分割条拖拽：调整面板高度。写入下一帧生效，避免帧内布局抖动。
fn handle_split_drag(
    ui: &mut egui::Ui,
    panel: &mut AutomationPanelView,
    handle_rect: egui::Rect,
    index: usize,
) {
    let handle_resp =
        crate::widgets::split_handle::horizontal(ui, format!("auto_handle_{}", index), handle_rect);
    if handle_resp.dragged() {
        let delta = handle_resp.drag_delta().y;
        panel.panel_height = (panel.panel_height - delta).clamp(
            yinhe_types::automation_panel_view::MIN_PANEL_HEIGHT,
            yinhe_types::automation_panel_view::MAX_PANEL_HEIGHT,
        );
        panel.dirty = true;
        ui.ctx().request_repaint();
    }
}

/// 面板的滚动/缩放交互。
/// 内容区（grid_area）：
///   触控板双指滑动 x → pianoroll 水平滚动（feedback）
///   触控板双指滑动 y → value_scroll（仅单面板时；多面板时面板间滚动已在上方处理）
///   触控板捏合 (zoom_delta) → pianoroll 水平缩放（feedback）
///   Cmd+滚轮 → pianoroll 水平缩放（feedback）
///   中键拖拽 → 水平 pan (feedback) + value_scroll
/// 左侧面板（combo_area）：
///   触控板捏合 / Cmd+滚轮 → 垂直缩放
///   普通滚轮 → 不操作
#[allow(clippy::too_many_arguments)]
fn handle_panel_scroll_zoom(
    ui: &mut egui::Ui,
    panel: &mut AutomationPanelView,
    grid_area: egui::Rect,
    combo_area: egui::Rect,
    panel_rect: egui::Rect,
    max_val_f: f32,
    zoom_min: f32,
    max_scroll: f32,
    feedback: &mut PanelPianorollFeedback,
) {
    let pointer_pos = ui.input(|i| i.pointer.hover_pos());
    let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
    let zoom_delta = ui.input(|i| i.zoom_delta());
    let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);

    // 垂直缩放辅助闭包
    let apply_vertical_zoom = |panel: &mut AutomationPanelView, factor: f32| {
        panel.value_zoom = (panel.value_zoom * factor).clamp(zoom_min, 8.0);
        panel.clamp_value_scroll(max_val_f);
        panel.dirty = true;
        ui.ctx().request_repaint();
    };

    let Some(p) = pointer_pos else { return };
    if grid_area.contains(p) {
        // 触控板捏合 → 水平缩放（联动 pianoroll）
        if (zoom_delta - 1.0).abs() > 0.001 {
            feedback.zoom_factor = zoom_delta;
            feedback.zoom_center_x = p.x - panel_rect.min.x;
        }
        // Cmd+滚轮 → 水平缩放（联动 pianoroll）
        if cmd && scroll_delta.y.abs() > 0.5 {
            // 方向：上滚 = 放大，下滚 = 缩小（与 PR 内容区/滚动条一致）
            let factor = if scroll_delta.y > 0.0 { 1.0 / 1.1 } else { 1.1 };
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
    } else if combo_area.contains(p) {
        // 左侧面板：触控板捏合 → 垂直缩放
        if (zoom_delta - 1.0).abs() > 0.001 {
            apply_vertical_zoom(panel, zoom_delta);
        }
        // Cmd+滚轮 → 垂直缩放
        if cmd && scroll_delta.y.abs() > 0.5 {
            // 方向：上滚 = 放大，下滚 = 缩小（与 PR 内容区/滚动条一致）
            let factor = if scroll_delta.y > 0.0 { 1.0 / 1.1 } else { 1.1 };
            apply_vertical_zoom(panel, factor);
        }
    }
}

/// 单个面板的编辑交互输出。
struct PanelInteractionOut {
    automation_edits: Vec<AutomationEdit>,
    velocity_edits: Vec<VelocityEdit>,
    /// wgpu Layer 3 的 lane ghost（仅 lane 编辑）
    ghost: Option<AutomationGhost>,
    /// velocity 笔划预览（仅 velocity 模式）
    preview: Option<velocity::VelocityPreview>,
    /// 锚点拖拽的实时 (tick, value)，供 InfoPanel 显示
    anchor_drag: Option<(u32, f32)>,
    /// Select 工具框选矩形（egui painter 绘制 + 渲染层高亮预览）
    marquee_rect: Option<egui::Rect>,
    /// Select 工具选区变更操作（应用到 panel.anchor_sel_rects）
    sel_op: Option<interaction::SelOp>,
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
            // Velocity：铅笔笔划修改力度条（命中 noteon，只作用于 active_track）
            if ctx.active_tool == Tool::Pencil
                && let Some(track) = ctx.active_track
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
                    tempo_lane,
                    0,
                    ctx,
                    panel_index,
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
                    tempo_lane,
                    track,
                    ctx,
                    panel_index,
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

/// 渲染单个面板的 wgpu 内容：prepare（含 ghost）+ 背景/中线/网格 + paint。
#[allow(clippy::too_many_arguments)]
fn render_panel_content(
    ui: &mut egui::Ui,
    renderer: &mut InstanceRenderer,
    render_ctx: &mut RenderContext,
    panel: &mut AutomationPanelView,
    grid_rect: egui::Rect,
    gpw: u32,
    gph: u32,
    render_lanes: &[&AutomationLane],
    tempo_lane: &AutomationLane,
    midi: Option<&dyn yinhe_types::NoteSource>,
    track_visible: &[bool],
    track_colors: &[[f32; 4]],
    scroll_mode: u32,
    min_border_width: f32,
    show_anchors: bool,
    max_val_f: f32,
    panel_ghost: Option<AutomationGhost>,
    revision: u64,
    info_content: &Option<InfoContent>,
    panel_index: usize,
    combo_width: f32,
) {
    let gw = grid_rect.width() as u32;
    let gh = grid_rect.height() as u32;
    render_ctx.ensure_size(gpw, gph);

    // Tempo 模式：lanes 只包含 conductor.tempo；其他模式按 selected_target 过滤。
    let lanes: Vec<&AutomationLane> = if panel.selected_target == AutomationTarget::Tempo {
        vec![tempo_lane]
    } else {
        render_lanes
            .iter()
            .filter(|l| l.target == panel.selected_target)
            .copied()
            .collect()
    };

    // 高亮锚点 tick 集合（Select 工具多选 + Pencil 工具单选 info_content）。
    // render_lanes 可能含多个音轨：按 track 匹配锚点所属 lane；
    // Tempo 的 conductor lane track 恒为 0（语义占位），不参与 track 匹配。
    // Select 工具的选中状态由 anchor_sel_rects 决定：从 lanes 筛选落在任一 sel_rect 内的锚点。
    let mut highlight_ticks: Vec<u32> = Vec::new();
    for l in &lanes {
        for e in &l.events {
            if panel
                .anchor_sel_rects
                .iter()
                .any(|r| r.contains(e.tick, e.value))
            {
                highlight_ticks.push(e.tick);
            }
        }
    }
    if let Some(InfoContent::Anchor {
        target: anchor_target,
        track_idx,
        event_idx,
        ..
    }) = info_content
        && *anchor_target == panel.selected_target
    {
        // 通过 event_idx 定位锚点的实际 tick
        if let Some(tick) = lanes
            .iter()
            .find(|l| {
                l.target == panel.selected_target
                    && (l.target == AutomationTarget::Tempo || l.track == *track_idx)
            })
            .and_then(|l| l.events.get(*event_idx))
            .map(|e| e.tick)
            && !highlight_ticks.contains(&tick)
        {
            highlight_ticks.push(tick);
        }
    }

    let gpu_dirty = prepare_automation(
        renderer,
        gw,
        gh,
        panel,
        &lanes,
        midi,
        track_visible,
        track_colors,
        scroll_mode,
        min_border_width,
        show_anchors,
        max_val_f,
        panel_ghost,
        revision,
        &highlight_ticks,
    );

    let content_changed = panel.dirty || gpu_dirty;
    panel.dirty = false;

    let painter = ui.painter();
    let theme = renderer.theme();

    // ── Background（app_bg 一层 + 条纹着色底；combo 列由 show_target_combo 画）──
    let content_rect = egui::Rect::from_min_max(
        egui::pos2(grid_rect.min.x + combo_width, grid_rect.min.y),
        grid_rect.max,
    );
    painter.rect_filled(content_rect, 0.0, crate::theme::stripe_bg());

    // ── Center line (only for targets that have one) ──
    if !panel.show_velocity {
        let target = &panel.selected_target;
        let max_val = target.max_value();
        if max_val > 0.0 && target.has_center_line() {
            let center_val = target.default_value();
            let y_center = panel.value_to_y(center_val, max_val);
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(content_rect.min.x, content_rect.min.y + y_center - 0.5),
                    egui::vec2(content_rect.width(), 1.0),
                ),
                0.0,
                crate::theme::rgba_to_color32(theme.center_line),
            );
        }
    }

    // ── Grid lines (egui, before wgpu texture) ──
    // automation 不补 ruler（共享 pianoroll 顶部 ruler），但网格线需要画。
    if let Some(midi) = midi
        && let Some(tpb) = midi.ticks_per_beat()
    {
        let (def_num, def_den) = midi.time_sig_default();
        let sig_events = midi.time_sig_events();
        let grid_draw_rect = egui::Rect::from_min_max(
            egui::pos2(grid_rect.min.x + combo_width, grid_rect.min.y),
            grid_rect.max,
        );
        crate::widgets::grid_lines::paint_grid_lines(
            painter,
            grid_draw_rect,
            &panel.base,
            tpb,
            def_num,
            def_den,
            sig_events,
            &crate::widgets::grid_lines::GridColors::pianoroll(),
        );
    }

    render_ctx.paint(
        renderer,
        gpw,
        gph,
        &format!("auto_panel_{}", panel_index),
        painter,
        grid_rect,
        content_changed,
    );
}

/// 左侧 target 选择器：图标按钮 + 弹出菜单（velocity / curated targets / 自定义 CC）。
fn show_target_combo(
    ui: &mut egui::Ui,
    panel: &mut AutomationPanelView,
    combo_rect: egui::Rect,
    panels_area_rect: egui::Rect,
    editing_is_conductor: bool,
) {
    // combo 列自己的背景（内容区背景只铺到 combo_width 之后）
    ui.painter().rect_filled(combo_rect, 0.0, theme::app_bg());

    let combo_inner = combo_rect.shrink(4.0);

    ui.scope_builder(egui::UiBuilder::new().max_rect(combo_inner), |ui| {
        ui.set_clip_rect(combo_inner.intersect(panels_area_rect));
        let layout = egui::Layout::top_down(egui::Align::Center);
        ui.with_layout(layout, |ui| {
            // ── Target selector button (tools panel style) ──
            let target_resp = crate::widgets::hover::hover_button(
                ui,
                ICON_AUTOMATION.codepoint,
                egui::FontId::new(crate::theme::ICON_FONT, ICON_AUTOMATION.font_family()),
                crate::theme::text_label(),
                false,
            );

            // ── Popup menu (manually managed Area to support DragValue interaction) ──
            let popup_id = ui.id().with("auto_target_popup");
            let is_open = ui
                .data_mut(|d| d.get_persisted::<bool>(popup_id))
                .unwrap_or(false);

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
                            // Conductor 下只可编辑 Tempo：Velocity / 其他 target / 自定义 CC 都不提供。
                            if !editing_is_conductor {
                                // Velocity (special: not an AutomationTarget, renders from notes)
                                let vel_selected = panel.show_velocity;
                                if ui
                                    .add(egui::Button::selectable(
                                        vel_selected,
                                        t!("automation.velocity").as_ref(),
                                    ))
                                    .clicked()
                                {
                                    panel.show_velocity = true;
                                    panel.dirty = true;
                                    ui.ctx().data_mut(|d| d.insert_persisted(popup_id, false));
                                }
                                ui.separator();
                            }
                            for target in AUTOMATION_TARGETS {
                                // Conductor 下只保留 Tempo。
                                if editing_is_conductor && *target != AutomationTarget::Tempo {
                                    continue;
                                }
                                let name = target.display_name();
                                let selected =
                                    !panel.show_velocity && panel.selected_target == *target;
                                if ui.add(egui::Button::selectable(selected, &name)).clicked() {
                                    panel.selected_target = target.clone();
                                    panel.show_velocity = false;
                                    panel.dirty = true;
                                    ui.ctx().data_mut(|d| d.insert_persisted(popup_id, false));
                                }
                            }
                            if !editing_is_conductor {
                                ui.separator();
                                ui.label(t!("automation.custom_cc").as_ref());
                                let mut cc_input = match &panel.selected_target {
                                    AutomationTarget::CC { controller } => *controller as i32,
                                    _ => 0,
                                };
                                let old_cc = cc_input;
                                ui.add(
                                    crate::widgets::numeric_input::decimal_drag_value(
                                        &mut cc_input,
                                    )
                                    .range(0..=127)
                                    .speed(1),
                                );
                                if cc_input != old_cc {
                                    panel.selected_target = AutomationTarget::CC {
                                        controller: cc_input as u8,
                                    };
                                    panel.show_velocity = false;
                                    panel.dirty = true;
                                }
                            }
                        });
                    });

                // Close only when clicking outside the popup area (not on any interactive element)
                if ui.input(|i| i.pointer.any_pressed())
                    && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
                    && !area_resp.response.rect.contains(pos)
                    && !target_resp.rect.contains(pos)
                {
                    ui.data_mut(|d| d.insert_persisted(popup_id, false));
                }
            }

            ui.add_space(4.0);
        });
    });
}

/// 右键锚点：设置 info_content 打开信息面板，并清理 interaction 记录的 temp data。
fn apply_right_click_anchor(
    ui: &mut egui::Ui,
    panel_count: usize,
    automation_lanes: &[AutomationLane],
    info_content: &mut Option<InfoContent>,
    right_tab: &mut Option<RightTab>,
) {
    for i in 0..panel_count {
        let right_click_id = ui.id().with("auto_right_click").with(i);
        if let Some(anchor) = ui
            .ctx()
            .data(|d| d.get_temp::<interaction::RightClickAnchor>(right_click_id))
        {
            // 通过 tick 查找 event_idx
            let event_idx = automation_lanes
                .iter()
                .find(|l| l.target == anchor.target)
                .and_then(|l| l.events.iter().position(|e| e.tick == anchor.old_tick))
                .unwrap_or(0);

            *info_content = Some(InfoContent::Anchor {
                track_idx: anchor.track_idx,
                lane_idx: anchor.lane_idx,
                event_idx,
                target: anchor.target.clone(),
            });
            *right_tab = Some(RightTab::Info);

            // 清理 temp data
            let edit_tick_id = ui.id().with("auto_right_tick").with(i);
            let edit_value_id = ui.id().with("auto_right_value").with(i);
            let was_open_id = ui.id().with("auto_right_was_open").with(i);
            ui.ctx().data_mut(|d| {
                d.remove::<interaction::RightClickAnchor>(right_click_id);
                d.remove::<f64>(edit_tick_id);
                d.remove::<f64>(edit_value_id);
                d.remove::<bool>(was_open_id);
            });
        }
    }
}

/// Show the toggle / add / remove buttons horizontally.
///
/// Designed to be called inside a `ui.horizontal()` or `ui.horizontal_centered()`
/// scope (e.g. inside the scrollbar left blank area).
pub fn show_toggle_buttons(ui: &mut egui::Ui, show_panels: &mut bool, panel_count: &mut usize) {
    ui.spacing_mut().item_spacing.x = 6.0;
    ui.add_space(6.0);

    // Toggle button
    let toggle_resp = crate::widgets::hover::hover_button(
        ui,
        ICON_SIGNAL_CELLULAR_ALT.codepoint,
        egui::FontId::new(
            crate::theme::PANEL_TOGGLE_FONT,
            ICON_SIGNAL_CELLULAR_ALT.font_family(),
        ),
        crate::theme::text_label(),
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
        let plus_resp = crate::widgets::hover::hover_button(
            ui,
            ICON_ADD.codepoint,
            egui::FontId::new(crate::theme::PANEL_TOGGLE_FONT, ICON_ADD.font_family()),
            crate::theme::text_label(),
            false,
        );
        if plus_resp.clicked() {
            *panel_count += 1;
        }

        // - button (remove panel)
        let minus_resp = crate::widgets::hover::hover_button(
            ui,
            ICON_REMOVE.codepoint,
            egui::FontId::new(crate::theme::PANEL_TOGGLE_FONT, ICON_REMOVE.font_family()),
            crate::theme::text_label(),
            false,
        );
        if minus_resp.clicked() && *panel_count > 0 {
            *panel_count -= 1;
        }
    }
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
        let _ = ctx.run_ui(raw, |ui| {
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
            )
            .automation_edits;
        });
        edits
    }

    /// 回归测试：Tempo 编辑不依赖 active_track——没有任何编辑目标
    /// （editing_track=None）时 Tempo 锚点依然可拖拽（Conductor 下编辑的前提）。
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
