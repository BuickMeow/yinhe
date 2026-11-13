//! per-voice 合成参数展开：从 `KeyInfo` + `ChannelState` 产出 CPU/GPU 共用的
//! 参数快照（speed/增益/声像/滤波/包络时长/循环）。
//!
//! 此前 CPU `CpuVoice::new` 与 GPU `GpuSynth::note_on` 各自展开同一套公式
//! （约 60~80 行重复，改一处忘另一处就会产生后端差异）；本模块是唯一展开点。
//!
//! 采样定位（CPU 的 `Arc` 切片 / GPU 的拼接 `offset`）与 `sample_length`
//! 的计算仍留在各后端（长度来源不同），不在本结构内。

use crate::channel_state::{ChannelState, env_curve_frames};
use crate::sf_parser::KeyInfo;

/// SF2 `initialFilterFc` 全开值（绝对音分）换算成 Hz：8.176 × 2^(13500/1200)。
/// 音色库没有声明滤波器时，CC74 从这个"全开"位置往下降。
const FILTER_OPEN_HZ: f32 = 19_912.0;

/// CC74 亮度 → cutoff 的相对偏移（音分）。
///
/// GM2 / BASSMIDI 语义：双极线性，64 = 不改变，0 = -9600 cents（8 个八度），
/// 127 = +9450 cents。不是绝对频率，也不是"低于 64 才滤波"。
#[inline]
pub(crate) fn cutoff_cents_offset(cc: u8) -> f32 {
    (f32::from(cc) - 64.0) / 64.0 * 9600.0
}

/// CC71 共振 → Q 的相对偏移（dB）。同样以 64 为中心，±25 dB。
#[inline]
pub(crate) fn resonance_db_offset(cc: u8) -> f32 {
    (f32::from(cc) - 64.0) / 64.0 * 25.0
}

