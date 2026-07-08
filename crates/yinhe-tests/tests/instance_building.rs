use yinhe_types::Note;
use yinhe_pianoroll::build_notes;
use yinhe_types::PianoRollView;
use yinhe_core::YinModel;

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
    let hidden = std::collections::HashSet::new();

    let mut out = Vec::new();
    build_notes(&mut out, 800.0, 600.0, &m, &view, &hidden, &visible);
    assert_eq!(out.len(), m.note_count as usize);
}

#[test]
fn build_notes_empty_source() {
    let m = YinModel::default();
    let view = wide_view();
    let visible = vec![];
    let hidden = std::collections::HashSet::new();

    let mut out = Vec::new();
    build_notes(&mut out, 800.0, 600.0, &m, &view, &hidden, &visible);
    assert!(out.is_empty());
}

#[test]
fn build_notes_with_selection() {
    let m = make_test_model();
    let view = wide_view();
    let visible = vec![true; m.tracks.len()];
    // Select note at track 0, start_tick 0, key 60 (end_tick 480 based on test model)
    let mut selected = yinhe_core::Selection::default();
    selected.add_rect_track(0, 480, 60, 60, 0, 0);
    let hidden = std::collections::HashSet::new();

    let mut out = Vec::new();
    build_notes(&mut out, 800.0, 600.0, &m, &view, &hidden, &visible);
    assert_eq!(out.len(), 4);
}

#[test]
fn build_notes_visibility_filter() {
    let m = make_test_model();
    let view = wide_view();
    let mut visible = vec![true; m.tracks.len()];
    visible[0] = false; // hide track 0
    let hidden = std::collections::HashSet::new();

    let mut out = Vec::new();
    build_notes(&mut out, 800.0, 600.0, &m, &view, &hidden, &visible);
    // Track 0 has 2 notes, track 1 has 1, track 2 has 1
    // Hiding track 0 should leave 2 notes
    assert_eq!(out.len(), 2);
}

#[test]
fn build_notes_visible_range_culling() {
    let m = make_test_model();
    let view = wide_view();
    let visible = vec![true; m.tracks.len()];
    let hidden = std::collections::HashSet::new();

    let mut out = Vec::new();
    build_notes(&mut out, 800.0, 600.0, &m, &view, &hidden, &visible);
    // Default view shows all notes
    assert_eq!(out.len(), 4);
}

#[test]
fn build_notes_empty_data() {
    let m = YinModel::default();
    let view = wide_view();
    let visible = vec![true; 0];
    let hidden = std::collections::HashSet::new();

    let mut out = Vec::new();
    build_notes(&mut out, 800.0, 600.0, &m, &view, &hidden, &visible);
    assert!(out.is_empty());
}

#[test]
fn build_notes_stress_many_tracks() {
    let m = make_stress_model(8, 100);
    let mut view = wide_view();
    // Zoom out so all notes (up to tick 12100) are visible
    view.base.pixels_per_tick = 0.01;
    let visible = vec![true; m.tracks.len()];
    let hidden = std::collections::HashSet::new();

    let mut out = Vec::new();
    build_notes(&mut out, 800.0, 600.0, &m, &view, &hidden, &visible);
    assert_eq!(out.len(), 800);
}
