//! per-voice 合成参数展开：从 `KeyInfo` + `ChannelState` 产出 CPU/GPU 共用的
//! 参数快照（speed/增益/声像/滤波/包络时长/循环）。
//!
//! 此前 CPU `CpuVoice::new` 与 GPU `GpuSynth::note_on` 各自展开同一套公式
//! （约 60~80 行重复，改一处忘另一处就会产生后端差异）；本模块是唯一展开点。
//!
//! 采样定位（CPU 的 `Arc` 切片 / GPU 的拼接 `offset`）与 `sample_length`
//! 的计算仍留在各后端（长度来源不同），不在本结构内。

use crate::channel_state::{ChannelState, env_curve_frames};
use crate::sfz_parser::KeyInfo;

/// CC72/73 修正后的 attack/release 帧数（region 原始值 + 通道当前 CC）。
pub(crate) fn env_frames_for(
    ch: &ChannelState,
    orig_attack_frames: f32,
    orig_release_frames: f32,
    sample_rate: u32,
) -> (f32, f32) {
    let attack_frames = match ch.env_attack {
        Some(cc) => env_curve_frames(cc, orig_attack_frames, sample_rate, false),
        None => orig_attack_frames,
    };
    let release_frames = match ch.env_release {
        Some(cc) => env_curve_frames(cc, orig_release_frames, sample_rate, true),
        None => orig_release_frames,
    };
    (attack_frames, release_frames)
}

/// 展开后的 per-voice 参数（CPU/GPU 共用）。
pub(crate) struct VoiceParams {
    /// 播放倍率（region speed_mult × 通道弯音/调音倍率）。
    pub(crate) speed: f32,
    pub(crate) base_speed: f32,
    pub(crate) base_gain: f32,
    /// 加载期烘焙的等功率声像增益。
    pub(crate) pan_l: f32,
    pub(crate) pan_r: f32,
    /// 加载期烘焙的 biquad 系数（cutoff=0 时 None）。
    pub(crate) biquad: Option<[f32; 5]>,
    pub(crate) delay_frames: f32,
    pub(crate) attack_frames: f32,
    pub(crate) hold_frames: f32,
    pub(crate) decay_frames: f32,
    pub(crate) release_frames: f32,
    /// region 原始 attack/release 帧数（CC72/73 重算的基准）。
    pub(crate) orig_attack_frames: f32,
    pub(crate) orig_release_frames: f32,
    pub(crate) sustain_level: f32,
    pub(crate) env_start: f32,
    pub(crate) loop_start: u32,
    pub(crate) loop_end: u32,
    pub(crate) loop_mode: u32,
    pub(crate) is_stereo: bool,
    pub(crate) interp: u32,
}

impl VoiceParams {
    /// 由 (key, vel) 快照 + 通道状态展开（与 xsynth spawner 语义对齐）。
    pub(crate) fn from_key_info(info: &KeyInfo, ch: &ChannelState, sample_rate: u32) -> Self {
        let sr = sample_rate as f32;
        let orig_attack_frames = info.ampeg_attack * sr;
        let orig_release_frames = info.ampeg_release * sr;
        let (attack_frames, release_frames) =
            env_frames_for(ch, orig_attack_frames, orig_release_frames, sample_rate);
        Self {
            speed: info.speed_mult * ch.pitch_multiplier(),
            base_speed: info.speed_mult,
            base_gain: info.volume,
            pan_l: info.pan_l,
            pan_r: info.pan_r,
            biquad: info.biquad,
            delay_frames: info.ampeg_delay * sr,
            attack_frames,
            hold_frames: info.ampeg_hold * sr,
            decay_frames: info.ampeg_decay * sr,
            release_frames,
            orig_attack_frames,
            orig_release_frames,
            sustain_level: info.ampeg_sustain,
            env_start: info.ampeg_start,
            loop_start: info.loop_start,
            loop_end: info.loop_end,
            loop_mode: info.loop_mode as u32,
            is_stereo: info.is_stereo,
            interp: info.interp,
        }
    }
}
