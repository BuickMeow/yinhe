use std::sync::Arc;

use xsynth_core::channel::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent, ControlEvent};
use xsynth_core::channel_group::SynthEvent;
use xsynth_core::soundfont::SoundfontBase;

use yinhe_core::YinModel;
use yinhe_types::AutomationTarget;

use crate::audio_model::{ActiveNote, AudioModel, PreparedModel, SortedCC, tick_to_sample};
use crate::channel::ChannelState;
use crate::engine::AudioEngine;
use crate::spawn::track_global_channel;

impl AudioEngine {
    pub(crate) fn load_model(&mut self, model: &Arc<YinModel>) {
        let audio_model = AudioModel::from_model(model);
        self.setup_percussion(&audio_model);

        self.cc_events.clear();
        self.cc_cursor = 0;
        self.active_notes.clear();
        let sr = self.sample_rate as f64;

        // Flatten control events from each track and convert to ChannelAudioEvents.
        for (track_idx, track) in model.tracks.iter().enumerate() {
            let channel = track_global_channel(model, track_idx) as u32;

            // Automation lanes → xsynth events
            for lane in &track.automation_lanes {
                for e in &lane.events {
                    let sample = (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
                    match &lane.target {
                        AutomationTarget::CC { controller } => {
                            self.cc_events.push(SortedCC {
                                sample,
                                channel,
                                event: ChannelAudioEvent::Control(ControlEvent::Raw(
                                    *controller,
                                    (e.value & 0x7F) as u8,
                                )),
                            });
                        }
                        AutomationTarget::PitchBend => {
                            self.cc_events.push(SortedCC {
                                sample,
                                channel,
                                event: ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                                    (e.value as f32 - 8192.0) / 8192.0,
                                )),
                            });
                        }
                        AutomationTarget::Rpn { parameter } => {
                            let msb = ((parameter >> 8) & 0x7F) as u8;
                            let lsb = (parameter & 0x7F) as u8;
                            let (data_msb, data_lsb) = if lane.target.is_14bit() {
                                (((e.value >> 7) & 0x7F) as u8, (e.value & 0x7F) as u8)
                            } else {
                                (e.value as u8, 0u8)
                            };
                            self.cc_events.push(SortedCC {
                                sample, channel,
                                event: ChannelAudioEvent::Control(ControlEvent::Raw(101, msb)),
                            });
                            self.cc_events.push(SortedCC {
                                sample, channel,
                                event: ChannelAudioEvent::Control(ControlEvent::Raw(100, lsb)),
                            });
                            self.cc_events.push(SortedCC {
                                sample, channel,
                                event: ChannelAudioEvent::Control(ControlEvent::Raw(6, data_msb)),
                            });
                            if data_lsb != 0 {
                                self.cc_events.push(SortedCC {
                                    sample, channel,
                                    event: ChannelAudioEvent::Control(ControlEvent::Raw(38, data_lsb)),
                                });
                            }
                        }
                        AutomationTarget::Nrpn { parameter } => {
                            let msb = ((parameter >> 8) & 0x7F) as u8;
                            let lsb = (parameter & 0x7F) as u8;
                            let data_msb = ((e.value >> 7) & 0x7F) as u8;
                            let data_lsb = (e.value & 0x7F) as u8;
                            self.cc_events.push(SortedCC {
                                sample, channel,
                                event: ChannelAudioEvent::Control(ControlEvent::Raw(99, msb)),
                            });
                            self.cc_events.push(SortedCC {
                                sample, channel,
                                event: ChannelAudioEvent::Control(ControlEvent::Raw(98, lsb)),
                            });
                            self.cc_events.push(SortedCC {
                                sample, channel,
                                event: ChannelAudioEvent::Control(ControlEvent::Raw(6, data_msb)),
                            });
                            if data_lsb != 0 {
                                self.cc_events.push(SortedCC {
                                    sample, channel,
                                    event: ChannelAudioEvent::Control(ControlEvent::Raw(38, data_lsb)),
                                });
                            }
                        }
                    }
                }
            }

            // Program change (with bank MSB/LSB if available)
            for e in &track.program_change {
                let sample = (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
                if e.bank_msb != 0xFF {
                    self.cc_events.push(SortedCC {
                        sample,
                        channel,
                        event: ChannelAudioEvent::Control(ControlEvent::Raw(0, e.bank_msb)),
                    });
                }
                if e.bank_lsb != 0xFF {
                    self.cc_events.push(SortedCC {
                        sample,
                        channel,
                        event: ChannelAudioEvent::Control(ControlEvent::Raw(32, e.bank_lsb)),
                    });
                }
                self.cc_events.push(SortedCC {
                    sample,
                    channel,
                    event: ChannelAudioEvent::ProgramChange(e.program),
                });
            }
        }
        self.cc_events.sort_by_key(|e| e.sample);
        // 去重：同 sample 同 channel 的重复事件 + 连续相同值的事件（不改变 xsynth 状态）
        self.cc_events.dedup_by(|a, b| a.channel == b.channel && a.event == b.event);

        self.duration_samples = (model.tempo_map.tick_to_seconds(model.tick_length) * sr) as u64;

        self.skip_track = model
            .track_has_audio_cache
            .iter()
            .map(|&has| !has)
            .collect();

        self.note_cursor = [0; 128];
        self.yin_model = Some(Arc::clone(model));
        self.model = Some(audio_model);
    }

