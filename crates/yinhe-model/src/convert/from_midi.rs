use std::collections::BTreeMap;
use uuid::Uuid;
use yinhe_midi::{MidiControlEvent, MidiFile};

use crate::events::*;
use crate::model::*;

/// Convert a `MidiFile` into a `YinModel`.
///
/// The conversion:
/// 1. Groups notes by track
/// 2. Groups CC events by track and controller number
/// 3. Parses CC 101/100/6/38 sequences into structured RPN events
/// 4. Extracts conductor events (tempo, time signature)
pub fn midi_to_yinmodel(midi: &MidiFile) -> YinModel {
    let num_tracks = midi.track_ports.len();

    // ── Conductor events ──
    let conductor = ConductorData {
        tempo: midi
            .tempo_segments
            .iter()
            .map(|s| TempoEvent {
                tick: s.start_tick,
                bpm: yinhe_midi::bpm_from_mpq(s.micros_per_quarter),
            })
            .collect(),
        time_sig: midi
            .time_sig_events
            .iter()
            .map(|e| TimeSigEvent {
                tick: e.tick,
                numerator: e.numerator,
                denominator: e.denominator,
            })
            .collect(),
    };

    // ── Build tracks ──
    let mut tracks: Vec<TrackData> = (0..num_tracks)
        .map(|i| TrackData {
            uuid: Uuid::new_v4().to_string(),
            name: midi
                .track_names
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("Track {}", i + 1)),
            port: midi.track_ports.get(i).copied().unwrap_or(0),
            channel: midi.track_channels.get(i).copied().unwrap_or(0),
            notes: Vec::new(),
            cc: BTreeMap::new(),
            pitch_bend: Vec::new(),
            program_change: Vec::new(),
            rpn: BTreeMap::new(),
        })
        .collect();

    // ── Distribute notes by track ──
    for (key_idx, key_notes) in midi.key_notes.iter().enumerate() {
        for note in key_notes {
            let track_idx = note.track as usize;
            if track_idx < num_tracks {
                tracks[track_idx].notes.push(NoteEvent {
                    tick: note.start_tick,
                    duration: note.end_tick.saturating_sub(note.start_tick),
                    key: key_idx as u8,
                    velocity: note.velocity,
                });
            }
        }
    }

    // Sort notes by tick within each track
    for track in &mut tracks {
        track.notes.sort_by_key(|n| n.tick);
    }

    // ── Distribute control events by track ──
    // First pass: collect raw CC events
    let mut cc_raw: Vec<(usize, u8, u32, u8)> = Vec::new(); // (track, controller, tick, value)
    for ev in &midi.control_events {
        match ev {
            MidiControlEvent::ControlChange {
                tick,
                controller,
                value,
                track,
            } => {
                cc_raw.push((*track as usize, *controller, *tick, *value));
            }
            MidiControlEvent::ProgramChange {
                tick,
                program,
                track,
            } => {
                let t = *track as usize;
                if t < num_tracks {
                    tracks[t].program_change.push(PcEvent {
                        tick: *tick,
                        program: *program,
                        bank_msb: 0xFF,
                        bank_lsb: 0xFF,
                    });
                }
            }
            MidiControlEvent::PitchBend {
                tick,
                value,
                track,
            } => {
                let t = *track as usize;
                if t < num_tracks {
                    tracks[t].pitch_bend.push(PitchBendEvent {
                        tick: *tick,
                        value: *value,
                    });
                }
            }
        }
    }

    // Second pass: parse RPN sequences from CC events
    // Group CC events by (track, tick) to detect RPN sequences
    let mut cc_by_tick: BTreeMap<(usize, u32), Vec<(u8, u8)>> = BTreeMap::new();
    for &(track, controller, tick, value) in &cc_raw {
        cc_by_tick
            .entry((track, tick))
            .or_default()
            .push((controller, value));
    }

    // Detect and consume RPN sequences
    let mut consumed_rpn: std::collections::HashSet<(usize, u32)> = std::collections::HashSet::new();
    for (&(track, tick), events) in &cc_by_tick {
        let msb = events.iter().find(|(c, _)| *c == 101).map(|(_, v)| *v);
        let lsb = events.iter().find(|(c, _)| *c == 100).map(|(_, v)| *v);
        let data_msb = events.iter().find(|(c, _)| *c == 6).map(|(_, v)| *v);

        if let (Some(msb), Some(lsb), Some(data_msb)) = (msb, lsb, data_msb) {
            if let Some(rpn_num) = rpn_number(msb, lsb) {
                let value = data_msb as u16;
                if track < num_tracks {
                    tracks[track]
                        .rpn
                        .entry(rpn_num)
                        .or_default()
                        .push(RpnEvent { tick, value });
                }
                consumed_rpn.insert((track, tick));
            }
        }
    }

    // Third pass: distribute non-RPN CC events
    for &(track, controller, tick, value) in &cc_raw {
        if consumed_rpn.contains(&(track, tick)) {
            // Check if this CC is part of an RPN sequence (101, 100, 6, 38)
            if matches!(controller, 101 | 100 | 6 | 38) {
                continue;
            }
        }
        if track < num_tracks {
            tracks[track]
                .cc
                .entry(controller)
                .or_default()
                .push(CcEvent { tick, value });
        }
    }

    // Sort events by tick within each track
    for track in &mut tracks {
        for cc_events in track.cc.values_mut() {
            cc_events.sort_by_key(|e| e.tick);
        }
        track.pitch_bend.sort_by_key(|e| e.tick);
        track.program_change.sort_by_key(|e| e.tick);
        for rpn_events in track.rpn.values_mut() {
            rpn_events.sort_by_key(|e| e.tick);
        }
    }

    // ── Project metadata ──
    let meta = ProjectMeta {
        name: String::new(),
        artist: String::new(),
        description: String::new(),
        ppq: midi.ticks_per_beat,
        compression_level: 0,
    };

    let mut model = YinModel {
        conductor,
        tracks,
        meta,
        key_index: KeyIndex::default(),
        key_notes_cache: (0..128).map(|_| Vec::new()).collect(),
        note_count: 0,
        tick_length: 0,
    };

    model.rebuild();
    model
}

