use std::sync::Arc;

use xsynth_core::channel::ChannelInitOptions;
use xsynth_core::channel::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent, ControlEvent};
use xsynth_core::channel_group::{
    ChannelGroup, ChannelGroupConfig, ParallelismOptions, SynthEvent, SynthFormat,
};
use xsynth_core::{AudioPipe, AudioStreamParams, ChannelCount};

use yinhe_midi::MidiFile;
use yinhe_types::MidiControlEvent;

use crate::channel::ChannelState;
use crate::soundfont::SoundFontManager;
use crate::spawn::AudioCommand;

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

/// Core MIDI synthesis engine.  Owned by the audio callback.
pub(crate) struct AudioEngine {
    channel_group: ChannelGroup,
    /// Number of XSynth channels actually instantiated (== count of `true` in
    /// `active_mask`, rounded up to a minimum of 16 for sane defaults).
    /// This is what XSynth mixes per callback, NOT the maximum MIDI channel
    /// index that appears in the file.
    compacted_channels: u32,
    /// Map: source MIDI channel (0..256) → compacted XSynth channel index.
    /// `u32::MAX` for source channels that are inactive (not used by MIDI).
    /// Built once at MIDI load from `active_mask`.
    channel_map: Box<[u32; 256]>,
    /// Per-source-channel active flag from the MIDI file. Kept for reference
    /// (mute / solo overrides still operate on source channels).
    active_mask: Vec<bool>,
    sf_manager: SoundFontManager,
    sample_rate: u32,
    sample_position: u64,
    playing: bool,
    interleaved_buffer: Vec<f32>,
    duration_samples: u64,

    /// Per-key index list pointing into `midi.key_notes[k]`. Only contains
    /// notes that are actually audible (velocity > 1 AND channel is active).
    /// Built once in `load_midi`; mute/unmute does NOT rebuild this — it only
    /// flips `skip_track` bits checked at render time.
    audible_index: [Vec<u32>; 128],
    /// Current cursor into `audible_index[k]` per key.
    audible_cursor: [usize; 128],
    /// Cached start_sample of the note that `audible_cursor[k]` currently points
    /// to. Equals `u64::MAX` when the cursor has reached the end of the list.
    /// Lets the render loop decide "is the next note due yet?" without calling
    /// `tick_to_seconds`.
    next_note_sample: [u64; 128],

    cc_events: Vec<SortedCC>,
    cc_cursor: usize,
    active_notes: Vec<ActiveNote>,
    midi: Option<Arc<MidiFile>>,
    /// Per-track visibility: true = skip this track's notes during render.
    skip_track: Vec<bool>,
}

