use eframe::egui;
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::*;
use rust_i18n::t;

/// 浮动工具条按钮对应的讲解行 key。
fn action_hint(action: SelectionAction) -> String {
    match action {
        SelectionAction::Delete => t!("hint.pr.sel_delete"),
        SelectionAction::Duplicate => t!("hint.pr.sel_duplicate"),
        SelectionAction::TransposeUp => t!("hint.pr.sel_transpose_up"),
        SelectionAction::TransposeDown => t!("hint.pr.sel_transpose_down"),
        SelectionAction::FlipHorizontal => t!("hint.pr.sel_flip_h"),
        SelectionAction::FlipVertical => t!("hint.pr.sel_flip_v"),
        SelectionAction::LengthDouble => t!("hint.pr.sel_length_double"),
        SelectionAction::LengthHalf => t!("hint.pr.sel_length_half"),
        SelectionAction::Humanize => t!("hint.pr.sel_humanize"),
        SelectionAction::FillGaps => t!("hint.pr.sel_fill_gaps"),
        SelectionAction::MergeNotes => t!("hint.pr.sel_merge_notes"),
        SelectionAction::GridConfirm => t!("hint.pr.sel_grid_split"),
        SelectionAction::LineConfirm | SelectionAction::ScissorsConfirm => {
            t!("hint.pr.sel_confirm")
        }
    }
    .to_string()
}

/// Actions that can be triggered from the floating action bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionAction {
    Delete,
    Duplicate,
    TransposeUp,
    TransposeDown,
    FlipHorizontal,
    FlipVertical,
    /// 网格切割：按当前量化切分框内音符（transport bar 按钮）。
    GridConfirm,
    /// 直线工具：确认按锚点线生成音符。
    LineConfirm,
    /// 剪刀工具：确认按锚点线切割音符。
    ScissorsConfirm,
    /// 变速：选中音符长度 ×2。
    LengthDouble,
    /// 变速：选中音符长度 ÷2。
    LengthHalf,
    /// 人性化：给选中音符注入随机偏差（时间/力度/时值）。
    Humanize,
    /// 填缝：把框选内音符终点贴合到同轨道下一个不同起点（消除断续与叠音）。
    FillGaps,
    /// 合并：把框选内首尾严格相接的同 key 音符合并成一个。
    MergeNotes,
}

/// 浮动工具条按钮：`(icon, action)`。
pub type BarButton = (MaterialIcon, SelectionAction);

/// 选择工具的浮动条按钮（删除/复制/移调/变速/翻转/人性化/填缝/合并/网格切割）。
/// 图标旋转由 [`bar_button_rotation`] 决定（如 DOWNLOAD 倒置 180° 表示升八度）。
pub const SELECT_BAR_BUTTONS: [BarButton; 12] = [
    (ICON_DELETE, SelectionAction::Delete),
    (ICON_CONTENT_COPY, SelectionAction::Duplicate),
    (ICON_DOWNLOAD, SelectionAction::TransposeUp),
    (ICON_DOWNLOAD, SelectionAction::TransposeDown),
    (ICON_DOWNLOAD, SelectionAction::LengthHalf),
    (ICON_UPLOAD, SelectionAction::LengthDouble),
    (ICON_FLIP, SelectionAction::FlipHorizontal),
    (ICON_FLIP, SelectionAction::FlipVertical),
    (ICON_PERSON, SelectionAction::Humanize),
    (ICON_COMPARE_ARROWS, SelectionAction::FillGaps),
    (ICON_LINE_STYLE, SelectionAction::MergeNotes),
    (ICON_GRID_ON, SelectionAction::GridConfirm),
];

