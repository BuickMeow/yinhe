use eframe::egui;

use yinhe_types::AutomationPanelView;

use super::types::PanelPianorollFeedback;

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
pub(crate) fn handle_panel_scroll_zoom(
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
    let apply_vertical_zoom = |panel: &mut AutomationPanelView, factor: f32| {
        panel.value_zoom = (panel.value_zoom * factor).clamp(zoom_min, 8.0);
        panel.clamp_value_scroll(max_val_f);
        panel.dirty = true;
        ui.ctx().request_repaint();
    };
    let Some(p) = pointer_pos else { return };
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return;
    }
    if grid_area.contains(p) {
        if (zoom_delta - 1.0).abs() > 0.001 {
            feedback.zoom_factor = zoom_delta;
            feedback.zoom_center_x = p.x - panel_rect.min.x;
        }
        if cmd && scroll_delta.y.abs() > 0.5 {
            let factor = if scroll_delta.y > 0.0 { 1.0 / 1.1 } else { 1.1 };
            feedback.zoom_factor = factor;
            feedback.zoom_center_x = p.x - panel_rect.min.x;
        }
        if !cmd && scroll_delta.x.abs() > 0.5 {
            feedback.scroll_x_delta += scroll_delta.x;
        }
        if !cmd && scroll_delta.y.abs() > 0.5 && max_scroll <= 0.0 {
            let visible_range = max_val_f / panel.value_zoom;
            let scroll_amount = (scroll_delta.y / 100.0) * visible_range * 0.2;
            let max_scroll_val = (max_val_f - visible_range).max(0.0);
            panel.value_scroll = (panel.value_scroll + scroll_amount).clamp(0.0, max_scroll_val);
            panel.dirty = true;
            ui.ctx().request_repaint();
        }
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
        if (zoom_delta - 1.0).abs() > 0.001 {
            apply_vertical_zoom(panel, zoom_delta);
        }
        if cmd && scroll_delta.y.abs() > 0.5 {
            let factor = if scroll_delta.y > 0.0 { 1.0 / 1.1 } else { 1.1 };
            apply_vertical_zoom(panel, factor);
        }
    }
}
