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
    num_channels: u32,
    active_mask: Vec<bool>,
    sf_manager: SoundFontManager,
    sample_rate: u32,
    sample_position: u64,
    playing: bool,
    interleaved_buffer: Vec<f32>,
    duration_samples: u64,

    note_cursors: [usize; 128],
    cc_events: Vec<SortedCC>,
    cc_cursor: usize,
    active_notes: Vec<ActiveNote>,
    midi: Option<Arc<MidiFile>>,
    /// Per-track visibility: true = skip this track's notes during render.
    skip_track: Vec<bool>,
}

impl AudioEngine {
    pub(crate) fn new(sample_rate: u32, num_channels: u32, active_mask: Vec<bool>) -> Self {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Audio, || {
            let num_channels = num_channels.max(16);
            let config = ChannelGroupConfig {
                channel_init_options: ChannelInitOptions {
                    fade_out_killing: true,
                },
                format: SynthFormat::Custom {
                    channels: num_channels,
                },
                audio_params: AudioStreamParams {
                    sample_rate,
                    channels: ChannelCount::Stereo,
                },
                parallelism: ParallelismOptions::AUTO_PER_CHANNEL,
            };

            Self {
                channel_group: ChannelGroup::new(config),
                num_channels,
                active_mask,
                sf_manager: SoundFontManager::new(sample_rate),
                sample_rate,
                sample_position: 0,
                playing: false,
                interleaved_buffer: vec![0.0f32; sample_rate as usize * STEREO_CHANNELS],
                duration_samples: 0,
                note_cursors: [0; 128],
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
        let sr = self.sample_rate as f64;

        // Push CC events
        while self.cc_cursor < self.cc_events.len() && self.cc_events[self.cc_cursor].sample < end {
            let cc = &self.cc_events[self.cc_cursor];
            self.channel_group.send_event(SynthEvent::Channel(
                cc.channel,
                ChannelEvent::Audio(cc.event),
            ));
            self.cc_cursor += 1;
        }

        if let Some(ref midi) = self.midi {
            // NoteOn + track active notes (single pass over 128 keys)
            for key in 0..128usize {
                let notes = &midi.key_notes[key];
                while self.note_cursors[key] < notes.len() {
                    let note = &notes[self.note_cursors[key]];
                    if note.velocity <= 1 {
                        self.note_cursors[key] += 1;
                        continue;
                    }

                    // Cheap checks first (no tick_to_seconds):
                    // Skip notes on inactive channels
                    let ch = note.channel as usize;
                    if !self.active_mask.get(ch).copied().unwrap_or(false) {
                        self.note_cursors[key] += 1;
                        continue;
                    }
                    // Skip notes on hidden tracks
                    let track = note.track as usize;
                    if self.skip_track.get(track).copied().unwrap_or(false) {
                        self.note_cursors[key] += 1;
                        continue;
                    }

                    // Expensive: convert tick to sample position
                    let note_start = (midi.tick_to_seconds(note.start_tick as u64) * sr) as u64;
                    if note_start >= end {
                        break;
                    }

                    self.channel_group.send_event(SynthEvent::Channel(
                        note.channel as u32,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                            key: key as u8,
                            vel: note.velocity,
                        }),
                    ));

                    self.active_notes.push(ActiveNote {
                        key: key as u8,
                        channel: note.channel,
                        end_sample: (midi.tick_to_seconds(note.end_tick as u64) * sr) as u64,
                    });

                    self.note_cursors[key] += 1;
                }
            }

            // NoteOff: only check active notes (O(active) not O(128 * 1024))
            self.active_notes.retain(|an| {
                if an.end_sample >= start && an.end_sample < end {
                    self.channel_group.send_event(SynthEvent::Channel(
                        an.channel as u32,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: an.key }),
                    ));
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

        self.note_cursors = [0; 128];
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
    }

    fn setup_percussion(&mut self, midi: &MidiFile) {
        let num_ports = self.num_channels / 16;
        for port in 0..num_ports {
            let ch = (port * 16 + 9) as usize;
            if ch < self.num_channels as usize && self.active_mask.get(ch).copied().unwrap_or(false)
            {
                self.channel_group.send_event(SynthEvent::Channel(
                    ch as u32,
                    ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(true)),
                ));
            }
        }
        for evt in &midi.control_events {
            if let MidiControlEvent::ControlChange {
                channel,
                controller: 0,
                value,
                ..
            } = evt
            {
                let ch = *channel as usize;
                if ch < self.num_channels as usize
                    && self.active_mask.get(ch).copied().unwrap_or(false)
                {
                    self.channel_group.send_event(SynthEvent::Channel(
                        ch as u32,
                        ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(*value >= 120)),
                    ));
                }
            }
        }
    }

    fn load_soundfont_for_port(&mut self, port: u8, paths: &[String]) {
        let base_ch = (port as u32) * 16;
        if base_ch >= self.num_channels {
            return;
        }
        let end_ch = (base_ch + 16).min(self.num_channels);
        let has_active =
            (base_ch..end_ch).any(|ch| self.active_mask.get(ch as usize).copied().unwrap_or(false));
        if !has_active {
            return;
        }
        let _ =
            self.sf_manager
                .load_for_port(port, paths, &mut self.channel_group, &self.active_mask);
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
        self.note_cursors = [0; 128];
        self.cc_cursor = 0;
        self.active_notes.clear();

        self.cc_cursor = self.cc_events.partition_point(|cc| cc.sample < sample);

        if let Some(ref midi) = self.midi {
            let sr = self.sample_rate as f64;
            for key in 0..128usize {
                let notes = &midi.key_notes[key];
                self.note_cursors[key] = notes.partition_point(|n| {
                    let note_start = (midi.tick_to_seconds(n.start_tick as u64) * sr) as u64;
                    note_start < sample
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
}