/// 按通道 CC71/74 计算一个 voice 的最终滤波器系数与实际 cutoff。
///
/// - `base_cutoff_hz == 0`：音色库没声明滤波器（SF2 默认 initialFilterFc
///   13500 cents ≈ 全开）。此时只有 **变暗**（CC74 < 64）才生成滤波器，
///   与 BASSMIDI "让滤波控制始终可听"的取向一致。
/// - `base_cutoff_hz > 0`：在库自身 cutoff 上叠加 CC74 的相对偏移。
///
/// 返回 `(系数, 实际 cutoff Hz)`；`cutoff == 0` 表示旁路。
pub(crate) fn filter_coeffs(
    base_cutoff_hz: f32,
    base_resonance: f32,
    filter_type: u32,
    ch: &ChannelState,
    sample_rate: u32,
) -> (Option<[f32; 5]>, f32) {
    let cents = cutoff_cents_offset(ch.filter_cutoff_cc);
    let db = resonance_db_offset(ch.filter_resonance_cc);
    // 库无滤波 + 不变暗：保持"无滤波"（最省 CPU，且与旧行为一致）。
    if base_cutoff_hz <= 0.0 && cents >= 0.0 {
        return (None, 0.0);
    }
    let base_hz = if base_cutoff_hz > 0.0 {
        base_cutoff_hz
    } else {
        FILTER_OPEN_HZ
    };
    let cutoff =
        (base_hz * 2.0f32.powf(cents / 1200.0)).clamp(1.0, sample_rate as f32 / 2.0 - 100.0);
    // Q 上限防止 CC71 拉满 + 库自身高 Q 时 biquad 数值不稳定（自激啸叫）。
    let q = (base_resonance * 10.0f32.powf(db / 20.0)).clamp(0.1, 20.0);
    let (b0, b1, b2, a1, a2) =
        crate::synth::biquad_coeffs(filter_type, cutoff, q, sample_rate as f32);
    (Some([b0, b1, b2, a1, a2]), cutoff)
}

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
    /// per-voice biquad 系数（按当前 CC71/74 计算；`None` = 旁路）。
    pub(crate) biquad: Option<[f32; 5]>,
    /// 实际 cutoff（0 = 旁路；shader 用它做 on/off 判定）。
    pub(crate) cutoff: f32,
    /// 音色库自身 cutoff/resonance/filter_type：CC71/74 变化时重算的基准。
    pub(crate) base_cutoff: f32,
    pub(crate) base_resonance: f32,
    pub(crate) filter_type: u32,
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
        let filter_type = crate::sf_parser::filter_type_code(info.filter_type);
        let (biquad, cutoff) =
            filter_coeffs(info.cutoff, info.resonance, filter_type, ch, sample_rate);
        Self {
            speed: info.speed_mult * ch.pitch_multiplier(),
            base_speed: info.speed_mult,
            base_gain: info.volume,
            pan_l: info.pan_l,
            pan_r: info.pan_r,
            biquad,
            cutoff,
            base_cutoff: info.cutoff,
            base_resonance: info.resonance,
            filter_type,
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
            interp: info.interp as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch_with(cutoff: u8, resonance: u8) -> ChannelState {
        let mut ch = ChannelState::new(48_000);
        ch.filter_cutoff_cc = cutoff;
        ch.filter_resonance_cc = resonance;
        ch
    }

    /// CC71/74 = 64 时必须与音色库原值完全一致（旧行为不回退）。
    #[test]
    fn neutral_cc_keeps_library_filter() {
        let ch = ch_with(64, 64);
        let (coeffs, cutoff) = filter_coeffs(8_000.0, 0.707, 0, &ch, 48_000);
        let reference = crate::synth::biquad_coeffs(0, 8_000.0, 0.707, 48_000.0);
        assert!((cutoff - 8_000.0).abs() < 1e-3);
        let c = coeffs.expect("有滤波器");
        assert!((c[0] - reference.0).abs() < 1e-6, "系数必须与原实现一致");
        assert!((c[3] - reference.3).abs() < 1e-6);
    }

    /// 音色库没有滤波器 + CC74 中性 = 旁路；CC74 < 64 才生成滤波器
    /// （BASSMIDI 的"让亮度控制始终可听"取向）。
    #[test]
    fn no_filter_library_responds_to_darkening_only() {
        let (coeffs, cutoff) = filter_coeffs(0.0, 0.707, 0, &ch_with(64, 64), 48_000);
        assert!(coeffs.is_none() && cutoff == 0.0);
        let (coeffs, cutoff) = filter_coeffs(0.0, 0.707, 0, &ch_with(48, 64), 48_000);
        assert!(coeffs.is_some());
        // CC74=48 → -2400 cents = /4。
        assert!((cutoff - 19_912.0 / 4.0).abs() < 2.0, "cutoff={cutoff}");
        // 变亮（>64）不改变"无滤波器"。
        let (coeffs, _) = filter_coeffs(0.0, 0.707, 0, &ch_with(127, 64), 48_000);
        assert!(coeffs.is_none());
    }

    /// CC74 是双极相对偏移：0 = -9600 cents，127 = +9450 cents。
    #[test]
    fn cc74_is_relative_bipolar() {
        let base = 1_000.0;
        assert!((cutoff_cents_offset(64)).abs() < 1e-6);
        let (_, dark) = filter_coeffs(base, 0.707, 0, &ch_with(0, 64), 48_000);
        let (_, bright) = filter_coeffs(base, 0.707, 0, &ch_with(127, 64), 48_000);
        assert!((dark - base / 256.0).abs() < 0.01, "dark={dark}");
        assert!(bright > base * 2.0, "bright={bright}");
    }

    /// CC71 只改 Q：cutoff 不变，系数变化。
    #[test]
    fn cc71_changes_q_only() {
        let (a, ca) = filter_coeffs(4_000.0, 0.707, 0, &ch_with(64, 64), 48_000);
        let (b, cb) = filter_coeffs(4_000.0, 0.707, 0, &ch_with(64, 127), 48_000);
        assert!((ca - cb).abs() < 1e-6, "CC71 不应改变 cutoff");
        assert!(
            (a.unwrap()[0] - b.unwrap()[0]).abs() > 1e-4,
            "Q 改变应改变系数"
        );
    }
}
