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

    /// Find the tempo segment containing the given time.
    fn find_segment_at(&self, time: f64) -> Option<&TempoSegment> {
        if self.tempo_segments.is_empty() {
            return None;
        }
        self.tempo_segments
            .iter()
            .rposition(|s| s.start_time <= time)
            .map(|idx| &self.tempo_segments[idx])
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
        let idx = self
            .time_sig_events
            .iter()
            .rposition(|e| e.tick <= tick)
            .unwrap_or(0);
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
        let seg_idx = self
            .tempo_segments
            .iter()
            .rposition(|s| s.start_tick <= tick);
        let Some(seg_idx) = seg_idx else {
            return crate::time::ticks_to_seconds(tick, self.ticks_per_beat, DEFAULT_MPQ);
        };
        let seg = &self.tempo_segments[seg_idx];
        let dtick = tick - seg.start_tick;
        seg.start_time
            + crate::time::ticks_to_seconds(dtick, self.ticks_per_beat, seg.micros_per_quarter)
    }

    /// Get port number for a track.
    pub fn track_port(&self, track_idx: usize) -> u8 {
        self.track_ports.get(track_idx).copied().unwrap_or(0)
    }

    /// Get info for all tracks (name, note count, port).
    pub fn track_info(&self) -> Vec<TrackInfo> {
        let num_tracks = self.track_ports.len();
        let mut note_counts = vec![0u64; num_tracks];
        for notes in &self.key_notes {
            for note in notes {
                let idx = note.track as usize;
                if idx < num_tracks {
                    note_counts[idx] += 1;
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
                port: self.track_ports.get(i).copied().unwrap_or(0),
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
}
