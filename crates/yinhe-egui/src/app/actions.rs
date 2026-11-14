use std::sync::mpsc;

use eframe::egui;
use rust_i18n::t;

use crate::app::{App, PasteChain, PendingFileAction};
use crate::chrome::transport_bar;
use crate::chrome::transport_bar::{EditAction, FileAction};
use yinhe_editor_core::document::Document;
use yinhe_editor_core::shortcuts;

/// 退出保存队列的推进结果。
enum ExitSaveStep {
    /// 已发起保存（或等待上一个保存完成），保持 pending 等回调推进。
    Started,
    /// 所有脏文档都已保存，可以退出。
    Done,
    /// 无路径文档的另存为对话框被取消：取消退出。
    Cancelled,
}

/// 离线保存所需的文档快照（用户保存与自动保存共用）。
/// 抓取后不再借用 `App`，可整体移动到后台线程。
pub(crate) struct SaveSnapshot {
    pub model: std::sync::Arc<yinhe_core::YinModel>,
    pub project_file: yinhe_yin::ProjectFile,
    pub mapping_file: yinhe_yin::MappingFile,
    pub mixer: yinhe_mixer::MixerParams,
}

/// Actions detected from keyboard input in the current frame.
///
/// 编辑类动作收敛成单个 `edit_action`（与顶栏菜单 / macOS 原生菜单同一条分发路径
/// `App::handle_edit_action`）：否则每加一个动作都要在键盘分发里补一段 if，
/// 漏补就会「表里有快捷键但按了没反应」。
#[derive(Default)]
pub(crate) struct KeyboardActions {
    pub toggle_play: bool,
    pub pause_return: bool,
    pub stop_play: bool,
    /// 编辑动作（删除/复制/移调/变速/翻转/人性化/填缝/合并/网格切割/去重…）。
    pub edit_action: Option<EditAction>,
    /// 上下文动作 id（轨道/片段/事件类，见 `ACTION_TRACK_*` 等）。
    /// 这些动作默认没有快捷键，用户分配后走这里。
    pub context_action: Option<&'static str>,
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
            || self.audio_settings.ui_session.show_settings
            || self.audio_settings.ui_session.shortcut_recording
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

            // ── 编辑动作 ──
            // 全部走 `EditAction::ALL` + `handle_edit_action`：单一分发路径，
            // 不会出现「配置表里有这个动作、键盘却没人检查」的漏项。
            // macOS：第一个快捷键由原生菜单加速键处理（上面的 matches 已跳过）。
            for action in EditAction::ALL {
                let id = action.action_id();
                if !id.is_empty() && matches(id, key, modifiers) {
                    actions.edit_action = Some(action);
                    break;
                }
            }

            // ── 上下文动作（轨道 / 片段 / 事件，默认无快捷键）──
            // 用户若在设置页分配了键位，这里才能触发；未分配时 `kb.get` 返回空列表。
            if actions.edit_action.is_none() {
                for &id in yinhe_editor_core::shortcuts::CONTEXT_ACTION_IDS {
                    if matches(id, key, modifiers) {
                        actions.context_action = Some(id);
                        break;
                    }
                }
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
                actions.edit_action = Some(EditAction::Delete);
            }
            // Cmd/Ctrl+Y 也触发重做
            let cmd = i.modifiers.command || i.modifiers.ctrl;
            if cmd && i.key_pressed(egui::Key::Y) {
                actions.edit_action = Some(EditAction::Redo);
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
        let ppq = doc.data.model.meta.ppq;
        let (sig_num, sig_den) = doc.data.model.tempo_map.time_sig_default;
        doc.edit
            .quantize_pianoroll
            .resolved_with_time_sig(
                doc.edit.pianoroll_view.base.pixels_per_tick,
                ppq,
                sig_num,
                sig_den,
                true,
            )
            .tick_interval(ppq)
            .max(1)
    }

