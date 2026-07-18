use std::sync::Arc;

use xsynth_core::channel::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent};
use xsynth_core::channel_group::SynthEvent;
use xsynth_core::soundfont::SoundfontBase;

use yinhe_core::YinModel;

use crate::audio_model::{ActiveNote, AudioModel, PreparedModel, flatten_automation_to_cc_events};
use crate::channel::ChannelState;
use crate::engine::AudioEngine;
use crate::prepare_model::build_audible_notes;

impl AudioEngine {
    pub(crate) fn load_model(&mut self, model: &Arc<YinModel>) {
        let audio_model = AudioModel::from_model(model);
        self.setup_percussion(&audio_model);

        self.cc_events = flatten_automation_to_cc_events(model, self.sample_rate, self.automation_density);
        self.cc_cursor = 0;
        self.active_notes.clear();

        self.duration_samples = (model.tempo_map.tick_to_seconds(model.tick_length) * self.sample_rate as f64) as u64;

        self.skip_track = model
            .track_audible_count
            .iter()
            .map(|&c| c == 0)
            .collect();

        self.note_cursor = [0; 128];
        self.yin_model = Some(Arc::clone(model));
        self.audible_notes = build_audible_notes(model, self.sample_rate);
        self.model = Some(audio_model);
    }

    /// Apply a `PreparedModel` computed on a worker thread.
    pub(crate) fn apply_prepared_model(&mut self, prepared: PreparedModel) {
        self.setup_percussion(&prepared.model);

        self.cc_events = prepared.cc_events;
        self.duration_samples = prepared.duration_samples;
        // Skip is ignored here — we keep whatever the user set via SkipTracks.
        self.yin_model = Some(prepared.yin_model);
        self.audible_notes = prepared.audible_notes;
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

        // Reset note cursors to the correct position based on pre-built audible_notes.
        // 桶内 start_sample 严格升序，partition_point 谓词单调，结果正确（修 P0-2）。
        for key in 0..128usize {
            let notes = self.audible_notes[key].as_slice();
            let cursor = notes.partition_point(|n| n.start_sample < sample);
            self.note_cursor[key] = cursor;

            // 扫描 seek 点之前开始、seek 点之后才结束的所有音符，全部重启（修 P2-10）。
            // 桶按 start_sample 升序，但 end_sample 不保证有序，必须线性扫 [..cursor]。
            // 黑乐谱叠层场景下 cursor 前通常有几十个跨点音符，O(cursor) 完全可接受。
            for n in &notes[..cursor] {
                if n.end_sample <= sample {
                    continue;
                }
                let track = n.track as usize;
                let ch = self
                    .model
                    .as_ref()
                    .map(|m| m.track_channel(track) as usize)
                    .unwrap_or(0);
                if !self.active_mask.get(ch).copied().unwrap_or(false)
                    || self.skip_track.get(track).copied().unwrap_or(false)
                {
                    continue;
                }
                let dense = self.channel_map.get(ch).copied().unwrap_or(u32::MAX);
                if dense == u32::MAX {
                    continue;
                }
                self.channel_group.send_event(SynthEvent::Channel(
                    dense,
                    ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                        key: key as u8,
                        vel: n.velocity,
                    }),
                ));
                self.active_notes.push(ActiveNote {
                    key: key as u8,
                    channel: ch as u8,
                    end_sample: n.end_sample,
                });
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