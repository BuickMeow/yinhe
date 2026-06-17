use yinhe_types::{AutomationLane, Note, NoteScanIndex, NoteSource, TickBuckets, TimeSigEvent};

use crate::model::YinModel;

impl NoteSource for YinModel {
    fn key_notes(&self, key: u8) -> &[Note] {
        &self.key_notes_cache[key as usize]
    }

    fn duration(&self) -> f64 {
        // Approximate: tick_length / ppq * 60 / bpm
        let bpm = self.conductor.tempo.first().map(|t| t.bpm).unwrap_or(120.0);
        self.tick_length as f64 / self.meta.ppq as f64 * 60.0 / bpm
    }

    fn ticks_per_beat(&self) -> Option<u32> {
        Some(self.meta.ppq)
    }

    fn tick_at_time(&self, time: f64) -> Option<f64> {
        // Simple linear approximation
        let bpm = self.conductor.tempo.first().map(|t| t.bpm).unwrap_or(120.0);
        Some(time * self.meta.ppq as f64 * bpm / 60.0)
    }

    fn tick_length(&self) -> Option<u64> {
        Some(self.tick_length)
    }

    fn scan_index(&self) -> Option<&NoteScanIndex> {
        self.key_index.scan_index.as_ref()
    }

    fn tick_buckets(&self) -> Option<&TickBuckets> {
        self.key_index.tick_buckets.as_ref()
    }

    fn time_sig_default(&self) -> (u8, u8) {
        self.conductor
            .time_sig
            .first()
            .map(|ts| (ts.numerator, ts.denominator))
            .unwrap_or((4, 2))
    }

    fn time_sig_events(&self) -> &[TimeSigEvent] {
        // YinModel stores TimeSigEvent differently from yinhe_types::TimeSigEvent
        // We need to convert. For now, return empty and rely on default.
        // TODO: store yinhe_types::TimeSigEvent in YinModel
        &[]
    }

    fn automation_lanes(&self) -> &[AutomationLane] {
        // TODO: build automation lanes from CC/PB/PC/RPN events
        &[]
    }
}
