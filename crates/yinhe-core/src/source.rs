//! Implement `yinhe_types::NoteSource` for `YinModel`.
//!
//! This trait is the contract between data sources and rendering / audio
//! consumers. By implementing it on YinModel directly, every component
//! that already speaks NoteSource (PianoRoll, Arrangement) works without
//! changes.

use yinhe_types::{Note, NoteSource, TimeSigEvent};

use crate::model::YinModel;

impl NoteSource for YinModel {
    fn key_notes(&self, key: u8) -> &[Note] {
        self.notes[key as usize].as_slice()
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

    fn time_sig_default(&self) -> (u8, u8) {
        self.tempo_map.time_sig_default
    }

    fn time_sig_events(&self) -> &[TimeSigEvent] {
        &self.tempo_map.time_sig_events
    }
}
