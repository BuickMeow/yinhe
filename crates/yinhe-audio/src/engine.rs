use std::sync::Arc;

use xsynth_core::channel::ChannelInitOptions;
use xsynth_core::channel::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent, ControlEvent};
use xsynth_core::channel_group::{
    ChannelGroup, ChannelGroupConfig, ParallelismOptions, SynthEvent, SynthFormat,
};
use xsynth_core::{AudioPipe, AudioStreamParams, ChannelCount};

use yinhe_core::YinModel;

use crate::channel::ChannelState;
use crate::soundfont::SoundFontManager;
use crate::spawn::{AudioCommand, track_global_channel};

/// Number of output channels (stereo).
const STEREO_CHANNELS: usize = 2;

struct SortedCC {
    sample: u64,
    channel: u32,
    event: ChannelAudioEvent,
}

struct ActiveNote {
    key: u8,
    channel: u8,
    end_sample: u64,
}

/// A pre-filtered note that is guaranteed to be audible (velocity > 1 and
/// channel active).  `start_sample` / `end_sample` are pre-computed at load
/// time to eliminate runtime `tick_to_seconds` calls.
struct AudibleNote {
    start_sample: u64,
    end_sample: u64,
    track: u16,
    velocity: u8,
}

/// Core MIDI synthesis engine.  Owned by the audio callback.
pub(crate) struct AudioEngine {
    channel_group: ChannelGroup,
    compacted_channels: u32,
    /// Map: source MIDI channel (0..256) → compacted XSynth channel index.
    channel_map: Box<[u32; 256]>,
    active_mask: Vec<bool>,
    sf_manager: SoundFontManager,
    sample_rate: u32,
    sample_position: u64,
    playing: bool,
    interleaved_buffer: Vec<f32>,
    duration_samples: u64,

    note_cursor: [usize; 128],
    audible_notes: [Vec<AudibleNote>; 128],

    cc_events: Vec<SortedCC>,
    cc_cursor: usize,
    active_notes: Vec<ActiveNote>,
    model: Option<Arc<YinModel>>,
    skip_track: Vec<bool>,
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
                compacted_channels,
                channel_map,
                active_mask,
                sf_manager: SoundFontManager::new(sample_rate),
                sample_rate,
                sample_position: 0,
                playing: false,
                interleaved_buffer: vec![0.0f32; sample_rate as usize * STEREO_CHANNELS],
                duration_samples: 0,
                note_cursor: [0; 128],
                audible_notes: core::array::from_fn(|_| Vec::new()),
                cc_events: Vec::new(),
                cc_cursor: 0,
                active_notes: Vec::new(),
                model: None,
                skip_track: Vec::new(),
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

    pub(crate) fn set_layer_count(&mut self, count: Option<usize>) {
        use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
        use xsynth_core::channel_group::SynthEvent;
        self.channel_group.send_event(SynthEvent::AllChannels(
            ChannelEvent::Config(ChannelConfigEvent::SetLayerCount(count)),
        ));
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
                self.model = Some(model);
            }
            AudioCommand::ReloadNotes { model } => {
                self.channel_group
                    .send_event(SynthEvent::AllChannels(ChannelEvent::Audio(
                        ChannelAudioEvent::AllNotesOff,
                    )));
                self.active_notes.clear();
                self.model = Some(model);
                self.reset_note_cursors();
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

        let start = self.sample_position;
        let end = start + frames as u64;

        // Push CC events
        while self.cc_cursor < self.cc_events.len() && self.cc_events[self.cc_cursor].sample < end {
            let cc = &self.cc_events[self.cc_cursor];
            let dense = self
                .channel_map
                .get(cc.channel as usize)
                .copied()
                .unwrap_or(u32::MAX);
            if dense != u32::MAX {
                self.channel_group.send_event(SynthEvent::Channel(
                    dense,
                    ChannelEvent::Audio(cc.event),
                ));
            }
            self.cc_cursor += 1;
        }

        let mut _notes_dispatched: usize = 0;

        if let Some(ref model) = self.model {
            for key in 0..128usize {
                let notes = &self.audible_notes[key];
                let mut cursor = self.note_cursor[key];
                while cursor < notes.len() && notes[cursor].start_sample < end {
                    let note = &notes[cursor];

                    let track = note.track as usize;
                    if !self.skip_track.get(track).copied().unwrap_or(false) {
                        let ch = track_global_channel(model, track) as usize;
                        let dense = self
                            .channel_map
                            .get(ch)
                            .copied()
                            .unwrap_or(u32::MAX);
                        if dense != u32::MAX {
                            self.channel_group.send_event(SynthEvent::Channel(
                                dense,
                                ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                                    key: key as u8,
                                    vel: note.velocity,
                                }),
                            ));

                            self.active_notes.push(ActiveNote {
                                key: key as u8,
                                channel: ch as u8,
                                end_sample: note.end_sample,
                            });
                            _notes_dispatched += 1;
                        }
                    }

                    cursor += 1;
                }
                self.note_cursor[key] = cursor;
            }

