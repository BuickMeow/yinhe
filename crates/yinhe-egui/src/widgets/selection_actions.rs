use eframe::egui;
use egui_material_icons::icons::*;

/// Actions that can be triggered from the floating action bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionAction {
    Delete,
    Duplicate,
    TransposeUp,
    TransposeDown,
    FlipHorizontal,
    FlipVertical,
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
pub fn compute_bar_rect(content_rect: egui::Rect, sel_view_rect: egui::Rect) -> Option<egui::Rect> {
    let sel_screen = egui::Rect::from_min_max(
        egui::pos2(
            content_rect.min.x + sel_view_rect.min.x,
            content_rect.min.y + sel_view_rect.min.y,
        ),
        egui::pos2(
            content_rect.min.x + sel_view_rect.max.x,
            content_rect.min.y + sel_view_rect.max.y,
        ),
    );

    let btn_count = 6;
    let bar_w = ICON_SIZE + H_PAD * 2.0;
    let bar_h = ICON_SIZE * btn_count as f32 + V_PAD * 2.0 + (btn_count - 1) as f32 * BTN_SPACING;

    let bar_x = sel_screen.max.x + GAP;
    let bar_y = sel_screen.center().y - bar_h / 2.0;

    // 整体平移到 content 区域内（不压缩高度，避免背景与按钮脱节）
    let max_y = (content_rect.max.y - bar_h).max(content_rect.min.y);
    let bar_y = bar_y.clamp(content_rect.min.y, max_y);
    let bar_rect = egui::Rect::from_min_max(
        egui::pos2(bar_x, bar_y),
        egui::pos2(bar_x + bar_w, bar_y + bar_h),
    );

    if bar_rect.max.x > content_rect.max.x - 4.0 {
        return None;
    }
    // content 区域本身太小时隐藏
    if content_rect.height() < bar_h * 0.5 {
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
        egui::pos2(
            content_rect.min.x + sel.min.x,
            content_rect.min.y + sel.min.y,
        ),
        egui::pos2(
            content_rect.min.x + sel.max.x,
            content_rect.min.y + sel.max.y,
        ),
    );

    // Bar dimensions
    let btn_count = 6;
    let bar_w = ICON_SIZE + H_PAD * 2.0;
    let bar_h = ICON_SIZE * btn_count as f32 + V_PAD * 2.0 + (btn_count - 1) as f32 * BTN_SPACING;

    // Position: right of selection, vertically centered
    let bar_x = sel_screen.max.x + GAP;
    let bar_y = sel_screen.center().y - bar_h / 2.0;

    // 整体平移到 content 区域内（不压缩高度，避免背景与按钮脱节）
    let max_y = (content_rect.max.y - bar_h).max(content_rect.min.y);
    let bar_y = bar_y.clamp(content_rect.min.y, max_y);
    let bar_rect = egui::Rect::from_min_max(
        egui::pos2(bar_x, bar_y),
        egui::pos2(bar_x + bar_w, bar_y + bar_h),
    );

    // Don't show if too close to right edge
    if bar_rect.max.x > content_rect.max.x - 4.0 {
        return None;
    }

    // content 区域本身太小时隐藏
    if content_rect.height() < bar_h * 0.5 {
        return None;
    }

    // Draw background pill (rounded rect with semi-circle ends)
    let bg_color = crate::theme::line_fg();
    let corner_radius = bar_w / 2.0;
    ui.painter().rect_filled(bar_rect, corner_radius, bg_color);

    // Draw buttons
    let icons = [
        ICON_DELETE,
        ICON_CONTENT_COPY,
        ICON_KEYBOARD_ARROW_UP,
        ICON_KEYBOARD_ARROW_DOWN,
        ICON_FLIP,
        ICON_FLIP,
    ];
    let actions = [
        SelectionAction::Delete,
        SelectionAction::Duplicate,
        SelectionAction::TransposeUp,
        SelectionAction::TransposeDown,
        SelectionAction::FlipHorizontal,
        SelectionAction::FlipVertical,
    ];

    let mut result = None;
    let pointer_pos = ui.input(|i| i.pointer.hover_pos());
    let released = ui.input(|i| i.pointer.primary_released());
    let pressed = ui.input(|i| i.pointer.primary_pressed());

    // 预计算按钮矩形（press 帧也要用）。
    let btn_rects: Vec<egui::Rect> = (0..btn_count)
        .map(|i| {
            let btn_y = bar_rect.min.y + V_PAD + i as f32 * (ICON_SIZE + BTN_SPACING);
            egui::Rect::from_min_max(
                egui::pos2(bar_rect.min.x, btn_y),
                egui::pos2(bar_rect.max.x, btn_y + ICON_SIZE),
            )
        })
        .collect();

    // 记录按下时所在的按钮：只有 press 和 release 落在同一个按钮上才算点击。
    // 否则从音乐区拖拽到工具条上释放会误触发按钮动作（事件穿透的反向）。
    let press_btn_id = ui.id().with("sel_bar_press_btn");
    let mut press_btn: Option<usize> = ui
        .data_mut(|d| d.get_persisted(press_btn_id))
        .unwrap_or(None);
    if pressed {
        press_btn = pointer_pos.and_then(|p| btn_rects.iter().position(|r| r.contains(p)));
        ui.data_mut(|d| d.insert_persisted(press_btn_id, press_btn));
    }

    for (i, (&icon, action)) in icons.iter().zip(actions.iter()).enumerate() {
        let btn_rect = btn_rects[i];

        // Hover detection（hover 变白 + 统一增益底色，与全项目图标按钮基准风格一致）
        let hovered = pointer_pos.is_some_and(|p| btn_rect.contains(p));
        if hovered {
            let down = pressed && press_btn == Some(i);
            let bg = if down {
                crate::theme::pressed_color(crate::theme::app_bg())
            } else {
                crate::theme::hover_color(crate::theme::app_bg())
            };
            ui.painter().rect_filled(btn_rect, 4.0, bg);
        }
        let color = if hovered {
            crate::theme::contrast_fg()
        } else {
            crate::theme::text_label()
        };

        // Draw icon
        let icon_font_id = egui::FontId::new(ICON_SIZE, icon.font_family());
        if action == &SelectionAction::FlipVertical {
            // ICON_FLIP 旋转 90° = 垂直翻转（绕按钮中心）
            let galley =
                ui.painter()
                    .layout_no_wrap(icon.codepoint.to_string(), icon_font_id, color);
            let pos = egui::Align2::CENTER_CENTER
                .anchor_size(btn_rect.center(), galley.size())
                .min;
            ui.painter().add(
                egui::epaint::TextShape::new(pos, galley, color).with_angle_and_anchor(
                    std::f32::consts::FRAC_PI_2,
                    egui::Align2::CENTER_CENTER,
                ),
            );
        } else {
            ui.painter().text(
                btn_rect.center(),
                egui::Align2::CENTER_CENTER,
                icon.codepoint,
                icon_font_id,
                color,
            );
        }

        // Manual click detection: press and release on the same button,
        // with release still hovering it (press 起点记录防拖拽穿透误触)。
        if released && hovered && press_btn == Some(i) {
            result = Some(*action);
        }
    }

    // release 后清除 press 记录，避免影响下一次点击。
    if released {
        ui.data_mut(|d| d.insert_persisted(press_btn_id, Option::<usize>::None));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))
    }

    #[test]
    fn compute_bar_rect_normal() {
        let sel = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(200.0, 50.0));
        let bar = compute_bar_rect(content(), sel);
        assert!(bar.is_some());
        let bar = bar.unwrap();
        // bar should be to the right of selection
        assert!(bar.min.x > sel.max.x);
    }

    #[test]
    fn compute_bar_rect_near_right_edge() {
        // Selection near right edge → bar would be clipped
        let sel = egui::Rect::from_min_size(egui::pos2(600.0, 100.0), egui::vec2(180.0, 50.0));
        let bar = compute_bar_rect(content(), sel);
        assert!(bar.is_none(), "bar should be clipped at right edge");
    }

    #[test]
    fn compute_bar_rect_small_selection() {
        let sel = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(20.0, 20.0));
        let bar = compute_bar_rect(content(), sel);
        assert!(bar.is_some());
    }

    #[test]
    fn compute_bar_rect_wide_content() {
        let wide = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(2000.0, 600.0));
        let sel = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(200.0, 50.0));
        let bar = compute_bar_rect(wide, sel);
        assert!(bar.is_some());
    }
}
