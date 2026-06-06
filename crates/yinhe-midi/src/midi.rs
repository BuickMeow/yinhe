use crate::TempoSegment;
use crate::error::MidiError;
use crate::parser::MidiParser;
use crate::time::{DEFAULT_BPM, DEFAULT_MPQ, bpm_from_mpq, seconds_to_ticks};
use std::path::Path;

pub use yinhe_types::{MidiControlEvent, Note, TimeSigEvent};

#[derive(Clone, Debug)]
pub struct MidiFile {
    /// Notes grouped by key; key_notes[i] = all notes for MIDI key=i, sorted by start.
    pub key_notes: [Vec<Note>; 128],
    pub duration: f64,
    pub ticks_per_beat: u32,
    /// Tempo segments sorted by start_tick / start_time.
    pub tempo_segments: Vec<TempoSegment>,
    pub note_count: u64,
    /// Tick position of the last note end.
    pub tick_length: u64,
    /// Time signature numerator (e.g., 4 in 4/4).
    pub time_sig_numerator: u8,
    /// Time signature denominator power (e.g., 2 for 4/4, meaning 2^2 = 4).
    pub time_sig_denominator: u8,
    /// MIDI port per track for channel mapping (port * 16 + channel).
    pub track_ports: Vec<u8>,
    /// Track names parsed from MetaMessage::TrackName.
    pub track_names: Vec<String>,
    /// All time signature events sorted by tick.
    pub time_sig_events: Vec<TimeSigEvent>,
    /// Non-note MIDI events (CC, Program Change, Pitch Bend).
    pub control_events: Vec<MidiControlEvent>,
    /// Scan index for fast visible-note seeking.  Built at load time.
    pub scan_index: Option<yinhe_types::NoteScanIndex>,
}

impl Default for MidiFile {
    fn default() -> Self {
        Self {
            key_notes: core::array::from_fn(|_| Vec::new()),
            duration: 0.0,
            ticks_per_beat: 480,
            tempo_segments: Vec::new(),
            note_count: 0,
            tick_length: 0,
            time_sig_numerator: 4,
            time_sig_denominator: 2,
            track_ports: Vec::new(),
            track_names: Vec::new(),
            time_sig_events: Vec::new(),
            control_events: Vec::new(),
            scan_index: None,
        }
    }
}

