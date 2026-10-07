use eframe::egui;

use yinhe_types::AutomationPanelView;

use super::types::PanelPianorollFeedback;

/// 以 `y_rel`（0=面板顶，1=面板底）处的值为锚点做垂直缩放，
/// 返回 `(new_value_zoom, new_value_scroll)`。缩放后该 y 处的值保持不变。
fn anchored_vertical_zoom(
    value_zoom: f32,
    value_scroll: f32,
    max_val: f32,
    y_rel: f32,
    factor: f32,
    zoom_min: f32,
) -> (f32, f32) {
    let y_rel = y_rel.clamp(0.0, 1.0);
    let visible_old = max_val / value_zoom;
    let anchor_value = value_scroll + (1.0 - y_rel) * visible_old;
    let new_zoom = (value_zoom * factor).clamp(zoom_min, 8.0);
    let visible_new = max_val / new_zoom;
    (new_zoom, anchor_value - (1.0 - y_rel) * visible_new)
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
    let apply_vertical_zoom = |panel: &mut AutomationPanelView, factor: f32, anchor_y: f32| {
        // 以指针所在的值位置为锚点缩放（与其他视图一致）：先取锚点值，
        // 缩放后调 value_scroll 让该值仍落在同一 y。
        let y_rel = anchor_y / panel_rect.height().max(1.0);
        let (nz, ns) = anchored_vertical_zoom(
            panel.value_zoom,
            panel.value_scroll,
            max_val_f,
            y_rel,
            factor,
            zoom_min,
        );
        panel.value_zoom = nz;
        panel.value_scroll = ns;
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
        let anchor_y = p.y - panel_rect.min.y;
        if (zoom_delta - 1.0).abs() > 0.001 {
            apply_vertical_zoom(panel, zoom_delta, anchor_y);
        }
        if cmd && scroll_delta.y.abs() > 0.5 {
            let factor = if scroll_delta.y > 0.0 { 1.0 / 1.1 } else { 1.1 };
            apply_vertical_zoom(panel, factor, anchor_y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value_at(value_scroll: f32, value_zoom: f32, max_val: f32, y_rel: f32) -> f32 {
        value_scroll + (1.0 - y_rel) * (max_val / value_zoom)
    }

    /// 垂直缩放的锚点值在缩放前后保持不变（以指针位置为锚）。
    #[test]
    fn vertical_zoom_preserves_anchor_value() {
        let (max, vz, vs) = (127.0f32, 2.0f32, 10.0f32);
        for y_rel in [0.0f32, 0.25, 0.5, 0.9] {
            let anchor = value_at(vs, vz, max, y_rel);
            let (nz, ns) = anchored_vertical_zoom(vz, vs, max, y_rel, 1.5, 1.0);
            let anchor_after = value_at(ns, nz, max, y_rel);
            assert!(
                (anchor - anchor_after).abs() < 1e-3,
                "y_rel={y_rel}: 锚点值应保持 {anchor} → {anchor_after}"
            );
        }
    }

    /// 缩放到上限/下限时仍不 panic，且锚点公式稳定。
    #[test]
    fn vertical_zoom_clamps() {
        let (max, vz, vs) = (127.0f32, 1.0f32, 0.0f32);
        let (nz, _) = anchored_vertical_zoom(vz, vs, max, 0.5, 0.001, 1.0);
        assert_eq!(nz, 1.0, "不得低于 zoom_min");
        let (nz2, _) = anchored_vertical_zoom(4.0, 0.0, max, 0.5, 100.0, 1.0);
        assert_eq!(nz2, 8.0, "不得高于上限 8");
    }
}
