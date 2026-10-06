//! 历史记录面板：列出撤销/重做栈里的每一步操作。
//!
//! 直接读取 `UndoStack` 的标签（`undo_labels` / `redo_labels`），最新操作在
//! 最上方。仅展示，不显示数量。

use eframe::egui;

use yinhe_editor_core::document::Document;

/// 标签 key（如 "undo.add_note"）→ 本地化文案；已是纯文本则原样返回。
fn label_text(label: &str) -> String {
    let text = rust_i18n::t!(label).to_string();
    // rust_i18n 未命中时返回 key 本身；避免显示生僻 key。
    text
}

/// 显示历史记录列表。
pub(crate) fn show(ui: &mut egui::Ui, doc: &Document) {
    let past = doc.undo_labels();
    let future = doc.redo_labels();

    if past.is_empty() && future.is_empty() {
        crate::widgets::hint::empty_hint(ui, rust_i18n::t!("panel.history_empty").as_ref());
        return;
    }

    crate::widgets::scroll::rows_scroll(
        ui,
        "history_scroll",
        crate::widgets::control::h(ui.ctx()),
        None,
        |ui| {
            // 最新的撤销步骤在最上方。
            for label in past.iter().rev() {
                draw_row(ui, &label_text(label), false);
            }
            // 可重做步骤：灰色、在撤销步骤上方（最新可重做在最靠上）。
            for label in future.iter().rev() {
                draw_row(ui, &label_text(label), true);
            }
        },
    );
}

fn draw_row(ui: &mut egui::Ui, text: &str, dimmed: bool) {
    let color = if dimmed {
        crate::theme::text_label()
    } else {
        crate::theme::text_secondary()
    };
    crate::widgets::rows::list_row(ui, false, |ui| {
        ui.label(
            egui::RichText::new(text)
                .size(crate::scaling::scaled_font(
                    ui.ctx(),
                    crate::theme::SMALL_FONT,
                ))
                .color(color),
        );
    });
}