            let channel_map = &self.channel_map;
            let cg = &mut self.channel_group;
            self.active_notes.retain(|an| {
                if an.end_sample >= start && an.end_sample < end {
                    let dense = channel_map
                        .get(an.channel as usize)
                        .copied()
                        .unwrap_or(u32::MAX);
                    if dense != u32::MAX {
                        cg.send_event(SynthEvent::Channel(
                            dense,
                            ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: an.key }),
                        ));
                    }
                    false
                } else if an.end_sample < start {
                    false
                } else {
                    true
                }
            });
        }

        let interleaved = &mut self.interleaved_buffer[..frames * STEREO_CHANNELS];
        interleaved.fill(0.0);
        self.channel_group.read_samples(interleaved);
        output[..frames * STEREO_CHANNELS].copy_from_slice(interleaved);

        self.sample_position = end;
    }
}

impl AudioEngine {
    fn load_model(&mut self, model: &YinModel) {
        self.setup_percussion(model);

        self.cc_events.clear();
        self.cc_cursor = 0;
        self.active_notes.clear();
        let sr = self.sample_rate as f64;

        // Flatten control events from each track and convert to ChannelAudioEvents.
        for (track_idx, track) in model.tracks.iter().enumerate() {
            let channel = track_global_channel(model, track_idx) as u32;

            // CC
            for (&controller, evs) in &track.cc {
                for e in evs {
                    let sample =
                        (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
                    self.cc_events.push(SortedCC {
                        sample,
                        channel,
                        event: ChannelAudioEvent::Control(ControlEvent::Raw(controller, e.value)),
                    });
                }
            }
            // Pitch bend
            for e in &track.pitch_bend {
                let sample =
                    (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
                self.cc_events.push(SortedCC {
                    sample,
                    channel,
                    event: ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                        e.value as f32 / 8192.0,
                    )),
                });
            }
            // Program change
            for e in &track.program_change {
                let sample =
                    (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
                self.cc_events.push(SortedCC {
                    sample,
                    channel,
                    event: ChannelAudioEvent::ProgramChange(e.program),
                });
            }
            // RPN expanded back to CC101 + CC100 + CC6 (+ CC38 if LSB != 0)
            // so the synth sees the same wire-level sequence as a normal MIDI file.
            for (&rpn_key, evs) in &track.rpn {
                let msb = ((rpn_key >> 8) & 0x7F) as u8;
                let lsb = (rpn_key & 0x7F) as u8;
                for e in evs {
                    let data_msb = ((e.value >> 7) & 0x7F) as u8;
                    let data_lsb = (e.value & 0x7F) as u8;
                    let sample =
                        (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
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
        }
        self.cc_events.sort_by_key(|e| e.sample);

        self.duration_samples = (model.tempo_map.tick_to_seconds(model.tick_length) * sr) as u64;

        // Auto-detect note-drawing tracks: tracks where every note has
        // velocity ≤ 1 produce no audible sound and should be skipped.
        let mut track_has_audio = vec![false; model.tracks.len()];
        for (track_idx, track) in model.tracks.iter().enumerate() {
            for n in &track.notes {
                if n.velocity > 1 {
                    track_has_audio[track_idx] = true;
                    break;
                }
            }
        }
        self.skip_track = track_has_audio.iter().map(|&has| !has).collect();

        // Build audible_notes from key_notes_cache (already per-key, sorted
        // by start_tick, with track tagging by yinhe_types::Note).
        self.note_cursor = [0; 128];
        for key in 0..128usize {
            let mut audible = Vec::new();
            for note in &model.key_notes_cache[key] {
                if note.velocity > 1 {
                    let ch = track_global_channel(model, note.track as usize) as usize;
                    if self.active_mask.get(ch).copied().unwrap_or(false) {
                        audible.push(AudibleNote {
                            start_sample: (model.tempo_map.tick_to_seconds(note.start_tick as u64)
                                * sr) as u64,
                            end_sample: (model.tempo_map.tick_to_seconds(note.end_tick as u64)
                                * sr) as u64,
                            track: note.track,
                            velocity: note.velocity,
                        });
                    }
                }
            }
            self.audible_notes[key] = audible;
        }
    }

    /// Rebuild `audible_notes` from the current model. Called on `ReloadNotes`.
    fn rebuild_audible_notes(&mut self, model: &YinModel) {
        let sr = self.sample_rate as f64;
        for key in 0..128usize {
            let mut audible = Vec::new();
            for note in &model.key_notes_cache[key] {
                if note.velocity > 1 {
                    let ch = track_global_channel(model, note.track as usize) as usize;
                    if self.active_mask.get(ch).copied().unwrap_or(false) {
                        audible.push(AudibleNote {
                            start_sample: (model.tempo_map.tick_to_seconds(note.start_tick as u64)
                                * sr) as u64,
                            end_sample: (model.tempo_map.tick_to_seconds(note.end_tick as u64)
                                * sr) as u64,
                            track: note.track,
                            velocity: note.velocity,
                        });
                    }
                }
            }
            self.audible_notes[key] = audible;
        }
    }

    fn reset_note_cursors(&mut self) {
        self.note_cursor = [0; 128];
        if let Some(model) = self.model.clone() {
            self.rebuild_audible_notes(&model);
        }
    }

    fn setup_percussion(&mut self, model: &YinModel) {
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
        for (track_idx, track) in model.tracks.iter().enumerate() {
            if let Some(cc0_events) = track.cc.get(&0) {
                let src_ch = track_global_channel(model, track_idx) as usize;
                if src_ch >= 256 {
                    continue;
                }
                let dense = self.channel_map[src_ch];
                if dense == u32::MAX {
                    continue;
                }
                for e in cc0_events {
                    self.channel_group.send_event(SynthEvent::Channel(
                        dense,
                        ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(e.value >= 120)),
                    ));
                }
            }
        }
    }

    fn load_soundfont_for_port(&mut self, port: u8, paths: &[String]) {
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
        if dense_channels.is_empty() {
            return;
        }
        let _ = self
            .sf_manager
            .load_for_port_with_dense(port, paths, &mut self.channel_group, &dense_channels);
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

        for key in 0..128usize {
            let notes = &self.audible_notes[key];
            let cursor = notes.partition_point(|n| n.start_sample < sample);
            self.note_cursor[key] = cursor;
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
        CcEvent, ConductorData, NoteEvent, ProjectMeta, TempoEvent,
        TrackData, YinModel,
    };

    fn make_model_with_notes(notes: Vec<(u8, u32, u32, u8, u8)>) -> YinModel {
        let conductor = ConductorData {
            tempo: vec![TempoEvent { tick: 0, bpm: 120.0 }],
            time_sig: Vec::new(),
        };
        let first_ch = notes.first().map(|n| n.4).unwrap_or(0);
        let mut t = TrackData::new(0, first_ch);
        t.name = "Track 1".into();
        t.notes = notes
            .into_iter()
            .map(|(key, start, end, vel, _ch)| NoteEvent {
                start_tick: start,
                end_tick: end,
                key,
                velocity: vel,
                dup_index: 0,
            })
            .collect();
        let meta = ProjectMeta { ppq: 480, ..ProjectMeta::default() };
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t)],
            meta,
            ..Default::default()
        };
        model.rebuild();
        model
    }

    fn make_model_3_tracks() -> YinModel {
        let conductor = ConductorData {
            tempo: vec![TempoEvent { tick: 0, bpm: 120.0 }],
            time_sig: Vec::new(),
        };
        let mk = |ch: u8, key: u8| {
            let mut t = TrackData::new(0, ch);
            t.notes = vec![NoteEvent {
                start_tick: 0, end_tick: 480, key, velocity: 100, dup_index: 0,
            }];
            Arc::new(t)
        };
        let meta = ProjectMeta { ppq: 480, ..ProjectMeta::default() };
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![mk(0, 60), mk(1, 64), mk(9, 67)],
            meta,
            ..Default::default()
        };
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
            tempo: vec![TempoEvent { tick: 0, bpm: 120.0 }],
            time_sig: Vec::new(),
        };
        let mut t1 = TrackData::new(0, 0);
        t1.notes = vec![NoteEvent {
            start_tick: 0, end_tick: 480, key: 60, velocity: 100, dup_index: 0,
        }];
        let mut t2 = TrackData::new(1, 0);
        t2.notes = vec![NoteEvent {
            start_tick: 0, end_tick: 480, key: 60, velocity: 100, dup_index: 0,
        }];
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t1), Arc::new(t2)],
            meta: ProjectMeta { ppq: 480, ..ProjectMeta::default() },
            ..Default::default()
        };
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
        cc.insert(7, vec![CcEvent { tick: 0, value: 100 }]);
        t.cc = cc;
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t)],
            meta: ProjectMeta { ppq: 480, ..ProjectMeta::default() },
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
            SortedCC { sample: 100, channel: 0, event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 80)) },
            SortedCC { sample: 50, channel: 0, event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)) },
            SortedCC { sample: 200, channel: 0, event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 60)) },
        ];
        cc.sort_by_key(|e| e.sample);
        assert_eq!(cc[0].sample, 50);
        assert_eq!(cc[1].sample, 100);
        assert_eq!(cc[2].sample, 200);
    }

    #[test]
    fn test_active_mask_length() {
        let mask = vec![false; 16];
        let _engine = AudioEngine::new(44100, 16, mask);
    }

    #[test]
    fn test_audible_index_filters_vel_and_inactive_channel() {
        let conductor = ConductorData {
            tempo: vec![TempoEvent { tick: 0, bpm: 120.0 }],
            time_sig: Vec::new(),
        };
        let mut t0 = TrackData::new(0, 0);
        t0.notes = vec![
            NoteEvent { start_tick: 0, end_tick: 480, key: 60, velocity: 0, dup_index: 0 },
            NoteEvent { start_tick: 480, end_tick: 960, key: 60, velocity: 1, dup_index: 0 },
            NoteEvent { start_tick: 960, end_tick: 1440, key: 60, velocity: 100, dup_index: 0 },
        ];
        let mut t1 = TrackData::new(0, 3);
        t1.notes = vec![NoteEvent {
            start_tick: 1440, end_tick: 1920, key: 60, velocity: 100, dup_index: 0,
        }];
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t0), Arc::new(t1)],
            meta: ProjectMeta { ppq: 480, ..ProjectMeta::default() },
            ..Default::default()
        };
        model.rebuild();
        let model = Arc::new(model);

        let mut mask = vec![false; 16];
        mask[0] = true;
        let mut engine = AudioEngine::new(44100, 16, mask);
        engine.load_model(&model);

        assert_eq!(engine.audible_notes[60].len(), 1);
        assert_eq!(engine.note_cursor[60], 0);
        for key in 0..128usize {
            if key != 60 {
                assert_eq!(engine.audible_notes[key].len(), 0);
                assert_eq!(engine.note_cursor[key], 0);
            }
        }
        assert_eq!(engine.audible_notes[60][0].start_sample, 44100);
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

        assert!(engine.audible_notes[60].is_empty());
        assert!(engine.audible_notes[61].is_empty());
        assert!(engine.audible_notes[0].is_empty());
        for key in 0..128usize {
            assert_eq!(engine.note_cursor[key], 0);
        }
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

    #[test]
    fn test_engine_load_model_and_reload() {
        let model = Arc::new(make_model_with_notes(vec![(60, 0, 480, 100, 0)]));
        let mask = vec![true; 16];
        let mut engine = AudioEngine::new(44100, 16, mask);

        engine.handle_command(AudioCommand::LoadModel { model: model.clone() });
        assert!(!engine.playing());

        engine.handle_command(AudioCommand::ReloadNotes { model });
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
