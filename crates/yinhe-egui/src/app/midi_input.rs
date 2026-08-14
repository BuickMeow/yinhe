//! MIDI 输入管理：连接状态对齐 + 事件消费（直通试听）。
//!
//! 每帧由 main_loop 调用 [App::poll_midi_input]：
//! - 设置关闭直通/未选设备 → 断开连接；
//! - 设置选中设备 → 打开连接（设备名变更时重连）；
//! - 连接存活 → drain 缓冲，NoteOn 走 `PreviewReq::Note`（持续音，duration_ticks=0），
//!   NoteOff 走 `PreviewReq::Stop` + 重发仍按住的键（和弦中松开一键其余音保持）。

use crate::app::App;
use crate::piano_view::{NotePreview, PreviewReq};
use yinhe_midi_io::{MidiEvent, MidiInputStream};

impl App {
    /// 每帧调用：对齐 MIDI 输入连接状态并消费事件。
    pub(crate) fn poll_midi_input(&mut self) {
        self.sync_midi_connection();
        self.consume_midi_events();
    }

    /// 按设置（直通开关 + 设备选择）对齐连接：
    /// 关闭/未选 → 断开；设备名变化 → 重连。
    fn sync_midi_connection(&mut self) {
        let enabled = self.audio_settings.midi_thru;
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

    /// 消费连接缓冲中的 MIDI 消息并触发预览发声。
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
                    if let std::collections::hash_map::Entry::Vacant(e) =
                        self.midi_thru_keys.entry(key)
                    {
                        e.insert(velocity);
                        previews.push(PreviewReq::Note(NotePreview {
                            track: self.current_editing_track(),
                            key,
                            velocity: Some(velocity),
                            target_tick: self.current_preview_tick(),
                            duration_ticks: 0, // 持续音，NoteOff 时停止
                        }));
                    }
                }
                MidiEvent::NoteOff { key, .. } if self.midi_thru_keys.remove(&key).is_some() => {
                    any_note_off = true;
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
