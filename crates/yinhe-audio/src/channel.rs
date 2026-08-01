use xsynth_core::channel::{ChannelAudioEvent, ChannelEvent, ControlEvent};
use xsynth_core::channel_group::{ChannelGroup, SynthEvent};

/// MIDI channel state for chase (restoring controller values after seek).
///
/// Default values match xsynth-core's internal defaults and GM spec:
///   - Volume 127, Pan 64, Expression 127
///   - Pitch bend sensitivity 2 semitones
///   - Sustain 0 (damper off), Cutoff 64 (disabled)
///   - Attack/Release are `None` in xsynth → only sent when MIDI file sets them
#[derive(Clone, Copy)]
pub(crate) struct ChannelState {
    pub(crate) bank_msb: u8,
    pub(crate) bank_lsb: u8,
    pub(crate) program: u8,
    pub(crate) volume: u8,
    pub(crate) pan: u8,
    pub(crate) expression: u8,
    pub(crate) sustain: u8,
    pub(crate) cutoff: u8,
    pub(crate) resonance: u8,
    pub(crate) attack: u8,
    pub(crate) release: u8,
    pub(crate) pitch_bend: f32,
    /// RPN MSB (CC 101). `None` if never selected.
    pub(crate) rpn_msb: Option<u8>,
    /// RPN LSB (CC 100). `None` if never selected.
    pub(crate) rpn_lsb: Option<u8>,
    /// Raw Data Entry MSB (CC 6).
    pub(crate) data_entry_msb: u8,
    /// Raw Data Entry LSB (CC 38).
    pub(crate) data_entry_lsb: u8,
    /// Resolved Pitch Bend Sensitivity in semitones (RPN 0). Default 2.0.
    pub(crate) pitch_bend_sensitivity: f32,
    /// Resolved Fine Tune in cents (RPN 1). Default 0.0.
    pub(crate) fine_tune: f32,
    /// Resolved Coarse Tune in semitones (RPN 2). Default 0.0.
    pub(crate) coarse_tune: f32,
    /// Tracks whether attack/release were explicitly set by MIDI events.
    /// If false, send_to skips CC 73/72 to avoid overriding xsynth's `None`.
    pub(crate) env_set: bool,
    /// Generic CC values for all 128 controllers.
    /// Used to chase CC numbers not covered by the specific fields above.
    pub(crate) cc_values: [u8; 128],
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            bank_msb: 0,
            bank_lsb: 0,
            program: 0,
            volume: 127,
            pan: 64,
            expression: 127,
            sustain: 0,
            cutoff: 64,
            resonance: 0,
            attack: 0,
            release: 0,
            pitch_bend: 0.0,
            rpn_msb: None,
            rpn_lsb: None,
            data_entry_msb: 2,
            data_entry_lsb: 0,
            pitch_bend_sensitivity: 2.0,
            fine_tune: 0.0,
            coarse_tune: 0.0,
            env_set: false,
            cc_values: [0; 128],
        }
    }
}

/// chase 应用时的跳过信息：seek 后已被实时事件 dispatch 过的控制器。
///
/// chase 是异步的，结果到达时渲染器可能已经 dispatch 了 seek 点之后的
/// 事件（包括 seek 点同 sample 的事件）。若整体覆盖，这些新值会被打回
/// seek 前的旧值（从中间小节开始播放时 PBS/PitchBend 被覆盖的根因），
/// 因此这些控制器必须跳过，只补齐尚未被实时事件覆盖的状态。
#[derive(Clone, Copy)]
pub(crate) struct ChaseSkip {
    /// 每 channel 128 bit：bit cc = 该 Raw CC 已被 dispatch。
    pub(crate) cc_mask: [u128; 256],
    /// PitchBendValue 已被 dispatch。
    pub(crate) pitch_bend: [bool; 256],
    /// PitchBendSensitivity（RPN 0）已被 dispatch。
    pub(crate) pbs: [bool; 256],
    /// FineTune（RPN 1）已被 dispatch。
    pub(crate) fine_tune: [bool; 256],
    /// CoarseTune（RPN 2）已被 dispatch。
    pub(crate) coarse_tune: [bool; 256],
    /// ProgramChange 已被 dispatch。
    pub(crate) program: [bool; 256],
}

