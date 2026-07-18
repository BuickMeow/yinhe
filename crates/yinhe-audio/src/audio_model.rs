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
    /// `Arc` so the same cc_events can be shared between the renderer thread
    /// (for seek/chase dispatch) and the worker thread (for chase computation)
    /// without cloning the (potentially hundreds of thousands of) events.
    pub cc_events: Arc<Vec<SortedCC>>,
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
                    .map(|lane| lane.events.iter().map(|e| (e.value.round() as u16 & 0x7F) as u8).collect())
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
///
/// Returns `Arc<Vec>` so the same events can be shared between the renderer and
/// the worker thread (for chase computation) without cloning.
pub(crate) fn flatten_automation_to_cc_events(
    model: &YinModel,
    sample_rate: u32,
    density: u32,
) -> Arc<Vec<SortedCC>> {
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
                        let v1 = e.value;
                        let v2 = next.value;
                        let span = (tick2 - tick1) as f32;
                        let mut t = tick1.saturating_add(density);
                        while t < tick2 {
                            let frac = (t - tick1) as f32 / span;
                            let f = e.shape.interpolate(frac);
                            let v = v1 + (v2 - v1) * f;
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

    // 排序：同 sample 同 channel 下，RPN/参数类事件必须排在 PitchBendValue 之前。
    // 原因：xsynth 收到 PitchBendValue 时会按当前 PBS 立即计算弯音并作用于已响 voice，
    // 若 PBS 尚未更新，PB 会用旧 PBS 算出错误音高。见 commit 3490e02。
    // sort_by_key 稳定，同 priority 仍按插入顺序。
    cc_events.sort_by_key(|e| (e.sample, e.channel, dispatch_priority(&e.event)));
    cc_events.dedup_by(|a, b| a.channel == b.channel && a.event == b.event);
    Arc::new(cc_events)
}

/// 同 sample 同 channel 内的分发优先级：0 = 参数/控制类（RPN、CC、PC），
/// 1 = PitchBendValue。数值小的先发，保证 PBS/FineTune/CoarseTune 等 RPN
/// 参数在 PB 使用它们之前就位。
fn dispatch_priority(event: &ChannelAudioEvent) -> u8 {
    match event {
        ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_)) => 1,
        _ => 0,
    }
}

