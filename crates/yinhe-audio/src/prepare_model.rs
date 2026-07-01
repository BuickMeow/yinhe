use std::sync::Arc;

use xsynth_core::channel::{ChannelAudioEvent, ControlEvent};

use yinhe_core::YinModel;
use yinhe_types::AutomationTarget;

use crate::audio_model::{AudioModel, PreparedModel, SortedCC};
use crate::spawn::track_global_channel;

/// Build `PreparedModel` on a worker thread (no `&mut AudioEngine` needed).
/// This is the expensive part; the result is applied cheaply on the audio thread.
pub(crate) fn prepare_model(
    model: &Arc<YinModel>,
    sample_rate: u32,
    _active_mask: &[bool],
    _channel_map: &[u32; 256],
) -> PreparedModel {
    let sr = sample_rate as f64;
    let mut cc_events = Vec::new();

    for (track_idx, track) in model.tracks.iter().enumerate() {
        let channel = track_global_channel(model, track_idx) as u32;

        // Automation lanes → xsynth events
        for lane in &track.automation_lanes {
            for e in &lane.events {
                let sample = (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
                match &lane.target {
                    AutomationTarget::CC { controller } => {
                        cc_events.push(SortedCC {
                            sample,
                            channel,
                            event: ChannelAudioEvent::Control(ControlEvent::Raw(
                                *controller,
                                (e.value & 0x7F) as u8,
                            )),
                        });
                    }
                    AutomationTarget::PitchBend => {
                        cc_events.push(SortedCC {
                            sample,
                            channel,
                            event: ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                                e.value as f32 / 8192.0,
                            )),
                        });
                    }
                    AutomationTarget::Rpn { parameter } => {
                        let msb = ((parameter >> 8) & 0x7F) as u8;
                        let lsb = (parameter & 0x7F) as u8;
                        let data_msb = ((e.value >> 7) & 0x7F) as u8;
                        let data_lsb = (e.value & 0x7F) as u8;
                        cc_events.push(SortedCC {
                            sample,
                            channel,
                            event: ChannelAudioEvent::Control(ControlEvent::Raw(101, msb)),
                        });
                        cc_events.push(SortedCC {
                            sample,
                            channel,
                            event: ChannelAudioEvent::Control(ControlEvent::Raw(100, lsb)),
                        });
                        cc_events.push(SortedCC {
                            sample,
                            channel,
                            event: ChannelAudioEvent::Control(ControlEvent::Raw(6, data_msb)),
                        });
                        if data_lsb != 0 {
                            cc_events.push(SortedCC {
                                sample,
                                channel,
                                event: ChannelAudioEvent::Control(ControlEvent::Raw(38, data_lsb)),
                            });
                        }
                    }
                    AutomationTarget::Nrpn { parameter } => {
                        let msb = ((parameter >> 8) & 0x7F) as u8;
                        let lsb = (parameter & 0x7F) as u8;
                        let data_msb = ((e.value >> 7) & 0x7F) as u8;
                        let data_lsb = (e.value & 0x7F) as u8;
                        cc_events.push(SortedCC {
                            sample,
                            channel,
                            event: ChannelAudioEvent::Control(ControlEvent::Raw(99, msb)),
                        });
                        cc_events.push(SortedCC {
                            sample,
                            channel,
                            event: ChannelAudioEvent::Control(ControlEvent::Raw(98, lsb)),
                        });
                        cc_events.push(SortedCC {
                            sample,
                            channel,
                            event: ChannelAudioEvent::Control(ControlEvent::Raw(6, data_msb)),
                        });
                        if data_lsb != 0 {
                            cc_events.push(SortedCC {
                                sample,
                                channel,
                                event: ChannelAudioEvent::Control(ControlEvent::Raw(38, data_lsb)),
                            });
                        }
                    }
                }
            }
        }

        // Program Change (discrete, not automation)
        for e in &track.program_change {
            let sample = (model.tempo_map.tick_to_seconds(e.tick as u64) * sr) as u64;
            if e.bank_msb != 0xFF {
                cc_events.push(SortedCC {
                    sample,
                    channel,
                    event: ChannelAudioEvent::Control(ControlEvent::Raw(0, e.bank_msb)),
                });
            }
            if e.bank_lsb != 0xFF {
                cc_events.push(SortedCC {
                    sample,
                    channel,
                    event: ChannelAudioEvent::Control(ControlEvent::Raw(32, e.bank_lsb)),
                });
            }
            cc_events.push(SortedCC {
                sample,
                channel,
                event: ChannelAudioEvent::ProgramChange(e.program),
            });
        }
    }
    cc_events.sort_by_key(|e| e.sample);
    // 去重：同 sample 同 channel 的重复事件 + 连续相同值的事件（不改变 xsynth 状态）
    cc_events.dedup_by(|a, b| a.channel == b.channel && a.event == b.event);

    let duration_samples = (model.tempo_map.tick_to_seconds(model.tick_length) * sr) as u64;

    let skip_track: Vec<bool> = model
        .track_has_audio_cache
        .iter()
        .map(|&has| !has)
        .collect();

    PreparedModel {
        model: AudioModel::from_model(model),
        yin_model: Arc::clone(model),
        cc_events,
        duration_samples,
        skip_track,
    }
}