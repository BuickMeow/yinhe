//! 音符增删、移调、变速、人性化、合并等编辑动作。

use rust_i18n::t;

use crate::app::App;

impl App {
    /// Delete all selected notes from the active document.
    pub(crate) fn delete_selected_notes(&mut self) {
        // AR 音频选择优先：有选中的音频片段时删除片段（与音符选择互斥使用）。
        if self.delete_selected_audio_clips() {
            return;
        }
        let is_arrange = self.view_focus == crate::app::ViewFocus::Arrange;
        self.with_undo(t!("undo.delete_notes").as_ref(), |doc| {
            if is_arrange {
                doc.delete_selected_with_automation()
            } else {
                doc.delete_selected()
            }
        });
    }

    /// 右键拖拽擦除：沿路径命中的音符整笔合并为一个 undo 条目。
    pub(crate) fn quick_erase_notes(&mut self, notes: Vec<(u16, u32, u8)>) {
        if notes.is_empty() {
            return;
        }
        self.with_undo(t!("undo.delete_notes").as_ref(), |doc| {
            doc.delete_notes(&notes)
        });
    }

    /// 删除 AR 选中的音频片段；返回是否执行了删除。
    pub(crate) fn delete_selected_audio_clips(&mut self) -> bool {
        let Some(idx) = self.workspace.active_doc else {
            return false;
        };
        if self.workspace.documents[idx]
            .edit
            .selected_audio_clips
            .is_empty()
        {
            return false;
        }
        let ids: Vec<(u16, u32)> = self.workspace.documents[idx]
            .edit
            .selected_audio_clips
            .iter()
            .copied()
            .collect();
        let before = self.workspace.documents[idx].capture_snapshot();
        let mut actions = Vec::new();
        for (track, id) in ids {
            if let Some(action) =
                self.workspace.documents[idx].delete_audio_clips(track as usize, &[id])
            {
                actions.push(action);
            }
        }
        if actions.is_empty() {
            return false;
        }
        self.workspace.documents[idx]
            .edit
            .selected_audio_clips
            .clear();
        let action = yinhe_editor_core::history::UndoAction::Composite(actions);
        self.workspace.documents[idx].push_undo(action, t!("undo.edit_audio").as_ref(), before);
        self.notify_audio_model_changed();
        true
    }

    /// Duplicate all selected notes (Ctrl+D / Cmd+D).
    /// New notes are placed after the original selection, offset by the selection duration.
    pub(crate) fn duplicate_selected_notes(&mut self) {
        let is_arrange = self.view_focus == crate::app::ViewFocus::Arrange;
        self.with_undo(t!("undo.duplicate_notes").as_ref(), |doc| {
            if is_arrange {
                doc.duplicate_selected_with_automation()
            } else {
                doc.duplicate_selected()
            }
        });
    }

    /// Transpose selected notes by `semitones` (e.g. +12 for up an octave, -12 for down).
    pub(crate) fn transpose_selected_notes(&mut self, semitones: i8) {
        let label = if semitones >= 0 {
            t!("undo.transpose_up")
        } else {
            t!("undo.transpose_down")
        };
        self.with_undo(label.as_ref(), |doc| doc.transpose_selected(semitones));
    }

    /// Flip selected notes horizontally (tick) or vertically (key).
    pub(crate) fn flip_selected_notes(&mut self, axis: yinhe_editor_core::FlipAxis) {
        let label = match axis {
            yinhe_editor_core::FlipAxis::Horizontal => t!("undo.flip_horizontal"),
            yinhe_editor_core::FlipAxis::Vertical => t!("undo.flip_vertical"),
        };
        let is_arrange = self.view_focus == crate::app::ViewFocus::Arrange;
        let with_automation = is_arrange && axis == yinhe_editor_core::FlipAxis::Horizontal;
        self.with_undo(label.as_ref(), |doc| {
            if with_automation {
                doc.flip_selected_with_automation(axis)
            } else {
                doc.flip_selected_notes(axis)
            }
        });
    }

    /// 变速：选中跨度 ×2（变慢）。
    pub(crate) fn double_selected_length(&mut self) {
        let is_arrange = self.view_focus == crate::app::ViewFocus::Arrange;
        self.with_undo(t!("undo.rescale_span").as_ref(), |doc| {
            if is_arrange {
                doc.scale_selected_span_with_automation(2, 1)
            } else {
                doc.scale_selected_span(2, 1)
            }
        });
    }