impl yinhe_types::NoteSource for MidiFile {
    fn key_notes(&self, key: u8) -> &[Note] {
        &self.key_notes[key as usize]
    }
    fn duration(&self) -> f64 {
        self.duration
    }
    fn ticks_per_beat(&self) -> Option<u32> {
        Some(self.ticks_per_beat)
    }
    fn tick_at_time(&self, time: f64) -> Option<f64> {
        Some(MidiFile::tick_at_time(self, time))
    }
    fn tick_length(&self) -> Option<u64> {
        Some(self.tick_length)
    }
    fn scan_index(&self) -> Option<&yinhe_types::NoteScanIndex> {
        self.scan_index.as_ref()
    }
    fn time_sig_default(&self) -> (u8, u8) {
        (self.time_sig_numerator, self.time_sig_denominator)
    }
    fn time_sig_events(&self) -> &[TimeSigEvent] {
        &self.time_sig_events
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LoadProgress {
    pub current_track: usize,
    pub total_tracks: usize,
}

impl MidiFile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, MidiError> {
        MidiParser::load(path)
    }

    pub fn load_with_progress(
        path: impl AsRef<Path>,
        progress: impl FnMut(LoadProgress),
    ) -> Result<Self, MidiError> {
        MidiParser::load_with_progress(path, progress)
    }

    pub fn load_from_bytes(data: &[u8]) -> Result<Self, MidiError> {
        MidiParser::parse_bytes_with_progress(data, |_| {})
    }

    pub fn load_from_bytes_with_progress(
        data: &[u8],
        progress: impl FnMut(LoadProgress),
    ) -> Result<Self, MidiError> {
        MidiParser::parse_bytes_with_progress(data, progress)
    }

    /// Same as `load_from_bytes_with_progress` but takes ownership of the byte
    /// vector so the raw file data can be dropped immediately after parsing.
    pub fn load_from_bytes_with_progress_owned(
        data: Vec<u8>,
        progress: impl FnMut(LoadProgress),
    ) -> Result<Self, MidiError> {
        MidiParser::parse_bytes_with_progress_owned(data, progress)
    }

    /// Find the tempo segment containing the given time.
    fn find_segment_at(&self, time: f64) -> Option<&TempoSegment> {
        if self.tempo_segments.is_empty() {
            return None;
        }
        let idx = self.tempo_segments.partition_point(|s| s.start_time <= time);
        if idx == 0 {
            return None;
        }
        Some(&self.tempo_segments[idx - 1])
    }

    /// Convert seconds to MIDI tick (considering tempo changes).
    pub fn tick_at_time(&self, time: f64) -> f64 {
        let Some(seg) = self.find_segment_at(time) else {
            return seconds_to_ticks(time, self.ticks_per_beat, DEFAULT_MPQ);
        };
        let dt = time - seg.start_time;
        seg.start_tick as f64 + seconds_to_ticks(dt, self.ticks_per_beat, seg.micros_per_quarter)
    }

    /// Get BPM at a given time.
    pub fn bpm_at_time(&self, time: f64) -> f32 {
        let Some(seg) = self.find_segment_at(time) else {
            return DEFAULT_BPM as f32;
        };
        bpm_from_mpq(seg.micros_per_quarter) as f32
    }

    /// Get the time signature active at a given tick.
    pub fn time_sig_at_tick(&self, tick: u32) -> (u8, u8) {
        if self.time_sig_events.is_empty() {
            return (self.time_sig_numerator, self.time_sig_denominator);
        }
        let count = self.time_sig_events.partition_point(|e| e.tick <= tick);
        let idx = count.saturating_sub(1);
        let ev = &self.time_sig_events[idx];
        (ev.numerator, ev.denominator)
    }

    /// Calculate ticks per measure (bar divide).
    pub fn bar_divide(&self) -> f64 {
        crate::time::bar_divide(
            self.ticks_per_beat,
            self.time_sig_numerator,
            self.time_sig_denominator,
        )
    }

    /// Calculate bar number at a given tick (1-based).
    pub fn bar_at_tick(&self, tick: u64) -> u64 {
        crate::time::bar_at_tick(tick, self.bar_divide())
    }

    /// Calculate total number of bars.
    pub fn total_bars(&self) -> u64 {
        crate::time::total_bars(self.tick_length, self.bar_divide())
    }

    /// Convert absolute tick to seconds (considering tempo changes).
    pub fn tick_to_seconds(&self, tick: u32) -> f64 {
        let count = self.tempo_segments.partition_point(|s| s.start_tick <= tick);
        if count == 0 {
            return crate::time::ticks_to_seconds(tick, self.ticks_per_beat, DEFAULT_MPQ);
        }
        let seg = &self.tempo_segments[count - 1];
        let dtick = tick - seg.start_tick;
        seg.start_time
            + crate::time::ticks_to_seconds(dtick, self.ticks_per_beat, seg.micros_per_quarter)
    }

    /// Get port number for a track.
    pub fn track_port(&self, track_idx: usize) -> u8 {
        self.track_ports.get(track_idx).copied().unwrap_or(0)
    }

    /// Get info for all tracks (name, note count, port, channel).
    ///
    /// Port and channel are both derived from `note.channel` which encodes
    /// `port * 16 + midi_channel`.  For tracks with no notes, falls back to
    /// `control_events` (CC, PC, PitchBend) which also carry a `track` field.
    /// Final fallback to `track_ports` when nothing is found.
    pub fn track_info(&self) -> Vec<TrackInfo> {
        let num_tracks = self.track_ports.len();
        let mut note_counts = vec![0u64; num_tracks];
        let mut track_channels = vec![0u8; num_tracks];
        let mut track_ports_from_notes = vec![0u8; num_tracks];
        let mut note_port_set = vec![false; num_tracks];
        for notes in &self.key_notes {
            for note in notes {
                let idx = note.track as usize;
                if idx < num_tracks {
                    note_counts[idx] += 1;
                    if track_channels[idx] == 0 {
                        track_channels[idx] = (note.channel & 0x0F) + 1;
                    }
                    if !note_port_set[idx] {
                        note_port_set[idx] = true;
                        track_ports_from_notes[idx] = (note.channel >> 4) & 0x0F;
                    }
                }
            }
        }
        // Second pass: for tracks with no notes, scan control events
        for ev in &self.control_events {
            let (track, ch) = match ev {
                MidiControlEvent::ControlChange { track, channel, .. }
                | MidiControlEvent::ProgramChange { track, channel, .. }
                | MidiControlEvent::PitchBend { track, channel, .. } => (*track, *channel),
            };
            let idx = track as usize;
            if idx < num_tracks && track_channels[idx] == 0 {
                track_channels[idx] = (ch & 0x0F) + 1;
                if !note_port_set[idx] {
                    note_port_set[idx] = true;
                    track_ports_from_notes[idx] = (ch >> 4) & 0x0F;
                }
            }
        }
        (0..num_tracks)
            .map(|i| TrackInfo {
                index: i as u16,
                name: self
                    .track_names
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("Track {}", i + 1)),
                note_count: note_counts[i],
                port: if note_port_set[i] {
                    track_ports_from_notes[i]
                } else {
                    self.track_ports.get(i).copied().unwrap_or(0)
                },
                channel: track_channels[i].clamp(1, 16),
            })
            .collect()
    }
}

/// Info about a single MIDI track.
#[derive(Clone, Debug)]
pub struct TrackInfo {
    pub index: u16,
    pub name: String,
    pub note_count: u64,
    pub port: u8,
    pub channel: u8,
}
