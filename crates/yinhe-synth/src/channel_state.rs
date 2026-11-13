//! 音源层通道状态机（CC/RPN/弯音/鼓组）与 chase 跳过信息。
//!
//! CPU（`cpu_synth`）与 GPU（`gpu_synth`）两条渲染路径共用的纯状态逻辑。

use crate::gpu_synth::ControlEvent;

/// 合成器通道数上限（dense 通道槽位；两条渲染路径一致）。
///
/// 256 = MIDI 源通道全集（16 port × 16 通道）。实际启用的数量由
/// `ChannelLayout` 压缩后的 dense 通道数决定；这里只是数组容量上限。
pub const MAX_CHANNELS: usize = 256;

/// `process_control` 的处理结果（CPU/GPU 共用语义）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ControlAction {
    /// 延音踏板松开（CC64 抬起或 CC121 重置）：需要释放被踏板保持的 voice。
    pub damper_released: bool,
    /// CC120 All Sound Off：立即结束该通道全部 voice（1ms 淡出）。
    pub all_sound_off: bool,
    /// CC123 All Notes Off：释放该通道全部未踏板保持的 voice。
    pub all_notes_off: bool,
    /// CC71/74 变化：需要按新值重算该通道活跃 voice 的滤波器系数。
    pub filter_changed: bool,
}

/// 单通道 MIDI 控制状态（仅音源层参数；音量/声像在 yinhe-dsp 通道处理段）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct ChannelState {
    pub(crate) damper: bool,                // CC64 >= 64
    pub(crate) pitch_bend: f32,             // -1..1
    pub(crate) pitch_bend_sensitivity: f32, // 半音（RPN0 = msb + lsb/100），默认 2
    pub(crate) pbs_msb: u8,                 // RPN0 data msb（CC6）
    pub(crate) pbs_lsb: u8,                 // RPN0 data lsb（CC38）
    pub(crate) fine_tune: f32,              // 音分（RPN1）
    pub(crate) fine_tune_msb: u8,           // RPN1 data msb（CC6）
    pub(crate) fine_tune_lsb: u8,           // RPN1 data lsb（CC38）
    pub(crate) coarse_tune: f32,            // 半音（RPN2）
    pub(crate) program: u8,
    /// 音色库 bank MSB（CC0）：note_on 时与 program 一起选择音色库条目。
    /// 鼓组通道固定 128（`PercussionMode`），CC0 不能改。
    pub(crate) bank: u8,
    /// 音色库 bank LSB（CC32）。与 MSB 组合成 14-bit bank 优先匹配；
    /// 库里没有组合 bank 时回退到 MSB（GS/GM 的常见语义）。
    pub(crate) bank_lsb: u8,
    /// CC74 亮度（0..127，64 = 不改变音色库自身 cutoff）。
    pub(crate) filter_cutoff_cc: u8,
    /// CC71 共振（0..127，64 = 不改变音色库自身 Q）。
    pub(crate) filter_resonance_cc: u8,
    /// 滤波器 CC 已变化、活跃 voice 尚未重算系数（块内首个段边界消费）。
    pub(crate) filter_dirty: bool,
    // RPN 选择器状态（CC100/101）
    rpn_msb: i8,
    rpn_lsb: i8,
    /// 渐变长度基准（CC79 重置时需要重建 ValueLerp）
    sample_rate: u32,
    /// CC73 attack 时长倍率（u8，None = 用 region 原始值）
    pub(crate) env_attack: Option<u8>,
    /// CC72 release 时长倍率（u8，None = 用 region 原始值）
    pub(crate) env_release: Option<u8>,
    /// 全部 128 个 CC 的当前值。SFZ 门控（`xfin_*` / `on_loccN`）与
    /// `<control> set_ccN=v` 默认值都读写这里。
    pub(crate) cc: [u8; 128],
    /// keyswitch 当前值（SFZ `sw_last`；0xFF = 未设置）。
    pub(crate) sw_last: u8,
}

