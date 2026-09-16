//! 音源层通道状态机（CC/RPN/弯音/鼓组）与 chase 跳过信息。

use super::{ControlEvent, MAX_CHANNELS};

/// 单通道 MIDI 控制状态（仅音源层参数；音量/声像/滤波已迁至 yinhe-dsp）。
#[derive(Clone, Copy, Debug)]
pub(super) struct ChannelState {
    pub(super) damper: bool,                // CC64 >= 64
    pub(super) pitch_bend: f32,             // -1..1
    pub(super) pitch_bend_sensitivity: f32, // 半音（RPN0 = msb + lsb/100），默认 2
    pub(super) pbs_msb: u8,                 // RPN0 data msb（CC6）
    pub(super) pbs_lsb: u8,                 // RPN0 data lsb（CC38）
    pub(super) fine_tune: f32,              // 音分（RPN1）
    pub(super) fine_tune_msb: u8,           // RPN1 data msb（CC6）
    pub(super) fine_tune_lsb: u8,           // RPN1 data lsb（CC38）
    pub(super) coarse_tune: f32,            // 半音（RPN2）
    pub(super) program: u8,
    /// 音色库 bank（xsynth `ProgramDescriptor.bank` 语义）：CC0 设置（鼓组 128 锁定），
    /// `PercussionMode` 直接置 128/0。note_on 时与 program 一起选择音色库条目。
    pub(super) bank: u8,
    // RPN 选择器状态（CC100/101）
    rpn_msb: i8,
    rpn_lsb: i8,
    /// 渐变长度基准（CC79 重置时需要重建 ValueLerp）
    sample_rate: u32,
    /// CC73 attack 时长倍率（u8，None = 用 region 原始值）
    pub(super) env_attack: Option<u8>,
    /// CC72 release 时长倍率（u8，None = 用 region 原始值）
    pub(super) env_release: Option<u8>,
}

impl ChannelState {
    pub(super) fn new(sample_rate: u32) -> Self {
        Self {
            damper: false,
            pitch_bend: 0.0,
            pitch_bend_sensitivity: 2.0,
            pbs_msb: 2,
            pbs_lsb: 0,
            fine_tune: 0.0,
            fine_tune_msb: 0,
            fine_tune_lsb: 0,
            coarse_tune: 0.0,
            program: 0,
            bank: 0,
            rpn_msb: -1,
            rpn_lsb: -1,
            sample_rate,
            env_attack: None,
            env_release: None,
        }
    }

    /// 弯音倍率：2^((bend×sensitivity + coarse + fine/100) / 12)（与 xsynth 一致）。
    pub(super) fn pitch_multiplier(&self) -> f32 {
        let combined = self.pitch_bend * self.pitch_bend_sensitivity
            + self.coarse_tune
            + self.fine_tune / 100.0;
        2.0f32.powf(combined / 12.0)
    }

    /// 处理一个控制事件（语义对齐 xsynth `process_control_event`）。
    /// 返回是否触发了 damper 松开（需要释放 held voices）。
    pub(super) fn process_control(&mut self, event: ControlEvent) -> bool {
        match event {
            ControlEvent::Raw(controller, value) => match controller {
                0x00 => {
                    // Bank select MSB：鼓组通道（bank==128）锁定不变（xsynth set_bank）
                    if self.bank != 128 {
                        self.bank = value;
                    }
                }
                0x64 => self.rpn_lsb = value as i8,
                0x65 => self.rpn_msb = value as i8,
                0x06 | 0x26 => {
                    if self.rpn_msb == 0 {
                        match self.rpn_lsb {
                            0 => {
                                // Pitch bend sensitivity（RPN0 = msb + lsb/100）
                                if controller == 0x06 {
                                    self.pbs_msb = value;
                                } else {
                                    self.pbs_lsb = value;
                                }
                                self.pitch_bend_sensitivity =
                                    self.pbs_msb as f32 + self.pbs_lsb as f32 / 100.0;
                            }
                            1 => {
                                // Fine tune（RPN1，14-bit：msb<<6 + lsb）
                                if controller == 0x06 {
                                    self.fine_tune_msb = value;
                                } else {
                                    self.fine_tune_lsb = value;
                                }
                                let val: u16 =
                                    ((self.fine_tune_msb as u16) << 6) + self.fine_tune_lsb as u16;
                                self.fine_tune = (val as f32 - 4096.0) / 4096.0 * 100.0;
                            }
                            2 if controller == 0x06 => {
                                // Coarse tune（RPN2）
                                self.coarse_tune = value as f32 - 64.0;
                            }
                            _ => {}
                        }
                    }
                }
                0x48 => self.env_release = Some(value),
                0x49 => self.env_attack = Some(value),
                0x40 => {
                    let damper = value >= 64;
                    let released = self.damper && !damper;
                    self.damper = damper;
                    return released;
                }
                0x79 if value == 0 => {
                    // Reset All Controllers。
                    // xsynth 的 reset_control 不重置 program.bank，这里保留 bank。
                    let bank = self.bank;
                    *self = ChannelState::new(self.sample_rate);
                    self.bank = bank;
                    return true; // damper 松开语义
                }
                _ => {}
            },
            ControlEvent::PitchBend(value) => self.pitch_bend = value,
            ControlEvent::PitchBendSensitivity(value) => self.pitch_bend_sensitivity = value,
            ControlEvent::FineTune(value) => self.fine_tune = value,
            ControlEvent::CoarseTune(value) => self.coarse_tune = value,
            ControlEvent::ProgramChange(value) => self.program = value,
            ControlEvent::PercussionMode(set) => self.bank = if set { 128 } else { 0 },
        }
        false
    }
}

