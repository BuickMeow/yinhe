//! 标尺标签（MIDI marker）编辑动作落地：把 `time_ruler` 返回的 [`MarkerEdit`]
//! 应用到文档并接入 undo。标尺 widget 本身不依赖 `Document`，由调用方（AR / PR）
//! 经此处落地。

use crate::widgets::time_ruler::MarkerEdit;
use yinhe_editor_core::document::Document;

/// 应用一个标尺标签编辑动作（自动 push undo）。
pub(crate) fn apply_marker_edit(doc: &mut Document, edit: MarkerEdit) {
    match edit {
        MarkerEdit::Rename { tick, text } => {
            doc.set_marker_with_undo(tick, tick, text, "undo.rename_marker_event");
        }
        MarkerEdit::Delete { tick } => {
            doc.delete_marker_with_undo(tick, "undo.delete_marker_event");
        }
        MarkerEdit::Move { from, to } => {
            let text = doc
                .data
                .model
                .conductor
                .markers
                .iter()
                .find(|m| m.tick == from)
                .map(|m| m.text.clone())
                .unwrap_or_default();
            doc.set_marker_with_undo(from, to, text, "undo.move_marker_event");
        }
    }
}
