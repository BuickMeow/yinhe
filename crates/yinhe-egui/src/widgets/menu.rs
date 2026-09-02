//! 菜单项按钮的通用样式（铺满整行 + 无边框 + 左右=上下）。

use eframe::egui;

/// 菜单项按钮：铺满整行宽度 + 左对齐 + 左右边距≈上下边距。
///
/// 高度从 30 → 27 稍微缩减一点点上下边距，左内边距 7 与上下 (27-文本≈13)/2≈7 对齐，
/// 文字左对齐而非居中，避免 `Button` 居中导致的左宽右窄。
pub(crate) fn menu_item_button(
    ui: &egui::Ui,
    selected: bool,
    text: impl Into<egui::WidgetText>,
) -> MenuItemButton {
    let width = ui.available_width();
    let text = text.into();
    MenuItemButton {
        selected,
        text,
        width,
        height: 27.0,
        wrap: None,
        shortcut: None,
    }
}

pub(crate) struct MenuItemButton {
    selected: bool,
    text: egui::WidgetText,
    width: f32,
    height: f32,
    wrap: Option<egui::TextWrapMode>,
    shortcut: Option<egui::WidgetText>,
}

impl MenuItemButton {
    pub fn min_size(mut self, size: egui::Vec2) -> Self {
        if size.x > 0.0 {
            self.width = size.x;
        }
        if size.y > 0.0 {
            self.height = size.y;
        }
        self
    }

    pub fn wrap_mode(mut self, wrap: egui::TextWrapMode) -> Self {
        self.wrap = Some(wrap);
        self
    }

    pub fn shortcut_text(mut self, text: impl Into<egui::WidgetText>) -> Self {
        self.shortcut = Some(text.into());
        self
    }
}

impl egui::Widget for MenuItemButton {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        // 若通过 `ui.put(rect, widget)` 放入（如 transport_bar 的 popup_menu_row
        // 行高 row_h = interact_size.min(24)），则等待高度由父容器决定，
        // 此时 available_height ≈ row_h（20 左右），应跟随父容器而非固定 27，
        // 否则行间距会被撑到 27+spacing 导致测试失败。
        let avail_h = ui.available_height();
        let height = if avail_h > 0.0 && avail_h < self.height {
            avail_h
        } else if ui.max_rect().height() > 0.0 && ui.max_rect().height() < self.height {
            ui.max_rect().height()
        } else {
            self.height
        };
        // 左边距≈上下边距：上下 = (height-13)/2，左取相同值，限制 4..8
        let v_pad = ((height - 13.0) * 0.5).clamp(4.0, 8.0);
        let h_pad = v_pad;
        let desired_size = egui::vec2(self.width, height);
        let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::SelectableLabel,
                ui.is_enabled(),
                self.text.text(),
            )
        });

        let visuals = ui.style().interact_selectable(&response, self.selected);
        let rounding = egui::CornerRadius::same(4);

        if self.selected {
            ui.painter().rect_filled(rect, rounding, visuals.bg_fill);
            if visuals.bg_stroke != egui::Stroke::NONE {
                ui.painter().rect_stroke(
                    rect,
                    rounding,
                    visuals.bg_stroke,
                    egui::StrokeKind::Inside,
                );
            }
        } else if response.hovered() {
            ui.painter().rect_filled(rect, rounding, visuals.bg_fill);
        } else if response.has_focus() {
            ui.painter()
                .rect_stroke(rect, rounding, visuals.fg_stroke, egui::StrokeKind::Inside);
        }

        // 主文本左对齐，快捷键右对齐
        let wrap = self.wrap.unwrap_or(egui::TextWrapMode::Truncate);
        if let Some(shortcut) = self.shortcut {
            // 左右分区：中间留 gap
            let gap = 12.0;
            let shortcut_available = 80.0;
            let main_available = (rect.width() - 2.0 * h_pad - gap - shortcut_available).max(0.0);
            let main_galley =
                self.text
                    .into_galley(ui, Some(wrap), main_available, egui::TextStyle::Button);
            let shortcut_galley = shortcut.into_galley(
                ui,
                Some(egui::TextWrapMode::Truncate),
                shortcut_available,
                egui::TextStyle::Button,
            );
            let main_pos = egui::pos2(
                rect.min.x + h_pad,
                rect.center().y - main_galley.size().y * 0.5,
            );
            let shortcut_pos = egui::pos2(
                rect.max.x - h_pad - shortcut_galley.size().x,
                rect.center().y - shortcut_galley.size().y * 0.5,
            );
            ui.painter()
                .galley(main_pos, main_galley, visuals.text_color());
            // 快捷键用弱化色
            let weak_color = ui.visuals().weak_text_color();
            ui.painter()
                .galley(shortcut_pos, shortcut_galley, weak_color);
        } else {
            let available_w = (rect.width() - 2.0 * h_pad).max(0.0);
            let galley =
                self.text
                    .into_galley(ui, Some(wrap), available_w, egui::TextStyle::Button);
            let text_pos = egui::pos2(rect.min.x + h_pad, rect.center().y - galley.size().y * 0.5);
            ui.painter().galley(text_pos, galley, visuals.text_color());
        }

        response
    }
}
