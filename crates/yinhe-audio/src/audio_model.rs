use std::sync::Arc;

use xsynth_core::channel::{ChannelAudioEvent, ControlEvent};
use yinhe_core::YinModel;
use yinhe_types::{AutomationTarget, SegmentShape};

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

/// 音频线程消费的可听音事件（vel > 1），tick 已预转换为 sample。
/// 桶内按 `start_sample` 严格升序排列（YinModel.notes[key] 本身按 start_tick 升序，
/// tick→sample 单调，所以转换后保持有序）。
///
/// `key` 不存（桶索引即 key）。`id` 用于 undo/redo 后跨 prepared model
/// 引用同一音符（暂未使用，预留）。
#[repr(C)]
pub(crate) struct AudibleNote {
    pub start_sample: u64,
    pub end_sample: u64,
    pub id: u32,
    pub track: u16,
    pub velocity: u8,
}

/// Pre-computed model data, built on a worker thread and applied
/// atomically on the audio thread.
pub(crate) struct PreparedModel {
    pub model: AudioModel,
    pub yin_model: Arc<YinModel>,
    pub cc_events: Vec<SortedCC>,
    /// 128 个 key 桶的可听音（vel > 1），tick 已转 sample。
    /// 音频线程的 seek / dispatch 只读这份列表，不再访问 YinModel.notes。
    pub audible_notes: Box<[Vec<AudibleNote>; 128]>,
    pub duration_samples: u64,
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
///
/// `density`: Linear/Curve 段在播放时按多少 tick 间隔展开中间事件。1 = 每 tick 一个事件
/// （最平滑），值越大中间事件越少。Step 段不受影响（保持值到下一点）。
pub(crate) fn flatten_automation_to_cc_events(
    model: &YinModel,
    sample_rate: u32,
    density: u32,
) -> Vec<SortedCC> {
    let sr = sample_rate as f64;
    let density = density.max(1);
    let mut cc_events = Vec::new();

    for (track_idx, track) in model.tracks.iter().enumerate() {
        let channel = track_global_channel(model, track_idx) as u32;

        for lane in &track.automation_lanes {
            let n = lane.events.len();
            for (i, e) in lane.events.iter().enumerate() {
                let sample = (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
                emit_automation_event(&lane.target, e.value, sample, channel, &mut cc_events);

                // Linear/Curve 段：在当前事件与下一事件之间按 density 间隔展开中间事件
                if i + 1 < n {
                    let next = &lane.events[i + 1];
                    let tick1 = e.tick;
                    let tick2 = next.tick;
                    if tick2 > tick1 && !matches!(e.shape, SegmentShape::Step) {
                        let v1 = e.value as f32;
                        let v2 = next.value as f32;
                        let span = (tick2 - tick1) as f32;
                        let mut t = tick1.saturating_add(density);
                        while t < tick2 {
                            let frac = (t - tick1) as f32 / span;
                            let f = e.shape.interpolate(frac);
                            let v = (v1 + (v2 - v1) * f).round() as u16;
                            let s = (model.tempo_map.tick_to_seconds(t as u64) * sr) as u64;
                            emit_automation_event(&lane.target, v, s, channel, &mut cc_events);
                            t = t.saturating_add(density);
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

/// 将单个 automation 值转换成 XSynth 事件并推入 `out`。
fn emit_automation_event(
    target: &AutomationTarget,
    value: u16,
    sample: u64,
    channel: u32,
    out: &mut Vec<SortedCC>,
) {
    match target {
        AutomationTarget::CC { controller } => {
            out.push(SortedCC {
                sample, channel,
                event: ChannelAudioEvent::Control(ControlEvent::Raw(
                    *controller, (value & 0x7F) as u8,
                )),
            });
        }
        AutomationTarget::PitchBend => {
            out.push(SortedCC {
                sample, channel,
                event: ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                    (value as f32 - 8192.0) / 8192.0,
                )),
            });
        }
        AutomationTarget::Rpn { parameter } => {
            match parameter {
                0 => {
                    out.push(SortedCC {
                        sample, channel,
                        event: ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(value as f32)),
                    });
                }
                1 => {
                    let fine = (value as f32 - 8192.0) / 8192.0 * 100.0;
                    out.push(SortedCC {
                        sample, channel,
                        event: ChannelAudioEvent::Control(ControlEvent::FineTune(fine)),
                    });
                }
                2 => {
                    let coarse = value as f32 - 64.0;
                    out.push(SortedCC {
                        sample, channel,
                        event: ChannelAudioEvent::Control(ControlEvent::CoarseTune(coarse)),
                    });
                }
                _ => {
                    // Non-standard RPN: fall back to CC sequence
                    let msb = ((parameter >> 8) & 0x7F) as u8;
                    let lsb = (parameter & 0x7F) as u8;
                    let (data_msb, data_lsb) = if target.is_14bit() {
                        (((value >> 7) & 0x7F) as u8, (value & 0x7F) as u8)
                    } else {
                        (value as u8, 0u8)
                    };
                    out.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(101, msb)) });
                    out.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(100, lsb)) });
                    out.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(6, data_msb)) });
                    if data_lsb != 0 {
                        out.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(38, data_lsb)) });
                    }
                }
            }
        }
        AutomationTarget::Nrpn { parameter } => {
            let msb = ((parameter >> 8) & 0x7F) as u8;
            let lsb = (parameter & 0x7F) as u8;
            let data_msb = ((value >> 7) & 0x7F) as u8;
            let data_lsb = (value & 0x7F) as u8;
            out.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(99, msb)) });
            out.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(98, lsb)) });
            out.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(6, data_msb)) });
            if data_lsb != 0 {
                out.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(38, data_lsb)) });
            }
        }
    }
}