    /// 变速：选中跨度 ÷2（变快）。
    pub(crate) fn half_selected_length(&mut self) {
        let is_arrange = self.view_focus == crate::app::ViewFocus::Arrange;
        self.with_undo(t!("undo.rescale_span").as_ref(), |doc| {
            if is_arrange {
                doc.scale_selected_span_with_automation(1, 2)
            } else {
                doc.scale_selected_span(1, 2)
            }
        });
    }

    /// 一键为整首歌去重重叠音符（黑乐谱叠音清理）。
    pub(crate) fn dedup_overlapping_notes(&mut self, cross_track: bool) {
        let label = if cross_track {
            t!("undo.dedup_across_tracks")
        } else {
            t!("undo.dedup_within_track")
        };
        self.with_undo(label.as_ref(), |doc| {
            doc.dedup_overlapping_notes(cross_track)
        });
    }

    /// 人性化：给选中音符注入温和随机偏差（时间/力度/时值各 ±8）。
    pub(crate) fn humanize_selected_notes(&mut self) {
        self.with_undo(t!("undo.humanize").as_ref(), |doc| {
            doc.humanize_selected(yinhe_editor_core::HumanizeParams {
                timing: 8,
                velocity: 8,
                gate: 8,
            })
        });
    }

    /// 填缝：把框选内音符终点贴合到同轨道「下一个不同起点」。
    pub(crate) fn fill_gaps_selected_notes(&mut self) {
        self.with_undo(t!("undo.fill_gaps").as_ref(), |doc| {
            doc.fill_gaps_selected()
        });
    }

    /// 合并：把框选内首尾严格相接的同 key 音符合并成一个。
    pub(crate) fn merge_selected_notes(&mut self) {
        self.with_undo(t!("undo.merge_notes").as_ref(), |doc| {
            doc.merge_adjacent_selected()
        });
    }
    /// Add a single note to the given track and record an undo entry.
    pub(crate) fn add_note_with_undo(&mut self, track_idx: u16, note: yinhe_core::NoteEvent) {
        self.with_undo(t!("undo.add_note").as_ref(), |doc| {
            doc.add_note(track_idx, note)
        });
    }

    /// 批量添加音符（刷子绘制）：力度 = 该轨记忆力度，一次落笔一个 undo entry。
    pub(crate) fn add_notes_with_undo(
        &mut self,
        track_idx: u16,
        mut notes: Vec<yinhe_core::NoteEvent>,
    ) {
        if notes.is_empty() {
            return;
        }
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let velocity = self.workspace.documents[idx]
            .edit
            .default_velocity(track_idx);
        for n in &mut notes {
            n.velocity = velocity;
        }
        self.with_undo(t!("undo.add_note").as_ref(), |doc| {
            doc.add_notes_batch(track_idx, &notes)
        });
    }

    /// 钢笔工具确认：沿曲线逐行生成音符，然后清空路径（一个 undo entry）。
    pub(crate) fn line_tool_confirm(&mut self) {
        self.with_undo(t!("undo.add_note").as_ref(), |doc| doc.generate_pen_notes());
        if let Some(idx) = self.workspace.active_doc {
            let doc = &mut self.workspace.documents[idx];
            doc.edit.pen_path = None;
            doc.edit.pianoroll_view.base.dirty = true;
        }
    }

    /// 剪刀确认：按锚点线切割，然后清空线（一个 undo entry）。
    pub(crate) fn scissors_confirm(&mut self) {
        self.with_undo(t!("undo.scissors_split").as_ref(), |doc| {
            doc.split_scissors_line()
        });
        if let Some(idx) = self.workspace.active_doc {
            let doc = &mut self.workspace.documents[idx];
            doc.edit.scissors_line = None;
            doc.edit.pianoroll_view.base.dirty = true;
        }
    }

    /// 网格切割：按当前视图（PR/AR）的量化网格切开选框内音符，
    /// `Auto` 先用该视图的缩放解析出具体网格；然后清空选框（一个 undo entry）。
    /// 无切点时保留选框，方便换档/调框重试。
    pub(crate) fn grid_split_selection(&mut self) {
        let is_pr = self.view_focus == crate::app::ViewFocus::Pianoroll;
        let changed = self
            .with_undo_result(t!("undo.grid_split").as_ref(), |doc| {
                let action = if is_pr {
                    doc.split_selection_by_grid()
                } else {
                    doc.split_selection_by_grid_arrangement()
                };
                action.map(|action| (action, ()))
            })
            .is_some();
        // 只有真的切到音符才清空选框与选区。
        if changed && let Some(idx) = self.workspace.active_doc {
            let doc = &mut self.workspace.documents[idx];
            doc.edit.sel_rect.clear();
            doc.edit.arr_sel_rect.clear();
            doc.edit.selected.clear();
            doc.edit.pianoroll_view.base.dirty = true;
        }
    }
}
