//! Implement `yinhe_types::NoteSource` for `YinModel`.
//!
//! This trait is the contract between data sources and rendering / audio
//! consumers. By implementing it on YinModel directly, every component
//! that already speaks NoteSource (PianoRoll, Arrangement, automation
//! browser) works without changes.

use yinhe_types::{AutomationLane, Note, NoteScanIndex, NoteSource, TickBuckets, TimeSigEvent};

use crate::model::YinModel;

impl NoteSource for YinModel {
    fn key_notes(&self, key: u8) -> &[Note] {
        &self.key_notes_cache[key as usize]
    }

    fn duration(&self) -> f64 {
        self.tempo_map.duration_seconds()
    }

    fn ticks_per_beat(&self) -> Option<u32> {
        Some(self.meta.ppq)
    }

    fn tick_at_time(&self, time: f64) -> Option<f64> {
        Some(self.tempo_map.tick_at_time(time))
    }

    fn tick_length(&self) -> Option<u64> {
        Some(self.tick_length)
    }

    fn scan_index(&self) -> Option<&NoteScanIndex> {
        self.scan_index.as_ref()
    }

    fn tick_buckets(&self) -> Option<&TickBuckets> {
        self.tick_buckets.as_ref()
    }

    fn time_sig_default(&self) -> (u8, u8) {
        self.tempo_map.time_sig_default
    }

    fn time_sig_events(&self) -> &[TimeSigEvent] {
        &self.tempo_map.time_sig_events
    }

    fn automation_lanes(&self) -> &[AutomationLane] {
        // Automation lanes are derived from TrackData on demand. The
        // current renderer pulls them via a separate call; we don't cache
        // them here yet (no consumer requires this returned slice today).
        &[]
    }
}
