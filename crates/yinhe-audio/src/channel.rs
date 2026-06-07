use xsynth_core::channel::{ChannelAudioEvent, ChannelEvent, ControlEvent};
use xsynth_core::channel_group::{ChannelGroup, SynthEvent};

/// MIDI channel state for chase (restoring controller values after seek).
///
/// Default values match xsynth-core's internal defaults and GM spec:
///   - Volume 127, Pan 64, Expression 127
///   - Pitch bend sensitivity 2 semitones (RPN 0 + DataEntry MSB = 2)
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
    pub(crate) rpn_msb: u8,
    pub(crate) rpn_lsb: u8,
    pub(crate) data_entry_msb: u8,
    pub(crate) data_entry_lsb: u8,
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
            rpn_msb: 0,
            rpn_lsb: 0,
            data_entry_msb: 2,
            data_entry_lsb: 0,
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
                    6 => self.data_entry_msb = *val,
                    7 => self.volume = *val,
                    10 => self.pan = *val,
                    11 => self.expression = *val,
                    32 => self.bank_lsb = *val,
                    38 => self.data_entry_lsb = *val,
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
                    100 => self.rpn_lsb = *val,
                    101 => self.rpn_msb = *val,
                    _ => {}
                }
            }
            ChannelAudioEvent::Control(ControlEvent::PitchBendValue(v)) => self.pitch_bend = *v,
            ChannelAudioEvent::ProgramChange(p) => self.program = *p,
            _ => {}
        }
    }

    pub(crate) fn send_to(&self, ch: u32, cg: &mut ChannelGroup) {
        let mut send = |event: ChannelAudioEvent| {
            cg.send_event(SynthEvent::Channel(ch, ChannelEvent::Audio(event)));
        };
        // NOTE: RPN 相关的 CC (101/100 RPN选择, 6/38 DataEntry) 不发。
        // MIDI 文件自身包含 RPN 事件时会自然处理；不需要 chase 额外注入。
        // 见 /Users/jieneng/Documents/GitHub/midirenderer/nezha/crates/nezha-xsynth/src/render.rs
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
        send(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
            self.pitch_bend,
        )));

        // Send generic CC values for controllers not covered by specific fields above.
        // CCs already sent: 0, 7, 10, 11, 32, 64, 71, 72, 73, 74.
        // Also skip RPN-related CCs (100, 101, 6, 38) per the comment above.
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
        assert_eq!(state.rpn_msb, 0);

        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(100, 0)));
        assert_eq!(state.rpn_lsb, 0);

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
    }
}