    /// Select all notes — PR or AR depending on the focused view.
    /// Arrange 模式下底部同时显示 PR 时，以鼠标所在视图为准。
    pub(crate) fn select_all(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let is_pr = self.view_focus == crate::app::ViewFocus::Pianoroll;
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
            if !panel.anchor_sel_rects.is_empty() || panel.anchor_members.is_some() {
                panel.anchor_sel_rects.clear();
                panel.anchor_members = None;
                panel.materialized_anchor_rects = 0;
                panel.dirty = true;
                changed = true;
            }
        }
        for view in doc.edit.arr_am_views.values_mut() {
            if !view.anchor_sel_rects.is_empty() || view.anchor_members.is_some() {
                view.anchor_sel_rects.clear();
                view.anchor_members = None;
                view.materialized_anchor_rects = 0;
                view.dirty = true;
                changed = true;
            }
        }
        // AR 删除自动化走共享 Selection 的成员位图；只清面板选框会留下
        // 不可见的“幽灵”自动化选择，Delete 仍会删掉它们。
        if doc.edit.selected.has_explicit_automation_members() {
            doc.edit.selected.clear_automation_selection();
            changed = true;
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

    /// 应用时间码编辑事件（transport bar 三列直接输入）。
    pub(crate) fn apply_timecode_events(
        &mut self,
        ctx: &egui::Context,
        events: Vec<crate::widgets::timecode::TimecodeEvent>,
    ) {
        use crate::widgets::timecode::TimecodeEvent;
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        for ev in events {
            match ev {
                TimecodeEvent::Bpm(v) => self.apply_timecode_bpm(idx, v),
                TimecodeEvent::TimeSig {
                    numerator,
                    denominator_power,
                } => self.apply_timecode_time_sig(idx, numerator, denominator_power),
                TimecodeEvent::Ppq { value, id } => self.apply_timecode_ppq(ctx, idx, value, id),
                TimecodeEvent::CursorTick(t) => {
                    self.workspace.documents[idx].edit.cursor_tick = Some(t.max(0.0));
                }
                TimecodeEvent::Quantize(preset) => {
                    let is_pr = self.view_focus == crate::app::ViewFocus::Pianoroll;
                    let edit = &mut self.workspace.documents[idx].edit;
                    if is_pr {
                        edit.quantize_pianoroll = preset;
                    } else {
                        edit.quantize_arrange = preset;
                    }
                }
            }
        }
    }

    /// BPM 直接输入（单/相同 tempo 事件 → 改；多个不同 → 光标处插入），进 undo。
    fn apply_timecode_bpm(&mut self, idx: usize, bpm: f64) {
        let changed = {
            let doc = &mut self.workspace.documents[idx];
            let at_tick = doc.edit.cursor_tick.unwrap_or(0.0).max(0.0) as u32;
            let before = doc.data.model.conductor.tempo.events.clone();
            let snapshot = doc.capture_snapshot();
            if !doc.set_tempo_bpm(at_tick, bpm) {
                false
            } else {
                let after = doc.data.model.conductor.tempo.events.clone();
                crate::right_panel::automation_undo::push_automation_undo(
                    doc,
                    0,
                    0,
                    &yinhe_types::AutomationTarget::Tempo,
                    before,
                    after,
                    t!("undo.edit_bpm").as_ref(),
                    snapshot,
                );
                true
            }
        };
        // tempo map 变了：音频引擎仍按旧 tempo 播放，必须重推模型。
        if changed {
            self.notify_audio_model_changed();
        }
    }

    /// 拍号直接输入（单/相同 → 改；多个不同 → 光标处插入），进 undo。
    fn apply_timecode_time_sig(&mut self, idx: usize, numerator: u8, denominator_power: u8) {
        use yinhe_editor_core::history::{
            EventListDelta, EventListItem, EventListTarget, UndoAction,
        };
        let doc = &mut self.workspace.documents[idx];
        let at_tick = doc.edit.cursor_tick.unwrap_or(0.0).max(0.0) as u32;
        let before = doc.data.model.conductor.time_sig.clone();
        let snapshot = doc.capture_snapshot();
        if !doc.set_time_sig_value(at_tick, numerator, denominator_power) {
            return;
        }
        let after = doc.data.model.conductor.time_sig.clone();
        doc.push_undo(
            UndoAction::EventList(EventListDelta {
                target: EventListTarget::TimeSig,
                old: before.into_iter().map(EventListItem::TimeSig).collect(),
                new: after.into_iter().map(EventListItem::TimeSig).collect(),
            }),
            t!("undo.edit_time_sig").as_ref(),
            snapshot,
        );
    }

    /// PPQ 直接输入（时间码）：有音符直接 rescale（保留绝对时间），无音符直接提交。
    fn apply_timecode_ppq(&mut self, ctx: &egui::Context, idx: usize, value: u32, id: u64) {
        use yinhe_editor_core::history::{begin_edit, commit_ppq};
        let mut ppq_changed = false;
        {
            let doc = &mut self.workspace.documents[idx];
            let old = doc.data.model.meta.ppq;
            if old == value {
                return;
            }
            begin_edit(&mut doc.edit.pending_edits, id, &old.to_string());
            let has_notes = doc.data.model.note_count > 0;
            if has_notes {
                // 异步 rescale：meta.ppq 保持 old 作子线程基准，完成后 poll 里 commit。
                ctx.data_mut(|d| {
                    d.insert_temp(
                        egui::Id::unique(crate::app::rescale_state::RESCALE_REQUEST_ID),
                        crate::app::rescale_state::RescaleRequest {
                            old_ppq: old,
                            new_ppq: value,
                            dragvalue_id: id,
                        },
                    )
                });
            } else {
                std::sync::Arc::make_mut(&mut doc.data.model).meta.ppq = value;
                doc.data.bump_revision();
                commit_ppq(doc, id, value, false);
                ppq_changed = true;
            }
        }
        // PPQ 改了 tick↔sample 映射：空工程没有 rescale 任务，需立即重推音频模型。
        if ppq_changed {
            self.notify_audio_model_changed();
        }
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

impl App {
    /// 上下文动作（右键菜单可达，用户可自行分配快捷键）。
    ///
    /// 这些动作不在 `EditAction` 里：它们作用的对象是「当前选中的轨道 / 音频片段 /
    /// 光标位置」，而不是音符选区，硬塞进 `EditAction` 会让分发语义变糊。
    /// 键盘和右键菜单都调用这一个入口，保证两边行为一致。
    ///
    /// 返回是否真的执行了（`false` = 当前上下文不适用，例如没选中轨道）。
    pub(crate) fn handle_context_action(&mut self, action_id: &str) -> bool {
        use yinhe_editor_core::shortcuts as sc;
        let Some(idx) = self.workspace.active_doc else {
            return false;
        };
        match action_id {
            sc::ACTION_TRACK_ADD_BELOW | sc::ACTION_TRACK_ADD_ABOVE => {
                let doc = &mut self.workspace.documents[idx];
                // 「下方」= 当前选中轨之后；「上方」= 之前。
                let after = doc.edit.track_selected.iter().min().copied();
                let last = doc.data.model.tracks.len().saturating_sub(1);
                let after_idx = match (action_id, after) {
                    (sc::ACTION_TRACK_ADD_ABOVE, Some(i)) => i.saturating_sub(1) as usize,
                    (_, Some(i)) => i as usize,
                    (_, None) => last,
                };
                let before = doc.capture_snapshot();
                if let Some(undo) = doc.add_track(after_idx) {
                    doc.push_undo(undo, t!("undo.add_track").as_ref(), before);
                }
                self.notify_audio_model_changed();
                true
            }
            sc::ACTION_TRACK_MOVE_UP | sc::ACTION_TRACK_MOVE_DOWN => {
                let doc = &mut self.workspace.documents[idx];
                let Some(&sel) = doc.edit.track_selected.iter().min() else {
                    return false;
                };
                let idx = sel as usize;
                let label = if action_id == sc::ACTION_TRACK_MOVE_UP {
                    t!("undo.move_track_up")
                } else {
                    t!("undo.move_track_down")
                };
                let target = if action_id == sc::ACTION_TRACK_MOVE_UP {
                    idx.checked_sub(1)
                } else {
                    Some(idx + 1).filter(|&n| n < doc.data.model.tracks.len())
                };
                let Some(target) = target else {
                    return false;
                };
                let before = doc.capture_snapshot();
                if let Some(undo) = doc.move_track(idx, target) {
                    doc.push_undo(undo, label.as_ref(), before);
                }
                self.notify_audio_model_changed();
                true
            }
            sc::ACTION_TRACK_DELETE => {
                let doc = &mut self.workspace.documents[idx];
                let Some(&sel) = doc.edit.track_selected.iter().min() else {
                    return false;
                };
                let before = doc.capture_snapshot();
                if let Some(undo) = doc.remove_track(sel as usize) {
                    doc.push_undo(undo, t!("undo.remove_track").as_ref(), before);
                }
                self.notify_audio_model_changed();
                true
            }
            sc::ACTION_TRACK_PROPERTIES => {
                let doc = &mut self.workspace.documents[idx];
                if doc.edit.track_selected.is_empty() {
                    return false;
                }
                self.right_tab = Some(crate::right_panel::RightTab::Info);
                self.info_content = Some(crate::right_panel::InfoContent::Track);
                true
            }
            sc::ACTION_ADD_AUTOMATION => {
                // 需要用户挑参数：记下请求，由主循环（有 `ui.ctx()`）打开选择器。
                let doc = &mut self.workspace.documents[idx];
                let Some(&sel) = doc.edit.track_selected.iter().min() else {
                    return false;
                };
                self.pending_automation_picker = Some(sel as usize);
                true
            }
            sc::ACTION_DELETE_AUTOMATION => {
                let doc = &mut self.workspace.documents[idx];
                let Some(&sel) = doc.edit.track_selected.iter().min() else {
                    return false;
                };
                let Some(lane) = doc
                    .data
                    .model
                    .tracks
                    .get(sel as usize)
                    .and_then(|t| t.automation_lanes.len().checked_sub(1))
                else {
                    return false;
                };
                let before = doc.capture_snapshot();
                if let Some(undo) = doc.remove_automation_lane(sel as usize, lane) {
                    doc.push_undo(undo, t!("undo.delete_automation_lane").as_ref(), before);
                }
                self.notify_audio_model_changed();
                true
            }
            sc::ACTION_AUDIO_DELETE => self.delete_selected_audio_clips(),
            sc::ACTION_INSERT_MARKER => {
                self.insert_marker_at_cursor();
                true
            }
            sc::ACTION_DELETE_MARKER => {
                let doc = &mut self.workspace.documents[idx];
                let Some(tick) = doc.edit.cursor_tick.map(|t| t.max(0.0) as u32) else {
                    return false;
                };
                let has = doc
                    .data
                    .model
                    .conductor
                    .markers
                    .iter()
                    .any(|m| m.tick == tick);
                if !has {
                    return false;
                }
                doc.delete_marker_with_undo(tick, t!("undo.delete_marker_event").as_ref());
                true
            }
            // 其余音频片段操作（分割/复制/反向/归一化/增益）需要指针位置或片段
            // 上下文，只在右键菜单里提供；这里不猜测用户意图。
            _ => false,
        }
    }

    /// 处理编辑动作（transport bar 编辑 popup / 图钉与 macOS 菜单共用）。
    /// 与选择按钮共用的动作（删除/重复/移调/变速/翻转/人性化/填缝/合并/
    /// 网格切割）统一转发 `handle_selection_action`，避免两套实现漂移；
    /// copy/cut 在自动化锚点选中时作用于锚点；
    /// paste 按剪贴板内容类型分派（见 paste_clipboard）。
    pub(crate) fn handle_edit_action(&mut self, action: transport_bar::EditAction) {
        use crate::widgets::selection_actions::SelectionAction as S;
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
            // ── 与 transport bar 选择按钮共用：单一实现 ──
            A::Duplicate => self.handle_selection_action(S::Duplicate),
            A::Delete => self.handle_selection_action(S::Delete),
            A::TransposeUp => self.handle_selection_action(S::TransposeUp),
            A::TransposeDown => self.handle_selection_action(S::TransposeDown),
            A::LengthDouble => self.handle_selection_action(S::LengthDouble),
            A::LengthHalf => self.handle_selection_action(S::LengthHalf),
            A::FlipHorizontal => self.handle_selection_action(S::FlipHorizontal),
            A::FlipVertical => self.handle_selection_action(S::FlipVertical),
            A::Humanize => self.handle_selection_action(S::Humanize),
            A::FillGaps => self.handle_selection_action(S::FillGaps),
            A::MergeNotes => self.handle_selection_action(S::MergeNotes),
            A::GridSplit => self.handle_selection_action(S::GridConfirm),
            A::DedupWithinTrack => self.dedup_overlapping_notes(false),
            A::DedupAcrossTracks => self.dedup_overlapping_notes(true),
        }
    }

    /// 处理选择动作（transport bar 选择按钮 / PR 浮动工具条共用）。
    ///
    /// 有自动化锚点选中时，删除/重复/变速/水平翻转作用于锚点（与编辑菜单一致）；
    /// 移调/垂直翻转保持音符专用。
    pub(crate) fn handle_selection_action(
        &mut self,
        action: crate::widgets::selection_actions::SelectionAction,
    ) {
        use crate::widgets::selection_actions::SelectionAction as A;
        let route_to_automation = self.has_selected_automation_anchors();
        let ar_auto = self.has_selected_ar_automation();
        match action {
            A::Delete => {
                if ar_auto {
                    self.delete_ar_selection();
                } else if route_to_automation {
                    self.delete_automation_anchors();
                } else {
                    self.delete_selected_notes();
                }
            }
            A::Duplicate => {
                if ar_auto {
                    self.duplicate_ar_selection();
                } else if route_to_automation {
                    self.duplicate_automation_anchors();
                } else {
                    self.duplicate_selected_notes();
                }
            }
            A::LengthDouble => {
                if ar_auto {
                    self.stretch_ar_selection(2, 1);
                } else if route_to_automation {
                    self.stretch_automation_anchors(2, 1);
                } else {
                    self.double_selected_length();
                }
            }
            A::LengthHalf => {
                if ar_auto {
                    self.stretch_ar_selection(1, 2);
                } else if route_to_automation {
                    self.stretch_automation_anchors(1, 2);
                } else {
                    self.half_selected_length();
                }
            }
            A::FlipHorizontal => {
                if ar_auto {
                    self.flip_ar_selection_horizontal();
                } else if route_to_automation {
                    self.flip_automation_anchors_horizontal();
                } else {
                    self.flip_selected_notes(yinhe_editor_core::FlipAxis::Horizontal);
                }
            }
            A::TransposeUp => self.transpose_selected_notes(12),
            A::TransposeDown => self.transpose_selected_notes(-12),
            A::FlipVertical => self.flip_selected_notes(yinhe_editor_core::FlipAxis::Vertical),
            A::Humanize => self.humanize_selected_notes(),
            A::FillGaps => self.fill_gaps_selected_notes(),
            A::MergeNotes => self.merge_selected_notes(),
            A::GridConfirm => self.grid_split_selection(),
            A::LineConfirm => self.line_tool_confirm(),
            A::ScissorsConfirm => self.scissors_confirm(),
        }
    }

    /// 在光标 tick 处插入一个标签（MIDI marker），默认名 = `小节.拍.tick`。
    pub(crate) fn insert_marker_at_cursor(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        let tick = doc.edit.cursor_tick.unwrap_or(0.0).max(0.0) as u32;
        let text = {
            let model = &doc.data.model;
            let tpb = model.meta.ppq;
            let (def_num, def_den) = model.tempo_map.time_sig_default;
            let sig = model.tempo_map.time_sig_events.as_slice();
            yinhe_types::time_format::format_tick_bar_beat_with_time_sig(
                tick as f64,
                tpb,
                sig,
                def_num,
                def_den,
            )
        };
        doc.insert_marker_with_undo(tick, text, "undo.insert_marker_event");
    }

    /// 视图菜单「自动化面板」开关：设置当前文档的面板显示状态；
    /// 开启且无面板时补一个默认面板（与工具栏底部按钮的显隐逻辑一致）。
    /// 纵向瀑布流不提供 AM 视图，忽略请求。
    pub(crate) fn set_automation_panels(&mut self, show: bool) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        if doc.edit.pianoroll_view.is_vertical() {
            return;
        }
        doc.edit.show_controller_panels = show;
        if show && doc.edit.controller_panels.is_empty() {
            doc.edit
                .controller_panels
                .push(yinhe_types::AutomationPanelView::default());
        }
    }

    /// 钢琴卷帘方向落地（transport bar 方向按钮 / 视图菜单共用）：
    /// 持久化到 layout 设置（新文档沿用），并写入当前文档视口。
    pub(crate) fn set_pr_orientation(&mut self, orientation: yinhe_types::Orientation) {
        let vertical = orientation == yinhe_types::Orientation::Vertical;
        if self.audio_settings.layout.pr_vertical != vertical {
            self.audio_settings.layout.pr_vertical = vertical;
            self.audio_settings.save();
        }
        if let Some(doc) = self.workspace.active_doc_mut() {
            doc.edit.pianoroll_view.set_orientation(orientation);
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
            | transport_bar::FileAction::ExportMidi7z
            | transport_bar::FileAction::ExportAudio
            | transport_bar::FileAction::Settings
            | transport_bar::FileAction::ProjectSettings
            | transport_bar::FileAction::ImportAudio
            | transport_bar::FileAction::ImportMidi
            | transport_bar::FileAction::Welcome
            | transport_bar::FileAction::About
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

    /// 打开「最近修改的文件」（transport bar 子菜单 / 标题栏 popup / macOS 菜单栏共用）。
    /// 文件已被移动/删除时从列表移除并报错。
    ///
    /// 打开是**追加新文档**（不取代/关闭现有文档），因此不检查未保存修改：
    /// 触发保存确认只在会丢失当前文档的场景（关闭文档 / 新建 / 退出）。
    pub(crate) fn open_recent_file(&mut self, path: &str) {
        if self.prune_missing_recent_file(path) {
            return;
        }
        // 打开意图是"进入编辑"，关闭可能显示中的欢迎页面。
        self.show_welcome_page = false;
        self.file_loader
            .load_path(path.to_string(), self.audio_settings.midi_import_encoding);
    }

    /// 最近文件已被移动/删除：移除记录、保存并报错，返回 true（调用方直接返回）。
    /// 「打开为文档」与「新窗口打开」两个入口共用。
    pub(crate) fn prune_missing_recent_file(&mut self, path: &str) -> bool {
        if std::path::Path::new(path).exists() {
            return false;
        }
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
        true
    }

    /// 执行欢迎页动作（独立欢迎窗口 / 主界面无工程占位共用入口）。
    ///
    /// 映射到现有文件动作与最近文件逻辑，保证与菜单/走带栏入口行为一致。
    pub(crate) fn handle_welcome_action(
        &mut self,
        action: crate::welcome::WelcomeAction,
        ctx: &egui::Context,
    ) {
        use crate::welcome::WelcomeAction;
        match action {
            // 新建 / 打开 / 最近：目标动作内部会关闭欢迎页面（进入编辑语义）。
            WelcomeAction::NewProject => {
                self.handle_file_action(transport_bar::FileAction::NewProject, ctx);
            }
            WelcomeAction::OpenProject => {
                self.handle_file_action(transport_bar::FileAction::Open, ctx);
            }
            WelcomeAction::OpenRecent(path) => self.open_recent_file(&path),
            WelcomeAction::OpenSettings => {
                self.handle_file_action(transport_bar::FileAction::Settings, ctx);
            }
        }
    }

    /// Execute a file action immediately without checking for unsaved changes.
    fn execute_file_action(&mut self, action: transport_bar::FileAction, ctx: &egui::Context) {
        match action {
            transport_bar::FileAction::NewProject => {
                self.new_project();
            }
            transport_bar::FileAction::Open => {
                // 打开意图是"进入编辑"，关闭可能显示中的欢迎页面。
                self.show_welcome_page = false;
                self.file_loader
                    .pick_file(self.audio_settings.midi_import_encoding);
            }
            transport_bar::FileAction::ImportAudio => {
                self.import_audio_dialog();
            }
            transport_bar::FileAction::ImportMidi => {
                self.import_midi_dialog(ctx);
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
            transport_bar::FileAction::ExportMidi7z => {
                self.export_midi_archive_dialog();
            }
            transport_bar::FileAction::ExportAudio => {
                self.export_audio_dialog(ctx);
            }
            transport_bar::FileAction::Exit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            transport_bar::FileAction::Settings => {
                self.audio_settings.ui_session.show_settings = true;
                crate::chrome::dialog::raise_viewport(
                    ctx,
                    egui::ViewportId::from_hash_of("settings_dialog"),
                );
            }
            transport_bar::FileAction::Welcome => {
                // 打开独立欢迎窗口（与启动欢迎窗口同款）；已打开时拉到前台。
                self.show_welcome_page = true;
                crate::chrome::dialog::raise_viewport(ctx, crate::welcome::viewport_id());
            }
            transport_bar::FileAction::About => {
                self.show_about = true;
                crate::chrome::dialog::raise_viewport(ctx, crate::dialogs::about::viewport_id());
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
                // 退出 = 保存所有脏文档，而不是只保存当前活动文档（此前其余
                // 脏标签会被直接丢弃）。逐个保存，每次保存完成后由 poll 再次
                // 进入这里推进队列。
                match self.save_next_dirty_for_exit() {
                    ExitSaveStep::Started => {
                        self.pending_unsaved = Some(PendingFileAction::Exit);
                    }
                    ExitSaveStep::Done => {
                        self.should_exit = true;
                    }
                    ExitSaveStep::Cancelled => {}
                }
            }
        }
    }

    /// 用户明确选择“不保存”：直接执行目标动作，不做任何保存。
    pub(crate) fn discard_pending_file_action(&mut self) {
        let Some(pending) = self.pending_unsaved.take() else {
            return;
        };
        match pending {
            PendingFileAction::NewProject => self.new_project(),
            PendingFileAction::CloseDocument(idx) => self.close_document(idx),
            PendingFileAction::Exit => self.should_exit = true,
        }
    }

    /// 退出流程：保存下一个脏文档。全部保存完返回 `Done`；无路径文档的
    /// “另存为”被取消时返回 `Cancelled`（取消退出，避免死循环弹窗）。
    fn save_next_dirty_for_exit(&mut self) -> ExitSaveStep {
        // 上一个保存任务还没结束：保持 pending，等回调推进。
        if self.jobs.is_saving() {
            return ExitSaveStep::Started;
        }
        let Some(idx) = self.workspace.documents.iter().position(|d| d.is_dirty()) else {
            return ExitSaveStep::Done;
        };
        match self.workspace.documents[idx].file_path.clone() {
            Some(path) => {
                self.save_project_async(idx, path);
                ExitSaveStep::Started
            }
            None => {
                if self.save_as_dialog_for(idx) {
                    ExitSaveStep::Started
                } else {
                    ExitSaveStep::Cancelled
                }
            }
        }
    }

    /// 保存前把编辑态同步进 model/project_file，并抓取可离线保存的快照。
    /// 保存线程只持有快照，不再借用 `self`；保存与自动保存共用。
    pub(crate) fn take_save_snapshot(
        &mut self,
        idx: usize,
        sync_plugin_state: bool,
    ) -> SaveSnapshot {
        let doc = &mut self.workspace.documents[idx];
        doc.sync_overrides_to_model();
        doc.data.sync_project_file();
        doc.data.sync_mapping_file();
        doc.sync_sf_overrides_to_project_file();

        // 插件实例不是 Send（CLAP/VST3 带原生窗口/COM 对象），无法搬到后台线程；
        // 自动保存跳过这一步，避免每 5 分钟在主线程逐个插件调 save_state。
        // 手工保存仍然同步，保证插件状态落盘。
        if sync_plugin_state {
            // 混音台插件：保存前把实例的 CLAP state 写回 InsertRef（旁通标志同步）。
            if let Some(rack) = self.mixer_racks.get_mut(idx) {
                rack.sync_states_to(&mut self.workspace.documents[idx].mixer);
            }
            // 乐器插件：同样把实例 state 写回 mixer.instruments[channel]。
            if let Some(irack) = self.instrument_racks.get_mut(idx) {
                irack.sync_states_to(&mut self.workspace.documents[idx].mixer);
            }
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
        // 同一时刻只允许一个保存任务：否则两个后台线程会同时写同一路径，
        // 且 `jobs.save_rx` 会被后来的覆盖，前一次结果丢失、脏标记错乱。
        if self.jobs.is_saving() {
            tracing::warn!("[save] 上一次保存尚未完成，忽略本次请求: {path}");
            return;
        }
        let snap = self.take_save_snapshot(idx, true);
        let path_for_thread = path.clone();
        // 发起时的状态标记：保存期间的新编辑不应被误标为已保存
        let (saved_marker, saved_mixer_revision) = self
            .workspace
            .documents
            .get(idx)
            .map(|d| (d.undo_marker(), d.mixer_revision))
            .unwrap_or((None, 0));

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
            let _ = tx.send((
                idx,
                saved_marker,
                saved_mixer_revision,
                path_for_thread,
                result,
            ));
        });

        self.jobs.save_rx = Some(rx);
        self.jobs.save_progress_rx = Some(progress_rx);
    }

    pub(crate) fn save_as_dialog(&mut self) {
        if let Some(idx) = self.workspace.active_doc {
            self.save_as_dialog_for(idx);
        }
    }

    /// 另存为（指定文档；未保存确认弹窗的目标可能是非 active 文档）。
    /// 路径/文件名在**保存成功后**才写入（见 poll 的保存完成处理）。
    /// 返回是否已发起保存（`false` = 用户取消了文件对话框）。
    pub(crate) fn save_as_dialog_for(&mut self, idx: usize) -> bool {
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
            true
        } else {
            false
        }
    }

    fn export_midi_dialog(&mut self) {
        if self.jobs.is_midi_exporting() {
            return;
        }
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
            let Some(idx) = self.workspace.active_doc else {
                return;
            };
            let path_str = path.to_string_lossy().to_string();
            let (model, opts) = {
                let doc = &self.workspace.documents[idx];
                (
                    std::sync::Arc::clone(&doc.data.model),
                    yinhe_midi::MidiExportOptions {
                        encoding: self.audio_settings.midi_export_encoding,
                        rpn_full: self.audio_settings.midi_export_rpn_full,
                        curve_interpolate: self.audio_settings.midi_export_curve_interpolate,
                        curve_density: self.audio_settings.midi_export_curve_density,
                        strip_empty_tracks: self.audio_settings.midi_export_strip_empty_tracks,
                        dedup_overlaps: self.audio_settings.midi_export_dedup_overlaps,
                    },
                )
            };
            let live = std::sync::Arc::new(std::sync::Mutex::new(
                crate::app::export_state::MidiExportLive { progress: None },
            ));
            self.jobs.midi_export.live = std::sync::Arc::clone(&live);
            self.jobs.midi_export.path = path_str;
            let (tx, rx) = mpsc::channel();
            self.jobs.midi_export.rx = Some(rx);
            self.jobs.midi_export.running = true;
            let path_buf = path.clone();
            let spawned = std::thread::Builder::new()
                .name("midi-export".into())
                .spawn(move || {
                    let result = yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Midi, || {
                        yinhe_midi::write_to_file_with_progress(&model, &path_buf, &opts, |p| {
                            if let Ok(mut l) = live.lock() {
                                l.progress = Some(p);
                            }
                        })
                    })
                    .map_err(|e| e.to_string());
                    let _ = tx.send(result);
                });
            if let Err(e) = spawned {
                self.jobs.midi_export.running = false;
                self.jobs.midi_export.rx = None;
                self.notifications
                    .error("导出MIDI失败", format!("无法启动导出线程: {e}"));
            }
        }
    }

