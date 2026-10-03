//! 历史记录面板：列出撤销/重做栈里的每一步操作。
//!
//! 直接读取 `UndoStack` 的标签（`undo_labels` / `redo_labels`），最新操作在
//! 最上方。仅展示，不显示数量。

use eframe::egui;

use yinhe_editor_core::document::Document;

const ROW_H: f32 = 20.0;

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

    egui::ScrollArea::vertical()
        .id_salt("history_scroll")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            // 最新的撤销步骤在最上方。
            for label in past.iter().rev() {
                draw_row(ui, &label_text(label), false);
            }
            // 可重做步骤：灰色、在撤销步骤上方（最新可重做在最靠上）。
            for label in future.iter().rev() {
                draw_row(ui, &label_text(label), true);
            }
        });
}

fn draw_row(ui: &mut egui::Ui, text: &str, dimmed: bool) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_H),
        egui::Sense::hover(),
    );
    let color = if dimmed {
        crate::theme::text_label()
    } else {
        crate::theme::text_secondary()
    };
    ui.painter().text(
        egui::pos2(rect.min.x + 4.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(crate::theme::SMALL_FONT),
        color,
    );
}
