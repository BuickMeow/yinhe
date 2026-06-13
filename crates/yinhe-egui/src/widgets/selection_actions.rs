use eframe::egui;
use egui_material_icons::icons::*;

/// Actions that can be triggered from the floating action bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionAction {
    Delete,
    Duplicate,
    TransposeUp,
    TransposeDown,
}

/// Gap between selection box right edge and the floating bar.
const GAP: f32 = 8.0;
/// Button icon size.
const ICON_SIZE: f32 = 18.0;
/// Horizontal padding inside the pill.
const H_PAD: f32 = 8.0;
/// Vertical padding inside the pill.
const V_PAD: f32 = 6.0;
/// Spacing between buttons.
const BTN_SPACING: f32 = 4.0;

/// Compute the screen-space rect of the floating action bar for a given
/// selection rect, or `None` if the bar would be clipped / off-screen.
/// This is used by `sel_drag_frame` to detect clicks on the bar.
pub fn compute_bar_rect(
    content_rect: egui::Rect,
    sel_view_rect: egui::Rect,
) -> Option<egui::Rect> {
    let sel_screen = egui::Rect::from_min_max(
        egui::pos2(content_rect.min.x + sel_view_rect.min.x, content_rect.min.y + sel_view_rect.min.y),
        egui::pos2(content_rect.min.x + sel_view_rect.max.x, content_rect.min.y + sel_view_rect.max.y),
    );

    let btn_count = 4;
    let bar_w = ICON_SIZE + H_PAD * 2.0;
    let bar_h = ICON_SIZE * btn_count as f32 + V_PAD * 2.0 + (btn_count - 1) as f32 * BTN_SPACING;

    let bar_x = sel_screen.max.x + GAP;
    let bar_y = sel_screen.center().y - bar_h / 2.0;

    let bar_rect = egui::Rect::from_min_max(
        egui::pos2(bar_x, bar_y),
        egui::pos2(bar_x + bar_w, bar_y + bar_h),
    );

    let bar_rect = egui::Rect::from_min_max(
        egui::pos2(bar_rect.min.x, bar_rect.min.y.max(content_rect.min.y)),
        egui::pos2(bar_rect.max.x, bar_rect.max.y.min(content_rect.max.y)),
    );

    if bar_rect.max.x > content_rect.max.x - 4.0 {
        return None;
    }
    let visible_h = bar_rect.height();
    if visible_h < bar_h * 0.5 {
        return None;
    }

    Some(bar_rect)
}

/// Show a vertical floating action bar to the right of the selection box.
///
/// Returns the action that was clicked, if any.
pub fn show(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    sel_view_rect: Option<egui::Rect>,
) -> Option<SelectionAction> {
    let sel = sel_view_rect?;

    // Convert view-local to screen coordinates
    let sel_screen = egui::Rect::from_min_max(
        egui::pos2(content_rect.min.x + sel.min.x, content_rect.min.y + sel.min.y),
        egui::pos2(content_rect.min.x + sel.max.x, content_rect.min.y + sel.max.y),
    );

    // Bar dimensions
    let btn_count = 4;
    let bar_w = ICON_SIZE + H_PAD * 2.0;
    let bar_h = ICON_SIZE * btn_count as f32 + V_PAD * 2.0 + (btn_count - 1) as f32 * BTN_SPACING;

    // Position: right of selection, vertically centered
    let bar_x = sel_screen.max.x + GAP;
    let bar_y = sel_screen.center().y - bar_h / 2.0;

    let bar_rect = egui::Rect::from_min_max(
        egui::pos2(bar_x, bar_y),
        egui::pos2(bar_x + bar_w, bar_y + bar_h),
    );

    // Clamp to content area vertically
    let bar_rect = egui::Rect::from_min_max(
        egui::pos2(bar_rect.min.x, bar_rect.min.y.max(content_rect.min.y)),
        egui::pos2(bar_rect.max.x, bar_rect.max.y.min(content_rect.max.y)),
    );

    // Don't show if too close to right edge
    if bar_rect.max.x > content_rect.max.x - 4.0 {
        return None;
    }

    // Don't show if clipped too much vertically
    let visible_h = bar_rect.height();
    if visible_h < bar_h * 0.5 {
        return None;
    }

    // Draw background pill (rounded rect with semi-circle ends)
    let bg_color = egui::Color32::from_rgba_premultiplied(30, 30, 35, 210);
    let corner_radius = bar_w / 2.0;
    ui.painter().rect_filled(bar_rect, corner_radius, bg_color);

    // Draw buttons
    let icons = [ICON_DELETE, ICON_CONTENT_COPY, ICON_KEYBOARD_ARROW_UP, ICON_KEYBOARD_ARROW_DOWN];
    let actions = [
        SelectionAction::Delete,
        SelectionAction::Duplicate,
        SelectionAction::TransposeUp,
        SelectionAction::TransposeDown,
    ];

    let mut result = None;
    let pointer_pos = ui.input(|i| i.pointer.hover_pos());
    let released = ui.input(|i| i.pointer.primary_released());

    for (i, (&icon, action)) in icons.iter().zip(actions.iter()).enumerate() {
        let btn_y = bar_rect.min.y + V_PAD + i as f32 * (ICON_SIZE + BTN_SPACING);
        let btn_rect = egui::Rect::from_min_max(
            egui::pos2(bar_rect.min.x, btn_y),
            egui::pos2(bar_rect.max.x, btn_y + ICON_SIZE),
        );

        // Hover detection
        let hovered = pointer_pos.is_some_and(|p| btn_rect.contains(p));
        let color = if hovered {
            crate::widgets::theme::ACCENT_ACTIVE
        } else {
            egui::Color32::GRAY
        };

        // Draw icon
        let icon_font_id = egui::FontId::new(ICON_SIZE, icon.font_family());
        ui.painter().text(
            btn_rect.center(),
            egui::Align2::CENTER_CENTER,
            icon.codepoint,
            icon_font_id,
            color,
        );

        // Manual click detection using primary_released (not consumed by widgets)
        if released && hovered {
            result = Some(*action);
        }
    }

    result
}
