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
/// SF2 规范允许的 cutoff 范围（绝对音分）：约 20.6 Hz .. 19.912 kHz。
const FILTER_MIN_CENTS: f32 = 1_500.0;
const FILTER_MAX_CENTS: f32 = 13_500.0;
/// BASSMIDI 在 CC71>64 或 CC74<64 时把音色库 cutoff 压到这个上限，
/// 保证滤波/共振一定可听（Ian 在 un4seen 论坛的原话；实测行为一致）。
const FILTER_AUDIBLE_CAP_HZ: f32 = 8_000.0;

fn hz_to_cents(hz: f32) -> f32 {
    1200.0 * (hz / 8.176).log2()
}

fn cents_to_hz(cents: f32) -> f32 {
    8.176 * 2.0f32.powf(cents / 1200.0)
}

/// 工程级声像偏移的左右增益（pan=0 时恰为 1.0，不改变原始声像）。
fn pan_offset_gains(pan: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    ((angle.cos() * 1.42).min(1.0), (angle.sin() * 1.42).min(1.0))
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

/// CC74 亮度 → cutoff 的相对偏移（音分）。
///
/// 实测 BASSMIDI：`(cc - 64) * 75`（±4800 cents），不是 SF2 2.04 默认
/// modulator 的 ±9600。旧实现按 ±9600 走，扫频比 BASSMIDI 陡一倍。
#[inline]
pub(crate) fn cutoff_cents_offset(cc: u8) -> f32 {
    (f32::from(cc) - 64.0) * 75.0
}

/// CC71 共振 → Q 的相对偏移（dB）。
///
/// BASSMIDI 对齐 SF2.04 默认 modulator 的 250 cB 双极量程：
/// `(cc - 64) / 64 * 25 dB`（±25 dB），实测峰值与之一致。
/// 最终 Q 不允许低于音色库原始值（CC71<64 在无共振音色上完全无效）。
#[inline]
pub(crate) fn resonance_db_offset(cc: u8) -> f32 {
    (f32::from(cc) - 64.0) / 64.0 * 25.0
}

/// 按通道 CC71/74 计算一个 voice 的最终滤波器系数与实际 cutoff。
///
/// 语义对齐 BASSMIDI（实测）：
/// - 64 = 中性；CC71/74 都只做**相对偏移**。
/// - CC71>64 或 CC74<64 时，先把音色库 cutoff 限制到 8 kHz 以内，
///   保证滤波器始终可听。
/// - cutoff 最终 clamp 到 SF2 范围 [1500, 13500] 绝对音分。
/// - CC71 的负偏移不能把 Q 压到音色库原始值以下（以 0 dB 为下限）。
///
/// 返回 `(系数, 实际 cutoff Hz)`；`cutoff == 0` 表示旁路。
pub(crate) fn filter_coeffs(
    base_cutoff_hz: f32,
    base_resonance: f32,
    filter_type: u32,
    ch: &ChannelState,
    sample_rate: u32,
) -> (Option<[f32; 5]>, f32) {
    let cents = cutoff_cents_offset(ch.filter_cutoff_cc) + ch.synth.brightness_cents;
    let db = resonance_db_offset(ch.filter_resonance_cc) + ch.synth.resonance_db;
    let wants_darken = cents < 0.0;
    let wants_resonance = db > 0.0;
    // 中性 CC：完全按音色库原值（不做 cents 往返，保证与旧行为逐值一致）。
    if cents == 0.0 && db == 0.0 {
        if base_cutoff_hz <= 0.0 {
            return (None, 0.0);
        }
        let (b0, b1, b2, a1, a2) = crate::synth::biquad_coeffs(
            filter_type,
            base_cutoff_hz,
            base_resonance,
            sample_rate as f32,
        );
        return (Some([b0, b1, b2, a1, a2]), base_cutoff_hz);
    }
    // 库无滤波 + 两个 CC 都没有"需要可听"的意图：保持旁路。
    if base_cutoff_hz <= 0.0 && !wants_darken && !wants_resonance {
        return (None, 0.0);
    }
    let mut base_hz = if base_cutoff_hz > 0.0 {
        base_cutoff_hz
    } else {
        FILTER_OPEN_HZ
    };
    if wants_darken || wants_resonance {
        base_hz = base_hz.min(FILTER_AUDIBLE_CAP_HZ);
    }
    let nyquist = sample_rate as f32 / 2.0;
    let cutoff = if cents == 0.0 {
        base_hz
    } else {
        let base_cents = hz_to_cents(base_hz);
        let cutoff_cents = (base_cents + cents).clamp(FILTER_MIN_CENTS, FILTER_MAX_CENTS);
        cents_to_hz(cutoff_cents)
    }
    .clamp(1.0, (nyquist - 100.0).max(1.0));
    // CC71 负偏移只能把 Q 降到音色库原始值，不能更低（BASSMIDI 实测：
    // 无共振音色上 CC71<64 完全无效；有共振音色降到 Q≈1 后停止）。
    let q_base = base_resonance.max(1e-3);
    let q = (q_base * 10.0f32.powf(db / 20.0)).max(q_base.min(1.0));
    // 上限防止极端参数让 biquad 数值不稳定（自激啸叫）；
    // 上限取 40：再高只会让 127 档的峰值更窄、频带均值反而更低。
    let q = q.clamp(0.1, 40.0);
    let (b0, b1, b2, a1, a2) =
        crate::synth::biquad_coeffs(filter_type, cutoff, q, sample_rate as f32);
    (Some([b0, b1, b2, a1, a2]), cutoff)
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
        let synth = ch.synth;
        let attack_scale = synth.attack_scale.clamp(0.1, 10.0);
        let release_scale = synth.release_scale.clamp(0.1, 10.0);
        let orig_attack_frames = info.ampeg_attack * sr * attack_scale;
        let orig_release_frames = info.ampeg_release * sr * release_scale;
        let (attack_frames, release_frames) =
            env_frames_for(ch, orig_attack_frames, orig_release_frames, sample_rate);
        let filter_type = crate::sf_parser::filter_type_code(info.filter_type);
        let (biquad, cutoff) =
            filter_coeffs(info.cutoff, info.resonance, filter_type, ch, sample_rate);
        let tune_mult = 2.0f32.powf(synth.tune_cents / 1200.0);
        let (pan_l, pan_r) = pan_offset_gains(synth.pan);
        Self {
            speed: info.speed_mult * ch.pitch_multiplier() * tune_mult,
            base_speed: info.speed_mult * tune_mult,
            base_gain: info.volume * 10.0f32.powf(synth.volume_db / 20.0),
            pan_l: info.pan_l * pan_l,
            pan_r: info.pan_r * pan_r,
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
        // CC74=48 → -1200 cents；无滤波库先压到 8 kHz → 4 kHz。
        assert!((cutoff - 4_000.0).abs() < 5.0, "cutoff={cutoff}");
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
        // 1 kHz + (-4800 cents) = 62.5 Hz。
        assert!((dark - base / 16.0).abs() < 0.05, "dark={dark}");
        assert!(bright > base * 2.0, "bright={bright}");
    }

    /// CC71<64 不能把 Q 压到音色库原始值以下（BASSMIDI 实测）。
    #[test]
    fn cc71_negative_never_lowers_library_q() {
        let neutral = filter_coeffs(4_000.0, 1.0, 0, &ch_with(64, 64), 48_000);
        let lowered = filter_coeffs(4_000.0, 1.0, 0, &ch_with(64, 0), 48_000);
        assert_eq!(neutral.0, lowered.0, "无共振音色上 CC71<64 必须完全无效");
        assert!((neutral.1 - lowered.1).abs() < 1e-6);
    }

    /// 音色库无滤波 + CC71>64：BASSMIDI 会启用一个 8 kHz 的共振滤波。
    #[test]
    fn cc71_alone_engages_audible_filter_at_8k() {
        let (coeffs, cutoff) = filter_coeffs(0.0, 1.0, 0, &ch_with(64, 96), 48_000);
        assert!(coeffs.is_some());
        assert!((cutoff - 8_000.0).abs() < 1e-3);
    }

    /// CC74<64 时音色库 cutoff 先压到 8 kHz（否则高截止音色听不出变化）。
    #[test]
    fn darken_caps_library_cutoff_at_8k() {
        let (_, cutoff) = filter_coeffs(19_912.0, 1.0, 0, &ch_with(48, 64), 48_000);
        assert!((cutoff - 4_000.0).abs() < 1e-3, "cutoff={cutoff}");
        // 同时 CC74 不能把 cutoff 压到 SF2 下限（1500 cents ≈ 19.4 Hz）以下。
        let (_, cutoff) = filter_coeffs(262.0, 1.0, 0, &ch_with(0, 64), 48_000);
        assert!(cutoff > 19.0 && cutoff < 20.0, "cutoff={cutoff}");
    }

    /// 工程级参数：亮度是相对 cutoff 偏移；中性时与库值一致。
    #[test]
    fn synth_brightness_offsets_cutoff() {
        let mut ch = ch_with(64, 64);
        ch.synth.brightness_cents = 1200.0;
        let (_, cutoff) = filter_coeffs(1_000.0, 1.0, 0, &ch, 48_000);
        assert!((cutoff - 2_000.0).abs() < 0.5, "cutoff={cutoff}");
    }

    /// 工程级声像：0 = 中性（左右都不衰减），两端各自压到另一侧。
    #[test]
    fn pan_offset_is_unity_at_center() {
        let (l, r) = pan_offset_gains(0.0);
        assert!((l - 1.0).abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
        let (l, r) = pan_offset_gains(-1.0);
        assert!(l > 0.99 && r < 0.01, "l={l} r={r}");
        let (l, r) = pan_offset_gains(1.0);
        assert!(r > 0.99 && l < 0.01, "l={l} r={r}");
    }

    /// 工程级参数：音量/音高/包络倍率进入 per-voice 展开。
    #[test]
    fn synth_params_apply_to_voice() {
        let info = KeyInfo::default();
        let mut ch = ChannelState::new(48_000);
        let neutral = VoiceParams::from_key_info(&info, &ch, 48_000);

        ch.synth.volume_db = 6.0;
        ch.synth.tune_cents = 1200.0;
        ch.synth.attack_scale = 2.0;
        let p = VoiceParams::from_key_info(&info, &ch, 48_000);

        let gain_ratio = p.base_gain / neutral.base_gain;
        assert!((gain_ratio - 10f32.powf(6.0 / 20.0)).abs() < 1e-3);
        let speed_ratio = p.base_speed / neutral.base_speed;
        assert!((speed_ratio - 2.0).abs() < 1e-4);
        assert!((p.attack_frames / neutral.attack_frames - 2.0).abs() < 1e-4);
        assert!((p.orig_attack_frames / neutral.orig_attack_frames - 2.0).abs() < 1e-4);
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