/// 图标旋转角（弧度）。0 = 不旋转。
/// - 升八度：DOWNLOAD 倒置 180°；
/// - 变速 ×2：UPLOAD 顺时针 90°；
/// - 变速 ÷2：DOWNLOAD 顺时针 90°；
/// - 垂直翻转：FLIP 旋转 90°。
fn bar_button_rotation(action: SelectionAction) -> f32 {
    use std::f32::consts::{FRAC_PI_2, PI};
    match action {
        SelectionAction::TransposeUp => PI,
        SelectionAction::LengthDouble => FRAC_PI_2,
        SelectionAction::LengthHalf => FRAC_PI_2,
        SelectionAction::FlipVertical => FRAC_PI_2,
        _ => 0.0,
    }
}

/// Gap between selection box right edge and the floating bar.
const GAP: f32 = 8.0;
/// transport bar 上选择动作按钮之间的间距。
const TRANSPORT_GAP: f32 = 4.0;
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
/// `btn_count` 决定条高（直线/剪刀工具各 1 个确认按钮）。
pub fn bar_rect(
    content_rect: egui::Rect,
    sel_view_rect: egui::Rect,
    btn_count: usize,
) -> Option<egui::Rect> {
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

    let bar_w = ICON_SIZE + H_PAD * 2.0;
    let bar_h = ICON_SIZE * btn_count as f32
        + V_PAD * 2.0
        + btn_count.saturating_sub(1) as f32 * BTN_SPACING;

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

/// 在 transport bar 上横排绘制选择动作按钮
/// （删除/复制/移调↑/移调↓/变速×2/变速÷2/水平翻转/垂直翻转）。
///
/// 常驻显示；`enabled == false`（无框选）时灰显且不可点。
/// `hovered_hint` 在可用且指针悬停时写入讲解行文案。
/// 返回本帧被点击的动作。
pub fn show_transport_buttons(
    ui: &mut egui::Ui,
    enabled: bool,
    hovered_hint: &mut Option<String>,
) -> Option<SelectionAction> {
    let mut clicked = None;
    for (i, &(icon, action)) in SELECT_BAR_BUTTONS.iter().enumerate() {
        let font = egui::FontId::new(crate::theme::TRANSPORT_BTN_FONT, icon.font_family());
        let angle = bar_button_rotation(action);
        let resp = if angle == 0.0 {
            crate::widgets::hover::hover_button_enabled(
                ui,
                icon.codepoint,
                font,
                crate::theme::text_label(),
                false,
                enabled,
            )
        } else {
            crate::widgets::hover::hover_button_enabled_rotated(
                ui,
                icon.codepoint,
                font,
                crate::theme::text_label(),
                false,
                angle,
                enabled,
            )
        };
        if enabled && resp.clicked() {
            clicked = Some(action);
        }
        // 悬停即显示讲解（禁用态也提示，便于用户了解按钮用途）。
        if resp.hovered() {
            *hovered_hint = Some(action_hint(action));
        }
        if i + 1 < SELECT_BAR_BUTTONS.len() {
            ui.add_space(TRANSPORT_GAP);
        }
    }
    clicked
}

/// Show a vertical floating action bar to the right of the selection box.
///
/// `buttons` 决定按钮数量与动作（直线/剪刀工具各一个确认按钮）。
/// Returns the action that was clicked, if any.
pub fn show(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    sel_view_rect: Option<egui::Rect>,
    buttons: &[BarButton],
) -> Option<SelectionAction> {
    let sel = sel_view_rect?;
    let btn_count = buttons.len();
    if btn_count == 0 {
        return None;
    }

    // 几何与命中测试共享同一实现（bar_rect），避免两处重复计算。
    let bar = bar_rect(content_rect, sel, btn_count)?;

    // Bar dimensions（按钮排布用）
    let bar_w = ICON_SIZE + H_PAD * 2.0;

    // Draw background pill (rounded rect with semi-circle ends)
    let bg_color = crate::theme::line_fg();
    let corner_radius = bar_w / 2.0;
    ui.painter().rect_filled(bar, corner_radius, bg_color);

    let mut result = None;
    let pointer_pos = ui.input(|i| i.pointer.hover_pos());
    let released = ui.input(|i| i.pointer.primary_released());
    let pressed = ui.input(|i| i.pointer.primary_pressed());

    // 预计算按钮矩形（press 帧也要用）。
    let btn_rects: Vec<egui::Rect> = (0..btn_count)
        .map(|i| {
            let btn_y = bar.min.y + V_PAD + i as f32 * (ICON_SIZE + BTN_SPACING);
            egui::Rect::from_min_max(
                egui::pos2(bar.min.x, btn_y),
                egui::pos2(bar.max.x, btn_y + ICON_SIZE),
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

    for (i, &(icon, action)) in buttons.iter().enumerate() {
        let btn_rect = btn_rects[i];

        // Hover detection（hover 变白 + 统一增益底色，与全项目图标按钮基准风格一致）
        let hovered = pointer_pos.is_some_and(|p| btn_rect.contains(p));
        if hovered {
            crate::widgets::hint::set(ui.ctx(), action_hint(action));
        }
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

        // Draw icon（按动作旋转，如垂直翻转 = FLIP 旋转 90°）
        let icon_font_id = egui::FontId::new(ICON_SIZE, icon.font_family());
        let angle = bar_button_rotation(action);
        if angle == 0.0 {
            ui.painter().text(
                btn_rect.center(),
                egui::Align2::CENTER_CENTER,
                icon.codepoint,
                icon_font_id,
                color,
            );
        } else {
            let galley =
                ui.painter()
                    .layout_no_wrap(icon.codepoint.to_string(), icon_font_id, color);
            let pos = egui::Align2::CENTER_CENTER
                .anchor_size(btn_rect.center(), galley.size())
                .min;
            ui.painter().add(
                egui::epaint::TextShape::new(pos, galley, color)
                    .with_angle_and_anchor(angle, egui::Align2::CENTER_CENTER),
            );
        }

        // Manual click detection: press and release on the same button,
        // with release still hovering it (press 起点记录防拖拽穿透误触)。
        if released && hovered && press_btn == Some(i) {
            result = Some(action);
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

    /// 浮动 ✓ 条（Grid/Line/Scissors 工具的 1 个确认按钮）几何：
    /// 位于选框右侧、贴近右边缘时隐藏。
    #[test]
    fn bar_rect_normal() {
        let sel = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(200.0, 50.0));
        let bar = bar_rect(content(), sel, 1);
        assert!(bar.is_some());
        let bar = bar.unwrap();
        assert!(bar.min.x > sel.max.x);
    }

    #[test]
    fn bar_rect_near_right_edge_is_hidden() {
        // 选框贴近右边缘 → 条会被裁掉，返回 None。
        let sel = egui::Rect::from_min_size(egui::pos2(600.0, 100.0), egui::vec2(180.0, 50.0));
        let bar = bar_rect(content(), sel, 1);
        assert!(bar.is_none(), "bar should be clipped at right edge");
    }

    #[test]
    fn bar_rect_wide_content() {
        let wide = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(2000.0, 600.0));
        let sel = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(200.0, 50.0));
        let bar = bar_rect(wide, sel, 1);
        assert!(bar.is_some());
    }

    /// 选择按钮表：动作不重复（防复制粘贴时漏改），且每个动作都有讲解文案来源。
    /// 新按钮追加末尾，数量变化时此测试会提醒同步提示文案。
    #[test]
    fn select_bar_buttons_are_unique() {
        assert_eq!(SELECT_BAR_BUTTONS.len(), 12);
        for (i, &(_, a)) in SELECT_BAR_BUTTONS.iter().enumerate() {
            for &(_, b) in &SELECT_BAR_BUTTONS[i + 1..] {
                assert_ne!(a, b, "动作重复绑定: {a:?}");
            }
        }
        // 讲解行必须能取到非空文案（缺失 key 会回退为 key 本身，也非空；
        // 真正的 key 完整性由 i18n_tests 覆盖）。
        for &(_, action) in SELECT_BAR_BUTTONS.iter() {
            assert!(!action_hint(action).is_empty(), "缺少讲解文案: {action:?}");
        }
    }
}