impl AudioEngine {
    pub(crate) fn new(sample_rate: u32, _num_channels: u32, active_mask: Vec<bool>) -> Self {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Audio, || {
            // Build channel_map: compact source-channel indices down to a
            // dense range. XSynth will only allocate / mix the channels we
            // actually need, which is critical for big MIDI files whose
            // max channel index can be 4799 but only ~20 are alive.
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
                audible_index: core::array::from_fn(|_| Vec::new()),
                audible_cursor: [0; 128],
                next_note_sample: [u64::MAX; 128],
                cc_events: Vec::new(),
                cc_cursor: 0,
                active_notes: Vec::new(),
                midi: None,
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
            AudioCommand::LoadMidi { midi } => {
                self.playing = false;
                self.load_midi(&midi);
                self.midi = Some(midi);
            }
            AudioCommand::LoadSoundFont { port, paths } => {
                self.load_soundfont_for_port(port, &paths);
            }
            AudioCommand::SkipTracks { skip } => {
                self.skip_track = skip;
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

        if let Some(ref midi) = self.midi {
            let sr = self.sample_rate as f64;

            // NoteOn: walk pre-filtered audible_index per key.
            // vel ≤ 1 and inactive-channel notes were already excluded at
            // load_midi time, so they are never iterated here.
            // next_note_sample[key] caches the start_sample of the cursor's
            // current target, eliminating per-note tick_to_seconds calls in
            // the "is it time yet?" check.
            for key in 0..128usize {
                loop {
                    let start_sample = self.next_note_sample[key];
                    if start_sample >= end {
                        break;
                    }

                    let cursor = self.audible_cursor[key];
                    let list = &self.audible_index[key];
                    let note_idx = list[cursor] as usize;
                    let note = &midi.key_notes[key][note_idx];

                    // skip_track is dynamic (mute changes at runtime).
                    let track = note.track as usize;
                    if !self.skip_track.get(track).copied().unwrap_or(false) {
                        let dense = self
                            .channel_map
                            .get(note.channel as usize)
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
                                channel: note.channel,
                                end_sample: (midi.tick_to_seconds(note.end_tick as u64) * sr)
                                    as u64,
                            });
                            _notes_dispatched += 1;
                        }
                    }

                    // Advance cursor and refresh the cache for the next note
                    // on this key (B2': single tick_to_seconds per advance).
                    let next_cursor = cursor + 1;
                    self.audible_cursor[key] = next_cursor;
                    self.next_note_sample[key] = if next_cursor < list.len() {
                        let next_idx = list[next_cursor] as usize;
                        let next_note = &midi.key_notes[key][next_idx];
                        (midi.tick_to_seconds(next_note.start_tick as u64) * sr) as u64
                    } else {
                        u64::MAX
                    };
                }
            }

            // NoteOff: only check active notes (O(active) not O(128 * 1024))
            // Borrow channel_map + channel_group as separate mutable views
            // so the retain closure can call into them without borrowing
            // `self` recursively.
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
                    false // remove from active list
                } else if an.end_sample < start {
                    false // already past, clean up
                } else {
                    true // still active
                }
            });
        }

        let interleaved = &mut self.interleaved_buffer[..frames * STEREO_CHANNELS];
        interleaved.fill(0.0);
        self.channel_group.read_samples(interleaved);
        output[..frames * STEREO_CHANNELS].copy_from_slice(interleaved);

        self.sample_position = end;
    }

    // ── Private helpers ──

    fn load_midi(&mut self, midi: &MidiFile) {
        self.setup_percussion(midi);

        self.cc_events.clear();
        self.cc_cursor = 0;
        self.active_notes.clear();
        let sr = self.sample_rate as f64;

        for evt in &midi.control_events {
            let (sample, channel, event) = match evt {
                MidiControlEvent::ControlChange {
                    tick,
                    channel,
                    controller,
                    value,
                    ..
                } => (
                    (midi.tick_to_seconds(*tick as u64) * sr) as u64,
                    *channel as u32,
                    ChannelAudioEvent::Control(ControlEvent::Raw(*controller, *value)),
                ),
                MidiControlEvent::ProgramChange {
                    tick,
                    channel,
                    program,
                    ..
                } => (
                    (midi.tick_to_seconds(*tick as u64) * sr) as u64,
                    *channel as u32,
                    ChannelAudioEvent::ProgramChange(*program),
                ),
                MidiControlEvent::PitchBend {
                    tick,
                    channel,
                    value,
                    ..
                } => (
                    (midi.tick_to_seconds(*tick as u64) * sr) as u64,
                    *channel as u32,
                    ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                        *value as f32 / 8192.0,
                    )),
                ),
            };
            self.cc_events.push(SortedCC {
                sample,
                channel,
                event,
            });
        }
        self.cc_events.sort_by_key(|e| e.sample);

        self.duration_samples = (midi.tick_to_seconds(midi.tick_length) * sr) as u64;

        // Auto-detect note-drawing tracks: tracks where every note has
        // velocity ≤ 1 produce no audible sound and should be skipped
        // by the audio engine to reduce voice count in black MIDI files.
        let mut track_has_audio = Vec::new();
        for key in 0..128usize {
            for note in &midi.key_notes[key] {
                let t = note.track as usize;
                if t >= track_has_audio.len() {
                    track_has_audio.resize(t + 1, false);
                }
                if note.velocity > 1 {
                    track_has_audio[t] = true;
                }
            }
        }
        self.skip_track = track_has_audio.iter().map(|&has| !has).collect();

        // Build audible_index: pre-filter velocity ≤ 1 and inactive channels.
        // Notes filtered here are NEVER seen by the render loop.
        // (skip_track is mute state — dynamic, checked at render time.)
        for key in 0..128usize {
            let notes = &midi.key_notes[key];
            let mut idx_list: Vec<u32> = Vec::new();
            for (i, note) in notes.iter().enumerate() {
                if note.velocity <= 1 {
                    continue;
                }
                let ch = note.channel as usize;
                if !self.active_mask.get(ch).copied().unwrap_or(false) {
                    continue;
                }
                idx_list.push(i as u32);
            }
            idx_list.shrink_to_fit();
            self.audible_index[key] = idx_list;
        }

        self.audible_cursor = [0; 128];
        self.refresh_next_note_samples(midi);
    }

    /// Refresh `next_note_sample[k]` based on current `audible_cursor[k]`.
    /// Used after `load_midi` and `seek_to` to prime the cache.
    fn refresh_next_note_samples(&mut self, midi: &MidiFile) {
        let sr = self.sample_rate as f64;
        for key in 0..128usize {
            let cursor = self.audible_cursor[key];
            let list = &self.audible_index[key];
            self.next_note_sample[key] = if cursor < list.len() {
                let note_idx = list[cursor] as usize;
                let note = &midi.key_notes[key][note_idx];
                (midi.tick_to_seconds(note.start_tick as u64) * sr) as u64
            } else {
                u64::MAX
            };
        }
    }

    fn setup_percussion(&mut self, midi: &MidiFile) {
        // Drum channels in GM are channel 9 of each port (port*16 + 9).
        // Iterate every active source channel matching that pattern.
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
        for evt in &midi.control_events {
            if let MidiControlEvent::ControlChange {
                channel,
                controller: 0,
                value,
                ..
            } = evt
            {
                let src_ch = *channel as usize;
                if src_ch >= 256 {
                    continue;
                }
                let dense = self.channel_map[src_ch];
                if dense == u32::MAX {
                    continue;
                }
                self.channel_group.send_event(SynthEvent::Channel(
                    dense,
                    ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(*value >= 120)),
                ));
            }
        }
    }

    fn load_soundfont_for_port(&mut self, port: u8, paths: &[String]) {
        // A "port" is a logical group of 16 source MIDI channels. We need to
        // pass the SF manager the set of *dense* (XSynth) channels that
        // correspond to alive source channels of this port.
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
        self.audible_cursor = [0; 128];
        self.cc_cursor = 0;
        self.active_notes.clear();

        self.cc_cursor = self.cc_events.partition_point(|cc| cc.sample < sample);

        if let Some(midi) = self.midi.as_ref().map(Arc::clone) {
            let sr = self.sample_rate as f64;
            for key in 0..128usize {
                let list = &self.audible_index[key];
                // Binary search over audible_index (not key_notes): only audible
                // notes need to be considered as starting points after seek.
                self.audible_cursor[key] = list.partition_point(|&note_idx| {
                    let note = &midi.key_notes[key][note_idx as usize];
                    let note_start = (midi.tick_to_seconds(note.start_tick as u64) * sr) as u64;
                    note_start < sample
                });
            }
            self.refresh_next_note_samples(&midi);
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
    use yinhe_midi::MidiFile;

    fn make_midi_with_notes(notes: Vec<(u8, u32, u32, u8, u8)>) -> MidiFile {
        let mut midi = MidiFile::default();
        midi.ticks_per_beat = 480;
        midi.tempo_segments = vec![yinhe_midi::TempoSegment {
            start_tick: 0,
            start_time: 0.0,
            micros_per_quarter: 500_000, // 120 BPM
        }];
        for (key, start_tick, end_tick, velocity, channel) in notes {
            midi.key_notes[key as usize].push(yinhe_midi::Note {
                start_tick,
                end_tick,
                velocity,
                channel,
                track: 0,
            });
            midi.tick_length = midi.tick_length.max(end_tick as u64);
        }
        midi
    }

    #[test]
    fn test_channels_for_midi_basic() {
        let midi = make_midi_with_notes(vec![
            (60, 0, 480, 100, 0), // ch0
            (64, 0, 480, 100, 1), // ch1
            (67, 0, 480, 100, 9), // ch9 (drum)
        ]);
        let (num_ch, mask) = crate::spawn::channels_for_midi(&midi);
        assert_eq!(num_ch, 16);
        assert!(mask[0]);
        assert!(mask[1]);
        assert!(mask[9]);
        assert!(!mask[2]);
    }

    #[test]
    fn test_channels_for_midi_multi_port() {
        let midi = make_midi_with_notes(vec![
            (60, 0, 480, 100, 0),  // port 0, ch0
            (60, 0, 480, 100, 16), // port 1, ch0
        ]);
        let (num_ch, mask) = crate::spawn::channels_for_midi(&midi);
        assert_eq!(num_ch, 32);
        assert!(mask[0]);
        assert!(mask[16]);
        assert!(!mask[15]);
    }

    #[test]
    fn test_channels_for_midi_skips_velocity_0_1() {
        let midi = make_midi_with_notes(vec![
            (60, 0, 480, 0, 0), // vel 0 — should be skipped
            (61, 0, 480, 1, 0), // vel 1 — should be skipped
            (62, 0, 480, 2, 0), // vel 2 — active
        ]);
        let (_num_ch, mask) = crate::spawn::channels_for_midi(&midi);
        assert!(mask[0]);
    }

    #[test]
    fn test_channels_for_midi_cc_activates_channel() {
        let mut midi = MidiFile::default();
        midi.control_events.push(MidiControlEvent::ControlChange {
            tick: 0,
            channel: 5,
            controller: 7,
            value: 100,
            track: 0,
        });
        let (num_ch, mask) = crate::spawn::channels_for_midi(&midi);
        assert!(num_ch >= 16);
        assert!(mask[5]);
    }

    #[test]
    fn test_channels_for_midi_empty() {
        let midi = MidiFile::default();
        let (num_ch, mask) = crate::spawn::channels_for_midi(&midi);
        assert_eq!(num_ch, 16);
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
    fn test_active_mask_length() {
        let mask = vec![false; 16];
        let _engine = AudioEngine::new(44100, 16, mask);
    }

    #[test]
    fn test_audible_index_filters_vel_and_inactive_channel() {
        // key 60: 4 notes — vel=0 ch0 / vel=1 ch0 / vel=100 ch0 / vel=100 ch3
        // active_mask: ch0 active, ch3 inactive (and everything else)
        // Expected audible_index[60] = [2] (only vel=100 ch0)
        let midi = Arc::new(make_midi_with_notes(vec![
            (60, 0, 480, 0, 0),    // vel=0  → filtered
            (60, 480, 960, 1, 0),  // vel=1  → filtered
            (60, 960, 1440, 100, 0),  // vel=100 ch0 → kept (idx 2)
            (60, 1440, 1920, 100, 3), // vel=100 ch3 → filtered (ch3 inactive)
        ]));
        let mut mask = vec![false; 16];
        mask[0] = true;
        let mut engine = AudioEngine::new(44100, 16, mask);
        engine.load_midi(&midi);

        assert_eq!(engine.audible_index[60], vec![2u32]);
        for key in 0..128usize {
            if key != 60 {
                assert!(engine.audible_index[key].is_empty(), "key {} should be empty", key);
            }
        }

        // next_note_sample[60] should be the start_sample of note idx 2
        // (start_tick=960). With tpb=480 @ 120 BPM, 1 beat = 0.5s, so
        // 960 ticks = 1.0s = 44100 samples.
        assert_eq!(engine.next_note_sample[60], 44100);
        // Other keys: no audible notes → MAX
        assert_eq!(engine.next_note_sample[0], u64::MAX);
    }

    #[test]
    fn test_audible_index_empty_when_all_filtered() {
        let midi = Arc::new(make_midi_with_notes(vec![
            (60, 0, 480, 0, 0), // vel=0 only
            (61, 0, 480, 1, 0), // vel=1 only
        ]));
        let mask = vec![true; 16];
        let mut engine = AudioEngine::new(44100, 16, mask);
        engine.load_midi(&midi);

        for key in 0..128usize {
            assert!(engine.audible_index[key].is_empty());
            assert_eq!(engine.next_note_sample[key], u64::MAX);
        }
    }
}
