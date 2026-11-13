//! yinhe-synth: CPU 音频合成器。
//!
//! 独立的合成器 crate，**只负责音源层**：采样播放、voice 包络（ADSR）、
//! 音高（pitch bend/RPN 调音）、延音踏板、音色选择。
//! 通道级 DSP（音量/声像/滤波/效果/输出限幅）全部由 yinhe-dsp 与混音台负责
//! （见 `docs/spec-yinhe-dsp.md`）。
//!
//! - CpuSynth（播放/导出统一接口，多线程分片渲染）
//! - SFZ/SF2 解析器（委托 xsynth-soundfonts）
//! - Voice 状态管理（7 阶段 ADSR envelope + per-voice biquad 滤波器 = 音色自带 filter）
//! - 32 通道 MIDI 状态机（仅音源层：bank/program、pitch bend/RPN、damper、ADSR CC）

mod biquad;
pub mod channel_state;
pub mod cpu_synth;
pub mod denormals;
pub mod events;
pub(crate) mod sf_cache;
pub mod sf_diagnostics;
pub mod sf_parser;
pub mod sfz;
mod voice_params;

pub use channel_state::{ChaseSkip, MAX_CHANNELS};

/// 默认全局 voice 上限。**这是性能目标**，与**容量**（voice 槽位，只决定
/// 内存和 note_on 可容纳量）解耦：容量远大于本值时，`note_on` 不会因墓碑
/// 占位而拒绝新音（丢音）。公开：设置界面显示"系统推荐值"用。
pub const DEFAULT_MAX_VOICES: usize = 14336;
/// 默认每 key layer 上限（对齐 xsynth `VoiceChannelParams.layers`；共用）。
pub(crate) const DEFAULT_MAX_LAYERS: usize = 4;
pub use biquad::biquad_coeffs;
pub use cpu_synth::CpuSynth;
pub use events::{ControlEvent, SynthEvent};
pub use sf_cache::prefetch_key_maps;
pub use sf_cache::prefetch_key_maps_with_progress;
pub use sf_cache::sweep_key_map_cache;
pub use sf_parser::{
    KeyInfo, KeyMapEntry, LoopMode, build_key_maps, load_wav_as_f32, select_key_info,
    select_key_info_multi, select_key_infos, select_key_infos_multi,
};
