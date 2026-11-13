//! yinhe-synth 事件转换：`AudioEvent` → 后端控制事件、后端 ChaseSkip → 引擎掩码。
//!
//! 原先在 `engine_gpu` 里（GPU 后端已删除），CPU 后端/预览也在用，独立成模块。

use xsynth_core::channel::ControlEvent;

use crate::channel::ChaseSkip;

/// yinhe-synth 后端的 `ChaseSkip`（按 dense 槽位索引）→ audio 侧 `ChaseSkip`
/// （按源通道 0..256 索引）。
pub(crate) fn translate_backend_skip(
    layout: &crate::channel_layout::ChannelLayout,
    backend_skip: &yinhe_synth::ChaseSkip,
) -> ChaseSkip {
    let mut skip = ChaseSkip::default();
    for ch in 0..256usize {
        let dense = layout.dense_for(ch);
        if dense == u32::MAX || (dense as usize) >= yinhe_synth::MAX_CHANNELS {
            continue;
        }
        let idx = dense as usize;
        skip.cc_mask[ch] = backend_skip.cc_mask[idx];
        skip.pitch_bend[ch] = backend_skip.pitch_bend[idx];
        skip.pbs[ch] = backend_skip.pbs[idx];
        skip.fine_tune[ch] = backend_skip.fine_tune[idx];
        skip.coarse_tune[ch] = backend_skip.coarse_tune[idx];
        skip.program[ch] = backend_skip.program[idx];
    }
    skip
}

/// 事件流 `AudioEvent` → yinhe-synth 控制事件。
pub(crate) fn to_backend_control_event(
    ev: &crate::audio_model::AudioEvent,
) -> Option<yinhe_synth::ControlEvent> {
    use crate::audio_model::AudioEvent;
    match *ev {
        // 引擎内部浮点 CC → yinhe-synth f32（全精度）。
        AudioEvent::Cc { controller, value } => {
            // 通道级 DSP CC 由 yinhe-dsp 效果器处理，合成器不再接收。
            if yinhe_dsp::cc::DSP_CHANNEL_CCS.contains(&controller) {
                return None;
            }
            Some(yinhe_synth::ControlEvent::Raw(controller, value))
        }
        AudioEvent::Channel(xsynth_core::channel::ChannelAudioEvent::Control(
            ControlEvent::Raw(c, v),
        )) => {
            // 通道级 DSP CC 由 yinhe-dsp 效果器处理，合成器不再接收。
            if yinhe_dsp::cc::DSP_CHANNEL_CCS.contains(&c) {
                return None;
            }
            Some(yinhe_synth::ControlEvent::Raw(c, f32::from(v)))
        }
        AudioEvent::Channel(xsynth_core::channel::ChannelAudioEvent::Control(
            ControlEvent::PitchBendValue(v),
        )) => Some(yinhe_synth::ControlEvent::PitchBend(v)),
        AudioEvent::Channel(xsynth_core::channel::ChannelAudioEvent::ProgramChange(p)) => {
            Some(yinhe_synth::ControlEvent::ProgramChange(p))
        }
        // 引擎通用 PB / PC（收敛后不再走 xsynth 通道事件）。
        AudioEvent::PitchBend(v) => Some(yinhe_synth::ControlEvent::PitchBend(v)),
        AudioEvent::ProgramChange(p) => Some(yinhe_synth::ControlEvent::ProgramChange(p)),
        // 原生 RPN/NRPN：yinhe-synth 一等事件（u16 参数号），不拆 CC。
        AudioEvent::Rpn { parameter, value } => {
            Some(yinhe_synth::ControlEvent::Rpn { parameter, value })
        }
        AudioEvent::Nrpn { parameter, value } => {
            Some(yinhe_synth::ControlEvent::Nrpn { parameter, value })
        }
        _ => None,
    }
}
