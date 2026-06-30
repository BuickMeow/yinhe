use std::sync::Arc;

use xsynth_core::channel::ChannelInitOptions;
use xsynth_core::channel::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent, ControlEvent};
use xsynth_core::channel_group::{
    ChannelGroup, ChannelGroupConfig, ParallelismOptions, SynthEvent, SynthFormat,
};
use xsynth_core::soundfont::SoundfontBase;
use xsynth_core::{AudioPipe, AudioStreamParams, ChannelCount};

use yinhe_core::YinModel;

use crate::channel::ChannelState;
use crate::soundfont::SoundFontManager;
use crate::spawn::{AudioCommand, track_global_channel};

/// Number of output channels (stereo).
const STEREO_CHANNELS: usize = 2;

pub(crate) struct SortedCC {
    pub(crate) sample: u64,
    pub(crate) channel: u32,
    pub(crate) event: ChannelAudioEvent,
}

#[derive(Clone, Copy)]
struct ActiveNote {
    key: u8,
    channel: u8,
    end_sample: u64,
}

/// Pre-computed model data, built on a worker thread and applied
/// atomically on the audio thread.
pub(crate) struct PreparedModel {
    pub model: AudioModel,
    pub yin_model: Arc<YinModel>,
    pub cc_events: Vec<SortedCC>,
    pub duration_samples: u64,
    pub skip_track: Vec<bool>,
}

/// Lightweight per-track snapshot the audio engine actually needs.
///
/// We extract only `(global_channel)` per track plus the CC0 bank-select
/// events used for percussion-mode detection, so the audio thread holds a few
/// KB instead of a full deep clone of the model.
pub(crate) struct AudioModel {
    /// `track_channels[i]` = global channel `(port<<4)|channel` for track `i`.
    pub track_channels: Vec<u8>,
    /// CC0 (Bank Select MSB) values per track, for percussion-mode detection.
    /// Empty Vec for tracks with no CC0.
    pub track_cc0: Vec<Vec<u8>>,
    pub note_count: u64,
}

impl AudioModel {
    fn from_model(model: &YinModel) -> Self {
        let track_channels: Vec<u8> = (0..model.tracks.len())
            .map(|i| track_global_channel(model, i))
            .collect();
        let track_cc0: Vec<Vec<u8>> = model
            .tracks
            .iter()
            .map(|t| {
                t.cc
                    .get(&0)
                    .map(|evs| evs.iter().map(|e| e.value).collect())
                    .unwrap_or_default()
            })
            .collect();
        Self {
            track_channels,
            track_cc0,
            note_count: model.note_count,
        }
    }

    /// Global channel for a track index, or 0 if out of range.
    pub fn track_channel(&self, track_idx: usize) -> u8 {
        self.track_channels.get(track_idx).copied().unwrap_or(0)
    }
}

