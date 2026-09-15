//! CC 驱动的通道级效果模块。
//!
//! 每个模块声明自己接管的 CC（[`yinhe_mixer::InsertProcessor::handled_ccs`]），
//! 宿主把被接管的 CC 从合成器分流到模块（`apply_cc`），
//! 使 xsynth 只保留音源层参数。

pub mod filter;
pub mod gain;
pub mod pan;

/// 参数斜坡时长（秒）：与 xsynth `ValueLerp` 一致（10ms）。
pub const CC_RAMP_SECONDS: f32 = 0.01;

/// 由 yinhe-dsp 模块在**通道级**处理的 CC 白名单。
///
/// dispatch 对这些 CC 直接广播给通道 insert 链
/// （[`yinhe_mixer::MixerGraph::broadcast_channel_cc`]），不再下发合成器/乐器插件。
/// 音源层 CC（ADSR、Sustain、Portamento、Vibrato、Bank/PC、RPN 等）
/// 仍由合成器处理——那些必须对 voice 动刀。
pub const DSP_CHANNEL_CCS: &[u8] = &[
    7,  // Channel Volume → ChannelGain
    10, // Pan            → ChannelPan
    11, // Expression     → ChannelGain
    71, // Resonance      → ChannelFilter
    74, // Cutoff         → ChannelFilter
];