    /// Apply a `PreparedModel` computed on a worker thread.
    pub(crate) fn apply_prepared_model(&mut self, prepared: PreparedModel) {
        self.setup_percussion(&prepared.model);

        self.cc_events = prepared.cc_events;
        self.duration_samples = prepared.duration_samples;
        // Skip is ignored here — we keep whatever the user set via SkipTracks.
        self.yin_model = Some(prepared.yin_model);
        self.model = Some(prepared.model);

        // Seek to current playback position to avoid triggering all notes
        // before the current position (which would cause voice stealing).
        let current_sample = self.sample_position;
        self.seek_to(current_sample);

        // If Play arrived while loading, seek now
        if let Some(from_sample) = self.pending_play_from_sample.take() {
            self.seek_to(from_sample);
            self.playing = true;
        }
    }

    fn setup_percussion(&mut self, model: &AudioModel) {
        // Drum channels in GM are channel 9 of each port (port*16 + 9).
        for (src_ch, &alive) in self.active_mask.iter().enumerate().take(256) {
            if !alive || src_ch % 16 != 9 {
                continue;
            }
            let dense = self.channel_map[src_ch];
            if dense == u32::MAX {
                continue;
            }
            self.channel_group.send_event(SynthEvent::Channel(
                dense,
                ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(true)),
            ));
        }
        // Honour CC0 (Bank Select MSB) values >= 120 as percussion-mode toggle,
        // matching the legacy MidiFile path.
        for (track_idx, cc0_values) in model.track_cc0.iter().enumerate() {
            if cc0_values.is_empty() {
                continue;
            }
            let src_ch = model.track_channel(track_idx) as usize;
            if src_ch >= 256 {
                continue;
            }
            let dense = self.channel_map[src_ch];
            if dense == u32::MAX {
                continue;
            }
            for &value in cc0_values {
                self.channel_group.send_event(SynthEvent::Channel(
                    dense,
                    ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(value >= 120)),
                ));
            }
        }
    }

    pub(crate) fn load_soundfont_for_port(&mut self, port: u8, paths: &[String]) {
        let dense_channels = self.dense_channels_for_port(port);
        if dense_channels.is_empty() {
            return;
        }
        let _ = self.sf_manager.load_for_port_with_dense(
            port,
            paths,
            &mut self.channel_group,
            &dense_channels,
        );
    }

    pub(crate) fn dense_channels_for_port(&self, port: u8) -> Vec<u32> {
        let base_src = (port as u32 * 16) as usize;
        let end_src = (base_src + 16).min(256);
        let mut dense_channels: Vec<u32> = Vec::with_capacity(16);
        for src in base_src..end_src {
            if self.active_mask.get(src).copied().unwrap_or(false) {
                let dense = self.channel_map[src];
                if dense != u32::MAX {
                    dense_channels.push(dense);
                }
            }
        }
        dense_channels
    }

    pub(crate) fn apply_loaded_soundfont_for_port(
        &mut self,
        port: u8,
        soundfonts: Vec<Arc<dyn SoundfontBase>>,
        dense_channels: &[u32],
    ) {
        if dense_channels.is_empty() {
            return;
        }
        self.sf_manager.apply_loaded_for_port_with_dense(
            port,
            soundfonts,
            &mut self.channel_group,
            dense_channels,
        );
    }

    pub(crate) fn seek_to(&mut self, sample: u64) {
        self.channel_group
            .send_event(SynthEvent::AllChannels(ChannelEvent::Audio(
                ChannelAudioEvent::AllNotesOff,
            )));
        self.channel_group
            .send_event(SynthEvent::AllChannels(ChannelEvent::Audio(
                ChannelAudioEvent::ResetControl,
            )));

        self.sample_position = sample;
        self.note_cursor = [0; 128];
        self.cc_cursor = 0;
        self.active_notes.clear();

        self.cc_cursor = self.cc_events.partition_point(|cc| cc.sample < sample);

        // Reset note cursors to the correct position based on tick→sample conversion.
        if let Some(ref yin_model) = self.yin_model {
            let segments = &yin_model.tempo_map.tempo_segments;
            let tpb = yin_model.tempo_map.ticks_per_beat;
            let sr = self.sample_rate as f64;
            for key in 0..128usize {
                let notes = yin_model.notes[key].as_slice();
                let cursor = notes.partition_point(|n| {
                    if n.velocity <= 1 {
                        return false;
                    }
                    tick_to_sample(n.start_tick as u64, segments, tpb, sr) < sample
                });
                self.note_cursor[key] = cursor;

                // If the note just before cursor is a long note that started
                // before seek but hasn't ended yet, start it now.
                if cursor > 0 {
                    let prev = &notes[cursor - 1];
                    if prev.velocity > 1 {
                        let end_sample = tick_to_sample(prev.end_tick as u64, segments, tpb, sr);
                        if end_sample > sample {
                            let track = prev.track as usize;
                            let ch = self.model.as_ref().map(|m| m.track_channel(track) as usize).unwrap_or(0);
                            if self.active_mask.get(ch).copied().unwrap_or(false)
                                && !self.skip_track.get(track).copied().unwrap_or(false)
                            {
                                let dense = self.channel_map.get(ch).copied().unwrap_or(u32::MAX);
                                if dense != u32::MAX {
                                    self.channel_group.send_event(SynthEvent::Channel(
                                        dense,
                                        ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                                            key: key as u8,
                                            vel: prev.velocity,
                                        }),
                                    ));
                                    self.active_notes.push(ActiveNote {
                                        key: key as u8,
                                        channel: ch as u8,
                                        end_sample,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        self.inject_chase(sample);
    }

    fn inject_chase(&mut self, target_sample: u64) {
        let mut state = [ChannelState::default(); 256];
        for cc in &self.cc_events {
            if cc.sample >= target_sample {
                break;
            }
            let ch = cc.channel as usize;
            if ch >= 256 {
                continue;
            }
            state[ch].apply(&cc.event);
        }

        for ch in 0..256u32 {
            if !self.active_mask.get(ch as usize).copied().unwrap_or(false) {
                continue;
            }
            let dense = self
                .channel_map
                .get(ch as usize)
                .copied()
                .unwrap_or(u32::MAX);
            if dense == u32::MAX {
                continue;
            }
            state[ch as usize].send_to(dense, &mut self.channel_group);
        }
    }
}