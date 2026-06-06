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
/// Returns the MIDI port number used by this track.
pub(crate) fn parse_track(
    track: &midly::Track,
    segments: &[crate::TempoSegment],
    ticks_per_beat: u32,
    track_idx: u16,
    key_notes: &mut [Vec<Note>; 128],
    global_end_tick: &mut u64,
    control_events: &mut Vec<MidiControlEvent>,
) -> u8 {
    let mut active_notes: Vec<ActiveNote> = Vec::new();
    let mut current_tick: u32 = 0;
    let mut seg_idx: usize = 0;
    let mut current_port: u8 = 0;

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
            midly::TrackEventKind::Midi { channel, message } => {
                let ch = channel.as_int();
                let global_ch = current_port * 16 + ch;
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
                            channel: global_ch,
                            controller: controller.as_int(),
                            value: value.as_int(),
                            track: track_idx,
                        });
                    }
                    midly::MidiMessage::ProgramChange { program } => {
                        control_events.push(MidiControlEvent::ProgramChange {
                            tick: current_tick,
                            channel: global_ch,
                            program: program.as_int(),
                            track: track_idx,
                        });
                    }
                    midly::MidiMessage::PitchBend { bend } => {
                        control_events.push(MidiControlEvent::PitchBend {
                            tick: current_tick,
                            channel: global_ch,
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
    current_port
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
            key: n.key,
            velocity: n.velocity,
            channel: n.channel,
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
        assert_eq!(note.key, 60);
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
}