/// 将单个 automation 值转换成 XSynth 事件并推入 `out`。
fn emit_automation_event(
    target: &AutomationTarget,
    value: f32,
    sample: u64,
    channel: u32,
    out: &mut Vec<SortedCC>,
) {
    // f32 → u16 一次，所有位运算都用这个整数
    let v = value.round() as u16;
    match target {
        AutomationTarget::CC { controller } => {
            out.push(SortedCC {
                sample, channel,
                event: ChannelAudioEvent::Control(ControlEvent::Raw(
                    *controller, (v & 0x7F) as u8,
                )),
            });
        }
        AutomationTarget::PitchBend => {
            out.push(SortedCC {
                sample, channel,
                event: ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                    (value - 8192.0) / 8192.0,
                )),
            });
        }
        AutomationTarget::Rpn { parameter } => {
            match parameter {
                0 => {
                    out.push(SortedCC {
                        sample, channel,
                        event: ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(value)),
                    });
                }
                1 => {
                    let fine = (value - 8192.0) / 8192.0 * 100.0;
                    out.push(SortedCC {
                        sample, channel,
                        event: ChannelAudioEvent::Control(ControlEvent::FineTune(fine)),
                    });
                }
                2 => {
                    let coarse = value - 64.0;
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
                        (((v >> 7) & 0x7F) as u8, (v & 0x7F) as u8)
                    } else {
                        (v as u8, 0u8)
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
            let data_msb = ((v >> 7) & 0x7F) as u8;
            let data_lsb = (v & 0x7F) as u8;
            out.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(99, msb)) });
            out.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(98, lsb)) });
            out.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(6, data_msb)) });
            if data_lsb != 0 {
                out.push(SortedCC { sample, channel, event: ChannelAudioEvent::Control(ControlEvent::Raw(38, data_lsb)) });
            }
        }
        // Tempo 走 `conductor.tempo` 而非 `track.automation_lanes`，
        // 由 `build_tempo_map` 消费，不进入 CC 事件流。
        AutomationTarget::Tempo => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_core::{ConductorData, ProjectMeta, TrackData, YinModel};
    use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

    /// 构建 1 轨道模型，给定 automation lanes。
    fn model_with_lanes(lanes: Vec<AutomationLane>) -> YinModel {
        let conductor = ConductorData {
            tempo: AutomationLane {
                target: AutomationTarget::Tempo,
                track: 0,
                events: vec![AutomationEvent { tick: 0, value: 120.0, shape: SegmentShape::Step }],
            },
            time_sig: Vec::new(),
        };
        let mut t = TrackData::new(0, 0);
        t.automation_lanes = lanes;
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t)],
            meta: ProjectMeta { ppq: 480, ..ProjectMeta::default() },
            ..Default::default()
        };
        model.rebuild();
        model
    }

    /// 在 `cc_events` 中找第一个匹配 `pred` 事件的索引。
    fn index_of<F>(events: &[SortedCC], pred: F) -> Option<usize>
    where
        F: Fn(&ChannelAudioEvent) -> bool,
    {
        events.iter().position(|e| pred(&e.event))
    }

    /// 回归测试：同 tick 上 RPN 0 (PBS) 必须排在 PitchBend 之前。
    /// 见 commit 3490e02：若 PB 先于 PBS，PB 会用旧 PBS 计算弯音，导致音高异常。
    #[test]
    fn rpn_pbs_must_precede_pitch_bend_at_same_tick() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::PitchBend,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 16383.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Rpn { parameter: 0 },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 24.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let events = flatten_automation_to_cc_events(&model, 44100, 1);

        let pbs_idx = index_of(&events, |e| {
            matches!(e, ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(_)))
        });
        let pb_idx = index_of(&events, |e| {
            matches!(e, ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_)))
        });

        let pbs_idx = pbs_idx.expect("PBS event should exist");
        let pb_idx = pb_idx.expect("PitchBend event should exist");
        assert!(
            pbs_idx < pb_idx,
            "PBS (index {}) must precede PitchBend (index {}) at the same tick, \
             otherwise PB uses stale PBS and pitch is wrong (regression of 3490e02)",
            pbs_idx,
            pb_idx
        );
    }

    /// 覆盖非标准 RPN（走 raw CC101/100/6 序列）：同 tick 上 RPN 选择 + DataEntry
    /// 也必须排在 PB 之前。
    #[test]
    fn nonstandard_rpn_cc_sequence_must_precede_pitch_bend() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::PitchBend,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 16383.0,
                    shape: SegmentShape::Step,
                }],
            },
            // RPN 5（非标准）→ 走 raw CC101/100/6 序列
            AutomationLane {
                target: AutomationTarget::Rpn { parameter: 5 },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 100.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let events = flatten_automation_to_cc_events(&model, 44100, 1);

        let rpn_cc101_idx = index_of(&events, |e| {
            matches!(e, ChannelAudioEvent::Control(ControlEvent::Raw(101, _)))
        });
        let pb_idx = index_of(&events, |e| {
            matches!(e, ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_)))
        });

        let rpn_cc101_idx = rpn_cc101_idx.expect("RPN CC101 selector should exist");
        let pb_idx = pb_idx.expect("PitchBend event should exist");
        assert!(
            rpn_cc101_idx < pb_idx,
            "RPN selector CC101 (index {}) must precede PitchBend (index {}) at the same tick",
            rpn_cc101_idx,
            pb_idx
        );
    }

    /// 覆盖 NRPN：同 tick 上 NRPN 的 CC99/98/6 序列也必须排在 PB 之前。
    #[test]
    fn nrpn_cc_sequence_must_precede_pitch_bend() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::PitchBend,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 16383.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Nrpn { parameter: 10 },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 100.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let events = flatten_automation_to_cc_events(&model, 44100, 1);

        let nrpn_cc99_idx = index_of(&events, |e| {
            matches!(e, ChannelAudioEvent::Control(ControlEvent::Raw(99, _)))
        });
        let pb_idx = index_of(&events, |e| {
            matches!(e, ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_)))
        });

        let nrpn_cc99_idx = nrpn_cc99_idx.expect("NRPN CC99 selector should exist");
        let pb_idx = pb_idx.expect("PitchBend event should exist");
        assert!(
            nrpn_cc99_idx < pb_idx,
            "NRPN selector CC99 (index {}) must precede PitchBend (index {}) at the same tick",
            nrpn_cc99_idx,
            pb_idx
        );
    }

    /// 同 tick 上 FineTune (RPN 1) / CoarseTune (RPN 2) 也应排在 PB 前。
    #[test]
    fn rpn_fine_and_coarse_tune_precede_pitch_bend() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::PitchBend,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 16383.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Rpn { parameter: 1 },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 9000.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Rpn { parameter: 2 },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 70.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let events = flatten_automation_to_cc_events(&model, 44100, 1);

        let fine_idx = index_of(&events, |e| {
            matches!(e, ChannelAudioEvent::Control(ControlEvent::FineTune(_)))
        });
        let coarse_idx = index_of(&events, |e| {
            matches!(e, ChannelAudioEvent::Control(ControlEvent::CoarseTune(_)))
        });
        let pb_idx = index_of(&events, |e| {
            matches!(e, ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_)))
        });

        let fine_idx = fine_idx.expect("FineTune event should exist");
        let coarse_idx = coarse_idx.expect("CoarseTune event should exist");
        let pb_idx = pb_idx.expect("PitchBend event should exist");
        assert!(
            fine_idx < pb_idx && coarse_idx < pb_idx,
            "FineTune (idx {}) and CoarseTune (idx {}) must precede PitchBend (idx {}) at the same tick",
            fine_idx,
            coarse_idx,
            pb_idx
        );
    }
}