use yinhe_editor_core::document::Document;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_midi::{MidiControlEvent, MidiFile};
use yinhe_types::{Note, TimeSigEvent};

/// Create a minimal valid MIDI file from raw bytes (SMF format 0, 1 track, 480 tpb).
pub fn minimal_midi_bytes() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(b"MThd");
    data.extend_from_slice(&6u32.to_be_bytes());
    data.extend_from_slice(&[0, 0, 0, 1, 1, 0xE0]);
    data.extend_from_slice(b"MTrk");
    let track: &[u8] = &[
        0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, 0x00, 0x90, 60, 100, 0x82, 0x40, 0x80, 60, 0,
        0x00, 0xFF, 0x2F, 0x00,
    ];
    data.extend_from_slice(&(track.len() as u32).to_be_bytes());
    data.extend_from_slice(track);
    data
}

/// Create a minimal MIDI file from bytes and parse it.
pub fn parse_minimal_midi() -> MidiFile {
    let data = minimal_midi_bytes();
    MidiFile::load_from_bytes(&data).expect("failed to parse minimal MIDI")
}

/// Create a multi-track test MIDI file (programmatically, no raw bytes).
pub fn make_test_midi() -> MidiFile {
    let mut m = MidiFile::default();
    m.ticks_per_beat = 480;
    m.track_ports = vec![0, 0, 1];
    m.track_channel_prefixes = vec![None, None, None];
    m.track_channels = vec![0, 1, 16];
    m.track_names = vec!["Lead".into(), "Bass".into(), "Drums".into()];

    m.key_notes[60].push(Note {
        start_tick: 0,
        end_tick: 480,
        velocity: 100,
        track: 0,
    });
    m.key_notes[60].push(Note {
        start_tick: 480,
        end_tick: 960,
        velocity: 100,
        track: 0,
    });
    m.key_notes[48].push(Note {
        start_tick: 0,
        end_tick: 1920,
        velocity: 90,
        track: 1,
    });
    m.key_notes[36].push(Note {
        start_tick: 0,
        end_tick: 240,
        velocity: 120,
        track: 2,
    });

    m.control_events.push(MidiControlEvent::ControlChange {
        tick: 0,
        controller: 7,
        value: 100,
        track: 0,
    });
    m.control_events.push(MidiControlEvent::ControlChange {
        tick: 240,
        controller: 7,
        value: 80,
        track: 0,
    });
    m.control_events.push(MidiControlEvent::PitchBend {
        tick: 100,
        value: 1024,
        track: 1,
    });
    m.control_events.push(MidiControlEvent::ProgramChange {
        tick: 0,
        program: 7,
        track: 2,
    });

    m.tempo_segments = vec![
        yinhe_midi::TempoSegment {
            start_tick: 0,
            start_time: 0.0,
            micros_per_quarter: yinhe_midi::mpq_from_bpm(120.0),
        },
        yinhe_midi::TempoSegment {
            start_tick: 1920,
            start_time: 0.0,
            micros_per_quarter: yinhe_midi::mpq_from_bpm(140.0),
        },
    ];
    yinhe_midi::recompute_tempo_start_times(&mut m.tempo_segments, m.ticks_per_beat);

    m.time_sig_events = vec![
        TimeSigEvent {
            tick: 0,
            numerator: 4,
            denominator: 2,
        },
        TimeSigEvent {
            tick: 1920,
            numerator: 3,
            denominator: 2,
        },
    ];

    m.note_count = m.key_notes.iter().map(|n| n.len() as u64).sum();
    m.tick_length = 1920;
    m
}

/// Create a Document from a MidiFile.
pub fn make_test_document() -> Document {
    let midi = make_test_midi();
    Document::from_midi("test.mid", midi, QuantizePreset::default()).expect("from_midi failed")
}

/// Create a multi-track MIDI with notes on many keys for stress testing.
pub fn make_stress_midi(track_count: u16, notes_per_track: u32) -> MidiFile {
    let mut m = MidiFile::default();
    m.ticks_per_beat = 480;
    m.track_ports = (0..track_count).map(|i| i as u8).collect();
    m.track_channels = (0..track_count).map(|i| i as u8).collect();
    m.track_channel_prefixes = (0..track_count).map(|_| None).collect();
    m.track_names = (0..track_count).map(|i| format!("Track {}", i)).collect();

    let mut note_count = 0u64;
    for t in 0..track_count {
        for n in 0..notes_per_track {
            let key = (n % 128) as u8;
            let start = n * 120;
            m.key_notes[key as usize].push(Note {
                start_tick: start,
                end_tick: start + 100,
                velocity: 80,
                track: t,
            });
            note_count += 1;
        }
    }
    m.note_count = note_count;
    m.tick_length = (notes_per_track * 120 + 100) as u64;
    m
}
