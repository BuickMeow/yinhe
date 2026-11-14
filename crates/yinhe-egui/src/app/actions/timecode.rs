//! 走带栏时间码直接输入（BPM / 拍号 / PPQ）。

use eframe::egui;
use rust_i18n::t;

use crate::app::App;

impl App {
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
}