impl ChannelState {
    pub(crate) fn new(sample_rate: u32) -> Self {
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
            bank_lsb: 0,
            filter_cutoff_cc: 64,
            filter_resonance_cc: 64,
            filter_dirty: false,
            rpn_msb: -1,
            rpn_lsb: -1,
            sample_rate,
            env_attack: None,
            env_release: None,
            cc: [0; 128],
            sw_last: 0xFF,
        }
    }

    /// 弯音倍率：2^((bend×sensitivity + coarse + fine/100) / 12)（与 xsynth 一致）。
    pub(crate) fn pitch_multiplier(&self) -> f32 {
        let combined = self.pitch_bend * self.pitch_bend_sensitivity
            + self.coarse_tune
            + self.fine_tune / 100.0;
        2.0f32.powf(combined / 12.0)
    }

    /// 处理一个控制事件（音源层语义）。
    /// 返回本次事件触发的通道级动作（踏板松开 / All Sound Off / All Notes Off /
    /// 滤波器参数变化），由调用方决定如何处理活跃 voice。
    pub(crate) fn process_control(&mut self, event: ControlEvent) -> ControlAction {
        let mut action = ControlAction::default();
        match event {
            ControlEvent::Raw(controller, value) => {
                // 整数量语义的 CC（bank/RPN 选择与数据/包络倍率）取整；damper 用阈值。
                let vi = value.round() as u8;
                // 全部 CC 原值存档：SFZ 的 `xfin_*` / `on_loccN` 门控与
                // `<control> set_ccN` 默认值都从这里取。
                if (controller as usize) < 128 {
                    self.cc[controller as usize] = vi;
                }
                match controller {
                    0x00 => {
                        // Bank select MSB：鼓组通道（bank==128）锁定不变（xsynth set_bank）
                        if self.bank != 128 {
                            self.bank = vi;
                        }
                    }
                    0x20 => self.bank_lsb = vi,
                    0x64 => self.rpn_lsb = vi as i8,
                    0x65 => self.rpn_msb = vi as i8,
                    0x06 | 0x26 => {
                        if self.rpn_msb == 0 {
                            match self.rpn_lsb {
                                0 => {
                                    // Pitch bend sensitivity（RPN0 = msb + lsb/100）
                                    if controller == 0x06 {
                                        self.pbs_msb = vi;
                                    } else {
                                        self.pbs_lsb = vi;
                                    }
                                    self.pitch_bend_sensitivity =
                                        self.pbs_msb as f32 + self.pbs_lsb as f32 / 100.0;
                                }
                                1 => {
                                    // Fine tune（RPN1，14-bit：msb<<6 + lsb）
                                    if controller == 0x06 {
                                        self.fine_tune_msb = vi;
                                    } else {
                                        self.fine_tune_lsb = vi;
                                    }
                                    let val: u16 = ((self.fine_tune_msb as u16) << 6)
                                        + self.fine_tune_lsb as u16;
                                    self.fine_tune = (val as f32 - 4096.0) / 4096.0 * 100.0;
                                }
                                2 if controller == 0x06 => {
                                    // Coarse tune（RPN2）
                                    self.coarse_tune = vi as f32 - 64.0;
                                }
                                _ => {}
                            }
                        }
                    }
                    0x48 => self.env_release = Some(vi),
                    0x49 => self.env_attack = Some(vi),
                    // CC71 共振 / CC74 亮度：GM2/BASSMIDI 语义，64 = 中性。
                    // 实际作用在 note_on / 段边界的 per-voice 滤波器上
                    // （见 `voice_params::filter_coeffs`），不是通道总线滤波。
                    0x47 => {
                        if self.filter_resonance_cc != vi {
                            self.filter_resonance_cc = vi;
                            self.filter_dirty = true;
                            action.filter_changed = true;
                        }
                    }
                    0x4A => {
                        if self.filter_cutoff_cc != vi {
                            self.filter_cutoff_cc = vi;
                            self.filter_dirty = true;
                            action.filter_changed = true;
                        }
                    }
                    0x40 => {
                        let damper = value >= 64.0;
                        let released = self.damper && !damper;
                        self.damper = damper;
                        action.damper_released = released;
                    }
                    // CC121 Reset All Controllers。
                    // 标准语义：控制器回默认值，但**不改变** program/bank。
                    // GM2 默认 CC71/74 = 64（中性），由 `ChannelState::new` 保证；
                    // 重置后活跃 voice 需要按新值重算滤波器（filter_dirty）。
                    0x79 => {
                        let bank = self.bank;
                        let bank_lsb = self.bank_lsb;
                        let program = self.program;
                        *self = ChannelState::new(self.sample_rate);
                        self.bank = bank;
                        self.bank_lsb = bank_lsb;
                        self.program = program;
                        self.filter_dirty = true;
                        action.damper_released = true;
                        action.filter_changed = true;
                    }
                    // CC120 All Sound Off：立即结束该通道全部 voice。
                    0x78 => action.all_sound_off = true,
                    // CC123 All Notes Off：释放非踏板保持的 voice（held 等踏板松开）。
                    0x7B => action.all_notes_off = true,
                    _ => {}
                }
            }
            ControlEvent::PitchBend(value) => self.pitch_bend = value,
            ControlEvent::PitchBendSensitivity(value) => self.pitch_bend_sensitivity = value,
            ControlEvent::FineTune(value) => self.fine_tune = value,
            ControlEvent::CoarseTune(value) => self.coarse_tune = value,
            ControlEvent::Rpn { parameter, value } => self.apply_rpn(parameter, value),
            // NRPN：协议已是一等 u16 事件，当前无标准语义（留待扩展参数）。
            ControlEvent::Nrpn { .. } => {}
            ControlEvent::ProgramChange(value) => self.program = value,
            ControlEvent::PercussionMode(set) => self.bank = if set { 128 } else { 0 },
        }
        action
    }

    /// 加载音色库的 `<control> set_ccN=v`：写入通道初始 CC（含 bank LSB 与
    /// 滤波器 CC 的语义字段）。
    pub(crate) fn set_cc_default(&mut self, cc: u8, value: u8) {
        if (cc as usize) < 128 {
            self.cc[cc as usize] = value;
        }
        match cc {
            0x20 => self.bank_lsb = value,
            0x47 => self.filter_resonance_cc = value,
            0x4A => self.filter_cutoff_cc = value,
            _ => {}
        }
    }

    /// 原生 RPN（归一化 f32 0..=1）→ 内部参数。0/1/2 是 MIDI 标准：
    /// 0 = 弯音灵敏度（半音）、1 = 微调（±100 音分）、2 = 粗调（±64 半音）。
    /// 其余号是扩展位，在各自分支里实现（全 f32，无量化）。
    fn apply_rpn(&mut self, parameter: u16, value: f32) {
        let v = value.clamp(0.0, 1.0);
        match parameter {
            0 => self.pitch_bend_sensitivity = v * 127.0,
            1 => self.fine_tune = (v - 0.5) * 200.0,
            2 => self.coarse_tune = v * 127.0 - 64.0,
            _ => {}
        }
    }
}