/// Build `PreparedModel` on a worker thread (no `&mut AudioEngine` needed).
/// This is the expensive part; the result is applied cheaply on the audio thread.
pub(crate) fn prepare_model(
    model: &Arc<YinModel>,
    sample_rate: u32,
    active_mask: &[bool],
    _channel_map: &[u32; 256],
) -> PreparedModel {
    let sr = sample_rate as f64;
    let mut cc_events = Vec::new();

    for (track_idx, track) in model.tracks.iter().enumerate() {
        let channel = track_global_channel(model, track_idx) as u32;

        // RPN 展开必须在 PB 之前，这样 PB 才能使用正确的 PBS 值
        for (&rpn_key, evs) in &track.rpn {
            let msb = ((rpn_key >> 8) & 0x7F) as u8;
            let lsb = (rpn_key & 0x7F) as u8;
            for e in evs {
                let data_msb = ((e.value >> 7) & 0x7F) as u8;
                let data_lsb = (e.value & 0x7F) as u8;
                let sample = (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
                cc_events.push(SortedCC {
                    sample,
                    channel,
                    event: ChannelAudioEvent::Control(ControlEvent::Raw(101, msb)),
                });
                cc_events.push(SortedCC {
                    sample,
                    channel,
                    event: ChannelAudioEvent::Control(ControlEvent::Raw(100, lsb)),
                });
                cc_events.push(SortedCC {
                    sample,
                    channel,
                    event: ChannelAudioEvent::Control(ControlEvent::Raw(6, data_msb)),
                });
                if data_lsb != 0 {
                    cc_events.push(SortedCC {
                        sample,
                        channel,
                        event: ChannelAudioEvent::Control(ControlEvent::Raw(38, data_lsb)),
                    });
                }
            }
        }
        for (&controller, evs) in &track.cc {
            for e in evs {
                let sample = (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
                cc_events.push(SortedCC {
                    sample,
                    channel,
                    event: ChannelAudioEvent::Control(ControlEvent::Raw(controller, e.value)),
                });
            }
        }
        for e in &track.pitch_bend {
            let sample = (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
            cc_events.push(SortedCC {
                sample,
                channel,
                event: ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                    e.value as f32 / 8192.0,
                )),
            });
        }
        for e in &track.program_change {
            let sample = (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
            if e.bank_msb != 0xFF {
                cc_events.push(SortedCC {
                    sample,
                    channel,
                    event: ChannelAudioEvent::Control(ControlEvent::Raw(0, e.bank_msb)),
                });
            }
            if e.bank_lsb != 0xFF {
                cc_events.push(SortedCC {
                    sample,
                    channel,
                    event: ChannelAudioEvent::Control(ControlEvent::Raw(32, e.bank_lsb)),
                });
            }
            cc_events.push(SortedCC {
                sample,
                channel,
                event: ChannelAudioEvent::ProgramChange(e.program),
            });
        }
    }
    cc_events.sort_by_key(|e| e.sample);
    // 去重：同 sample 同 channel 的重复事件 + 连续相同值的事件（不改变 xsynth 状态）
    cc_events.dedup_by(|a, b| a.channel == b.channel && a.event == b.event);

    let duration_samples = (model.tempo_map.tick_to_seconds(model.tick_length) * sr) as u64;

    let skip_track: Vec<bool> = model
        .track_has_audio_cache
        .iter()
        .map(|&has| !has)
        .collect();

    PreparedModel {
        model: AudioModel::from_model(model),
        yin_model: Arc::clone(model),
        cc_events,
        duration_samples,
        skip_track,
    }
}

/// Core MIDI synthesis engine.  Owned by the audio callback.
pub(crate) struct AudioEngine {
    channel_group: ChannelGroup,
    /// Map: source MIDI channel (0..256) → compacted XSynth channel index.
    channel_map: Box<[u32; 256]>,
    active_mask: Vec<bool>,
    sf_manager: SoundFontManager,
    sample_rate: u32,
    sample_position: u64,
    playing: bool,
    duration_samples: u64,

    note_cursor: [usize; 128],
    /// Reference to the full YinModel (notes are read directly from
    /// `yin_model.notes[key]` with real-time tick→sample conversion).
    yin_model: Option<Arc<YinModel>>,

    cc_events: Vec<SortedCC>,
    cc_cursor: usize,
    active_notes: Vec<ActiveNote>,
    ended_notes: Vec<ActiveNote>,
    model: Option<AudioModel>,
    skip_track: Vec<bool>,
    /// Set when Play arrives during async model loading.
    pending_play_from_sample: Option<u64>,
}

impl AudioEngine {
    pub(crate) fn new(sample_rate: u32, _num_channels: u32, active_mask: Vec<bool>) -> Self {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Audio, || {
            let mut channel_map = Box::new([u32::MAX; 256]);
            let mut next_dense: u32 = 0;
            for (src, &alive) in active_mask.iter().enumerate().take(256) {
                if alive {
                    channel_map[src] = next_dense;
                    next_dense += 1;
                }
            }
            let compacted_channels = next_dense.max(1);

            let config = ChannelGroupConfig {
                channel_init_options: ChannelInitOptions {
                    fade_out_killing: true,
                },
                format: SynthFormat::Custom {
                    channels: compacted_channels,
                },
                audio_params: AudioStreamParams {
                    sample_rate,
                    channels: ChannelCount::Stereo,
                },
                parallelism: ParallelismOptions::AUTO_PER_CHANNEL,
            };

            Self {
                channel_group: ChannelGroup::new(config),
                channel_map,
                active_mask,
                sf_manager: SoundFontManager::new(sample_rate),
                sample_rate,
                sample_position: 0,
                playing: false,
                duration_samples: 0,
                note_cursor: [0; 128],
                yin_model: None,
                cc_events: Vec::new(),
                cc_cursor: 0,
                active_notes: Vec::new(),
                ended_notes: Vec::new(),
                model: None,
                skip_track: Vec::new(),
                pending_play_from_sample: None,
            }
        })
    }

    pub(crate) fn sample_position(&self) -> u64 {
        self.sample_position
    }

    pub(crate) fn playing(&self) -> bool {
        self.playing
    }

    pub(crate) fn sample_rate_hz(&self) -> u32 {
        self.sample_rate
    }

    pub(crate) fn duration_samples(&self) -> u64 {
        self.duration_samples
    }

    pub(crate) fn voice_count(&self) -> u64 {
        self.channel_group.voice_count()
    }

    pub(crate) fn channel_map_clone(&self) -> Box<[u32; 256]> {
        self.channel_map.clone()
    }

    pub(crate) fn active_mask(&self) -> &[bool] {
        &self.active_mask
    }

    pub(crate) fn model_loaded(&self) -> bool {
        self.model.is_some()
    }

    pub(crate) fn set_pending_play(&mut self, from_sample: u64) {
        self.pending_play_from_sample = Some(from_sample);
    }

    pub(crate) fn send_all_notes_off(&mut self) {
        self.channel_group
            .send_event(SynthEvent::AllChannels(ChannelEvent::Audio(
                ChannelAudioEvent::AllNotesOff,
            )));
    }

    pub(crate) fn clear_active_notes(&mut self) {
        self.active_notes.clear();
    }

    pub(crate) fn set_layer_count(&mut self, count: Option<usize>) {
        use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
        use xsynth_core::channel_group::SynthEvent;
        self.channel_group
            .send_event(SynthEvent::AllChannels(ChannelEvent::Config(
                ChannelConfigEvent::SetLayerCount(count),
            )));
    }

    pub(crate) fn handle_command(&mut self, cmd: AudioCommand) {
        match cmd {
            AudioCommand::Play { from_sample } => {
                self.seek_to(from_sample);
                self.playing = true;
            }
            AudioCommand::Resume => self.playing = true,
            AudioCommand::Pause => self.playing = false,
            AudioCommand::Stop => {
                self.playing = false;
                self.seek_to(0);
            }
            AudioCommand::Seek { sample } => self.seek_to(sample),
            AudioCommand::LoadModel { model } => {
                self.playing = false;
                self.load_model(&model);
            }
            AudioCommand::ReloadNotes { model } => {
                self.send_all_notes_off();
                self.active_notes.clear();
                self.load_model(&model);
            }
            AudioCommand::LoadSoundFont { port, paths } => {
                self.load_soundfont_for_port(port, &paths);
            }
            AudioCommand::SkipTracks { skip } => {
                self.skip_track = skip;
            }
            AudioCommand::SetLayerCount { count } => {
                self.set_layer_count(count);
            }
        }
    }

    pub(crate) fn render(&mut self, output: &mut [f32]) {
        let frames = output.len() / STEREO_CHANNELS;
        if frames == 0 || !self.playing {
            output.fill(0.0);
            return;
        }

        let block_start = self.sample_position;
        let block_end = block_start + frames as u64;
        let mut rendered_until = block_start;
        let mut offset_frames = 0usize;

        while rendered_until < block_end {
            let next_event_sample = self
                .next_event_sample(rendered_until, block_end)
                .unwrap_or(block_end)
                .max(rendered_until)
                .min(block_end);

            if next_event_sample > rendered_until {
                let segment_frames = (next_event_sample - rendered_until) as usize;
                let start = offset_frames * STEREO_CHANNELS;
                let end = (offset_frames + segment_frames) * STEREO_CHANNELS;
                self.channel_group.read_samples(&mut output[start..end]);
                rendered_until = next_event_sample;
                offset_frames += segment_frames;
            }

            if rendered_until >= block_end {
                break;
            }

            // CC/PB/RPN 在 render 之后发出，确保只在当前 sample 位置生效，
            // 不会影响之前已渲染的音频段。
            self.dispatch_cc_until(rendered_until);
            // NoteOn/NoteOff 在 render 之后发出，保证精确切分
            self.dispatch_notes_at(rendered_until);
        }

        self.sample_position = block_end;
    }

    /// 返回下一个需要切分渲染的位置（音符边界 + CC 事件位置）。
    /// 音符边界（NoteOn/NoteOff）用于精确切分音符起止；
    /// CC 事件位置确保快速变换的 CC（如 CC7/CC11）能在正确的 sample
    /// 位置生效，而不是被批量推迟到下一个音符边界才发出。
    fn next_event_sample(&self, rendered_until: u64, block_end: u64) -> Option<u64> {
        let mut next: Option<u64> = None;

        if let Some(ref yin_model) = self.yin_model {
            let segments = &yin_model.tempo_map.tempo_segments;
            let tpb = yin_model.tempo_map.ticks_per_beat;
            let sr = self.sample_rate as f64;

            for key in 0..128usize {
                let cursor = self.note_cursor[key];
                // Advance past low-velocity and inactive-channel notes.
                let notes = &yin_model.notes[key];
                let mut idx = cursor;
                while idx < notes.len() {
                    let n = &notes[idx];
                    if n.velocity <= 1 {
                        idx += 1;
                        continue;
                    }
                    let ch = self.model.as_ref().map(|m| m.track_channel(n.track as usize) as usize).unwrap_or(0);
                    if !self.active_mask.get(ch).copied().unwrap_or(false) {
                        idx += 1;
                        continue;
                    }
                    break;
                }
                if let Some(note) = notes.get(idx) {
                    let start_sample = tick_to_sample(note.start_tick as u64, segments, tpb, sr);
                    if start_sample >= rendered_until && start_sample < block_end {
                        next = Some(next.map_or(start_sample, |s| s.min(start_sample)));
                    }
                }
            }

            for note in &self.active_notes {
                if note.end_sample >= rendered_until && note.end_sample < block_end {
                    next = Some(next.map_or(note.end_sample, |s| s.min(note.end_sample)));
                }
            }
        }

        // 也考虑 CC 事件位置，确保快速变换的 CC 在正确的 sample 位置生效，
        // 而不是被推迟到下一个音符边界才一次性发出。
        if self.cc_cursor < self.cc_events.len() {
            let cc_sample = self.cc_events[self.cc_cursor].sample;
            if cc_sample >= rendered_until && cc_sample < block_end {
                next = Some(next.map_or(cc_sample, |s| s.min(cc_sample)));
            }
        }

        next
    }

    /// 批量发出 sample ≤ cutoff 的所有 CC / PitchBend / RPN 事件。
    /// 在 render 之前调用，效果立即生效。
    fn dispatch_cc_until(&mut self, sample: u64) {
        while self.cc_cursor < self.cc_events.len() && self.cc_events[self.cc_cursor].sample <= sample {
            let cc = &self.cc_events[self.cc_cursor];
            let dense = self
                .channel_map
                .get(cc.channel as usize)
                .copied()
                .unwrap_or(u32::MAX);
            if dense != u32::MAX {
                self.channel_group
                    .send_event(SynthEvent::Channel(dense, ChannelEvent::Audio(cc.event)));
            }
            self.cc_cursor += 1;
        }
    }

    /// 发出 sample 位置处的 NoteOn 和已结束的 NoteOff。
    /// 在 render 之后调用，保证精确切分在音符边界。
    fn dispatch_notes_at(&mut self, sample: u64) {
        if let Some(ref yin_model) = self.yin_model.clone() {
            let segments = &yin_model.tempo_map.tempo_segments;
            let tpb = yin_model.tempo_map.ticks_per_beat;
            let sr = self.sample_rate as f64;

            for key in 0..128usize {
                let notes = &yin_model.notes[key];
                let mut cursor = self.note_cursor[key];
                while cursor < notes.len() {
                    let note = &notes[cursor];
                    if note.velocity <= 1 {
                        cursor += 1;
                        continue;
                    }
                    let start_sample = tick_to_sample(note.start_tick as u64, segments, tpb, sr);
                    if start_sample > sample {
                        break;
                    }
                    let track = note.track as usize;
                    let ch = self.model.as_ref().map(|m| m.track_channel(track) as usize).unwrap_or(0);
                    if !self.active_mask.get(ch).copied().unwrap_or(false) {
                        cursor += 1;
                        continue;
                    }
                    if !self.skip_track.get(track).copied().unwrap_or(false) {
                        let dense = self.channel_map.get(ch).copied().unwrap_or(u32::MAX);
                        if dense != u32::MAX {
                            self.channel_group.send_event(SynthEvent::Channel(
                                dense,
                                ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                                    key: key as u8,
                                    vel: note.velocity,
                                }),
                            ));
                            let end_sample = tick_to_sample(note.end_tick as u64, segments, tpb, sr);
                            self.active_notes.push(ActiveNote {
                                key: key as u8,
                                channel: ch as u8,
                                end_sample,
                            });
                        }
                    }
                    cursor += 1;
                }
                self.note_cursor[key] = cursor;
            }
        }

        let channel_map = &self.channel_map;
        self.ended_notes.clear();
        self.active_notes.retain(|an| {
            if an.end_sample <= sample {
                self.ended_notes.push(*an);
                false
            } else {
                true
            }
        });
        for an in &self.ended_notes {
            if let Some(&dense) = channel_map.get(an.channel as usize) {
                if dense != u32::MAX {
                    self.channel_group.send_event(SynthEvent::Channel(
                        dense,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: an.key }),
                    ));
                }
            }
        }
    }
}

