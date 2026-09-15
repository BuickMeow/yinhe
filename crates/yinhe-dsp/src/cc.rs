//! CC 驱动的通道级效果模块。
//!
//! 效果器参数在工程底层**伪装成 MIDI CC**（用 CC lane 存储、导出 MIDI 时按
//! CC 写出），但它们是效果器自己的参数（Volume/Pan/Cutoff/…），
//! UI 与命名不得称其为"CC"。
//!
//! 每个模块声明自己接管的伪 CC（[`yinhe_mixer::InsertProcessor::handled_ccs`]），
//! 宿主把对应的事件分流到模块（`apply_cc`），使 xsynth 只保留音源层参数。

pub mod filter;
pub mod gain;
pub mod pan;

/// 参数斜坡时长（秒）：与 xsynth `ValueLerp` 一致（10ms）。
pub const CC_RAMP_SECONDS: f32 = 0.01;

/// 由 yinhe-dsp 模块处理的伪 CC 白名单（内部映射，不用于 UI 显示）。
///
/// dispatch 对这些 CC 直接广播给通道 insert 链
/// （[`yinhe_mixer::MixerGraph::broadcast_channel_cc`]），不再下发合成器/乐器插件。
/// 音源层 CC（ADSR、Sustain、Portamento、Vibrato、Bank/PC、RPN 等）
/// 仍由合成器处理——那些必须对 voice 动刀。
pub const DSP_CHANNEL_CCS: &[u8] = &[
    7,  // ChannelGain::Volume
    10, // ChannelPan::Pan
    11, // ChannelGain::Expression
    71, // ChannelFilter::Resonance
    74, // ChannelFilter::Cutoff
];