/// chase 应用时的跳过信息（与 yinhe-audio `crate::channel::ChaseSkip` 同语义，
/// 按 dense 通道索引）：seek 后已被实时事件处理过的控制器，异步 chase
/// 到达时跳过，避免 seek 前旧值覆盖 seek 后已生效的新值。
#[derive(Clone, Copy, Debug)]
pub struct ChaseSkip {
    /// 每通道 128 bit：bit cc = 该 Raw CC 已被处理。
    pub cc_mask: [u128; MAX_CHANNELS],
    pub pitch_bend: [bool; MAX_CHANNELS],
    pub pbs: [bool; MAX_CHANNELS],
    pub fine_tune: [bool; MAX_CHANNELS],
    pub coarse_tune: [bool; MAX_CHANNELS],
    pub program: [bool; MAX_CHANNELS],
}

// [T; 256] 不实现 Default（标准库只覆盖 N<=32），手写。
impl Default for ChaseSkip {
    fn default() -> Self {
        Self {
            cc_mask: [0; MAX_CHANNELS],
            pitch_bend: [false; MAX_CHANNELS],
            pbs: [false; MAX_CHANNELS],
            fine_tune: [false; MAX_CHANNELS],
            coarse_tune: [false; MAX_CHANNELS],
            program: [false; MAX_CHANNELS],
        }
    }
}

