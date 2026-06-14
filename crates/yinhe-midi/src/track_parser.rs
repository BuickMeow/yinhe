use crate::midi::{MidiControlEvent, Note};


/// A note currently being played (waiting for NoteOff).
#[derive(Clone, Copy, Debug)]
struct ActiveNote {
    key: u8,
    velocity: u8,
    channel: u8,
    start_tick: u32,
    track: u16,
}

/// Parse a single MIDI track, extracting notes and control events.
///
/// Returns `(MIDI port, channel prefix, global_channel)` — the channel
/// prefix is from `MetaMessage::MidiChannel` (0x20) and is `None` if not
/// present.  `global_channel` is the first MIDI channel seen in this track
/// (or `port * 16` if no events).
pub(crate) fn parse_track(
    track: &midly::Track,
    segments: &[crate::TempoSegment],
    ticks_per_beat: u32,
    track_idx: u16,
    key_notes: &mut [Vec<Note>; 128],
    global_end_tick: &mut u64,
    control_events: &mut Vec<MidiControlEvent>,
) -> (u8, Option<u8>, u8) {
    let mut active_notes: Vec<ActiveNote> = Vec::new();
    let mut current_tick: u32 = 0;
    let mut seg_idx: usize = 0;
    let mut current_port: u8 = 0;
    let mut channel_prefix: Option<u8> = None;
    let mut track_channel: Option<u8> = None;

    for event in track {
        let new_tick = current_tick + event.delta.as_int();
        let delta = new_tick - current_tick;

        if delta > 0 {
            seg_idx = advance_time_to_tick(
                current_tick,
                new_tick,
                seg_idx,
                segments,
                ticks_per_beat,
            );
            current_tick = new_tick;
        } else {
            current_tick = new_tick;
        }

        match event.kind {
            midly::TrackEventKind::Meta(midly::MetaMessage::MidiPort(port)) => {
                current_port = port.as_int();
            }
            midly::TrackEventKind::Meta(midly::MetaMessage::MidiChannel(ch)) => {
                channel_prefix = Some(ch.as_int());
            }
            midly::TrackEventKind::Midi { channel, message } => {
                let ch = channel.as_int();
                let global_ch = current_port * 16 + ch;
                track_channel.get_or_insert(global_ch);
                match message {
                    midly::MidiMessage::NoteOn { key, vel } => {
                        let k = key.as_int();
                        if vel.as_int() > 0 {
                            active_notes.push(ActiveNote {
                                key: k,
                                velocity: vel.as_int(),
                                channel: global_ch,
                                start_tick: current_tick,
                                track: track_idx,
                            });
                        } else {
                            resolve_note_off(
                                k,
                                global_ch,
                                current_tick,
                                &mut active_notes,
                                key_notes,
                                global_end_tick,
                            );
                        }
                    }
                    midly::MidiMessage::NoteOff { key, .. } => {
                        let k = key.as_int();
                        resolve_note_off(
                            k,
                            global_ch,
                            current_tick,
                            &mut active_notes,
                            key_notes,
                            global_end_tick,
                        );
                    }
                    midly::MidiMessage::Controller { controller, value } => {
                        control_events.push(MidiControlEvent::ControlChange {
                            tick: current_tick,
                            controller: controller.as_int(),
                            value: value.as_int(),
                            track: track_idx,
                        });
                    }
                    midly::MidiMessage::ProgramChange { program } => {
                        control_events.push(MidiControlEvent::ProgramChange {
                            tick: current_tick,
                            program: program.as_int(),
                            track: track_idx,
                        });
                    }
                    midly::MidiMessage::PitchBend { bend } => {
                        control_events.push(MidiControlEvent::PitchBend {
                            tick: current_tick,
                            value: bend.as_int(),
                            track: track_idx,
                        });
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    let track_ch = track_channel.unwrap_or(current_port * 16);
    (current_port, channel_prefix, track_ch)
}

/// Advance the tempo segment index from current_tick to target_tick.
/// Returns the new segment index; does not compute wall-clock time.
fn advance_time_to_tick(
    _current_tick: u32,
    target_tick: u32,
    mut seg_idx: usize,
    segments: &[crate::TempoSegment],
    _ticks_per_beat: u32,
) -> usize {
    while seg_idx + 1 < segments.len() && segments[seg_idx + 1].start_tick <= target_tick {
        seg_idx += 1;
    }
    seg_idx
}

/// Match a NoteOff (or NoteOn with velocity=0) to the most recent active NoteOn.
fn resolve_note_off(
    key: u8,
    channel: u8,
    end_tick: u32,
    active_notes: &mut Vec<ActiveNote>,
    key_notes: &mut [Vec<Note>; 128],
    global_end_tick: &mut u64,
) {
    if let Some(idx) = active_notes
        .iter()
        .rposition(|n| n.key == key && n.channel == channel)
    {
        let n = active_notes.swap_remove(idx);
        *global_end_tick = (*global_end_tick).max(end_tick as u64);
        key_notes[n.key as usize].push(Note {
            start_tick: n.start_tick,
            end_tick,
            velocity: n.velocity,
            track: n.track,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TempoSegment;
    use crate::time::DEFAULT_MPQ;

    #[test]
    fn test_advance_time_single_segment() {
        let segments = vec![TempoSegment {
            start_tick: 0,
            start_time: 0.0,
            micros_per_quarter: DEFAULT_MPQ, // 120 BPM
        }];
        let idx = advance_time_to_tick(0, 480, 0, &segments, 480);
        assert_eq!(idx, 0);
    }

    #[test]
    fn test_advance_time_crosses_segment_boundary() {
        let segments = vec![
            TempoSegment {
                start_tick: 0,
                start_time: 0.0,
                micros_per_quarter: DEFAULT_MPQ, // 120 BPM
            },
            TempoSegment {
                start_tick: 480,
                start_time: 0.5,
                micros_per_quarter: 250_000, // 240 BPM
            },
        ];
        let idx = advance_time_to_tick(0, 960, 0, &segments, 480);
        assert_eq!(idx, 1);
    }

    #[test]
    fn test_resolve_note_off_matches_active_note() {
        let mut active = vec![ActiveNote {
            key: 60,
            velocity: 100,
            channel: 0,
            start_tick: 0,
            track: 0,
        }];
        let mut key_notes: [Vec<Note>; 128] = std::array::from_fn(|_| Vec::new());
        let mut global_end_tick: u64 = 0;

        resolve_note_off(60, 0, 960, &mut active, &mut key_notes, &mut global_end_tick);

        assert!(active.is_empty());
        assert_eq!(key_notes[60].len(), 1);
        let note = &key_notes[60][0];
        assert_eq!(note.start_tick, 0);
        assert_eq!(note.end_tick, 960);
        assert_eq!(note.velocity, 100);
        assert_eq!(global_end_tick, 960);
    }

    #[test]
    fn test_resolve_note_off_no_match() {
        let mut active: Vec<ActiveNote> = Vec::new();
        let mut key_notes: [Vec<Note>; 128] = std::array::from_fn(|_| Vec::new());
        let mut global_end_tick: u64 = 0;

        resolve_note_off(60, 0, 960, &mut active, &mut key_notes, &mut global_end_tick);

        assert!(active.is_empty());
        assert!(key_notes[60].is_empty());
        assert_eq!(global_end_tick, 0);
    }

    #[test]
    fn test_parse_track_note_on_off() {
        use midly::{TrackEvent, TrackEventKind};
        use midly::MidiMessage::NoteOn;
        use midly::MidiMessage::NoteOff;
        use midly::num::u7;

        let track = vec![
            TrackEvent {
                delta: 0.into(),
                kind: TrackEventKind::Midi {
                    channel: 0.into(),
                    message: NoteOn { key: u7::new(60), vel: u7::new(100) },
                },
            },
            TrackEvent {
                delta: 480.into(),
                kind: TrackEventKind::Midi {
                    channel: 0.into(),
                    message: NoteOff { key: u7::new(60), vel: u7::new(0) },
                },
            },
        ];

        let segments = vec![crate::TempoSegment {
            start_tick: 0,
            start_time: 0.0,
            micros_per_quarter: crate::time::DEFAULT_MPQ,
        }];

        let mut key_notes: [Vec<Note>; 128] = std::array::from_fn(|_| Vec::new());
        let mut global_end_tick: u64 = 0;
        let mut control_events: Vec<MidiControlEvent> = Vec::new();

        let (port, prefix, _track_ch) = parse_track(&track, &segments, 480, 0, &mut key_notes, &mut global_end_tick, &mut control_events);

        assert_eq!(port, 0);
        assert!(prefix.is_none());
        assert_eq!(key_notes[60].len(), 1);
        assert_eq!(key_notes[60][0].start_tick, 0);
        assert_eq!(key_notes[60][0].end_tick, 480);
        assert_eq!(key_notes[60][0].velocity, 100);
        assert_eq!(global_end_tick, 480);
    }

    #[test]
    fn test_parse_track_note_on_with_vel_zero() {
        use midly::{TrackEvent, TrackEventKind};
        use midly::MidiMessage::NoteOn;
        use midly::num::u7;

        let track = vec![
            TrackEvent {
                delta: 0.into(),
                kind: TrackEventKind::Midi {
                    channel: 0.into(),
                    message: NoteOn { key: u7::new(60), vel: u7::new(100) },
                },
            },
            TrackEvent {
                delta: 480.into(),
                kind: TrackEventKind::Midi {
                    channel: 0.into(),
                    message: NoteOn { key: u7::new(60), vel: u7::new(0) },
                },
            },
        ];

        let segments = vec![crate::TempoSegment {
            start_tick: 0,
            start_time: 0.0,
            micros_per_quarter: crate::time::DEFAULT_MPQ,
        }];

        let mut key_notes: [Vec<Note>; 128] = std::array::from_fn(|_| Vec::new());
        let mut global_end_tick: u64 = 0;
        let mut control_events: Vec<MidiControlEvent> = Vec::new();

        parse_track(&track, &segments, 480, 0, &mut key_notes, &mut global_end_tick, &mut control_events);

        assert_eq!(key_notes[60].len(), 1);
        assert_eq!(key_notes[60][0].end_tick, 480);
    }

    #[test]
    fn test_parse_track_control_events() {
        use midly::{TrackEvent, TrackEventKind};
        use midly::MidiMessage::{Controller, ProgramChange, PitchBend};
        use midly::num::u7;

        let track = vec![
            TrackEvent {
                delta: 0.into(),
                kind: TrackEventKind::Midi {
                    channel: 0.into(),
                    message: Controller { controller: u7::new(7), value: u7::new(100) },
                },
            },
            TrackEvent {
                delta: 100.into(),
                kind: TrackEventKind::Midi {
                    channel: 1.into(),
                    message: ProgramChange { program: u7::new(5) },
                },
            },
            TrackEvent {
                delta: 200.into(),
                kind: TrackEventKind::Midi {
                    channel: 0.into(),
                    message: PitchBend { bend: midly::PitchBend::from_int(2000) },
                },
            },
        ];

        let segments = vec![crate::TempoSegment {
            start_tick: 0,
            start_time: 0.0,
            micros_per_quarter: crate::time::DEFAULT_MPQ,
        }];

        let mut key_notes: [Vec<Note>; 128] = std::array::from_fn(|_| Vec::new());
        let mut global_end_tick: u64 = 0;
        let mut control_events: Vec<MidiControlEvent> = Vec::new();

        parse_track(&track, &segments, 480, 1, &mut key_notes, &mut global_end_tick, &mut control_events);

        assert_eq!(control_events.len(), 3);
        match &control_events[0] {
            MidiControlEvent::ControlChange { tick, controller, value, track } => {
                assert_eq!(*tick, 0);
                assert_eq!(*controller, 7);
                assert_eq!(*value, 100);
                assert_eq!(*track, 1);
            }
            _ => panic!("expected ControlChange"),
        }
        match &control_events[1] {
            MidiControlEvent::ProgramChange { tick, program, track } => {
                assert_eq!(*tick, 100);
                assert_eq!(*program, 5);
                assert_eq!(*track, 1);
            }
            _ => panic!("expected ProgramChange"),
        }
        match &control_events[2] {
            MidiControlEvent::PitchBend { tick, value, track } => {
                assert_eq!(*tick, 300);
                assert_eq!(*value, 2000);
                assert_eq!(*track, 1);
            }
            _ => panic!("expected PitchBend"),
        }
    }

    #[test]
    fn test_parse_track_midi_port_and_channel_prefix() {
        use midly::{TrackEvent, TrackEventKind};
        use midly::MetaMessage::{MidiPort, MidiChannel};
        use midly::num::u7;

        let track = vec![
            TrackEvent {
                delta: 0.into(),
                kind: TrackEventKind::Meta(MidiPort(u7::new(1))),
            },
            TrackEvent {
                delta: 0.into(),
                kind: TrackEventKind::Meta(MidiChannel(midly::num::u4::new(5))),
            },
        ];

        let segments = vec![crate::TempoSegment {
            start_tick: 0,
            start_time: 0.0,
            micros_per_quarter: crate::time::DEFAULT_MPQ,
        }];

        let mut key_notes: [Vec<Note>; 128] = std::array::from_fn(|_| Vec::new());
        let mut global_end_tick: u64 = 0;
        let mut control_events: Vec<MidiControlEvent> = Vec::new();

        let (port, prefix, _track_ch) = parse_track(&track, &segments, 480, 0, &mut key_notes, &mut global_end_tick, &mut control_events);

        assert_eq!(port, 1);
        assert_eq!(prefix, Some(5));
    }

    #[test]
    fn test_parse_track_multi_port_channel() {
        use midly::{TrackEvent, TrackEventKind};
        use midly::MidiMessage::NoteOn;
        use midly::MetaMessage::MidiPort;
        use midly::num::u7;

        let track = vec![
            TrackEvent {
                delta: 0.into(),
                kind: TrackEventKind::Meta(MidiPort(u7::new(2))),
            },
            TrackEvent {
                delta: 0.into(),
                kind: TrackEventKind::Midi {
                    channel: 3.into(),
                    message: NoteOn { key: u7::new(60), vel: u7::new(100) },
                },
            },
            TrackEvent {
                delta: 480.into(),
                kind: TrackEventKind::Midi {
                    channel: 3.into(),
                    message: NoteOn { key: u7::new(60), vel: u7::new(0) },
                },
            },
        ];

        let segments = vec![crate::TempoSegment {
            start_tick: 0,
            start_time: 0.0,
            micros_per_quarter: crate::time::DEFAULT_MPQ,
        }];

        let mut key_notes: [Vec<Note>; 128] = std::array::from_fn(|_| Vec::new());
        let mut global_end_tick: u64 = 0;
        let mut control_events: Vec<MidiControlEvent> = Vec::new();

        let (port, prefix, track_ch) = parse_track(&track, &segments, 480, 0, &mut key_notes, &mut global_end_tick, &mut control_events);

        assert_eq!(key_notes[60].len(), 1);
        // global_ch = port 2 * 16 + ch 3 = 35
        assert_eq!(track_ch, 35);
    }

    #[test]
    fn test_parse_track_overlapping_notes_same_key() {
        use midly::{TrackEvent, TrackEventKind};
        use midly::MidiMessage::NoteOn;
        use midly::num::u7;

        let track = vec![
            TrackEvent {
                delta: 0.into(),
                kind: TrackEventKind::Midi {
                    channel: 0.into(),
                    message: NoteOn { key: u7::new(60), vel: u7::new(100) },
                },
            },
            TrackEvent {
                delta: 240.into(),
                kind: TrackEventKind::Midi {
                    channel: 0.into(),
                    message: NoteOn { key: u7::new(60), vel: u7::new(80) },
                },
            },
            TrackEvent {
                delta: 240.into(),
                kind: TrackEventKind::Midi {
                    channel: 0.into(),
                    message: NoteOn { key: u7::new(60), vel: u7::new(0) },
                },
            },
            TrackEvent {
                delta: 240.into(),
                kind: TrackEventKind::Midi {
                    channel: 0.into(),
                    message: NoteOn { key: u7::new(60), vel: u7::new(0) },
                },
            },
        ];

        let segments = vec![crate::TempoSegment {
            start_tick: 0,
            start_time: 0.0,
            micros_per_quarter: crate::time::DEFAULT_MPQ,
        }];

        let mut key_notes: [Vec<Note>; 128] = std::array::from_fn(|_| Vec::new());
        let mut global_end_tick: u64 = 0;
        let mut control_events: Vec<MidiControlEvent> = Vec::new();

        parse_track(&track, &segments, 480, 0, &mut key_notes, &mut global_end_tick, &mut control_events);

        // LIFO matching: second NoteOn (vel=80) resolves first, then first (vel=100)
        assert_eq!(key_notes[60].len(), 2);
        assert_eq!(key_notes[60][0].start_tick, 240);
        assert_eq!(key_notes[60][0].end_tick, 480);
        assert_eq!(key_notes[60][0].velocity, 80);
        assert_eq!(key_notes[60][1].start_tick, 0);
        assert_eq!(key_notes[60][1].end_tick, 720);
        assert_eq!(key_notes[60][1].velocity, 100);
    }
}
