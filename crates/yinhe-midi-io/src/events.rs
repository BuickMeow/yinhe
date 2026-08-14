//! MIDI 消息解析（纯函数，无硬件依赖，可单测）。
//!
//! 要求输入是完整消息（midir 的 CoreMIDI/winmm 后端均保证），
//! 不支持 running status 压缩格式。

/// 解析后的 MIDI 事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidiEvent {
    /// Note On（力度为 0 的 Note On 已被折叠为 NoteOff）。
    NoteOn {
        channel: u8,
        key: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        key: u8,
    },
    ControlChange {
        channel: u8,
        controller: u8,
        value: u8,
    },
    ProgramChange {
        channel: u8,
        program: u8,
    },
    ChannelPressure {
        channel: u8,
        value: u8,
    },
    PolyphonicPressure {
        channel: u8,
        key: u8,
        value: u8,
    },
    /// 弯音，14 bit 合并值（0..=16383，8192 为中心）。
    PitchBend {
        channel: u8,
        value: u16,
    },
    /// 系统消息（F0-F7，不含通道号）。
    System {
        status: u8,
        data: Vec<u8>,
    },
}

/// 解析一条原始 MIDI 消息。数据为空或长度不足时返回 `None`。
pub fn parse_event(data: &[u8]) -> Option<MidiEvent> {
    let status = *data.first()?;
    let kind = status & 0xF0;
    let channel = status & 0x0F;
    match kind {
        0x80 => Some(MidiEvent::NoteOff {
            channel,
            key: data.get(1)? & 0x7F,
        }),
        0x90 => {
            let velocity = *data.get(2)? & 0x7F;
            let key = data.get(1)? & 0x7F;
            if velocity == 0 {
                Some(MidiEvent::NoteOff { channel, key })
            } else {
                Some(MidiEvent::NoteOn {
                    channel,
                    key,
                    velocity,
                })
            }
        }
        0xA0 => Some(MidiEvent::PolyphonicPressure {
            channel,
            key: data.get(1)? & 0x7F,
            value: data.get(2)? & 0x7F,
        }),
        0xB0 => Some(MidiEvent::ControlChange {
            channel,
            controller: data.get(1)? & 0x7F,
            value: data.get(2)? & 0x7F,
        }),
        0xC0 => Some(MidiEvent::ProgramChange {
            channel,
            program: data.get(1)? & 0x7F,
        }),
        0xD0 => Some(MidiEvent::ChannelPressure {
            channel,
            value: data.get(1)? & 0x7F,
        }),
        0xE0 => {
            let lsb = *data.get(1)? & 0x7F;
            let msb = *data.get(2)? & 0x7F;
            Some(MidiEvent::PitchBend {
                channel,
                value: ((msb as u16) << 7) | lsb as u16,
            })
        }
        _ => Some(MidiEvent::System {
            status,
            data: data.to_vec(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_note_on() {
        let e = parse_event(&[0x90, 60, 100]).unwrap();
        assert_eq!(
            e,
            MidiEvent::NoteOn {
                channel: 0,
                key: 60,
                velocity: 100
            }
        );
    }

    #[test]
    fn parse_note_on_other_channel() {
        let e = parse_event(&[0x93, 72, 64]).unwrap();
        assert_eq!(
            e,
            MidiEvent::NoteOn {
                channel: 3,
                key: 72,
                velocity: 64
            }
        );
    }

    #[test]
    fn note_on_velocity_zero_is_note_off() {
        let e = parse_event(&[0x90, 60, 0]).unwrap();
        assert_eq!(
            e,
            MidiEvent::NoteOff {
                channel: 0,
                key: 60
            }
        );
    }

    #[test]
    fn parse_note_off() {
        let e = parse_event(&[0x80, 61, 64]).unwrap();
        assert_eq!(
            e,
            MidiEvent::NoteOff {
                channel: 0,
                key: 61
            }
        );
    }

    #[test]
    fn parse_control_change() {
        let e = parse_event(&[0xB5, 7, 127]).unwrap();
        assert_eq!(
            e,
            MidiEvent::ControlChange {
                channel: 5,
                controller: 7,
                value: 127
            }
        );
    }

    #[test]
    fn parse_program_change() {
        let e = parse_event(&[0xC1, 42]).unwrap();
        assert_eq!(
            e,
            MidiEvent::ProgramChange {
                channel: 1,
                program: 42
            }
        );
    }

    #[test]
    fn parse_channel_pressure() {
        let e = parse_event(&[0xD2, 90]).unwrap();
        assert_eq!(
            e,
            MidiEvent::ChannelPressure {
                channel: 2,
                value: 90
            }
        );
    }

    #[test]
    fn parse_polyphonic_pressure() {
        let e = parse_event(&[0xA4, 65, 80]).unwrap();
        assert_eq!(
            e,
            MidiEvent::PolyphonicPressure {
                channel: 4,
                key: 65,
                value: 80
            }
        );
    }

    #[test]
    fn parse_pitch_bend_merges_lsb_msb() {
        // 14-bit: lsb=0x01, msb=0x40 → 0x2001
        let e = parse_event(&[0xE0, 0x01, 0x40]).unwrap();
        assert_eq!(
            e,
            MidiEvent::PitchBend {
                channel: 0,
                value: 0x2001
            }
        );
    }

    #[test]
    fn parse_system_message() {
        let e = parse_event(&[0xF1, 5]).unwrap();
        match e {
            MidiEvent::System { status, data } => {
                assert_eq!(status, 0xF1);
                assert_eq!(data, vec![0xF1, 5]);
            }
            other => panic!("expected System, got {other:?}"),
        }
    }

    #[test]
    fn parse_empty_returns_none() {
        assert_eq!(parse_event(&[]), None);
    }

    #[test]
    fn parse_short_note_on_returns_none() {
        assert_eq!(parse_event(&[0x90, 60]), None);
    }

    #[test]
    fn parse_short_pitch_bend_returns_none() {
        assert_eq!(parse_event(&[0xE0, 5]), None);
    }

    #[test]
    fn parse_high_bits_masked() {
        // 数据字节中的高位（状态位）应被掩掉（7 bit 数据）
        let e = parse_event(&[0x90, 0xE0, 0x80]).unwrap();
        // key=0x60, velocity=0 → 折叠为 NoteOff
        assert_eq!(
            e,
            MidiEvent::NoteOff {
                channel: 0,
                key: 0x60
            }
        );
    }
}