/// dense 通道号 → 槽位索引；>= MAX_CHANNELS 返回 None（两个后端共用容量上限）。
pub fn dense_channel(channel: usize) -> Option<usize> {
    (channel < MAX_CHANNELS).then_some(channel)
}

/// 扫描一段事件，得出"哪些通道/控制类型已被 chase 覆盖"的位掩码
/// （CPU/GPU 共用；此前两份逐字重复）。
pub fn chase_skip(events: &[crate::SynthEvent]) -> ChaseSkip {
    let mut skip = ChaseSkip::default();
    for ev in events {
        let crate::SynthEvent::Control { channel, event, .. } = ev else {
            continue;
        };
        let Some(ch) = dense_channel(*channel as usize) else {
            continue;
        };
        match event {
            crate::ControlEvent::Raw(cc, _) => skip.cc_mask[ch] |= 1u128 << cc,
            crate::ControlEvent::PitchBend(_) => skip.pitch_bend[ch] = true,
            crate::ControlEvent::PitchBendSensitivity(_) => skip.pbs[ch] = true,
            crate::ControlEvent::FineTune(_) => skip.fine_tune[ch] = true,
            crate::ControlEvent::CoarseTune(_) => skip.coarse_tune[ch] = true,
            // 原生 RPN 0/1/2 与三个高层参数同语义（chase 跳过粒度一致）。
            crate::ControlEvent::Rpn { parameter, .. } => match parameter {
                0 => skip.pbs[ch] = true,
                1 => skip.fine_tune[ch] = true,
                2 => skip.coarse_tune[ch] = true,
                _ => {}
            },
            crate::ControlEvent::Nrpn { .. } => {}
            crate::ControlEvent::ProgramChange(_) => skip.program[ch] = true,
            crate::ControlEvent::PercussionMode(_) => {}
        }
    }
    skip
}

