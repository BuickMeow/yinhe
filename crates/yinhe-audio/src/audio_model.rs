use std::sync::Arc;

use xsynth_core::channel::ChannelAudioEvent;
use yinhe_core::YinModel;

use crate::spawn::track_global_channel;

pub(crate) struct SortedCC {
    pub(crate) sample: u64,
    pub(crate) channel: u32,
    pub(crate) event: ChannelAudioEvent,
}

#[derive(Clone, Copy)]
pub(crate) struct ActiveNote {
    pub(crate) key: u8,
    pub(crate) channel: u8,
    pub(crate) end_sample: u64,
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
    pub(crate) fn from_model(model: &YinModel) -> Self {
        let track_channels: Vec<u8> = (0..model.tracks.len())
            .map(|i| track_global_channel(model, i))
            .collect();
        let track_cc0: Vec<Vec<u8>> = model
            .tracks
            .iter()
            .map(|t| {
                t.automation_lanes
                    .iter()
                    .find(|l| matches!(l.target, yinhe_types::AutomationTarget::CC { controller: 0 }))
                    .map(|lane| lane.events.iter().map(|e| (e.value & 0x7F) as u8).collect())
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
    pub(crate) fn track_channel(&self, track_idx: usize) -> u8 {
        self.track_channels.get(track_idx).copied().unwrap_or(0)
    }
}

/// Convert a tick value to sample position using the tempo map.
pub(crate) fn tick_to_sample(tick: u64, segments: &[yinhe_core::TempoSegment], tpb: u32, sr: f64) -> u64 {
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