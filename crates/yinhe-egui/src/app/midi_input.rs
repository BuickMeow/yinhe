//! MIDI 输入管理：连接状态对齐 + 事件消费（直通试听 + 实时录音）。
//!
//! 每帧由 main_loop 调用 [App::poll_midi_input]：
//! - 设置关闭直通/未选设备 → 断开连接；
//! - 设置选中设备 → 打开连接（设备名变更时重连）；
//! - 连接存活 → drain 缓冲：
//!   - 直通：NoteOn → `PreviewReq::Note`（持续音），NoteOff → Stop + 重发仍按住的键；
//!   - 录音：NoteOn → 立即写入当前轨（gate=1 占位），NoteOff → 闭合 gate；
//!     停止录音时残留音符补默认 gate，全部动作合并为一次 undo。

use std::collections::HashMap;
use std::time::Instant;

use rust_i18n::t;

use crate::app::App;
use crate::piano_view::{NotePreview, PreviewReq};
use yinhe_editor_core::history::UndoAction;
use yinhe_midi_io::{MidiEvent, MidiInputStream};

/// 录音状态（App::recording 存活期间有效）。
pub(crate) struct RecordingState {
    /// 录音开始墙钟时刻（tick 由 tempo map 从它换算）。
    pub started_at: Instant,
    /// 录音起点对应歌曲时间（秒），tempo map 换算基准。
    pub start_secs: f64,
    /// 按住的键 → 已写入音符的 id 等信息（NoteOff 时闭合 gate）。
    pub pending: HashMap<u8, PendingRecordingNote>,
    /// 录音期间全部编辑动作（停止时合并为 Composite 一次提交 undo）。
    pub undo_actions: Vec<UndoAction>,
    /// 录音开始时的编辑快照（undo 恢复用）。
    pub before_snapshot: yinhe_editor_core::history::EditSnapshot,
}

/// 录音中按住的音符（待 NoteOff 闭合 gate）。
pub(crate) struct PendingRecordingNote {
    pub key: u8,
    pub note_id: u32,
}

impl App {
    /// 每帧调用：对齐 MIDI 输入连接状态并消费事件。
    pub(crate) fn poll_midi_input(&mut self) {
        self.sync_midi_connection();
        self.consume_midi_events();
    }

    /// 按设置（直通开关 + 设备选择）对齐连接：关闭/未选 → 断开；设备名变化 → 重连。
    /// 录音/步进输入也需要连接（即使直通关闭），所以三者任一激活即连接。
    fn sync_midi_connection(&mut self) {
        let enabled = self.audio_settings.midi_thru || self.recording.is_some() || self.step_input;
        let device = self.audio_settings.midi_input_device.clone();
        if !enabled || device.is_none() {
            if self.midi_input.take().is_some() {
                self.midi_thru_keys.clear();
                self.midi_connected_device = None;
            }
            return;
        }
        let Some(name) = device else {
            return;
        };
        if self.midi_connected_device.as_deref() != Some(name.as_str()) {
            self.midi_input = MidiInputStream::open(&name).ok();
            self.midi_connected_device = if self.midi_input.is_some() {
                Some(name)
            } else {
                None // 打开失败（设备刚拔出），下帧重试
            };
            self.midi_thru_keys.clear();
        }
    }

    /// 开始 MIDI 录音：记录起点（光标 tick + tempo map 时间），清空 pending。
    pub(crate) fn start_recording(&mut self) {
        if self.recording.is_some() {
            return;
        }
        let Some(idx) = self.active_doc else {
            return;
        };
        let start_tick = self.last_cursor_tick.unwrap_or(0.0).max(0.0) as u32;
        let tempo_map = &self.documents[idx].data.model.tempo_map;
        let start_secs = tempo_map.tick_to_seconds(start_tick as u64);
        let before_snapshot = self.documents[idx].capture_snapshot();
        self.recording = Some(RecordingState {
            started_at: Instant::now(),
            start_secs,
            pending: HashMap::new(),
            undo_actions: Vec::new(),
            before_snapshot,
        });
    }

