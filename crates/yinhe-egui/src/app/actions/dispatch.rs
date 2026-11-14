//! 编辑 / 选择 / 上下文动作的统一分发。

use rust_i18n::t;

use crate::app::App;
use crate::chrome::transport_bar;

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
}
