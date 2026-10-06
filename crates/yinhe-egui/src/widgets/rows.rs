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

/// 设置行：左侧标题（+可选描述），右侧控件；行尾分隔。与设置对话框同款。
pub(crate) fn setting_row(
    ui: &mut egui::Ui,
    title: &str,
    desc: &str,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    row_impl(ui, title, desc, true, add_control);
}

/// 面板行：左侧标题、右侧控件、垂直居中；**无**行间分隔线（右栏面板用，省空间）。
pub(crate) fn panel_row(ui: &mut egui::Ui, title: &str, add_control: impl FnOnce(&mut egui::Ui)) {
    row_impl(ui, title, "", false, add_control);
}

fn row_impl(
    ui: &mut egui::Ui,
    title: &str,
    desc: &str,
    divider: bool,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    if desc.is_empty() {
        // 单行：先固定行高为控件高再布局，保证标题与右侧控件垂直居中对齐。
        // （直接 horizontal 会因为标题先于控件落位、行高后变，导致标题偏上。）
        let h = super::control::h(ui.ctx());
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), h),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(title)
                        .strong()
                        .size(scaled_font(ui.ctx(), theme::SUB_TITLE_FONT)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    add_control(ui);
                });
            },
        );
    } else {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(title)
                        .strong()
                        .size(scaled_font(ui.ctx(), theme::SUB_TITLE_FONT)),
                );
                ui.add_space(theme::GAP_TIGHT);
                ui.label(
                    egui::RichText::new(desc)
                        .size(scaled_font(ui.ctx(), theme::SMALL_FONT))
                        .color(theme::text_secondary()),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                add_control(ui);
            });
        });
    }
    if divider {
        ui.add_space(theme::GAP);
        ui.separator();
        ui.add_space(theme::GAP);
    } else {
        ui.add_space(theme::GAP_SM);
    }
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
