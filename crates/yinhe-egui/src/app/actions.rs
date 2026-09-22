use std::sync::mpsc;

use eframe::egui;
use rust_i18n::t;

use crate::app::{App, PasteChain, PendingFileAction};
use crate::chrome::transport_bar;
use crate::chrome::transport_bar::FileAction;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::shortcuts;

/// 离线保存所需的文档快照（用户保存与自动保存共用）。
/// 抓取后不再借用 `App`，可整体移动到后台线程。
pub(crate) struct SaveSnapshot {
    pub model: std::sync::Arc<yinhe_core::YinModel>,
    pub project_file: yinhe_yin::ProjectFile,
    pub mapping_file: yinhe_yin::MappingFile,
    pub mixer: yinhe_mixer::MixerParams,
}

/// Actions detected from keyboard input in the current frame.
#[derive(Default)]
pub(crate) struct KeyboardActions {
    pub toggle_play: bool,
    pub pause_return: bool,
    pub stop_play: bool,
    pub delete_selected: bool,
    pub duplicate_selected: bool,
    pub transpose_up: bool,
    pub transpose_down: bool,
    pub undo: bool,
    pub redo: bool,
    pub copy: bool,
    pub cut: bool,
    pub paste: bool,
    pub paste_at_original: bool,
    pub paste_flipped: bool,
    pub select_all: bool,
    pub select_notes_only: bool,
    pub filter_selection: bool,
    /// 工具切换快捷键触发的目标工具（None = 本帧未触发）。
    pub tool_to_activate: Option<crate::widgets::tools_panel::Tool>,
    /// 文件菜单动作（非 macOS 平台由键盘触发；macOS 走原生菜单栏）。
    pub file_action: Option<FileAction>,
}

impl App {
    /// Handle keyboard shortcuts.
    /// Returns a `KeyboardActions` struct describing which actions were triggered.
    pub(crate) fn handle_keyboard_shortcuts(&self, ui: &egui::Ui) -> KeyboardActions {
        let mut actions = KeyboardActions::default();

        // 文本输入焦点（TextEdit/DragValue 等）优先：全局快捷键让位给输入框，
        // 与成熟 DAW 一致（Backspace/Delete/Cmd+C/V/Z 等作用于文本而非选区）。
        // 设置窗口打开或快捷键录制期间同样让位：设置页里不允许任何快捷键触发动作
        // （Esc 例外：由设置页录制器消费用于取消录制）。
        if ui.ctx().egui_wants_keyboard_input()
            || self.audio_settings.show_settings
            || self.audio_settings.shortcut_recording
        {
            return actions;
        }

        let is_playing_any = self
            .audio_state
            .handle
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);

        // 本帧唯一一次主键按下（排除纯修饰键）。
        let pressed = ui.input(|i| {
            i.events.iter().find_map(|ev| match ev {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if crate::shortcuts::is_recordable_key(*key) => Some((*key, *modifiers)),
                _ => None,
            })
        });

        let kb = &self.audio_settings.keybindings;
        // 一个动作可绑定多个快捷键，任一匹配即触发。
        // macOS：第一个快捷键若带修饰键（⌘/⇧/⌥），由原生菜单加速键在系统层面
        // 处理（AppKit 拦截，egui 收不到那个键），这里跳过它避免双触发；
        // 无修饰键的快捷键（如 Space）AppKit 不会拦截菜单加速键，必须由 egui
        // 处理，否则按键会失效。其余快捷键（第二个及以后）同样由 egui 处理。
        let matches = |id: &str, key: egui::Key, modifiers: egui::Modifiers| {
            kb.get(id).iter().enumerate().any(|(i, c)| {
                #[cfg(target_os = "macos")]
                {
                    if i == 0 && crate::shortcuts::native_menu_handles(c) {
                        return false;
                    }
                }
                crate::shortcuts::matches_combo(c, modifiers, key)
            })
        };

