//! 时间线右键菜单动作落地（AR / 通道 AR 共用）。
//!
//! 菜单在 `arrange::context_menu` 里只产出命令；这里统一转成已有的
//! `handle_edit_action` / `handle_context_action` / `handle_insert_action`
//! 路径，保证右键菜单、快捷键、顶栏菜单共用同一套 undo 与音频同步逻辑。

use eframe::egui;
use rust_i18n::t;

use crate::app::App;
use crate::arrange::context_menu::TimelineMenuAction;

impl App {
    /// 执行时间线右键菜单动作。
    pub(crate) fn apply_timeline_menu_actions(
        &mut self,
        ctx: &egui::Context,
        actions: Vec<TimelineMenuAction>,
    ) {
        for action in actions {
            match action {
                TimelineMenuAction::Edit { action, tick } => {
                    self.set_menu_cursor_tick(tick);
                    self.handle_edit_action(action);
                }
                TimelineMenuAction::Context { id, tick } => {
                    self.set_menu_cursor_tick(tick);
                    self.handle_context_action(id);
                }
                TimelineMenuAction::Insert { action, tick } => {
                    self.set_menu_cursor_tick(tick);
                    self.handle_insert_action(action, ctx);
                }
                TimelineMenuAction::Batch => {
                    self.right_tab = Some(crate::right_panel::RightTab::Info);
                    self.right_panel_layout
                        .focus_tab(yinhe_editor_core::right_panel_layout::PanelKind::Batch);
                }
                TimelineMenuAction::NewMidiClip { track, tick } => {
                    self.new_midi_clip_at(track, tick);
                }
                TimelineMenuAction::Deselect => {
                    if let Some(idx) = self.workspace.active_doc {
                        let doc = &mut self.workspace.documents[idx];
                        doc.edit.selected.clear();
                        doc.edit.selected.clear_rects();
                        doc.edit.arr_sel_rect.clear();
                        doc.edit.chan_ar_sel.clear();
                        doc.edit.selected_audio_clips.clear();
                        doc.edit.sel_rect.clear();
                        for panel in &mut doc.edit.controller_panels {
                            panel.anchor_sel_rects.clear();
                        }
                        for view in doc.edit.arr_am_views.values_mut() {
                            view.anchor_sel_rects.clear();
                        }
                    }
                }
            }
        }
    }

    fn set_menu_cursor_tick(&mut self, tick: f64) {
        if let Some(idx) = self.workspace.active_doc {
            self.workspace.documents[idx].edit.cursor_tick = Some(tick.max(0.0));
        }
    }

    /// 在右键落点新建 1 小节空 MIDI Clip（与 AR 双击空白一致）。
    fn new_midi_clip_at(&mut self, track: u16, tick: f64) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        if !doc
            .data
            .model
            .tracks
            .get(track as usize)
            .is_some_and(|t| t.kind == yinhe_core::TrackKind::Midi)
        {
            return;
        }
        let ppq = doc.data.model.meta.ppq;
        let (num, den) = doc
            .data
            .model
            .tempo_map
            .time_sig_at_tick(tick.max(0.0) as u32);
        let bar = yinhe_types::measure_ticks(ppq, num, den).max(1);
        let start = (tick.max(0.0) as u32 / bar) * bar;
        let before = doc.capture_snapshot();
        if let Some(action) = doc.create_empty_clip(track, start, bar) {
            doc.push_undo(action, t!("undo.move_notes").as_ref(), before);
        }
        self.notify_audio_model_changed();
    }
}
