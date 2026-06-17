use yinhe_types::{Note, NoteScanIndex, TickBuckets};
use yinhe_pianoroll::instances;
use yinhe_pianoroll::PianoRollView;

use yinhe_test_helpers::*;

fn wide_view() -> PianoRollView {
    PianoRollView {
        key_height: 4.0, // small key_height to show all 128 keys in 600px
        ..Default::default()
    }
}

#[test]
fn build_notes_output_count_matches_input() {
    let m = make_test_model();
    let view = wide_view();
    let visible = vec![true; m.tracks.len()];
    let colors = vec![[1.0f32, 1.0, 1.0]; m.tracks.len()];
    let selected = std::collections::HashSet::new();

    let mut out = Vec::new();
    instances::build_notes(&mut out, 800.0, 600.0, &m, &view, &selected, &visible, &colors, 0.0);
    assert_eq!(out.len(), m.note_count as usize);
}

#[test]
fn build_notes_empty_source() {
    let m = YinModel::default();
    let view = wide_view();
    let visible = vec![];
    let colors = vec![];
    let selected = std::collections::HashSet::new();

    let mut out = Vec::new();
    instances::build_notes(&mut out, 800.0, 600.0, &m, &view, &selected, &visible, &colors, 0.0);
    assert!(out.is_empty());
}

#[test]
fn build_notes_with_selection() {
    let m = make_test_model();
    let view = wide_view();
    let visible = vec![true; m.tracks.len()];
    let colors = vec![[1.0f32, 1.0, 1.0]; m.tracks.len()];
    let mut selected = std::collections::HashSet::new();
    selected.insert((0u16, 0u32, 60u8));

    let mut out = Vec::new();
    instances::build_notes(&mut out, 800.0, 600.0, &m, &view, &selected, &visible, &colors, 0.0);
    assert_eq!(out.len(), 4);
}

#[test]
fn build_notes_visibility_filter() {
    let m = make_test_model();
    let view = wide_view();
    let mut visible = vec![true; m.tracks.len()];
    visible[0] = false; // hide track 0
    let colors = vec![[1.0f32, 1.0, 1.0]; m.tracks.len()];
    let selected = std::collections::HashSet::new();

    let mut out = Vec::new();
    instances::build_notes(&mut out, 800.0, 600.0, &m, &view, &selected, &visible, &colors, 0.0);
    // Track 0 has 2 notes, track 1 has 1, track 2 has 1
    // Hiding track 0 should leave 2 notes
    assert_eq!(out.len(), 2);
}

#[test]
fn build_notes_visible_range_culling() {
    let m = make_test_model();
    let view = wide_view();
    let visible = vec![true; m.tracks.len()];
    let colors = vec![[1.0f32, 1.0, 1.0]; m.tracks.len()];
    let selected = std::collections::HashSet::new();

    let mut out = Vec::new();
    instances::build_notes(&mut out, 800.0, 600.0, &m, &view, &selected, &visible, &colors, 0.0);
    // Default view shows all notes
    assert_eq!(out.len(), 4);
}

#[test]
fn build_notes_empty_data() {
    let m = YinModel::default();
    let view = wide_view();
    let visible = vec![true; 0];
    let colors = vec![[1.0f32, 1.0, 1.0]; 0];
    let selected = std::collections::HashSet::new();

    let mut out = Vec::new();
    instances::build_notes(&mut out, 800.0, 600.0, &m, &view, &selected, &visible, &colors, 0.0);
    assert!(out.is_empty());
}

#[test]
fn build_notes_stress_many_tracks() {
    let m = make_stress_midi(8, 100);
    let mut view = wide_view();
    // Zoom out so all notes (up to tick 12100) are visible
    view.base.pixels_per_tick = 0.01;
    let visible = vec![true; m.tracks.len()];
    let colors = vec![[1.0f32, 1.0, 1.0]; m.tracks.len()];
    let selected = std::collections::HashSet::new();

    let mut out = Vec::new();
    instances::build_notes(&mut out, 800.0, 600.0, &m, &view, &selected, &visible, &colors, 0.0);
    assert_eq!(out.len(), 800);
}

// ── TickBuckets ──

#[test]
fn tick_buckets_build_and_range() {
    let mut key_notes: [Vec<Note>; 128] = std::array::from_fn(|_| Vec::new());
    key_notes[60].push(Note { start_tick: 0, end_tick: 100, velocity: 80, track: 0 });
    key_notes[60].push(Note { start_tick: 200, end_tick: 300, velocity: 80, track: 0 });
    key_notes[60].push(Note { start_tick: 1000, end_tick: 1100, velocity: 80, track: 0 });

    let buckets = TickBuckets::build(&key_notes, 1200, 500);
    let (start, end) = buckets.range_for(60, 0, 500);
    assert!(start < end);
}

// ── NoteScanIndex ──

#[test]
fn note_scan_index_build_and_seek() {
    let mut key_notes: [Vec<Note>; 128] = std::array::from_fn(|_| Vec::new());
    key_notes[60].push(Note { start_tick: 0, end_tick: 100, velocity: 80, track: 0 });
    key_notes[60].push(Note { start_tick: 500, end_tick: 600, velocity: 80, track: 0 });
    key_notes[60].push(Note { start_tick: 1000, end_tick: 1100, velocity: 80, track: 0 });

    let mut m = YinModel::default();
    m.key_notes = key_notes.clone();
    m.scan_index = Some(NoteScanIndex::build(&key_notes, 1200));

    let first = yinhe_types::seek_first_note(60, &m, 400);
    assert!(first < key_notes[60].len());
    assert!(key_notes[60][first].start_tick >= 400);
}
