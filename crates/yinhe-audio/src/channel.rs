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
            ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(v)) => self.pitch_bend_sensitivity = *v,
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
                let val =
                    ((self.data_entry_msb as u16) << 7) + self.data_entry_lsb as u16;
                self.fine_tune = (val as f32 - 8192.0) / 8192.0 * 100.0;
            }
            2 => {
                self.coarse_tune = self.data_entry_msb as f32 - 64.0;
            }
            _ => {}
        }
    }

    pub(crate) fn send_to(&self, ch: u32, cg: &mut ChannelGroup) {
        let mut send = |event: ChannelAudioEvent| {
            cg.send_event(SynthEvent::Channel(ch, ChannelEvent::Audio(event)));
        };
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            0,
            self.bank_msb,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            32,
            self.bank_lsb,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            7,
            self.volume,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(10, self.pan)));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            11,
            self.expression,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            64,
            self.sustain,
        )));
        if self.env_set {
            send(ChannelAudioEvent::Control(ControlEvent::Raw(
                73,
                self.attack,
            )));
            send(ChannelAudioEvent::Control(ControlEvent::Raw(
                72,
                self.release,
            )));
        }
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            74,
            self.cutoff,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            71,
            self.resonance,
        )));
        send(ChannelAudioEvent::ProgramChange(self.program));

        send(ChannelAudioEvent::Control(
            ControlEvent::PitchBendSensitivity(self.pitch_bend_sensitivity),
        ));
        send(ChannelAudioEvent::Control(ControlEvent::FineTune(
            self.fine_tune,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::CoarseTune(
            self.coarse_tune,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
            self.pitch_bend,
        )));

        // Send generic CC values for controllers not covered by specific fields above.
        // CCs already sent: 0, 7, 10, 11, 32, 64, 71, 72, 73, 74.
        // RPN-related CCs (100, 101, 6, 38) are handled by the high-level
        // PitchBendSensitivity / FineTune / CoarseTune events above.
        const ALREADY_SENT: [u8; 12] = [0, 6, 7, 10, 11, 32, 38, 64, 71, 72, 73, 74];
        for cc in 0u8..128u8 {
            let val = self.cc_values[cc as usize];
            if val != 0 && !ALREADY_SENT.contains(&cc) && cc != 100 && cc != 101 {
                send(ChannelAudioEvent::Control(ControlEvent::Raw(cc, val)));
            }
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
        assert_eq!(state.env_set, false);
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
    fn test_rpn_no_selection_no_resolve() {
        let mut state = ChannelState::default();
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(6, 10)));
        assert!((state.pitch_bend_sensitivity - 2.0).abs() < f32::EPSILON);
        assert!((state.fine_tune - 0.0).abs() < f32::EPSILON);
        assert!((state.coarse_tune - 0.0).abs() < f32::EPSILON);
    }
}