    /// 停止 MIDI 录音：闭合残留音符、合并 undo、刷新视图。
    pub(crate) fn stop_recording(&mut self) {
        let Some(state) = self.recording.take() else {
            return;
        };
        let Some(idx) = self.active_doc else {
            return;
        };
        let mut undo_actions = state.undo_actions;

        // 残留未松开的键：用停止时刻闭合 gate
        if !state.pending.is_empty() {
            let end_secs = state.start_secs + state.started_at.elapsed().as_secs_f64();
            let end_tick = self.tick_at_secs(end_secs, idx);
            for p in state.pending.values() {
                if let Some(a) = self.documents[idx].set_note_end_tick(p.key, p.note_id, end_tick) {
                    undo_actions.push(a);
                }
            }
        }

        if undo_actions.is_empty() {
            return;
        }
        let doc = &mut self.documents[idx];
        doc.push_undo(
            UndoAction::Composite(undo_actions),
            t!("undo.record").as_ref(),
            state.before_snapshot,
        );
        doc.data.bump_revision();
        self.pianoroll_view.base.dirty = true;
        self.arrange_view.base.dirty = true;
        self.notify_notes_changed();
    }

    /// 消费连接缓冲中的 MIDI 消息：录音写入 + 直通发声。
    fn consume_midi_events(&mut self) {
        let Some(stream) = &self.midi_input else {
            return;
        };
        let raw = stream.drain();
        if raw.is_empty() {
            return;
        }

        let mut previews: Vec<PreviewReq> = Vec::new();
        let mut any_note_off = false;
        for msg in &raw {
            let Some(ev) = yinhe_midi_io::parse_event(msg) else {
                continue;
            };
            match ev {
                MidiEvent::NoteOn { key, velocity, .. } => {
                    // 写入路径：录音优先（实时），否则步进输入（逐键写入）
                    if self.recording.is_some() {
                        self.handle_recording_note_on(key, velocity);
                    } else if self.step_input {
                        self.handle_step_input_note_on(key, velocity);
                    }
                    // 直通发声
                    if let std::collections::hash_map::Entry::Vacant(e) =
                        self.midi_thru_keys.entry(key)
                    {
                        e.insert(velocity);
                        previews.push(PreviewReq::Note(NotePreview {
                            track: self.current_editing_track(),
                            key,
                            velocity: Some(velocity),
                            target_tick: self.current_preview_tick(),
                            duration_ticks: 0,
                        }));
                    }
                }
                MidiEvent::NoteOff { key, .. } => {
                    if self.recording.is_some() {
                        self.handle_recording_note_off(key);
                    }
                    if self.midi_thru_keys.remove(&key).is_some() {
                        any_note_off = true;
                    }
                }
                _ => {}
            }
        }

        if any_note_off {
            // PreviewStop 是全局停：先停全部，再重发仍按住的键（和弦保持连续）。
            previews.push(PreviewReq::Stop);
            for (&key, &velocity) in &self.midi_thru_keys {
                previews.push(PreviewReq::Note(NotePreview {
                    track: self.current_editing_track(),
                    key,
                    velocity: Some(velocity),
                    target_tick: self.current_preview_tick(),
                    duration_ticks: 0,
                }));
            }
        }
        self.send_note_previews(&previews);
    }