/// chase 应用时的跳过信息（与 yinhe-audio `crate::channel::ChaseSkip` 同语义，
/// 按 dense 通道 % 32 索引）：seek 后已被实时事件处理过的控制器，异步 chase
/// 到达时跳过，避免 seek 前旧值覆盖 seek 后已生效的新值。
#[derive(Clone, Copy, Debug, Default)]
pub struct ChaseSkip {
    /// 每通道 128 bit：bit cc = 该 Raw CC 已被处理。
    pub cc_mask: [u128; MAX_CHANNELS],
    pub pitch_bend: [bool; MAX_CHANNELS],
    pub pbs: [bool; MAX_CHANNELS],
    pub fine_tune: [bool; MAX_CHANNELS],
    pub coarse_tune: [bool; MAX_CHANNELS],
    pub program: [bool; MAX_CHANNELS],
}

/// xsynth `calculate_curve`：CC72/73 值缩放 region 原始时长（秒）。
/// v<=64: (v/64)^5 × dur；v>64: dur + ((v-64)/64)^3 × 15
/// release 有 0.02s 下限；attack 无下限。返回帧数。
pub(super) fn env_curve_frames(
    value: u8,
    orig_frames: f32,
    sample_rate: u32,
    is_release: bool,
) -> f32 {
    let dur = orig_frames / sample_rate as f32;
    let curve = if value <= 64 {
        (value as f32 / 64.0).powi(5) * dur
    } else {
        dur + ((value as f32 - 64.0) / 64.0).powi(3) * 15.0
    };
    let secs = if is_release { curve.max(0.02) } else { curve };
    secs * sample_rate as f32
}

/// CC72/73（包络时长）与 CC121（重置包络）需要重算活跃 voice 的包络时长。
pub(super) fn is_env_effect_cc(event: &ControlEvent) -> bool {
    matches!(
        event,
        ControlEvent::Raw(0x48 | 0x49, _) | ControlEvent::Raw(0x79, 0)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 通道状态机（音源层）：CC64 damper 阈值、RPN、CC79 重置。
    /// 通道音量/声像/滤波已迁至 yinhe-dsp，这里不再涉及。
    #[test]
    fn channel_cc_semantics() {
        let mut ch = ChannelState::new(44100);

        // CC64：<64 关，>=64 开；松开返回 true
        assert!(!ch.process_control(ControlEvent::Raw(64, 63)));
        assert!(!ch.damper);
        assert!(!ch.process_control(ControlEvent::Raw(64, 127)));
        assert!(ch.damper);
        assert!(ch.process_control(ControlEvent::Raw(64, 0)));
        assert!(!ch.damper);

        // RPN0 弯音灵敏度（默认 2.0）：CC101/100 选择 RPN0，CC6 设 msb，CC38 设 lsb
        let mut ch = ChannelState::new(44100);
        ch.process_control(ControlEvent::Raw(0x65, 0)); // RPN msb
        ch.process_control(ControlEvent::Raw(0x64, 0)); // RPN lsb = 0
        ch.process_control(ControlEvent::Raw(0x06, 5));
        assert_eq!(ch.pitch_bend_sensitivity, 5.0);
        ch.process_control(ControlEvent::Raw(0x26, 50));
        assert_eq!(ch.pitch_bend_sensitivity, 5.5);

        // RPN1 微调（14-bit：msb<<6 + lsb）
        ch.process_control(ControlEvent::Raw(0x64, 1));
        ch.process_control(ControlEvent::Raw(0x06, 64));
        ch.process_control(ControlEvent::Raw(0x26, 0));
        assert_eq!(ch.fine_tune, (4096.0 - 4096.0) / 4096.0 * 100.0); // 中心
        ch.process_control(ControlEvent::Raw(0x06, 65));
        ch.process_control(ControlEvent::Raw(0x26, 0));
        assert!((ch.fine_tune - (4160.0 - 4096.0) / 4096.0 * 100.0).abs() < 1e-3);

        // RPN2 粗调：CC6 设值 - 64
        ch.process_control(ControlEvent::Raw(0x64, 2));
        ch.process_control(ControlEvent::Raw(0x06, 70));
        assert_eq!(ch.coarse_tune, 6.0);

        // 弯音：bend × 灵敏度（5.5）+ 粗调 6 + 微调 1.5625 音分
        ch.process_control(ControlEvent::PitchBend(0.5));
        assert!(
            (ch.pitch_multiplier() - 2.0f32.powf((2.75 + 6.0 + 1.5625 / 100.0) / 12.0)).abs()
                < 1e-5
        );

        // CC79 重置全部控制器
        assert!(ch.process_control(ControlEvent::Raw(0x79, 0)));
        assert_eq!(ch.pitch_bend_sensitivity, 2.0);
        assert_eq!(ch.coarse_tune, 0.0);
    }
}
