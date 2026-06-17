use yinhe_midi::{MidiControlEvent, MidiFile};

use crate::model::YinModel;

/// Convert a `YinModel` back to a `MidiFile` for MIDI export.
///
/// This reverses the `midi_to_yinmodel` conversion:
/// - TrackData.notes → key_notes[128]
/// - TrackData.cc → control_events (ControlChange)
/// - TrackData.pitch_bend → control_events (PitchBend)
/// - TrackData.program_change → control_events (ProgramChange)
/// - TrackData.rpn → control_events (CC 101/100/6/38 sequences)
/// - ConductorData → tempo_segments, time_sig_events
pub fn yinmodel_to_midi(model: &YinModel) -> MidiFile {
    let num_tracks = model.tracks.len();
    let mut midi = MidiFile::default();

    // ── Conductor events ──
    midi.ticks_per_beat = model.meta.ppq;
    midi.tempo_segments = model
        .conductor
        .tempo
        .iter()
        .map(|e| yinhe_midi::TempoSegment {
            start_tick: e.tick,
            start_time: 0.0, // Will be recomputed
            micros_per_quarter: yinhe_midi::mpq_from_bpm(e.bpm as f32),
        })
        .collect();
    yinhe_midi::recompute_tempo_start_times(&mut midi.tempo_segments, midi.ticks_per_beat);

    midi.time_sig_events = model
        .conductor
        .time_sig
        .iter()
        .map(|e| yinhe_types::TimeSigEvent {
            tick: e.tick,
            numerator: e.numerator,
            denominator: e.denominator,
        })
        .collect();
    if let Some(ts) = midi.time_sig_events.first() {
        midi.time_sig_numerator = ts.numerator;
        midi.time_sig_denominator = ts.denominator;
    }

    // ── Track metadata ──
    midi.track_ports = model.tracks.iter().map(|t| t.port).collect();
    midi.track_channels = model.tracks.iter().map(|t| t.channel).collect();
    midi.track_names = model.tracks.iter().map(|t| t.name.clone()).collect();
    midi.track_channel_prefixes = vec![None; num_tracks];

    // ── Notes: distribute from tracks back to key_notes[128] ──
    midi.key_notes = core::array::from_fn(|_| Vec::new());
    for (track_idx, track) in model.tracks.iter().enumerate() {
        for note in &track.notes {
            let key = note.key as usize;
            if key < 128 {
                midi.key_notes[key].push(yinhe_types::Note {
                    start_tick: note.tick,
                    end_tick: note.tick + note.duration,
                    velocity: note.velocity,
                    track: track_idx as u16,
                });
            }
        }
    }
    // Sort by start_tick within each key
    for key_notes in &mut midi.key_notes {
        key_notes.sort_by_key(|n| n.start_tick);
    }

    // ── Control events ──
    midi.control_events = Vec::new();

    for (track_idx, track) in model.tracks.iter().enumerate() {
        let track_u16 = track_idx as u16;

        // CC events
        for (&controller, events) in &track.cc {
            for e in events {
                midi.control_events
                    .push(MidiControlEvent::ControlChange {
                        tick: e.tick,
                        controller,
                        value: e.value,
                        track: track_u16,
                    });
            }
        }

        // Pitch bend
        for e in &track.pitch_bend {
            midi.control_events
                .push(MidiControlEvent::PitchBend {
                    tick: e.tick,
                    value: e.value,
                    track: track_u16,
                });
        }

        // Program change
        for e in &track.program_change {
            midi.control_events
                .push(MidiControlEvent::ProgramChange {
                    tick: e.tick,
                    program: e.program,
                    track: track_u16,
                });
        }

        // RPN → CC 101/100/6 sequence
        for (&rpn_num, events) in &track.rpn {
            let (msb, lsb) = match rpn_num {
                0 => (0, 0),
                1 => (1, 0),
                2 => (2, 0),
                _ => continue, // Unsupported RPN, skip
            };
            for e in events {
                // CC 101 = RPN MSB
                midi.control_events
                    .push(MidiControlEvent::ControlChange {
                        tick: e.tick,
                        controller: 101,
                        value: msb,
                        track: track_u16,
                    });
                // CC 100 = RPN LSB
                midi.control_events
                    .push(MidiControlEvent::ControlChange {
                        tick: e.tick,
                        controller: 100,
                        value: lsb,
                        track: track_u16,
                    });
                // CC 6 = Data Entry MSB
                midi.control_events
                    .push(MidiControlEvent::ControlChange {
                        tick: e.tick,
                        controller: 6,
                        value: (e.value & 0x7F) as u8,
                        track: track_u16,
                    });
                // CC 38 = Data Entry LSB
                midi.control_events
                    .push(MidiControlEvent::ControlChange {
                        tick: e.tick,
                        controller: 38,
                        value: ((e.value >> 7) & 0x7F) as u8,
                        track: track_u16,
                    });
            }
        }
    }

    // Sort control events by tick
    midi.control_events
        .sort_by_key(|e| match e {
            MidiControlEvent::ControlChange { tick, .. } => *tick,
            MidiControlEvent::ProgramChange { tick, .. } => *tick,
            MidiControlEvent::PitchBend { tick, .. } => *tick,
        });

    // ── Derived fields ──
    midi.note_count = midi.key_notes.iter().map(|kn| kn.len() as u64).sum();
    midi.tick_length = midi
        .key_notes
        .iter()
        .flatten()
        .map(|n| n.end_tick as u64)
        .max()
        .unwrap_or(0);

    midi
}