        if let Some((key, modifiers)) = pressed {
            // ── 文件动作 ──
            // macOS：第一个快捷键走原生菜单；其余快捷键在这里分发。
            for action in FileAction::ALL {
                if matches(action.action_id(), key, modifiers) {
                    actions.file_action = Some(action);
                    break;
                }
            }

            // ── 播放/停止 ──
            if matches(shortcuts::ACTION_TOGGLE_PLAY, key, modifiers) {
                if is_playing_any {
                    actions.pause_return = true;
                } else {
                    actions.toggle_play = true;
                }
            }
            if matches(shortcuts::ACTION_STOP, key, modifiers) {
                actions.stop_play = true;
            }

            // ── 编辑 ──
            if matches(shortcuts::ACTION_DELETE, key, modifiers) {
                actions.delete_selected = true;
            }
            if matches(shortcuts::ACTION_DUPLICATE, key, modifiers) {
                actions.duplicate_selected = true;
            }
            if matches(shortcuts::ACTION_TRANSPOSE_UP, key, modifiers) {
                actions.transpose_up = true;
            }
            if matches(shortcuts::ACTION_TRANSPOSE_DOWN, key, modifiers) {
                actions.transpose_down = true;
            }
            if matches(shortcuts::ACTION_UNDO, key, modifiers) {
                actions.undo = true;
            }
            if matches(shortcuts::ACTION_REDO, key, modifiers) {
                actions.redo = true;
            }
            if matches(shortcuts::ACTION_CUT, key, modifiers) {
                actions.cut = true;
            }
            if matches(shortcuts::ACTION_COPY, key, modifiers) {
                actions.copy = true;
            }
            if matches(shortcuts::ACTION_PASTE, key, modifiers) {
                actions.paste = true;
            }
            if matches(shortcuts::ACTION_PASTE_AT_ORIGINAL, key, modifiers) {
                actions.paste_at_original = true;
            }
            if matches(shortcuts::ACTION_PASTE_FLIPPED, key, modifiers) {
                actions.paste_flipped = true;
            }
            if matches(shortcuts::ACTION_SELECT_ALL, key, modifiers) {
                actions.select_all = true;
            }
            if matches(shortcuts::ACTION_SELECT_NOTES_ONLY, key, modifiers) {
                actions.select_notes_only = true;
            }
            if matches(shortcuts::ACTION_FILTER_SELECTION, key, modifiers) {
                actions.filter_selection = true;
            }

            // ── 工具切换 ──
            // 工具动作不在 macOS 原生菜单中，egui 一定能收到按键，
            // 因此这里不跳过第一个快捷键（与文件/编辑动作的 macOS 处理不同）。
            for tool in crate::widgets::tools_panel::ALL_TOOLS {
                if kb
                    .get(tool.action_id())
                    .iter()
                    .any(|c| crate::shortcuts::matches_combo(c, modifiers, key))
                {
                    actions.tool_to_activate = Some(tool);
                    break;
                }
            }
        }

        // ── 兼容别名（配置表之外的历史默认）──
        ui.input(|i| {
            // Backspace 等同 Delete
            if i.key_pressed(egui::Key::Backspace) {
                actions.delete_selected = true;
            }
            // Cmd/Ctrl+Y 也触发重做
            let cmd = i.modifiers.command || i.modifiers.ctrl;
            if cmd && i.key_pressed(egui::Key::Y) {
                actions.redo = true;
            }
        });

