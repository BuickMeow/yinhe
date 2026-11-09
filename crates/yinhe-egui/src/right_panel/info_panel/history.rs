//! 历史记录面板：列出撤销/重做栈里的每一步操作。
//!
//! 直接读取 `UndoStack` 的标签（`undo_labels` / `redo_labels`），最新操作在
//! 最上方。点击任意行可跳到该步骤之后；future 保留，之后再编辑才覆盖。

use eframe::egui;

use yinhe_editor_core::document::Document;

/// 标签 key（如 "undo.add_note"）→ 本地化文案；已是纯文本则原样返回。
fn label_text(label: &str) -> String {
    let text = rust_i18n::t!(label).to_string();
    // rust_i18n 未命中时返回 key 本身；避免显示生僻 key。
    text
}

/// 显示历史记录列表。返回被点击行的目标撤销栈长度（None = 本帧未点击）。
///
/// 列表顺序：最上方 = 最远的 future，向下到当前状态，再向下到最早的 past。
/// 点击任意行会跳到“该步骤完成之后”的状态，future 保留，可继续点其它行。
pub(crate) fn show(ui: &mut egui::Ui, doc: &Document) -> Option<usize> {
    crate::right_panel::region_hint(ui, rust_i18n::t!("hint.panel.history"));
    let past = doc.undo_labels();
    let future = doc.redo_labels();
    let past_len = past.len();

    if past.is_empty() && future.is_empty() {
        crate::widgets::hint::empty_hint(ui, rust_i18n::t!("panel.history_empty").as_ref());
        return None;
    }

    let mut target = None;
    crate::widgets::scroll::rows_scroll(
        ui,
        "history_scroll",
        crate::widgets::control::h(ui.ctx()),
        None,
        |ui| {
            // future 栈索引 0 = 最远未来；越靠上越远。
            for (i, label) in future.iter().enumerate() {
                let row_target = past_len + (future.len() - i);
                if draw_row(ui, &label_text(label), true, false).clicked() {
                    target = Some(row_target);
                }
            }
            // past 最新在最上方；当前状态 = past 的最后一项。
            for (i, label) in past.iter().enumerate().rev() {
                let is_current = i + 1 == past_len;
                if draw_row(ui, &label_text(label), false, is_current).clicked() {
                    target = Some(i + 1);
                }
            }
        },
    );
    target
}

fn draw_row(ui: &mut egui::Ui, text: &str, dimmed: bool, current: bool) -> egui::Response {
    let color = if dimmed {
        crate::theme::text_label()
    } else {
        crate::theme::text_secondary()
    };
    let resp = crate::widgets::rows::list_row(ui, current, |ui| {
        ui.label(
            egui::RichText::new(text)
                .size(crate::scaling::scaled_font(
                    ui.ctx(),
                    crate::theme::SMALL_FONT,
                ))
                .color(color),
        );
    });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}