    /// 录音 NoteOn：向当前轨写入 gate=1 占位音符，记录 id 待闭合。
    fn handle_recording_note_on(&mut self, key: u8, velocity: u8) {
        if self.recording.is_none()
            || self
                .recording
                .as_ref()
                .is_some_and(|r| r.pending.contains_key(&key))
        {
            return;
        }
        let Some(idx) = self.active_doc else {
            return;
        };
        let track = self.current_editing_track();
        let start_tick = self.recording_current_tick(idx);
        let note = yinhe_core::NoteEvent {
            id: 0,
            start_tick,
            end_tick: start_tick + 1,
            key,
            velocity,
        };
        let action = self.documents[idx].add_note(track, note);
        let Some(action) = action else {
            return;
        };
        let note_id = match &action {
            UndoAction::Notes(d) => d.after[0].0.id,
            _ => return,
        };
        let Some(state) = &mut self.recording else {
            return;
        };
        state
            .pending
            .insert(key, PendingRecordingNote { key, note_id });
        state.undo_actions.push(action);
        let doc = &mut self.documents[idx];
        doc.data.bump_revision();
        self.pianoroll_view.base.dirty = true;
        self.arrange_view.base.dirty = true;
        self.notify_notes_changed();
    }

    /// 录音 NoteOff：闭合对应音符的 gate。
    fn handle_recording_note_off(&mut self, key: u8) {
        let Some(idx) = self.active_doc else {
            return;
        };
        // 先算闭合时刻（借用 recording 只读），再取出 pending（可变借用），避免冲突
        let end_secs = {
            let Some(state) = &self.recording else {
                return;
            };
            state.start_secs + state.started_at.elapsed().as_secs_f64()
        };
        let Some(p) = ({
            let Some(state) = &mut self.recording else {
                return;
            };
            state.pending.remove(&key)
        }) else {
            return;
        };
        let end_tick = self.tick_at_secs(end_secs, idx);
        let action = self.documents[idx].set_note_end_tick(p.key, p.note_id, end_tick);
        if let Some(a) = action
            && let Some(state) = &mut self.recording
        {
            state.undo_actions.push(a);
        }
        let doc = &mut self.documents[idx];
        doc.data.bump_revision();
        self.pianoroll_view.base.dirty = true;
        self.arrange_view.base.dirty = true;
        self.notify_notes_changed();
    }

    /// 步进输入 NoteOn：在光标处写入一个默认长度（四分音符）音符并前进一个步长。
    fn handle_step_input_note_on(&mut self, key: u8, velocity: u8) {
        let Some(idx) = self.active_doc else {
            return;
        };
        let Some(cursor) = self.documents[idx].edit.cursor_tick else {
            return;
        };
        let track = self.current_editing_track();
        let step = self.documents[idx].data.model.meta.ppq.max(1);
        let start = cursor.max(0.0) as u32;
        let note = yinhe_core::NoteEvent {
            id: 0,
            start_tick: start,
            end_tick: start + step,
            key,
            velocity,
        };
        self.add_note_with_undo(track, note);
        // 前进一个步长（光标与跨视图同步 tick）
        let next = (start + step) as f64;
        self.documents[idx].edit.cursor_tick = Some(next);
        self.last_cursor_tick = Some(next);
    }

    /// 录音中当前 tick：录音起点 + 墙钟流逝，经 tempo map 换算（变速正确）。
    fn recording_current_tick(&self, doc_idx: usize) -> u32 {
        let Some(state) = &self.recording else {
            return 0;
        };
        let secs = state.start_secs + state.started_at.elapsed().as_secs_f64();
        self.tick_at_secs(secs, doc_idx)
    }

    /// 歌曲时间（秒）→ tick（tempo map 换算）。
    fn tick_at_secs(&self, secs: f64, doc_idx: usize) -> u32 {
        let tempo_map = &self.documents[doc_idx].data.model.tempo_map;
        tempo_map.tick_at_time(secs).max(0.0) as u32
    }

    /// 当前编辑音轨（无文档/无编辑轨时回退 0）。
    fn current_editing_track(&self) -> u16 {
        self.active_doc
            .and_then(|i| self.documents.get(i))
            .and_then(|d| d.edit.editing_track)
            .unwrap_or(0)
    }

    /// 直通预览的自动化采样点：当前光标 tick。
    fn current_preview_tick(&self) -> u32 {
        self.last_cursor_tick.unwrap_or(0.0).max(0.0) as u32
    }
}