    fn export_audio_dialog(&mut self, ctx: &egui::Context) {
        if self.workspace.active_doc.is_none() {
            return;
        }

        if self.jobs.is_exporting() {
            return; // already exporting
        }

        // Show export settings dialog first
        self.jobs.export.show_bit_depth = true;
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

        let sr = if self.jobs.export.sample_rate > 0 {
            self.jobs.export.sample_rate
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

        let bit_depth = self.jobs.export.bit_depth;
        let layer_count = if self.jobs.export.layer_count == 0 {
            None
        } else {
            Some(self.jobs.export.layer_count as usize)
        };
        let export_progress = self.jobs.export.progress.clone();
        let cancel_flag = self.jobs.export.cancel.clone();
        let pause_flag = self.jobs.export.pause.clone();
        // 记下输出路径：中止卡“打开文件夹”按钮用。
        self.jobs.export.last_output_path = Some(path_str.clone());
        // 音频完成后写 WAV LIST/INFO；元数据取发起导出时的快照。
        self.jobs.export.tags = if self.jobs.export.write_metadata {
            self.workspace.active_doc.map(|i| {
                (
                    path_str.clone(),
                    yinhe_core::SongInfo::from_meta(&self.workspace.documents[i].data.model.meta),
                )
            })
        } else {
            None
        };
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
            self.jobs.export.running = true;
        }
    }
}
