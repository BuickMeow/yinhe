//! 音符生命周期：跨点音符重启、mute 即时杀音、unmute 恢复。

use std::collections::BinaryHeap;

use yinhe_mixer::PluginEvent;
use yinhe_types::KEY_COUNT;

use crate::audio_model::ActiveNote;
use crate::engine::AudioEngine;

impl AudioEngine {
    /// 扫描与 `tick` 相交的可听音符（`start < tick < end`）。
    ///
    /// 用块级 `max_end_tick` 找到第一个可能跨过 `tick` 的块，不再依赖全局
    /// 最长音符窗口。旧实现扫 `notes[..cursor]` 是从曲首开始的 O(N) 前缀
    /// 扫描，seek 越晚越慢。返回 `(key, note, 0 BPM 截断后的 end_tick)`。
    pub(super) fn crossing_notes(&self, tick: u32) -> Vec<(usize, yinhe_types::Note, u32)> {
        let Some(model) = self.playback.yin_model.as_ref() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for key in 0..KEY_COUNT {
            for n in model.notes[key].range_including_long(tick, tick.saturating_add(1)) {
                if n.start_tick >= tick {
                    continue;
                }
                if let Some(end_tick) =
                    crate::prepare_model::audible_end(model, n, self.synth.ignore_velocity)
                    && end_tick > tick
                {
                    out.push((key, *n, end_tick));
                }
            }
        }
        out
    }

    /// 重启当前 tick 处仍有效的乐器跨点音符（pause→resume 用）。
    pub(super) fn restart_crossing_instrument_notes(&mut self) {
        let tick = self.playback.current_tick;
        for (key, n, end_tick) in self.crossing_notes(tick) {
            let Some(ch) = self
                .playback
                .model
                .as_ref()
                .and_then(|m| m.track_channel(n.track as usize))
            else {
                continue;
            };
            if self.channel_plugin_dense(ch).is_some() {
                self.restart_note(key, &n, end_tick);
            }
        }
    }

    /// 重启一个跨点音符（seek / unmute 复用）：NoteOn + 记入 active_notes。
    /// `key`：桶索引。`n`：源音符（已是当前 tick 之前的跨点音符，由调用方过滤
    /// `end_tick > tick`）；`end_tick` 为 0 BPM 截断后的实际结束 tick。
    /// 同步检查 skip_track（mute 轨跳过不重启）。
    pub(super) fn restart_note(&mut self, key: usize, n: &yinhe_types::Note, end_tick: u32) {
        let track = n.track as usize;
        let Some(ch) = self
            .playback
            .model
            .as_ref()
            .and_then(|m| m.track_channel(track))
        else {
            return;
        };
        let ch = ch as usize;
        if self
            .playback
            .skip_track
            .get(track)
            .copied()
            .unwrap_or(false)
        {
            return;
        }
        if let Some(dense) = self.channel_plugin_dense(ch as u8) {
            // 插件通道：chase 重启的音符喂乐器实例（time 0 = 下一块开头）。
            if let Some(Some(slot)) = self.plugins.instruments.get_mut(dense) {
                slot.events.push(PluginEvent::NoteOn {
                    time: 0,
                    channel: (ch & 0x0F) as u8,
                    key: key as u8,
                    velocity: n.velocity as f64 / 127.0,
                });
                self.playback
                    .active_notes
                    .push(std::cmp::Reverse(ActiveNote {
                        key: key as u8,
                        dense: dense as u32,
                        clap_channel: (ch & 0x0F) as u8,
                        is_instrument: true,
                        end_tick,
                        track: track as u16,
                    }));
            }
            return;
        }
        let dense = self.channel_layout.dense_for(ch);
        if dense == u32::MAX {
            return;
        }
        let sample = self.playback.sample_position;
        let end_sample = self.tick_to_sample(end_tick);
        self.synth
            .cpu_synth
            .send_event(yinhe_synth::SynthEvent::NoteOn {
                track: track as u16,
                sample,
                channel: dense as u8,
                key: key as u8,
                velocity: n.velocity,
                end_sample,
            });
        self.playback
            .active_notes
            .push(std::cmp::Reverse(ActiveNote {
                key: key as u8,
                dense,
                clap_channel: 0,
                is_instrument: false,
                end_tick,
                track: track as u16,
            }));
    }

    /// 即时 mute：从 active_notes 精确移除该轨在响音符并发 NoteOff
    ///（不误伤同通道其他轨道；即时静音语义）。
    pub(super) fn kill_track_notes(&mut self, track: u16) {
        let all = std::mem::take(&mut self.playback.active_notes);
        let mut remaining = BinaryHeap::new();
        for std::cmp::Reverse(an) in all.into_iter() {
            if an.track != track {
                remaining.push(std::cmp::Reverse(an));
                continue;
            }
            if an.is_instrument {
                if let Some(Some(slot)) = self.plugins.instruments.get_mut(an.dense as usize) {
                    slot.events.push(PluginEvent::NoteOff {
                        time: 0,
                        channel: an.clap_channel,
                        key: an.key,
                        velocity: 0.0,
                    });
                }
            } else if an.dense != u32::MAX {
                let sample = self.playback.sample_position;
                self.synth
                    .cpu_synth
                    .send_event(yinhe_synth::SynthEvent::NoteOff {
                        sample,
                        channel: an.dense as u8,
                        key: an.key,
                    });
            }
        }
        self.playback.active_notes = remaining;
    }

    /// 即时 unmute：重启该轨的跨点音符（start < current_tick < end）。
    /// 只扫 `max_note_len` 窗口，并按轨道过滤。
    pub(super) fn restart_track_crossing_notes(&mut self, track: u16) {
        let tick = self.playback.current_tick;
        for (key, n, end_tick) in self.crossing_notes(tick) {
            if n.track == track {
                self.restart_note(key, &n, end_tick);
            }
        }
    }
}
