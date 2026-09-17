//! 音符生命周期：跨点音符重启、mute 即时杀音、unmute 恢复。

use std::collections::BinaryHeap;

use xsynth_core::channel::{ChannelAudioEvent, ChannelEvent};
use xsynth_core::channel_group::SynthEvent;
use yinhe_mixer::PluginEvent;
use yinhe_types::KEY_COUNT;

use crate::audio_model::{ActiveNote, AudibleNote};
use crate::engine::AudioEngine;

impl AudioEngine {
    /// 重启当前 tick 处仍有效的乐器跨点音符（pause→resume 用）。
    pub(super) fn restart_crossing_instrument_notes(&mut self) {
        let tick = self.current_tick;
        for key in 0..KEY_COUNT {
            let notes = self.audible_notes[key].as_slice();
            let cursor = notes.partition_point(|n| n.start_tick < tick);
            let mut to_restart: Vec<AudibleNote> = Vec::new();
            for n in &notes[..cursor] {
                if n.end_tick > tick {
                    let ch = self
                        .model
                        .as_ref()
                        .map(|m| m.track_channel(n.track as usize))
                        .unwrap_or(0);
                    if self.channel_plugin_dense(ch).is_some() {
                        to_restart.push(*n);
                    }
                }
            }
            for n in to_restart {
                self.restart_note(key, &n);
            }
        }
    }

    /// 重启一个跨点音符（seek / unmute 复用）：NoteOn + 记入 active_notes。
    /// `key`：桶索引。`n`：audible_notes 里的源音符（已是当前 tick 之前的跨点音符，
    /// 由调用方过滤 end_tick > tick）。同步检查 skip_track（mute 轨跳过不重启）。
    pub(super) fn restart_note(&mut self, key: usize, n: &AudibleNote) {
        let track = n.track as usize;
        let ch = self
            .model
            .as_ref()
            .map(|m| m.track_channel(track) as usize)
            .unwrap_or(0);
        if self.skip_track.get(track).copied().unwrap_or(false) {
            return;
        }
        if let Some(dense) = self.channel_plugin_dense(ch as u8) {
            // 插件通道：chase 重启的音符喂乐器实例（time 0 = 下一块开头）。
            if let Some(Some(slot)) = self.instruments.get_mut(dense) {
                slot.events.push(PluginEvent::NoteOn {
                    time: 0,
                    channel: (ch & 0x0F) as u8,
                    key: key as u8,
                    velocity: n.velocity as f64 / 127.0,
                });
                self.active_notes.push(std::cmp::Reverse(ActiveNote {
                    key: key as u8,
                    dense: dense as u32,
                    clap_channel: (ch & 0x0F) as u8,
                    is_instrument: true,
                    end_tick: n.end_tick,
                    track: track as u16,
                }));
            }
            return;
        }
        let dense = self.channel_layout.dense_for(ch);
        if dense == u32::MAX {
            return;
        }
        // GPU 模式：非插件音符的复活由事件表在 seek_pos 重建时完成，
        // 这里不写 CPU 后端、也不入 active_notes（GPU 的 NoteOff 来自事件表）。
        #[cfg(feature = "gpu")]
        if self.gpu_synth.is_some() {
            return;
        }
        #[cfg(feature = "gpu")]
        let sample = self.sample_position;
        #[cfg(feature = "gpu")]
        let end_sample = self.tick_to_sample(n.end_tick);
        #[cfg(feature = "gpu")]
        if let Some(cs) = self.cpu_synth.as_mut() {
            cs.send_event(yinhe_synth::SynthEvent::NoteOn {
                sample,
                channel: dense as u8,
                key: key as u8,
                velocity: n.velocity,
                end_sample,
            });
        } else {
            self.channel_set.send_event(SynthEvent::Channel(
                dense,
                ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                    key: key as u8,
                    vel: n.velocity,
                }),
            ));
        }
        #[cfg(not(feature = "gpu"))]
        self.channel_set.send_event(SynthEvent::Channel(
            dense,
            ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                key: key as u8,
                vel: n.velocity,
            }),
        ));
        self.active_notes.push(std::cmp::Reverse(ActiveNote {
            key: key as u8,
            dense,
            clap_channel: 0,
            is_instrument: false,
            end_tick: n.end_tick,
            track: track as u16,
        }));
    }

    /// 即时 mute：从 active_notes 精确移除该轨在响音符并发 NoteOff
    ///（不误伤同通道其他轨道；CPU/GPU 路径统一为即时静音语义）。
    pub(super) fn kill_track_notes(&mut self, track: u16) {
        let all = std::mem::take(&mut self.active_notes);
        let mut remaining = BinaryHeap::new();
        for std::cmp::Reverse(an) in all.into_iter() {
            if an.track != track {
                remaining.push(std::cmp::Reverse(an));
                continue;
            }
            if an.is_instrument {
                if let Some(Some(slot)) = self.instruments.get_mut(an.dense as usize) {
                    slot.events.push(PluginEvent::NoteOff {
                        time: 0,
                        channel: an.clap_channel,
                        key: an.key,
                        velocity: 0.0,
                    });
                }
            } else if an.dense != u32::MAX && !self.gpu_synth_active() {
                #[cfg(feature = "gpu")]
                let sample = self.sample_position;
                #[cfg(feature = "gpu")]
                if let Some(cs) = self.cpu_synth.as_mut() {
                    cs.send_event(yinhe_synth::SynthEvent::NoteOff {
                        sample,
                        channel: an.dense as u8,
                        key: an.key,
                    });
                } else {
                    self.channel_set.send_event(SynthEvent::Channel(
                        an.dense,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: an.key }),
                    ));
                }
                #[cfg(not(feature = "gpu"))]
                {
                    self.channel_set.send_event(SynthEvent::Channel(
                        an.dense,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: an.key }),
                    ));
                }
            }
        }
        self.active_notes = remaining;
    }

    /// 即时 unmute：重启该轨的跨点音符（start < current_tick < end）。
    /// 代价 O(该轨在各 key 桶 [..cursor] 的音符数)，只扫受影响轨道。
    pub(super) fn restart_track_crossing_notes(&mut self, track: u16) {
        let tick = self.current_tick;
        for key in 0..KEY_COUNT {
            let notes = self.audible_notes[key].as_slice();
            let cursor = notes.partition_point(|n| n.start_tick < tick);
            let mut to_restart: Vec<AudibleNote> = Vec::new();
            for n in &notes[..cursor] {
                if n.track == track && n.end_tick > tick {
                    to_restart.push(*n);
                }
            }
            for n in to_restart {
                self.restart_note(key, &n);
            }
        }
    }
}