        actions
    }

    /// Delete all selected notes from the active document.
    pub(crate) fn delete_selected_notes(&mut self) {
        // AR 音频选择优先：有选中的音频片段时删除片段（与音符选择互斥使用）。
        if self.delete_selected_audio_clips() {
            return;
        }
        self.with_undo(t!("undo.delete_notes").as_ref(), |doc| {
            doc.delete_selected()
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
        self.with_undo(t!("undo.duplicate_notes").as_ref(), |doc| {
            doc.duplicate_selected()
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
        self.with_undo(label.as_ref(), |doc| doc.flip_selected_notes(axis));
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

    // ── Copy / Cut / Paste / Select All ──

    /// Copy selected notes to the clipboard as an O(1) model snapshot.
    /// Empty selection leaves the clipboard untouched (成熟软件惯例：空复制不覆盖).
    pub(crate) fn copy_selection(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &self.workspace.documents[idx];
        if doc.edit.selected.is_empty() {
            return;
        }
        self.clipboard = yinhe_editor_core::ClipboardContent::Notes(
            yinhe_editor_core::NotesClipboard::from_snapshot(
                doc.data.model.clone(),
                doc.edit.selected.clone(),
            ),
        );
        self.paste_chain = None;
        self.export_clipboard_to_system();
    }

    /// Cut: copy selection (snapshot), then delete selected notes.
    pub(crate) fn cut_selection(&mut self) {
        self.copy_selection();
        self.delete_selected_notes();
    }

    /// Paste the clipboard at cursor position, dispatching on clipboard content
    /// (notes vs automation), not on the current selection.
    pub(crate) fn paste_clipboard(&mut self) {
        self.paste_clipboard_with_mode(yinhe_editor_core::clipboard::PasteMode::AtCursor);
    }

    /// Paste with an explicit placement mode.
    ///
    /// `AtCursor` participates in the consecutive-paste chain: pressing paste
    /// again without moving the cursor advances by the pasted content span.
    pub(crate) fn paste_clipboard_with_mode(
        &mut self,
        mode: yinhe_editor_core::clipboard::PasteMode,
    ) {
        use yinhe_editor_core::clipboard::PasteMode;

        // 系统剪贴板是剪贴板内容的唯一真相：外部内容使内部剪贴板失效；
        // 其他实例的引用则从临时文件加载。已是最新内容时零开销。
        if !self.clipboard_sync.resolve_paste(&mut self.clipboard) {
            return;
        }

        let clipboard = self.clipboard.clone();
        let chained = mode == PasteMode::AtCursor;
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let cursor_tick = self.workspace.documents[idx]
            .edit
            .cursor_tick
            .unwrap_or(0.0);

        match clipboard {
            yinhe_editor_core::ClipboardContent::Empty => {}
            yinhe_editor_core::ClipboardContent::Notes(cb) => {
                let chain_offset = if chained {
                    self.paste_chain_offset(cursor_tick)
                } else {
                    0
                };
                let effective_cursor = cursor_tick + chain_offset as f64;
                let track_selected = self.workspace.documents[idx].edit.track_selected.clone();
                let span = self.with_undo_result(t!("undo.paste").as_ref(), |doc| {
                    doc.paste_notes(&cb, effective_cursor, &track_selected, mode)
                });
                if let Some(span) = span
                    && chained
                {
                    self.advance_paste_chain(cursor_tick, span.max(1));
                }
            }
            yinhe_editor_core::ClipboardContent::Automation(cb) => {
                let span = self.paste_automation_clipboard(&cb, mode);
                if let Some(span) = span
                    && chained
                {
                    let advance = self.automation_paste_advance(span);
                    self.advance_paste_chain(cursor_tick, advance);
                }
            }
        }
    }

    /// 连续粘贴链的当前偏移（光标未动时非零）。
    fn paste_chain_offset(&self, cursor_tick: f64) -> u32 {
        match self.paste_chain {
            Some(chain) if chain.cursor_tick == cursor_tick => chain.offset,
            _ => 0,
        }
    }

    /// 记录本次粘贴的内容跨度，下一次同光标粘贴将自动递增。
    fn advance_paste_chain(&mut self, cursor_tick: f64, advance: u32) {
        let advance = advance.max(1);
        match &mut self.paste_chain {
            Some(chain) if chain.cursor_tick == cursor_tick => {
                chain.offset = chain.offset.saturating_add(advance);
            }
            _ => {
                self.paste_chain = Some(PasteChain {
                    cursor_tick,
                    offset: advance,
                });
            }
        }
    }

    /// 自动化连续粘贴的递进量：多锚点用内容跨度；单锚点跨度 0 时用量化间隔。
    fn automation_paste_advance(&self, span: u32) -> u32 {
        if span > 0 {
            return span;
        }
        let Some(idx) = self.workspace.active_doc else {
            return 1;
        };
        let doc = &self.workspace.documents[idx];
        doc.edit
            .quantize_pianoroll
            .tick_interval(doc.data.model.meta.ppq)
            .max(1)
    }

    /// Select all notes — PR or AR depending on current view mode.
    pub(crate) fn select_all(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let is_pr = self.view_mode == crate::chrome::mode_bar::ViewMode::Edit;
        if is_pr {
            self.workspace.documents[idx].select_all_pr();
        } else {
            // select_all_ar 内部会同步设置 doc.edit.arr_sel_rect（AR 选框）。
            self.workspace.documents[idx].select_all_ar();
        }
        self.workspace.documents[idx].data.bump_revision();
        self.workspace.documents[idx].edit.pianoroll_view.base.dirty = true;
        self.workspace.documents[idx].edit.arrange_view.base.dirty = true;
    }

    /// 「仅选择音符」：清除所有自动化锚点选择（PR 面板 + AR 展开 lane），
    /// 只保留音符选框。
    pub(crate) fn select_notes_only(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        let mut changed = false;
        for panel in &mut doc.edit.controller_panels {
            if !panel.anchor_sel_rects.is_empty() {
                panel.anchor_sel_rects.clear();
                panel.dirty = true;
                changed = true;
            }
        }
        for view in doc.edit.arr_am_views.values_mut() {
            if !view.anchor_sel_rects.is_empty() {
                view.anchor_sel_rects.clear();
                view.dirty = true;
                changed = true;
            }
        }
        if changed {
            doc.edit.pianoroll_view.base.dirty = true;
            doc.edit.arrange_view.base.dirty = true;
        }
    }

    /// 打开选择筛选对话框（从当前选区/筛选初始化）。
    pub(crate) fn open_filter_dialog(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &self.workspace.documents[idx];
        self.filter_dialog.open(&doc.edit.selected, &doc.data.model);
    }

    /// 应用筛选：写属性边界（选框矩形保持不动）。
    pub(crate) fn apply_filter_dialog(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        doc.edit.selected.filter = self.filter_dialog.build_filter();
        doc.edit.pianoroll_view.base.dirty = true;
        doc.edit.arrange_view.base.dirty = true;
    }

    /// 清除筛选边界（保留选框与已收窄的空间范围）。
    pub(crate) fn clear_filter(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        doc.edit.selected.filter = Default::default();
        doc.edit.pianoroll_view.base.dirty = true;
        doc.edit.arrange_view.base.dirty = true;
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

    /// 直线工具确认：沿音高行生成音符，然后清空锚点线（一个 undo entry）。
    pub(crate) fn line_tool_confirm(&mut self) {
        self.with_undo(t!("undo.add_note").as_ref(), |doc| {
            doc.generate_line_notes()
        });
        if let Some(idx) = self.workspace.active_doc {
            let doc = &mut self.workspace.documents[idx];
            doc.edit.line_tool_line = None;
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

    /// 网格工具确认：按量化网格切开选框内音符，然后清空选框（一个 undo entry）。
    pub(crate) fn grid_split_selection(&mut self) {
        self.with_undo(t!("undo.grid_split").as_ref(), |doc| {
            doc.split_selection_by_grid()
        });
        // 确认后清空选框与选区（无论是否切到音符）。
        if let Some(idx) = self.workspace.active_doc {
            let doc = &mut self.workspace.documents[idx];
            doc.edit.sel_rect.clear();
            doc.edit.selected.clear();
            doc.edit.pianoroll_view.base.dirty = true;
        }
    }

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

    /// Restore the previous state on the active document's history stack.
    pub(crate) fn undo(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc: &mut Document = &mut self.workspace.documents[idx];
        let changed = doc.undo();
        if changed {
            doc.data.bump_revision();
            doc.edit.pianoroll_view.base.dirty = true;
            self.notify_audio_model_changed();
        }
    }

    /// Re-apply the most recently undone state on the active document.
    pub(crate) fn redo(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc: &mut Document = &mut self.workspace.documents[idx];
        let changed = doc.redo();
        if changed {
            doc.data.bump_revision();
            doc.edit.pianoroll_view.base.dirty = true;
            self.notify_audio_model_changed();
        }
    }
}

impl App {
    /// 处理编辑动作（transport bar 编辑 popup / 图钉与 macOS 菜单共用）。
    /// copy/cut/duplicate/delete 在自动化锚点选中时作用于锚点；
    /// paste 按剪贴板内容类型分派（见 paste_clipboard）。
    pub(crate) fn handle_edit_action(&mut self, action: transport_bar::EditAction) {
        use transport_bar::EditAction as A;
        let route_to_automation = self.has_selected_automation_anchors();
        match action {
            A::Undo => self.undo(),
            A::Redo => self.redo(),
            A::Cut => {
                if route_to_automation {
                    self.cut_automation_anchors();
                } else {
                    self.cut_selection();
                }
            }
            A::Copy => {
                if route_to_automation {
                    self.copy_automation_anchors();
                } else {
                    self.copy_selection();
                }
            }
            A::Paste => self.paste_clipboard(),
            A::PasteAtOriginal => {
                self.paste_clipboard_with_mode(yinhe_editor_core::clipboard::PasteMode::AtOriginal)
            }
            A::PasteFlipped => {
                self.paste_clipboard_with_mode(yinhe_editor_core::clipboard::PasteMode::Flipped)
            }
            A::SelectAll => self.select_all(),
            A::SelectNotesOnly => self.select_notes_only(),
            A::FilterSelection => self.open_filter_dialog(),
            A::Duplicate => {
                if route_to_automation {
                    self.duplicate_automation_anchors();
                } else {
                    self.duplicate_selected_notes();
                }
            }
            A::Delete => {
                if route_to_automation {
                    self.delete_automation_anchors();
                } else {
                    self.delete_selected_notes();
                }
            }
            A::TransposeUp => self.transpose_selected_notes(12),
            A::TransposeDown => self.transpose_selected_notes(-12),
            A::DedupWithinTrack => self.dedup_overlapping_notes(false),
            A::DedupAcrossTracks => self.dedup_overlapping_notes(true),
        }
    }

    /// Handle file menu actions from the transport bar.
    /// Checks for unsaved changes before destructive actions (New, Open, Close, Exit).
    pub(crate) fn handle_file_action(
        &mut self,
        action: transport_bar::FileAction,
        ctx: &egui::Context,
    ) {
        // Actions that never need the unsaved dialog
        match action {
            transport_bar::FileAction::Save
            | transport_bar::FileAction::SaveAs
            | transport_bar::FileAction::ExportMidi
            | transport_bar::FileAction::ExportAudio
            | transport_bar::FileAction::Settings
            | transport_bar::FileAction::ProjectSettings
            | transport_bar::FileAction::ImportAudio
            | transport_bar::FileAction::Open => {
                self.execute_file_action(action, ctx);
                return;
            }
            _ => {}
        }

        // Check for unsaved changes
        if let Some(idx) = self.workspace.active_doc
            && self.workspace.documents[idx].is_dirty()
        {
            let pending = match action {
                transport_bar::FileAction::NewProject => PendingFileAction::NewProject,
                transport_bar::FileAction::CloseDocument => PendingFileAction::CloseDocument(idx),
                transport_bar::FileAction::Exit => PendingFileAction::Exit,
                _ => unreachable!(), // filtered above
            };
            self.pending_unsaved = Some(pending);
            // 用户通过菜单/快捷键主动触发需要决策的操作，立刻把 unsaved 弹窗
            // 拉到主窗口前台（防止之前取消后弹窗被遮挡在主窗口后方）
            crate::chrome::dialog::raise_viewport(
                ctx,
                egui::ViewportId::from_hash_of("unsaved_dialog"),
            );
            return;
        }

        self.execute_file_action(action, ctx);
    }

    /// 打开「最近修改的文件」（transport bar 子菜单 / macOS 菜单栏共用）。
    /// 文件已被移动/删除时从列表移除并报错。
    ///
    /// 打开是**追加新文档**（不取代/关闭现有文档），因此不检查未保存修改：
    /// 触发保存确认只在会丢失当前文档的场景（关闭文档 / 新建 / 退出）。
    pub(crate) fn open_recent_file(&mut self, path: &str) {
        if !std::path::Path::new(path).exists() {
            self.audio_settings.remove_recent_file(path);
            self.audio_settings.save();
            let name = std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path);
            self.show_error(
                t!("toast.file_missing"),
                t!("file_dialog.not_found", name = name),
            );
            return;
        }
        self.file_loader
            .load_path(path.to_string(), self.audio_settings.midi_import_encoding);
    }

    /// Execute a file action immediately without checking for unsaved changes.
    fn execute_file_action(&mut self, action: transport_bar::FileAction, ctx: &egui::Context) {
        match action {
            transport_bar::FileAction::NewProject => {
                self.new_project();
            }
            transport_bar::FileAction::Open => {
                self.file_loader
                    .pick_file(self.audio_settings.midi_import_encoding);
            }
            transport_bar::FileAction::ImportAudio => {
                self.import_audio_dialog();
            }
            transport_bar::FileAction::Save => {
                if let Some(idx) = self.workspace.active_doc {
                    let path = self.workspace.documents[idx].file_path.clone();
                    if let Some(path) = path {
                        self.save_project_async(idx, path);
                    } else {
                        self.save_as_dialog();
                    }
                }
            }
            transport_bar::FileAction::SaveAs => {
                self.save_as_dialog();
            }
            transport_bar::FileAction::CloseDocument => {
                if let Some(idx) = self.workspace.active_doc {
                    self.close_document(idx);
                }
            }
            transport_bar::FileAction::ExportMidi => {
                self.export_midi_dialog();
            }
            transport_bar::FileAction::ExportAudio => {
                self.export_audio_dialog(ctx);
            }
            transport_bar::FileAction::Exit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            transport_bar::FileAction::Settings => {
                self.audio_settings.show_settings = true;
                crate::chrome::dialog::raise_viewport(
                    ctx,
                    egui::ViewportId::from_hash_of("settings_dialog"),
                );
            }
            transport_bar::FileAction::ProjectSettings => {
                self.set_float_panel(ctx, Some(crate::right_panel::FloatPanel::ProjectSettings));
            }
        }
    }

    /// Execute the deferred pending action (called after save completes or on discard).
    pub(crate) fn execute_pending_file_action(&mut self, _ctx: &egui::Context) {
        let Some(pending) = self.pending_unsaved.take() else {
            return;
        };
        match pending {
            PendingFileAction::NewProject => {
                self.new_project();
            }
            PendingFileAction::CloseDocument(idx) => {
                self.close_document(idx);
            }
            PendingFileAction::Exit => {
                self.should_exit = true;
            }
        }
    }

    /// 保存前把编辑态同步进 model/project_file，并抓取可离线保存的快照。
    /// 保存线程只持有快照，不再借用 `self`；保存与自动保存共用。
    pub(crate) fn take_save_snapshot(&mut self, idx: usize) -> SaveSnapshot {
        let doc = &mut self.workspace.documents[idx];
        doc.sync_overrides_to_model();
        doc.data.sync_project_file();
        doc.data.sync_mapping_file();

        // Sync SF state into project_file（每源通道覆盖；空 = 全部用全局）。
        doc.data.project_file.sf_channel_overrides = doc
            .edit
            .project_sf
            .overrides
            .iter()
            .map(|(channel, entries)| yinhe_yin::SfChannelOverride {
                channel: *channel,
                entries: entries
                    .iter()
                    .map(|e| yinhe_yin::SfEntryJson {
                        path: e.path.clone(),
                        name: e.name.clone(),
                        enabled: e.enabled,
                    })
                    .collect(),
            })
            .collect();

        // 混音台插件：保存前把实例的 CLAP state 写回 InsertRef（旁通标志同步）。
        if let Some(rack) = self.mixer_racks.get_mut(idx) {
            rack.sync_states_to(&mut self.workspace.documents[idx].mixer);
        }
        // 乐器插件：同样把实例 state 写回 mixer.instruments[channel]。
        if let Some(irack) = self.instrument_racks.get_mut(idx) {
            irack.sync_states_to(&mut self.workspace.documents[idx].mixer);
        }
        let doc = &self.workspace.documents[idx];
        SaveSnapshot {
            model: doc.data.model.clone(),
            project_file: doc.data.project_file.clone(),
            mapping_file: doc.data.mapping_file.clone(),
            mixer: doc.mixer.clone(),
        }
    }

    /// Spawn a background thread to save the project.
    pub(crate) fn save_project_async(&mut self, idx: usize, path: String) {
        let snap = self.take_save_snapshot(idx);
        let path_for_thread = path.clone();
        // 发起时的撤销栈长度快照：保存期间的新编辑不应被误标为已保存
        let saved_past_len = self
            .workspace
            .documents
            .get(idx)
            .map(|d| d.undo_past_len())
            .unwrap_or(0);

        let (tx, rx) = mpsc::channel();
        let (progress_tx, progress_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = yinhe_yin::save_yin_with_files_progress(
                &snap.model,
                &path_for_thread,
                &snap.project_file,
                &snap.mapping_file,
                Some(&snap.mixer),
                |p| {
                    let _ = progress_tx.send(p);
                },
            );
            let result = result.map_err(|e| e.to_string());
            if let Err(e) = &result {
                tracing::error!("Failed to save project: {}", e);
            }
            // 回传 (目标文档, 版本快照, 路径, 结果)：失败不得标记已保存/
            // 不得改路径/不得执行延迟动作（此前失败被当成功，可能丢数据）。
            let _ = tx.send((idx, saved_past_len, path_for_thread, result));
        });

        self.save_rx = Some(rx);
        self.save_progress_rx = Some(progress_rx);
    }

    pub(crate) fn save_as_dialog(&mut self) {
        if let Some(idx) = self.workspace.active_doc {
            self.save_as_dialog_for(idx);
        }
    }

    /// 另存为（指定文档；未保存确认弹窗的目标可能是非 active 文档）。
    /// 路径/文件名在**保存成功后**才写入（见 poll 的保存完成处理）。
    pub(crate) fn save_as_dialog_for(&mut self, idx: usize) {
        let default_name = if let Some(doc) = self.workspace.documents.get(idx) {
            format!("{}.yin", doc.file_name)
        } else {
            t!("file_dialog.untitled").to_string()
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(t!("file_dialog.yinhe_project").as_ref(), &["yin"])
            .set_file_name(&default_name)
            .save_file()
        {
            let mut path_str = path.to_string_lossy().to_string();
            // Ensure .yin extension
            if !path_str.ends_with(".yin") {
                path_str.push_str(".yin");
            }
            self.save_project_async(idx, path_str);
        }
    }

    fn export_midi_dialog(&mut self) {
        let default_name = if let Some(idx) = self.workspace.active_doc {
            format!("{}.mid", self.workspace.documents[idx].file_name)
        } else {
            t!("file_dialog.export_mid").to_string()
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("MIDI", &["mid", "midi"])
            .set_file_name(&default_name)
            .save_file()
        {
            let path_str = path.to_string_lossy().to_string();
            if let Some(idx) = self.workspace.active_doc {
                let doc = &self.workspace.documents[idx];
                let opts = yinhe_midi::MidiExportOptions {
                    encoding: self.audio_settings.midi_export_encoding,
                    rpn_full: self.audio_settings.midi_export_rpn_full,
                    curve_interpolate: self.audio_settings.midi_export_curve_interpolate,
                    curve_density: self.audio_settings.midi_export_curve_density,
                    strip_empty_tracks: self.audio_settings.midi_export_strip_empty_tracks,
                    dedup_overlaps: self.audio_settings.midi_export_dedup_overlaps,
                };
                match yinhe_midi::write_with_options(&doc.data.model, &opts) {
                    Ok(bytes) => {
                        if let Err(e) = std::fs::write(&path_str, &bytes) {
                            tracing::error!("Failed to export MIDI: {}", e);
                            self.notifications.error("导出MIDI失败", e.to_string());
                        } else {
                            let fname = std::path::Path::new(&path_str)
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or(&path_str)
                                .to_string();
                            self.notifications.success("导出MIDI完成", fname);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to export MIDI: {}", e);
                        self.notifications.error("导出MIDI失败", e.to_string());
                    }
                }
            }
        }
    }

    fn export_audio_dialog(&mut self, ctx: &egui::Context) {
        if self.workspace.active_doc.is_none() {
            return;
        }

        if self.export.running {
            return; // already exporting
        }

        // Show export settings dialog first
        self.export.show_bit_depth = true;
        crate::chrome::dialog::raise_viewport(
            ctx,
            egui::ViewportId::from_hash_of("export_settings_dialog"),
        );
    }

    /// Called after the bit-depth dialog is confirmed.
    /// Opens the file-save dialog and starts the export.
    ///
    /// 含插件链的导出在渲染线程内复用**实时引擎**（insert/乐器/PDC 全量参与）；
    /// GPU 合成器模式仍走独立 GPU 导出线程（不经混音台，不含插件链）。
    pub(crate) fn start_export(&mut self) {
        let idx = match self.workspace.active_doc {
            Some(idx) => idx,
            None => return,
        };

        // 真实计时起点：点击「开始导出」按钮的瞬间（包含后续文件对话框等待）。
        let button_time = std::time::Instant::now();
        let default_name = format!("{}.wav", self.workspace.documents[idx].file_name);
        let path = match rfd::FileDialog::new()
            .add_filter("WAV", &["wav"])
            .set_file_name(&default_name)
            .save_file()
        {
            Some(p) => p,
            None => return,
        };
        let mut path_str = path.to_string_lossy().to_string();
        if !path_str.ends_with(".wav") {
            path_str.push_str(".wav");
        }

        let sr = if self.export.sample_rate > 0 {
            self.export.sample_rate
        } else {
            self.audio_settings.sample_rate
        };
        // 导出复用实时引擎（含插件链与 PDC），采样率必须与当前设备一致：
        // 插件实例按设备采样率激活，无法在不重载插件的情况下换采样率。
        if sr != self.audio_settings.sample_rate {
            self.notifications.error(
                "导出采样率不匹配",
                format!(
                    "当前设备采样率为 {} Hz，含插件链的导出只能使用设备采样率；请在音频设置中切换采样率后重试。",
                    self.audio_settings.sample_rate
                ),
            );
            return;
        }

        let bit_depth = self.export.bit_depth;
        let layer_count = if self.export.layer_count == 0 {
            None
        } else {
            Some(self.export.layer_count as usize)
        };
        let export_progress = self.export.progress.clone();
        let cancel_flag = self.export.cancel.clone();
        let pause_flag = self.export.pause.clone();
        // 记下输出路径：中止卡“打开文件夹”按钮用。
        self.export.last_output_path = Some(path_str.clone());
        cancel_flag.store(false, std::sync::atomic::Ordering::Relaxed);
        pause_flag.store(false, std::sync::atomic::Ordering::Relaxed);
        // Reset progress state（计时起点为按钮点击时刻，保证壁钟时间真实）。
        if let Ok(mut p) = export_progress.lock() {
            p.reset();
            p.started_at = Some(button_time);
        }

        // 含插件链的导出：交给渲染线程复用实时引擎。
        if let Some(h) = &self.audio_state.handle {
            // 导出结束后恢复用户设置的层数（导出设置只影响本次导出）。
            let restore_layer_count = if self.audio_settings.xsynth_layers == 0 {
                None
            } else {
                Some(self.audio_settings.xsynth_layers as usize)
            };
            h.handle.send(yinhe_audio::AudioCommand::ExportStart {
                path: std::path::PathBuf::from(&path_str),
                bit_depth,
                layer_count,
                restore_layer_count,
                progress: export_progress,
                cancel: cancel_flag,
                pause: pause_flag,
            });
            self.export.running = true;
        }
    }
}
