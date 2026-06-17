use yinhe_midi::{MidiFile, MidiControlEvent};
use yinhe_test_helpers::*;

#[test]
fn parse_minimal_midi_from_bytes() {
    let midi = parse_minimal_midi();
    assert_eq!(midi.ticks_per_beat, 480);
    assert_eq!(midi.note_count, 1);
    assert_eq!(midi.key_notes[60].len(), 1);
    let note = &midi.key_notes[60][0];
    assert_eq!(note.velocity, 100);
    assert_eq!(note.start_tick, 0);
    assert_eq!(note.end_tick, 320);
}

#[test]
fn parse_minimal_midi_has_default_tempo() {
    let midi = parse_minimal_midi();
    assert!(!midi.tempo_segments.is_empty());
    let bpm = yinhe_midi::bpm_from_mpq(midi.tempo_segments[0].micros_per_quarter);
    assert!((bpm - 120.0).abs() < 0.5, "expected ~120 BPM, got {bpm}");
}

#[test]
fn parse_minimal_midi_has_correct_time_sig() {
    let midi = parse_minimal_midi();
    assert_eq!(midi.time_sig_numerator, 4);
}

#[test]
fn test_midi_tracks_preserved() {
    let m = make_test_model();
    assert_eq!(m.track_ports.len(), 3);
    assert_eq!(m.track_names, vec!["Lead", "Bass", "Drums"]);
}

#[test]
fn test_midi_notes_by_key() {
    let m = make_test_model();
    assert_eq!(m.key_notes[60].len(), 2);
    assert_eq!(m.key_notes[48].len(), 1);
    assert_eq!(m.key_notes[36].len(), 1);
    assert_eq!(m.note_count, 4);
}

#[test]
fn test_midi_tick_length() {
    let m = make_test_model();
    assert_eq!(m.tick_length, 1920);
}

#[test]
fn test_midi_tick_to_seconds_at_zero() {
    let m = make_test_model();
    let secs = m.tick_to_seconds(0);
    assert!(secs.abs() < 1e-9, "tick_to_seconds(0) should be 0.0, got {secs}");
}

#[test]
fn test_midi_tick_to_seconds_with_120_bpm() {
    let m = make_test_model();
    // 120 BPM = 500ms per beat, 480 tpb
    // 480 ticks = 0.5 seconds
    let secs = m.tick_to_seconds(480);
    assert!((secs - 0.5).abs() < 1e-6, "480 ticks at 120bpm = 0.5s, got {secs}");
}

#[test]
fn test_midi_bpm_at_time() {
    let m = make_test_model();
    let bpm0 = m.bpm_at_time(0.0);
    assert!((bpm0 - 120.0).abs() < 0.5, "expected ~120 BPM at t=0, got {bpm0}");
}

#[test]
fn test_midi_time_sig_at_tick() {
    let m = make_test_model();
    let (num, _den) = m.time_sig_at_tick(0);
    assert_eq!(num, 4);
    let (num2, _den2) = m.time_sig_at_tick(1920);
    assert_eq!(num2, 3);
}

#[test]
fn test_midi_bar_divide() {
    let m = make_test_model();
    // 4/4 at 480 tpb: bar_divide = 480 * 4 = 1920
    let bd = m.bar_divide();
    assert!((bd - 1920.0).abs() < 1.0, "bar_divide should be ~1920, got {bd}");
}

#[test]
fn test_midi_bar_at_tick() {
    let m = make_test_model();
    assert_eq!(m.bar_at_tick(0), 1);
    assert_eq!(m.bar_at_tick(1919), 1);
    assert_eq!(m.bar_at_tick(1920), 2);
}

#[test]
fn test_midi_total_bars() {
    let m = make_test_model();
    let bars = m.total_bars();
    // tick_length=1920, bar_divide=1920 (4/4 at 480tpb) → 1 bar
    assert!(bars >= 1, "total_bars should be >= 1, got {bars}");
}

#[test]
fn test_midi_track_port() {
    let m = make_test_model();
    assert_eq!(m.track_port(0), 0);
    assert_eq!(m.track_port(1), 0);
    assert_eq!(m.track_port(2), 1);
}

#[test]
fn test_midi_track_info() {
    let m = make_test_model();
    let info = m.track_info();
    assert_eq!(info.len(), 3);
    assert_eq!(info[0].name, "Lead");
    assert_eq!(info[0].note_count, 2);
    assert_eq!(info[1].name, "Bass");
    assert_eq!(info[1].note_count, 1);
}

#[test]
fn test_midi_control_events_preserved() {
    let m = make_test_model();
    let cc_count = m.control_events.iter()
        .filter(|e| matches!(e, MidiControlEvent::ControlChange { .. }))
        .count();
    assert_eq!(cc_count, 2);
    let pb_count = m.control_events.iter()
        .filter(|e| matches!(e, MidiControlEvent::PitchBend { .. }))
        .count();
    assert_eq!(pb_count, 1);
    let pc_count = m.control_events.iter()
        .filter(|e| matches!(e, MidiControlEvent::ProgramChange { .. }))
        .count();
    assert_eq!(pc_count, 1);
}

#[test]
fn test_midi_empty_file() {
    let m = MidiFile::default();
    assert_eq!(m.note_count, 0);
    assert_eq!(m.tick_length, 0);
    assert!(m.track_names.is_empty());
}

#[test]
fn test_midi_stress_many_notes() {
    let m = make_stress_midi(4, 1000);
    assert_eq!(m.note_count, 4000);
    assert_eq!(m.track_ports.len(), 4);
}

#[test]
fn test_note_source_trait_implementation() {
    use yinhe_types::NoteSource;
    let m = make_test_model();
    assert_eq!(m.ticks_per_beat(), Some(480));
    assert!(m.tick_length() > Some(0));
    assert_eq!(m.time_sig_default(), (4, 2));
    assert!(!m.time_sig_events().is_empty());
}
