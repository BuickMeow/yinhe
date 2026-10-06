//! 面板排版原语：章节标题 / 分隔 / 键值行 / 表单行 / 列表行。
//!
//! 右栏各面板（track/selection/anchor/layers/history/summary/event_browser）过去
//! 各写各的标题、标签值行、行高与缩进，视觉不统一。这里把共性收敛成少量原语：
//! 字号一律经 [`scaled_font`] 包装（跟随字体缩放），颜色取语义色，几何取自
//! [`crate::theme`] 的排版 token。

use eframe::egui;

use crate::scaling::scaled_font;
use crate::theme;

/// 章节标题：粗体 + `SUB_TITLE_FONT` + `text_bright`。
pub(crate) fn section_header(ui: &mut egui::Ui, text: &str) {
    ui.add_space(theme::GAP_SM);
    ui.label(
        egui::RichText::new(text)
            .strong()
            .size(scaled_font(ui.ctx(), theme::SUB_TITLE_FONT))
            .color(theme::text_bright()),
    );
}

/// 分隔块：`GAP` + 分隔线 + `GAP`。
pub(crate) fn divider(ui: &mut egui::Ui) {
    ui.add_space(theme::GAP);
    ui.separator();
    ui.add_space(theme::GAP);
}

/// 只读键值行：标签（`SMALL_FONT` + `text_label`）+ 值（`BODY_FONT` + `text_bright`）。
pub(crate) fn value_row(ui: &mut egui::Ui, label: impl Into<String>, value: impl Into<String>) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(label.into())
                .size(scaled_font(ui.ctx(), theme::SMALL_FONT))
                .color(theme::text_label()),
        );
        ui.label(
            egui::RichText::new(value.into())
                .size(scaled_font(ui.ctx(), theme::BODY_FONT))
                .color(theme::text_bright()),
        );
    });
}

/// 表单行：固定宽标签列 + 右侧自定义控件（几何随字体缩放）。
pub(crate) fn form_row(ui: &mut egui::Ui, label: &str, add_control: impl FnOnce(&mut egui::Ui)) {
    let w = scaled_font(ui.ctx(), theme::FIELD_LABEL_W);
    let h = scaled_font(ui.ctx(), theme::ROW_H_COMPACT);
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(w, h),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(label)
                        .size(scaled_font(ui.ctx(), theme::SMALL_FONT))
                        .color(theme::text_label()),
                );
            },
        );
        add_control(ui);
    });
}

/// 列表/树行：整行可点，选中/悬停底色 + 圆角。
///
/// 返回行 [`egui::Response`]；内容由 `add_contents` 在行内水平排布（垂直居中）。
/// 悬停用几何判定（`rect_contains_pointer`），避免行内交互控件顶掉行级 hover。
pub(crate) fn list_row(
    ui: &mut egui::Ui,
    selected: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let h = scaled_font(ui.ctx(), theme::ROW_H_LIST);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::click());
    if selected {
        ui.painter()
            .rect_filled(rect, theme::ROW_RADIUS, theme::selected_bg());
    } else if ui.rect_contains_pointer(rect) {
        ui.painter()
            .rect_filled(rect, theme::ROW_RADIUS, theme::hover_color(theme::app_bg()));
    }
    let pad = scaled_font(ui.ctx(), theme::PAD_X);
    let inner = egui::Rect::from_min_max(rect.min + egui::vec2(pad, 0.0), rect.max);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = theme::GAP_TIGHT;
            add_contents(ui);
        },
    );
    resp
}