/// Convert a tick value to sample position using the tempo map.
/// Uses a linear sweep through tempo segments (O(T) total across all calls
/// since segments are few and the sweep advances monotonically).
fn tick_to_sample(tick: u64, segments: &[yinhe_core::TempoSegment], tpb: u32, sr: f64) -> u64 {
    // Binary search for the segment containing this tick.
    let idx = match segments.binary_search_by_key(&tick, |s| s.start_tick as u64) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let seg = &segments[idx];
    let secs = seg.start_time
        + yinhe_core::ticks_to_seconds(
            tick - seg.start_tick as u64,
            tpb,
            seg.micros_per_quarter,
        );
    (secs * sr) as u64
}

impl AudioEngine {
    fn load_model(&mut self, model: &Arc<YinModel>) {
        let audio_model = AudioModel::from_model(model);
        self.setup_percussion(&audio_model);

        self.cc_events.clear();
        self.cc_cursor = 0;
        self.active_notes.clear();
        let sr = self.sample_rate as f64;

        // Flatten control events from each track and convert to ChannelAudioEvents.
        for (track_idx, track) in model.tracks.iter().enumerate() {
            let channel = track_global_channel(model, track_idx) as u32;

            // RPN expanded back to CC101 + CC100 + CC6 (+ CC38 if LSB != 0)
            // MUST come before PB so PB uses the correct PBS value.
            for (&rpn_key, evs) in &track.rpn {
                let msb = ((rpn_key >> 8) & 0x7F) as u8;
                let lsb = (rpn_key & 0x7F) as u8;
                for e in evs {
                    let data_msb = ((e.value >> 7) & 0x7F) as u8;
                    let data_lsb = (e.value & 0x7F) as u8;
                    let sample = (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
                    self.cc_events.push(SortedCC {
                        sample,
                        channel,
                        event: ChannelAudioEvent::Control(ControlEvent::Raw(101, msb)),
                    });
                    self.cc_events.push(SortedCC {
                        sample,
                        channel,
                        event: ChannelAudioEvent::Control(ControlEvent::Raw(100, lsb)),
                    });
                    self.cc_events.push(SortedCC {
                        sample,
                        channel,
                        event: ChannelAudioEvent::Control(ControlEvent::Raw(6, data_msb)),
                    });
                    if data_lsb != 0 {
                        self.cc_events.push(SortedCC {
                            sample,
                            channel,
                            event: ChannelAudioEvent::Control(ControlEvent::Raw(38, data_lsb)),
                        });
                    }
                }
            }
            // CC
            for (&controller, evs) in &track.cc {
                for e in evs {
                    let sample = (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
                    self.cc_events.push(SortedCC {
                        sample,
                        channel,
                        event: ChannelAudioEvent::Control(ControlEvent::Raw(controller, e.value)),
                    });
                }
            }
            // Pitch bend
            for e in &track.pitch_bend {
                let sample = (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
                self.cc_events.push(SortedCC {
                    sample,
                    channel,
                    event: ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                        e.value as f32 / 8192.0,
                    )),
                });
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
        self.skip_track = prepared.skip_track;
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

    fn load_soundfont_for_port(&mut self, port: u8, paths: &[String]) {
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

    pub(crate) fn load_soundfont_paths(
        sample_rate: u32,
        paths: &[String],
    ) -> Result<Vec<Arc<dyn SoundfontBase>>, String> {
        SoundFontManager::new(sample_rate).load_paths(paths)
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

    fn seek_to(&mut self, sample: u64) {
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
                let notes = &yin_model.notes[key];
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
            state[ch as usize].send_to(ch, &mut self.channel_group);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use yinhe_core::{
        CcEvent, ConductorData, NoteEvent, ProjectMeta, TempoEvent, TrackData, YinModel,
    };

    fn make_model_with_notes(notes: Vec<(u8, u32, u32, u8, u8)>) -> YinModel {
        let conductor = ConductorData {
            tempo: vec![TempoEvent {
                tick: 0,
                bpm: 120.0,
            }],
            time_sig: Vec::new(),
        };
        let first_ch = notes.first().map(|n| n.4).unwrap_or(0);
        let mut t = TrackData::new(0, first_ch);
        t.name = "Track 1".into();
        let per_track_notes: Vec<Vec<NoteEvent>> = vec![notes
            .into_iter()
            .map(|(key, start, end, vel, _ch)| NoteEvent {
                start_tick: start,
                end_tick: end,
                key,
                velocity: vel,
                dup_index: 0,
            })
            .collect()];
        let meta = ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        };
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t)],
            meta,
            ..Default::default()
        };
        model.load_track_notes(per_track_notes);
        model.rebuild();
        model
    }

    fn make_model_3_tracks() -> YinModel {
        let conductor = ConductorData {
            tempo: vec![TempoEvent {
                tick: 0,
                bpm: 120.0,
            }],
            time_sig: Vec::new(),
        };
        let mk = |ch: u8, key: u8| {
            let t = TrackData::new(0, ch);
            Arc::new(t)
        };
        let meta = ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        };
        let per_track_notes: Vec<Vec<NoteEvent>> = vec![
            vec![NoteEvent {
                start_tick: 0,
                end_tick: 480,
                key: 60,
                velocity: 100,
                dup_index: 0,
            }],
            vec![NoteEvent {
                start_tick: 0,
                end_tick: 480,
                key: 64,
                velocity: 100,
                dup_index: 0,
            }],
            vec![NoteEvent {
                start_tick: 0,
                end_tick: 480,
                key: 67,
                velocity: 100,
                dup_index: 0,
            }],
        ];
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![mk(0, 60), mk(1, 64), mk(9, 67)],
            meta,
            ..Default::default()
        };
        model.load_track_notes(per_track_notes);
        model.rebuild();
        model
    }

    #[test]
    fn test_channels_for_model_basic() {
        let model = make_model_3_tracks();
        let (num_ch, mask) = crate::spawn::channels_for_model(&model);
        assert_eq!(num_ch, 10);
        assert!(mask[0]);
        assert!(mask[1]);
        assert!(mask[9]);
        assert!(!mask[2]);
    }

    #[test]
    fn test_channels_for_model_multi_port() {
        let conductor = ConductorData {
            tempo: vec![TempoEvent {
                tick: 0,
                bpm: 120.0,
            }],
            time_sig: Vec::new(),
        };
        let t1 = TrackData::new(0, 0);
        let t2 = TrackData::new(1, 0);
        let per_track_notes: Vec<Vec<NoteEvent>> = vec![
            vec![NoteEvent {
                start_tick: 0,
                end_tick: 480,
                key: 60,
                velocity: 100,
                dup_index: 0,
            }],
            vec![NoteEvent {
                start_tick: 0,
                end_tick: 480,
                key: 60,
                velocity: 100,
                dup_index: 0,
            }],
        ];
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t1), Arc::new(t2)],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        model.load_track_notes(per_track_notes);
        model.rebuild();
        let (num_ch, mask) = crate::spawn::channels_for_model(&model);
        assert_eq!(num_ch, 17);
        assert!(mask[0]);
        assert!(mask[16]);
        assert!(!mask[15]);
    }

    #[test]
    fn test_channels_for_model_skips_velocity_0_1() {
        let model = make_model_with_notes(vec![
            (60, 0, 480, 0, 0),
            (61, 0, 480, 1, 0),
            (62, 0, 480, 2, 0),
        ]);
        let (_num_ch, mask) = crate::spawn::channels_for_model(&model);
        assert!(mask[0]);
    }

    #[test]
    fn test_channels_for_model_cc_activates_channel() {
        let conductor = ConductorData::default();
        let mut t = TrackData::new(0, 5);
        let mut cc: BTreeMap<u8, Vec<CcEvent>> = BTreeMap::new();
        cc.insert(
            7,
            vec![CcEvent {
                tick: 0,
                value: 100,
            }],
        );
        t.cc = cc;
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t)],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        model.rebuild();
        let (num_ch, mask) = crate::spawn::channels_for_model(&model);
        assert_eq!(num_ch, 6);
        assert!(mask[5]);
    }

    #[test]
    fn test_channels_for_model_empty() {
        let model = YinModel::default();
        let (num_ch, mask) = crate::spawn::channels_for_model(&model);
        assert_eq!(num_ch, 1);
        assert!(mask.iter().all(|&b| !b));
    }

    #[test]
    fn test_sorted_cc_ordering() {
        let mut cc = vec![
            SortedCC {
                sample: 100,
                channel: 0,
                event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 80)),
            },
            SortedCC {
                sample: 50,
                channel: 0,
                event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)),
            },
            SortedCC {
                sample: 200,
                channel: 0,
                event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 60)),
            },
        ];
        cc.sort_by_key(|e| e.sample);
        assert_eq!(cc[0].sample, 50);
        assert_eq!(cc[1].sample, 100);
        assert_eq!(cc[2].sample, 200);
    }

    #[test]
    fn test_render_dispatches_note_inside_large_buffer_at_exact_sample() {
        let model = make_model_with_notes(vec![(60, 960, 1440, 100, 0)]);
        assert_eq!(model.notes[60].len(), 1);
        let model = Arc::new(model);
        let mask = vec![true; 16];
        let mut engine = AudioEngine::new(48000, 16, mask);
        engine.load_model(&model);
        engine.playing = true;

        // Note at key 60, start_tick=960, velocity=100 → should dispatch at sample 48000.
        assert_eq!(engine.next_event_sample(0, 60000), Some(48000));
        engine.dispatch_cc_until(48000);
        engine.dispatch_notes_at(48000);

        assert_eq!(engine.note_cursor[60], 1);
        assert_eq!(engine.active_notes.len(), 1);
        assert_eq!(engine.sample_position(), 0);
    }

    #[test]
    fn test_active_mask_length() {
        let mask = vec![false; 16];
        let _engine = AudioEngine::new(44100, 16, mask);
    }

    #[test]
    fn test_audible_index_filters_vel_and_inactive_channel() {
        let conductor = ConductorData {
            tempo: vec![TempoEvent {
                tick: 0,
                bpm: 120.0,
            }],
            time_sig: Vec::new(),
        };
        let t0 = TrackData::new(0, 0);
        let t1 = TrackData::new(0, 3);
        let per_track_notes: Vec<Vec<NoteEvent>> = vec![
            vec![
                NoteEvent {
                    start_tick: 0,
                    end_tick: 480,
                    key: 60,
                    velocity: 0,
                    dup_index: 0,
                },
                NoteEvent {
                    start_tick: 480,
                    end_tick: 960,
                    key: 60,
                    velocity: 1,
                    dup_index: 0,
                },
                NoteEvent {
                    start_tick: 960,
                    end_tick: 1440,
                    key: 60,
                    velocity: 100,
                    dup_index: 0,
                },
            ],
            vec![NoteEvent {
                start_tick: 1440,
                end_tick: 1920,
                key: 60,
                velocity: 100,
                dup_index: 0,
            }],
        ];
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t0), Arc::new(t1)],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        model.load_track_notes(per_track_notes);
        model.rebuild();
        let model = Arc::new(model);

        let mut mask = vec![false; 16];
        mask[0] = true;
        let mut engine = AudioEngine::new(44100, 16, mask);
        engine.load_model(&model);

        assert_eq!(engine.note_cursor[60], 0);
        // Note at key 60, start_tick=960, velocity=100 → should dispatch at sample 44100.
        assert_eq!(engine.next_event_sample(0, 60000), Some(44100));
        engine.dispatch_cc_until(44100);
        engine.dispatch_notes_at(44100);
        // Cursor = 3: 2 low-vel skipped + 1 dispatched (4th note's start_sample > 44100).
        assert_eq!(engine.note_cursor[60], 3);
        assert_eq!(engine.active_notes.len(), 1);
        for key in 0..128usize {
            if key != 60 {
                assert_eq!(engine.note_cursor[key], 0);
            }
        }
    }

    #[test]
    fn test_audible_index_empty_when_all_filtered() {
        let model = Arc::new(make_model_with_notes(vec![
            (60, 0, 480, 0, 0),
            (61, 0, 480, 1, 0),
        ]));
        let mask = vec![true; 16];
        let mut engine = AudioEngine::new(44100, 16, mask);
        engine.load_model(&model);

        // All notes have velocity ≤ 1 → no events should dispatch.
        assert_eq!(engine.next_event_sample(0, 60000), None);
        for key in 0..128usize {
            assert_eq!(engine.note_cursor[key], 0);
        }
    }

    #[test]
    fn test_audible_index_uses_per_key_tempo_cursor() {
        let conductor = ConductorData {
            tempo: vec![
                TempoEvent { tick: 0, bpm: 120.0 },
                TempoEvent { tick: 1000, bpm: 60.0 },
            ],
            time_sig: Vec::new(),
        };
        let t = TrackData::new(0, 0);
        let per_track_notes: Vec<Vec<NoteEvent>> = vec![vec![
            NoteEvent {
                start_tick: 2000,
                end_tick: 2480,
                key: 0,
                velocity: 100,
                dup_index: 0,
            },
            NoteEvent {
                start_tick: 480,
                end_tick: 960,
                key: 60,
                velocity: 100,
                dup_index: 0,
            },
        ]];
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t)],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        model.load_track_notes(per_track_notes);
        model.rebuild();

        let mask = vec![true; 16];
        let mut engine = AudioEngine::new(48000, 16, mask);
        engine.load_model(&Arc::new(model));

        // Note at key 0, start_tick=2000 → ~150000 samples at 48000 Hz (120→60 BPM at tick 1000).
        // Note at key 60, start_tick=480 → 24000 samples at 48000 Hz.
        assert_eq!(engine.next_event_sample(0, 200000), Some(24000));
        engine.dispatch_cc_until(24000);
        engine.dispatch_notes_at(24000);
        assert_eq!(engine.note_cursor[60], 1);
        assert_eq!(engine.active_notes.len(), 1);
        engine.dispatch_cc_until(150000);
        engine.dispatch_notes_at(150000);
        assert_eq!(engine.note_cursor[0], 1);
        // Note at key 60 ended at sample 48000, so only key 0 is active.
        assert_eq!(engine.active_notes.len(), 1);
    }

    #[test]
    fn test_engine_accessors() {
        let mask = vec![true; 16];
        let engine = AudioEngine::new(44100, 16, mask);
        assert_eq!(engine.sample_rate_hz(), 44100);
        assert_eq!(engine.sample_position(), 0);
        assert!(!engine.playing());
    }

    #[test]
    fn test_engine_handle_command_play_pause_stop() {
        let mask = vec![true; 16];
        let mut engine = AudioEngine::new(44100, 16, mask);

        engine.handle_command(AudioCommand::Play { from_sample: 0 });
        assert!(engine.playing());
        assert_eq!(engine.sample_position(), 0);

        engine.handle_command(AudioCommand::Pause);
        assert!(!engine.playing());

        engine.handle_command(AudioCommand::Resume);
        assert!(engine.playing());

        engine.handle_command(AudioCommand::Stop);
        assert!(!engine.playing());
        assert_eq!(engine.sample_position(), 0);
    }

    #[test]
    fn test_engine_handle_command_seek() {
        let mask = vec![true; 16];
        let mut engine = AudioEngine::new(44100, 16, mask);
        engine.handle_command(AudioCommand::Seek { sample: 44100 });
        assert_eq!(engine.sample_position(), 44100);
    }

    #[test]
    fn test_engine_handle_command_skip_tracks() {
        let mask = vec![true; 16];
        let mut engine = AudioEngine::new(44100, 16, mask);
        let skip = vec![false, true, false];
        engine.handle_command(AudioCommand::SkipTracks { skip });
        assert_eq!(engine.skip_track, vec![false, true, false]);
    }

    #[test]
    fn test_engine_render_not_playing() {
        let mask = vec![true; 16];
        let mut engine = AudioEngine::new(44100, 16, mask);
        let mut output = vec![1.0f32; 100];
        engine.render(&mut output);
        assert!(output.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn test_engine_render_zero_frames() {
        let mask = vec![true; 16];
        let mut engine = AudioEngine::new(44100, 16, mask);
        engine.handle_command(AudioCommand::Play { from_sample: 0 });
        let mut output: Vec<f32> = Vec::new();
        engine.render(&mut output);
    }

    #[cfg(test)]
    use yinhe_core::{PcEvent, PitchBendEvent, RpnEvent};

    fn make_model_with_controls(
        cc: Vec<(u8, u32, u8)>,
        pb: Vec<(u32, i16)>,
        pc: Vec<(u32, u8)>,
        rpn: Vec<(u16, u32, u16)>,
    ) -> YinModel {
        let conductor = ConductorData {
            tempo: vec![TempoEvent { tick: 0, bpm: 120.0 }],
            time_sig: Vec::new(),
        };
        let mut t = TrackData::new(0, 0);
        for (controller, tick, value) in cc {
            t.cc.entry(controller).or_default().push(CcEvent { tick, value });
        }
        t.pitch_bend = pb.into_iter().map(|(tick, value)| PitchBendEvent { tick, value }).collect();
        t.program_change = pc.into_iter().map(|(tick, program)| PcEvent { tick, program, bank_msb: 0, bank_lsb: 0 }).collect();
        for (key, tick, value) in rpn {
            t.rpn.entry(key).or_default().push(RpnEvent { tick, value });
        }
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t)],
            meta: ProjectMeta { ppq: 480, ..ProjectMeta::default() },
            ..Default::default()
        };
        model.rebuild();
        model
    }

    #[test]
    fn test_engine_load_model_and_reload() {
        let model = Arc::new(make_model_with_notes(vec![(60, 0, 480, 100, 0)]));
        let mask = vec![true; 16];
        let mut engine = AudioEngine::new(44100, 16, mask);

        engine.handle_command(AudioCommand::LoadModel {
            model: model.clone(),
        });
        assert!(!engine.playing());

        engine.handle_command(AudioCommand::ReloadNotes { model });
    }

    /// Regression test: the MIMO refactor originally forgot to call
    /// `load_model()` inside `ReloadNotes`, which meant CC / pitch-bend /
    /// program-change / RPN events were never rebuilt after editing — they
    /// stayed at whatever the *previous* model had.  This test loads model
    /// A (rich controllers), reloads with model B (different controllers),
    /// and asserts `cc_events` reflects model B.
    #[test]
    fn test_reload_notes_rebuilds_cc_pb_pc_rpn() {
        let mask = vec![true; 16];
        let mut engine = AudioEngine::new(44100, 16, mask);

        let model_a = Arc::new(make_model_with_controls(
            vec![(7, 0, 100), (10, 0, 64)],
            vec![(0, 0)],
            vec![(0, 5)],
            vec![],
        ));
        engine.handle_command(AudioCommand::LoadModel { model: model_a });
        let cc_count_a = engine.cc_events.len();
        assert!(cc_count_a > 0, "model A should produce some events");

        // Model B: completely different shape — 3 CCs at different ticks,
        // 2 pitch bends, 2 program changes, 1 RPN (which expands to 3 raw CCs).
        let model_b = Arc::new(make_model_with_controls(
            vec![
                (7, 480, 80),
                (7, 960, 90),
                (11, 240, 100),
            ],
            vec![(120, 4096), (600, -2048)],
            vec![(0, 1), (480, 2)],
            vec![(0x0000, 240, 0x0200)],
        ));
        engine.handle_command(AudioCommand::ReloadNotes { model: model_b });

        // 3 CC + 2 PB + 2 PC (each with bank_msb=0 + bank_lsb=0 → 2 extra) + 3 RPN-expanded = 14
        assert_eq!(
            engine.cc_events.len(),
            14,
            "ReloadNotes must rebuild cc_events from the new model (was {} from model A)",
            cc_count_a
        );

        // Assert events are sorted (so the schedule loop's monotonic cursor works).
        for w in engine.cc_events.windows(2) {
            assert!(w[0].sample <= w[1].sample, "cc_events must be sorted by sample");
        }

        // Reload again with an empty model — cc_events must drain to zero.
        let model_c = Arc::new(make_model_with_controls(vec![], vec![], vec![], vec![]));
        engine.handle_command(AudioCommand::ReloadNotes { model: model_c });
        assert_eq!(
            engine.cc_events.len(),
            0,
            "ReloadNotes with empty model must clear cc_events"
        );
    }

    #[test]
    fn test_engine_channel_map_inactive_channel() {
        let mut mask = vec![false; 16];
        mask[5] = true;
        let engine = AudioEngine::new(44100, 16, mask);
        assert_eq!(engine.channel_map[5], 0);
        assert_eq!(engine.channel_map[0], u32::MAX);
    }

    #[test]
    fn test_engine_channel_map_multiple_active() {
        let mut mask = vec![false; 256];
        mask[0] = true;
        mask[2] = true;
        mask[10] = true;
        let engine = AudioEngine::new(44100, 256, mask);
        assert_eq!(engine.channel_map[0], 0);
        assert_eq!(engine.channel_map[1], u32::MAX);
        assert_eq!(engine.channel_map[2], 1);
        assert_eq!(engine.channel_map[10], 2);
    }
}
