use std::sync::Arc;

use xsynth_core::channel::{ChannelAudioEvent, ControlEvent};
use yinhe_core::YinModel;
use yinhe_types::AutomationTarget;

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

/// Flatten automation lanes + program changes into sorted, deduped SortedCC events.
///
/// Standard RPN 0/1/2 are sent as high-level xsynth events (PitchBendSensitivity,
/// FineTune, CoarseTune). Non-standard RPN and NRPN use the raw CC sequence.
pub(crate) fn flatten_automation_to_cc_events(
    model: &YinModel,
    sample_rate: u32,
) -> Vec<SortedCC> {
    let sr = sample_rate as f64;
    let mut cc_events = Vec::new();

    for (track_idx, track) in model.tracks.iter().enumerate() {
        let channel = track_global_channel(model, track_idx) as u32;

        for lane in &track.automation_lanes {
            for e in &lane.events {
                let sample = (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
                match &lane.target {
                    AutomationTarget::CC { controller } => {
                        cc_events.push(SortedCC {
                            sample, channel,
                            event: ChannelAudioEvent::Control(ControlEvent::Raw(
                                *controller, (e.value & 0x7F) as u8,
                            )),
                        });
                    }
                    AutomationTarget::PitchBend => {
                        cc_events.push(SortedCC {
                            sample, channel,
                            event: ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                                (e.value as f32 - 8192.0) / 8192.0,
                            )),
                        });
                    }
                    AutomationTarget::Rpn { parameter } => {
                        match parameter {
                            0 => {
                                cc_events.push(SortedCC {
                                    sample, channel,
                                    event: ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(e.value as f32)),
                                });
                            }
                            1 => {
                                let fine = (e.value as f32 - 8192.0) / 8192.0 * 100.0;
                                cc_events.push(SortedCC {
                                    sample, channel,
                                    event: ChannelAudioEvent::Control(ControlEvent::FineTune(fine)),
                                });
                            }
                            2 => {
                                let coarse = e.value as f32 - 64.0;
                                cc_events.push(SortedCC {
                                    sample, channel,
                                    event: ChannelAudioEvent::Control(ControlEvent::CoarseTune(coarse)),
                                });
                            }
                            _ => {
                                // Non-standard RPN: fall back to CC sequence
                                let msb = ((parameter >> 8) & 0x7F) as u8;
                                let lsb = (parameter & 0x7F) as u8;
                                let (data_msb, data_lsb) = if lane.target.is_14bit() {
                                    (((e.value >> 7) & 0x7F) as u8, (e.value & 0x7F) as u8)
                                } else {
                                    (e.value as u8, 0u8)
                                };
                                cc_events.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(101, msb)) });
                                cc_events.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(100, lsb)) });
                                cc_events.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(6, data_msb)) });
                                if data_lsb != 0 {
                                    cc_events.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(38, data_lsb)) });
                                }
                            }
                        }
                    }
                    AutomationTarget::Nrpn { parameter } => {
                        let msb = ((parameter >> 8) & 0x7F) as u8;
                        let lsb = (parameter & 0x7F) as u8;
                        let data_msb = ((e.value >> 7) & 0x7F) as u8;
                        let data_lsb = (e.value & 0x7F) as u8;
                        cc_events.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(99, msb)) });
                        cc_events.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(98, lsb)) });
                        cc_events.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(6, data_msb)) });
                        if data_lsb != 0 {
                            cc_events.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(38, data_lsb)) });
                        }
                    }
                }
            }
        }

        for e in &track.program_change {
            let sample = (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
            if e.bank_msb != 0xFF {
                cc_events.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(0, e.bank_msb)) });
            }
            if e.bank_lsb != 0xFF {
                cc_events.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(32, e.bank_lsb)) });
            }
            cc_events.push(SortedCC { sample, channel, event: ChannelAudioEvent::ProgramChange(e.program) });
        }
    }

    cc_events.sort_by_key(|e| e.sample);
    cc_events.dedup_by(|a, b| a.channel == b.channel && a.event == b.event);
    cc_events
}