/// xsynth `calculate_curve`：CC72/73 值缩放 region 原始时长（秒）。
/// v<=64: (v/64)^5 × dur；v>64: dur + ((v-64)/64)^3 × 15
/// release 有 0.02s 下限；attack 无下限。返回帧数。
pub(crate) fn env_curve_frames(
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
pub(crate) fn is_env_effect_cc(event: &ControlEvent) -> bool {
    matches!(event, ControlEvent::Raw(0x48 | 0x49, _))
        || matches!(event, &ControlEvent::Raw(0x79, v) if v < 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 通道状态机（音源层）：CC64 damper 阈值、RPN、CC121 重置。
    /// 通道音量/声像在 yinhe-dsp 处理段，CC71/74 在这里（per-voice 滤波）。
    #[test]
    fn channel_cc_semantics() {
        let mut ch = ChannelState::new(44100);

        // CC64：<64 关，>=64 开；松开返回 true
        assert!(
            !ch.process_control(ControlEvent::Raw(64, 63.0))
                .damper_released
        );
        assert!(!ch.damper);
        assert!(
            !ch.process_control(ControlEvent::Raw(64, 127.0))
                .damper_released
        );
        assert!(ch.damper);
        assert!(
            ch.process_control(ControlEvent::Raw(64, 0.0))
                .damper_released
        );
        assert!(!ch.damper);

        // RPN0 弯音灵敏度（默认 2.0）：CC101/100 选择 RPN0，CC6 设 msb，CC38 设 lsb
        let mut ch = ChannelState::new(44100);
        ch.process_control(ControlEvent::Raw(0x65, 0.0)); // RPN msb
        ch.process_control(ControlEvent::Raw(0x64, 0.0)); // RPN lsb = 0
        ch.process_control(ControlEvent::Raw(0x06, 5.0));
        assert_eq!(ch.pitch_bend_sensitivity, 5.0);
        ch.process_control(ControlEvent::Raw(0x26, 50.0));
        assert_eq!(ch.pitch_bend_sensitivity, 5.5);

        // RPN1 微调（14-bit：msb<<6 + lsb）
        ch.process_control(ControlEvent::Raw(0x64, 1.0));
        ch.process_control(ControlEvent::Raw(0x06, 64.0));
        ch.process_control(ControlEvent::Raw(0x26, 0.0));
        assert_eq!(ch.fine_tune, (4096.0 - 4096.0) / 4096.0 * 100.0); // 中心
        ch.process_control(ControlEvent::Raw(0x06, 65.0));
        ch.process_control(ControlEvent::Raw(0x26, 0.0));
        assert!((ch.fine_tune - (4160.0 - 4096.0) / 4096.0 * 100.0).abs() < 1e-3);

        // RPN2 粗调：CC6 设值 - 64
        ch.process_control(ControlEvent::Raw(0x64, 2.0));
        ch.process_control(ControlEvent::Raw(0x06, 70.0));
        assert_eq!(ch.coarse_tune, 6.0);

        // 弯音：bend × 灵敏度（5.5）+ 粗调 6 + 微调 1.5625 音分
        ch.process_control(ControlEvent::PitchBend(0.5));
        assert!(
            (ch.pitch_multiplier() - 2.0f32.powf((2.75 + 6.0 + 1.5625 / 100.0) / 12.0)).abs()
                < 1e-5
        );

        // CC79 重置全部控制器
        assert!(
            ch.process_control(ControlEvent::Raw(0x79, 0.0))
                .damper_released
        );
        assert_eq!(ch.pitch_bend_sensitivity, 2.0);
        assert_eq!(ch.coarse_tune, 0.0);
    }

    /// 原生 RPN 事件（归一化 f32）与标准 MIDI 语义一致：
    /// 0 = 弯音灵敏度、1 = 微调（±100 音分）、2 = 粗调（±64 半音）。全 f32 无量化。
    #[test]
    fn native_rpn_matches_standard_midi_semantics() {
        let mut ch = ChannelState::new(44100);

        // RPN0：5.5 半音
        ch.process_control(ControlEvent::Rpn {
            parameter: 0,
            value: 5.5 / 127.0,
        });
        assert!((ch.pitch_bend_sensitivity - 5.5).abs() < 1e-5);

        // RPN1：0.5 = 中心（0 音分）；0.75 = +50 音分
        ch.process_control(ControlEvent::Rpn {
            parameter: 1,
            value: 0.5,
        });
        assert!(ch.fine_tune.abs() < 1e-5);
        ch.process_control(ControlEvent::Rpn {
            parameter: 1,
            value: 0.75,
        });
        assert!((ch.fine_tune - 50.0).abs() < 1e-3);

        // RPN2：0 半音对应 64/127 → +6 半音
        ch.process_control(ControlEvent::Rpn {
            parameter: 2,
            value: 70.0 / 127.0,
        });
        assert!((ch.coarse_tune - 6.0).abs() < 1e-5);

        // 扩展 RPN 号：当前无内部参数，安全忽略且不影响既有状态。
        ch.process_control(ControlEvent::Rpn {
            parameter: 77,
            value: 0.9,
        });
        assert!((ch.pitch_bend_sensitivity - 5.5).abs() < 1e-5);
        assert!((ch.coarse_tune - 6.0).abs() < 1e-5);

        // ChaseSkip：0/1/2 分别映射到 pbs/fine/coarse 跳过位。
        let events = vec![
            crate::SynthEvent::Control {
                sample: 0,
                channel: 0,
                event: ControlEvent::Rpn {
                    parameter: 0,
                    value: 0.0,
                },
            },
            crate::SynthEvent::Control {
                sample: 0,
                channel: 0,
                event: ControlEvent::Rpn {
                    parameter: 1,
                    value: 0.0,
                },
            },
            crate::SynthEvent::Control {
                sample: 0,
                channel: 0,
                event: ControlEvent::Rpn {
                    parameter: 2,
                    value: 0.0,
                },
            },
        ];
        let skip = chase_skip(&events);
        assert!(skip.pbs[0] && skip.fine_tune[0] && skip.coarse_tune[0]);
    }

    /// CC71/74 是 GM2/BASSMIDI 的双极相对偏移：64 = 中性，
    /// 两侧都要有值（旧实现 >=64 直接旁路）。
    #[test]
    fn filter_cc_is_bipolar_and_marks_dirty() {
        use crate::voice_params::{cutoff_cents_offset, resonance_db_offset};
        let mut ch = ChannelState::new(44100);
        assert_eq!(ch.filter_cutoff_cc, 64);
        assert_eq!(ch.filter_resonance_cc, 64);
        assert_eq!(cutoff_cents_offset(64), 0.0);
        assert!((cutoff_cents_offset(0) + 4800.0).abs() < 1e-3);
        assert!((cutoff_cents_offset(127) - 4725.0).abs() < 1e-3);
        assert_eq!(resonance_db_offset(64), 0.0);
        assert!(resonance_db_offset(0) < 0.0);
        assert!(resonance_db_offset(127) > 0.0);

        let a = ch.process_control(ControlEvent::Raw(74, 0.0));
        assert!(a.filter_changed && ch.filter_dirty);
        assert_eq!(ch.filter_cutoff_cc, 0);
        ch.filter_dirty = false;
        let a = ch.process_control(ControlEvent::Raw(71, 127.0));
        assert!(a.filter_changed && ch.filter_dirty);
        assert_eq!(ch.filter_resonance_cc, 127);
        // 值不变不重复置脏。
        ch.filter_dirty = false;
        let a = ch.process_control(ControlEvent::Raw(71, 127.0));
        assert!(!a.filter_changed && !ch.filter_dirty);
    }

    /// CC120/123 产生通道动作；CC121 重置控制器但保留 program/bank/bank_lsb。
    #[test]
    fn channel_mode_cc_actions() {
        let mut ch = ChannelState::new(44100);
        ch.process_control(ControlEvent::ProgramChange(42));
        ch.process_control(ControlEvent::Raw(0, 8.0));
        ch.process_control(ControlEvent::Raw(32, 3.0));
        assert_eq!((ch.bank, ch.bank_lsb, ch.program), (8, 3, 42));

        assert!(
            ch.process_control(ControlEvent::Raw(0x78, 0.0))
                .all_sound_off
        );
        assert!(
            ch.process_control(ControlEvent::Raw(0x7B, 0.0))
                .all_notes_off
        );

        ch.process_control(ControlEvent::Raw(74, 0.0));
        let a = ch.process_control(ControlEvent::Raw(121, 0.0));
        assert!(a.damper_released && a.filter_changed);
        assert_eq!(ch.filter_cutoff_cc, 64);
        assert_eq!((ch.bank, ch.bank_lsb, ch.program), (8, 3, 42));
    }

    /// 乐器库 `<control> set_ccN=v` 必须同步语义字段（否则门控与滤波器对不上）。
    #[test]
    fn cc_defaults_update_semantic_fields() {
        let mut ch = ChannelState::new(44100);
        ch.set_cc_default(74, 20);
        assert_eq!(ch.filter_cutoff_cc, 20);
        assert_eq!(ch.cc[74], 20);
        ch.set_cc_default(32, 5);
        assert_eq!(ch.bank_lsb, 5);
    }
}