/// Map RPN MSB/LSB to RPN number (only 0, 1, 2 are supported).
fn rpn_number(msb: u8, lsb: u8) -> Option<u8> {
    match (msb, lsb) {
        (0, 0) => Some(0), // Pitch Bend Sensitivity
        (1, 0) => Some(1), // Fine Tune
        (2, 0) => Some(2), // Coarse Tune
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_midi::MidiFile;

    fn make_test_midi() -> MidiFile {
        let mut m = MidiFile::default();
        m.ticks_per_beat = 480;
        m.track_ports = vec![0, 0];
        m.track_channels = vec![0, 1];
        m.track_names = vec!["Piano".into(), "Bass".into()];
        m.track_channel_prefixes = vec![None, None];

        // Add notes
        m.key_notes[60].push(yinhe_types::Note {
            start_tick: 0,
            end_tick: 480,
            velocity: 100,
            track: 0,
        });
        m.key_notes[60].push(yinhe_types::Note {
            start_tick: 480,
            end_tick: 960,
            velocity: 80,
            track: 0,
        });
        m.key_notes[48].push(yinhe_types::Note {
            start_tick: 0,
            end_tick: 960,
            velocity: 90,
            track: 1,
        });

        // Add CC events
        m.control_events
            .push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: 0,
                controller: 7,
                value: 100,
                track: 0,
            });
        m.control_events
            .push(yinhe_midi::MidiControlEvent::PitchBend {
                tick: 100,
                value: 0,
                track: 0,
            });

        // Add RPN sequence (CC 101=0, CC 100=0, CC 6=2) at tick 200
        m.control_events
            .push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: 200,
                controller: 101,
                value: 0,
                track: 0,
            });
        m.control_events
            .push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: 200,
                controller: 100,
                value: 0,
                track: 0,
            });
        m.control_events
            .push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: 200,
                controller: 6,
                value: 2,
                track: 0,
            });

        // Tempo and time sig
        m.tempo_segments = vec![yinhe_midi::TempoSegment {
            start_tick: 0,
            start_time: 0.0,
            micros_per_quarter: yinhe_midi::mpq_from_bpm(120.0),
        }];
        m.time_sig_events = vec![yinhe_types::TimeSigEvent {
            tick: 0,
            numerator: 4,
            denominator: 2,
        }];

        m.note_count = 3;
        m.tick_length = 960;
        m
    }

    #[test]
    fn midi_to_yinmodel_basic() {
        let midi = make_test_midi();
        let model = midi_to_yinmodel(&midi);

        assert_eq!(model.tracks.len(), 2);
        assert_eq!(model.meta.ppq, 480);

        // Track 0: Piano
        assert_eq!(model.tracks[0].name, "Piano");
        assert_eq!(model.tracks[0].port, 0);
        assert_eq!(model.tracks[0].channel, 0);
        assert_eq!(model.tracks[0].notes.len(), 2);
        assert_eq!(model.tracks[0].notes[0].key, 60);
        assert_eq!(model.tracks[0].notes[0].tick, 0);
        assert_eq!(model.tracks[0].notes[0].duration, 480);
        assert_eq!(model.tracks[0].notes[0].velocity, 100);

        // Track 1: Bass
        assert_eq!(model.tracks[1].name, "Bass");
        assert_eq!(model.tracks[1].notes.len(), 1);
        assert_eq!(model.tracks[1].notes[0].key, 48);

        // CC events
        assert!(model.tracks[0].cc.contains_key(&7));
        assert_eq!(model.tracks[0].cc[&7].len(), 1);
        assert_eq!(model.tracks[0].cc[&7][0].value, 100);

        // RPN parsed from CC 101/100/6
        assert!(model.tracks[0].rpn.contains_key(&0));
        assert_eq!(model.tracks[0].rpn[&0].len(), 1);
        assert_eq!(model.tracks[0].rpn[&0][0].value, 2);

        // Conductor
        assert_eq!(model.conductor.tempo.len(), 1);
        assert!((model.conductor.tempo[0].bpm - 120.0).abs() < 0.5);
        assert_eq!(model.conductor.time_sig.len(), 1);
        assert_eq!(model.conductor.time_sig[0].numerator, 4);

        // Key index
        assert_eq!(model.key_index.notes_by_key[60].len(), 2);
        assert_eq!(model.key_index.notes_by_key[48].len(), 1);
        assert_eq!(model.note_count, 3);
    }

    #[test]
    fn roundtrip_notes() {
        let midi = make_test_midi();
        let model = midi_to_yinmodel(&midi);
        let midi2 = super::super::to_midi::yinmodel_to_midi(&model);

        // Notes should roundtrip
        assert_eq!(midi2.key_notes[60].len(), 2);
        assert_eq!(midi2.key_notes[48].len(), 1);
        assert_eq!(midi2.key_notes[60][0].start_tick, 0);
        assert_eq!(midi2.key_notes[60][0].end_tick, 480);
        assert_eq!(midi2.key_notes[60][0].velocity, 100);
    }

    #[test]
    fn roundtrip_cc() {
        let midi = make_test_midi();
        let model = midi_to_yinmodel(&midi);
        let midi2 = super::super::to_midi::yinmodel_to_midi(&model);

        // CC should roundtrip
        let cc_events: Vec<_> = midi2
            .control_events
            .iter()
            .filter(|e| matches!(e, MidiControlEvent::ControlChange { controller: 7, .. }))
            .collect();
        assert_eq!(cc_events.len(), 1);
    }

    #[test]
    fn roundtrip_rpn() {
        let midi = make_test_midi();
        let model = midi_to_yinmodel(&midi);
        let midi2 = super::super::to_midi::yinmodel_to_midi(&model);

        // RPN should be converted back to CC 101/100/6 sequence
        let rpn_ccs: Vec<_> = midi2
            .control_events
            .iter()
            .filter(|e| matches!(e, MidiControlEvent::ControlChange { controller: 101 | 100 | 6, .. }))
            .collect();
        assert_eq!(rpn_ccs.len(), 3); // 101 + 100 + 6
    }
}
