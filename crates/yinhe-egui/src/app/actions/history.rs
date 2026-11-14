//! undo / redo / 历史跳转与统一的编辑收尾。

use crate::app::App;
use yinhe_editor_core::document::Document;

impl App {
    /// Run an edit closure, recording an undo entry from the returned action
    /// and notifying audio afterwards.
    ///
    /// The closure receives `&mut Document` and should return
    /// `Some(UndoAction)` if it actually changed anything; on `None` no
    /// undo entry is pushed and audio is not notified.
    pub(crate) fn with_undo<F>(&mut self, label: &str, f: F)
    where
        F: FnOnce(&mut Document) -> Option<yinhe_editor_core::history::UndoAction>,
    {
        self.with_undo_result(label, |doc| f(doc).map(|action| (action, ())));
    }

    /// [`with_undo`] 的扩展：闭包除 undo action 外再返回一份数据，原样转发给调用方。
    pub(crate) fn with_undo_result<T, F>(&mut self, label: &str, f: F) -> Option<T>
    where
        F: FnOnce(&mut Document) -> Option<(yinhe_editor_core::history::UndoAction, T)>,
    {
        let idx = self.workspace.active_doc?;
        let before = self.workspace.documents[idx].capture_snapshot();
        let (action, extra) = f(&mut self.workspace.documents[idx])?;
        let doc = &mut self.workspace.documents[idx];
        doc.push_undo(action, label, before);
        doc.data.bump_revision();
        doc.edit.pianoroll_view.base.dirty = true;
        doc.edit.arrange_view.base.dirty = true;
        // 所有 with_undo 调用方目前都是纯音符操作（delete/duplicate/transpose/
        // paste/add_note/eraser/recode_track_names），不触碰 automation lanes，
        // 所以用便宜的 UpdateNotes 路径（不重建 CC，不 chase）。
        // 如果未来有自动化编辑走 with_undo，需要改用 notify_audio_model_changed。
        self.notify_notes_changed();
        Some(extra)
    }

    /// 在活跃文档的历史栈上执行一次状态变更，并统一处理模型/混音/音频同步。
    ///
    /// undo / redo / 点击历史行跳转共用，避免三份重复的收尾逻辑漂移。
    fn apply_history_change(&mut self, change: impl FnOnce(&mut Document) -> bool) -> bool {
        let Some(idx) = self.workspace.active_doc else {
            return false;
        };
        // 混音台参数被替换时需整体推回引擎（其余 undo 不受影响）。
        let before_mixer = self.workspace.documents[idx].mixer.clone();
        let changed = {
            let doc: &mut Document = &mut self.workspace.documents[idx];
            change(doc)
        };
        if changed {
            let doc = &mut self.workspace.documents[idx];
            doc.data.bump_revision();
            doc.edit.pianoroll_view.base.dirty = true;
            self.notify_audio_model_changed();
            if self.workspace.documents[idx].mixer != before_mixer {
                self.push_mixer_state_to_engine(idx);
            }
        }
        changed
    }

    /// Restore the previous state on the active document's history stack.
    pub(crate) fn undo(&mut self) {
        self.apply_history_change(|doc| doc.undo());
    }

    /// Re-apply the most recently undone state on the active document.
    pub(crate) fn redo(&mut self) {
        self.apply_history_change(|doc| doc.redo());
    }

    /// 点击历史行：跳到指定撤销栈长度，future 保留（新编辑才会覆盖）。
    pub(crate) fn jump_history_to(&mut self, target_past_len: usize) {
        self.apply_history_change(|doc| doc.jump_history_to(target_past_len));
    }
}