// [T; 256] 不实现 Default（标准库只覆盖 N<=32），手写。
impl Default for ChaseSkip {
    fn default() -> Self {
        Self {
            cc_mask: [0; 256],
            pitch_bend: [false; 256],
            pbs: [false; 256],
            fine_tune: [false; 256],
            coarse_tune: [false; 256],
            program: [false; 256],
        }
    }
}

impl ChaseSkip {
    /// 标记一个已 dispatch 的 CC 事件，chase 应用时跳过对应控制器。
    pub(crate) fn mark(&mut self, event: &ChannelAudioEvent, channel: usize) {
        match event {
            ChannelAudioEvent::Control(ControlEvent::Raw(cc, _)) => {
                self.cc_mask[channel] |= 1u128 << cc;
            }
            ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_)) => {
                self.pitch_bend[channel] = true;
            }
            ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(_)) => {
                self.pbs[channel] = true;
            }
            ChannelAudioEvent::Control(ControlEvent::FineTune(_)) => self.fine_tune[channel] = true,
            ChannelAudioEvent::Control(ControlEvent::CoarseTune(_)) => {
                self.coarse_tune[channel] = true;
            }
            ChannelAudioEvent::ProgramChange(_) => self.program[channel] = true,
            _ => {}
        }
    }
}

impl ChannelState {
    pub(crate) fn apply(&mut self, event: &ChannelAudioEvent) {
        match event {
            ChannelAudioEvent::Control(ControlEvent::Raw(cc, val)) => {
                let cc_idx = *cc as usize;
                if cc_idx < 128 {
                    self.cc_values[cc_idx] = *val;
                }
                match cc {
                    0 => self.bank_msb = *val,
                    6 => {
                        self.data_entry_msb = *val;
                        self.resolve_rpn();
                    }
                    7 => self.volume = *val,
                    10 => self.pan = *val,
                    11 => self.expression = *val,
                    32 => self.bank_lsb = *val,
                    38 => {
                        self.data_entry_lsb = *val;
                        self.resolve_rpn();
                    }
                    64 => self.sustain = *val,
                    71 => self.resonance = *val,
                    72 => {
                        self.release = *val;
                        self.env_set = true;
                    }
                    73 => {
                        self.attack = *val;
                        self.env_set = true;
                    }
                    74 => self.cutoff = *val,
                    100 => self.rpn_lsb = Some(*val),
                    101 => self.rpn_msb = Some(*val),
                    _ => {}
                }
            }
            ChannelAudioEvent::Control(ControlEvent::PitchBendValue(v)) => self.pitch_bend = *v,
            ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(v)) => {
                self.pitch_bend_sensitivity = *v
            }
            ChannelAudioEvent::Control(ControlEvent::FineTune(v)) => self.fine_tune = *v,
            ChannelAudioEvent::Control(ControlEvent::CoarseTune(v)) => self.coarse_tune = *v,
            ChannelAudioEvent::ProgramChange(p) => self.program = *p,
            _ => {}
        }
    }

    fn resolve_rpn(&mut self) {
        let (Some(msb), Some(lsb)) = (self.rpn_msb, self.rpn_lsb) else {
            return;
        };
        if msb != 0 {
            return;
        }
        match lsb {
            0 => {
                self.pitch_bend_sensitivity =
                    self.data_entry_msb as f32 + self.data_entry_lsb as f32 / 100.0;
            }
            1 => {
                let val = ((self.data_entry_msb as u16) << 7) + self.data_entry_lsb as u16;
                self.fine_tune = (val as f32 - 8192.0) / 8192.0 * 100.0;
            }
            2 => {
                self.coarse_tune = self.data_entry_msb as f32 - 64.0;
            }
            _ => {}
        }
    }

    /// 计算 chase 应用时要发送的事件列表；`skip` 中标记的控制器跳过。
    /// 独立为纯函数，便于单元测试直接观察跳过行为。
    fn events_to_send(&self, channel: usize, skip: &ChaseSkip) -> Vec<ChannelAudioEvent> {
        let mut out = Vec::with_capacity(24);
        let mut push_raw = |cc: u8, val: u8| {
            if skip.cc_mask[channel] & (1u128 << cc) == 0 {
                out.push(ChannelAudioEvent::Control(ControlEvent::Raw(cc, val)));
            }
        };
        push_raw(0, self.bank_msb);
        push_raw(32, self.bank_lsb);
        push_raw(7, self.volume);
        push_raw(10, self.pan);
        push_raw(11, self.expression);
        push_raw(64, self.sustain);
        if self.env_set {
            push_raw(73, self.attack);
            push_raw(72, self.release);
        }
        push_raw(74, self.cutoff);
        push_raw(71, self.resonance);
        if !skip.program[channel] {
            out.push(ChannelAudioEvent::ProgramChange(self.program));
        }
        if !skip.pbs[channel] {
            out.push(ChannelAudioEvent::Control(
                ControlEvent::PitchBendSensitivity(self.pitch_bend_sensitivity),
            ));
        }
        if !skip.fine_tune[channel] {
            out.push(ChannelAudioEvent::Control(ControlEvent::FineTune(
                self.fine_tune,
            )));
        }
        if !skip.coarse_tune[channel] {
            out.push(ChannelAudioEvent::Control(ControlEvent::CoarseTune(
                self.coarse_tune,
            )));
        }
        if !skip.pitch_bend[channel] {
            out.push(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                self.pitch_bend,
            )));
        }

        // 通用 CC：发送未被特定字段覆盖、未被 dispatch 覆盖且非 0 的控制器。
        // CCs already sent: 0, 7, 10, 11, 32, 64, 71, 72, 73, 74.
        // RPN-related CCs (100, 101, 6, 38) are handled by the high-level
        // PitchBendSensitivity / FineTune / CoarseTune events above.
        const ALREADY_SENT: [u8; 12] = [0, 6, 7, 10, 11, 32, 38, 64, 71, 72, 73, 74];
        for cc in 0u8..128u8 {
            let val = self.cc_values[cc as usize];
            if val != 0
                && !ALREADY_SENT.contains(&cc)
                && cc != 100
                && cc != 101
                && skip.cc_mask[channel] & (1u128 << cc) == 0
            {
                out.push(ChannelAudioEvent::Control(ControlEvent::Raw(cc, val)));
            }
        }
        out
    }

    pub(crate) fn send_to(&self, ch: u32, cg: &mut ChannelGroup, skip: &ChaseSkip) {
        for event in self.events_to_send(ch as usize, skip) {
            cg.send_event(SynthEvent::Channel(ch, ChannelEvent::Audio(event)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_state_apply() {
        let mut state = ChannelState::default();
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)));
        assert_eq!(state.volume, 100);

        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(10, 64)));
        assert_eq!(state.pan, 64);

        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(101, 0)));
        assert_eq!(state.rpn_msb, Some(0));

        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(100, 0)));
        assert_eq!(state.rpn_lsb, Some(0));

        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(6, 12)));
        assert_eq!(state.data_entry_msb, 12);

        state.apply(&ChannelAudioEvent::ProgramChange(42));
        assert_eq!(state.program, 42);

        state.apply(&ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
            0.5,
        )));
        assert!((state.pitch_bend - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_channel_state_default() {
        let state = ChannelState::default();
        assert_eq!(state.volume, 127);
        assert_eq!(state.pan, 64);
        assert_eq!(state.expression, 127);
        assert_eq!(state.program, 0);
        assert_eq!(state.data_entry_msb, 2);
        assert!(!state.env_set);
        assert!((state.pitch_bend).abs() < f32::EPSILON);
        assert_eq!(state.rpn_msb, None);
        assert_eq!(state.rpn_lsb, None);
        assert!((state.pitch_bend_sensitivity - 2.0).abs() < f32::EPSILON);
        assert!((state.fine_tune - 0.0).abs() < f32::EPSILON);
        assert!((state.coarse_tune - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_rpn_pitch_bend_sensitivity() {
        let mut state = ChannelState::default();
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(101, 0)));
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(100, 0)));
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(6, 5)));
        assert!((state.pitch_bend_sensitivity - 5.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_rpn_pitch_bend_sensitivity_with_lsb() {
        let mut state = ChannelState::default();
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(101, 0)));
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(100, 0)));
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(6, 2)));
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(38, 50)));
        assert!((state.pitch_bend_sensitivity - 2.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_rpn_fine_tune() {
        let mut state = ChannelState::default();
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(101, 0)));
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(100, 1)));
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(6, 64)));
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(38, 0)));
        let expected = ((64u16 << 6) as f32 - 4096.0) / 4096.0 * 100.0;
        assert!((state.fine_tune - expected).abs() < 0.01);
    }

    #[test]
    fn test_rpn_coarse_tune() {
        let mut state = ChannelState::default();
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(101, 0)));
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(100, 2)));
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(6, 70)));
        assert!((state.coarse_tune - 6.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_chase_skip_mark() {
        let mut skip = ChaseSkip::default();
        skip.mark(&ChannelAudioEvent::Control(ControlEvent::Raw(64, 1)), 3);
        skip.mark(
            &ChannelAudioEvent::Control(ControlEvent::PitchBendValue(0.5)),
            3,
        );
        skip.mark(&ChannelAudioEvent::ProgramChange(5), 3);
        assert_ne!(skip.cc_mask[3] & (1u128 << 64), 0);
        assert_eq!(skip.cc_mask[3] & (1u128 << 7), 0);
        assert!(skip.pitch_bend[3]);
        assert!(skip.program[3]);
        assert!(!skip.pbs[3]);
        assert_eq!(skip.cc_mask[0], 0); // 其他 channel 不受影响
    }

    #[test]
    fn test_events_to_send_skips_dispatched() {
        let mut state = ChannelState {
            volume: 100,
            pitch_bend_sensitivity: 48.0,
            pitch_bend: 0.5,
            ..Default::default()
        };
        state.cc_values[91] = 80; // 通用 CC（reverb），不走专门字段

        let mut skip = ChaseSkip::default();
        skip.cc_mask[0] |= 1u128 << 7; // CC7 已 dispatch
        skip.cc_mask[0] |= 1u128 << 91; // CC91 已 dispatch
        skip.pbs[0] = true; // PitchBendSensitivity 已 dispatch
        skip.pitch_bend[0] = true; // PitchBendValue 已 dispatch

        let events = state.events_to_send(0, &skip);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ChannelAudioEvent::Control(ControlEvent::Raw(7, _))))
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ChannelAudioEvent::Control(ControlEvent::Raw(91, _))))
        );
        assert!(!events.iter().any(|e| matches!(
            e,
            ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(_))
        )));
        assert!(!events.iter().any(|e| matches!(
            e,
            ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_))
        )));
        // 未跳过的控制器仍在
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ChannelAudioEvent::Control(ControlEvent::Raw(10, _))))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ChannelAudioEvent::Control(ControlEvent::Raw(11, _))))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ChannelAudioEvent::ProgramChange(_)))
        );
        // 其他 channel 不受跳过影响
        let other = state.events_to_send(1, &skip);
        assert!(
            other
                .iter()
                .any(|e| matches!(e, ChannelAudioEvent::Control(ControlEvent::Raw(7, 100))))
        );
    }

    #[test]
    fn test_rpn_no_selection_no_resolve() {
        let mut state = ChannelState::default();
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(6, 10)));
        assert!((state.pitch_bend_sensitivity - 2.0).abs() < f32::EPSILON);
        assert!((state.fine_tune - 0.0).abs() < f32::EPSILON);
        assert!((state.coarse_tune - 0.0).abs() < f32::EPSILON);
    }
}
