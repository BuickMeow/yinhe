use yinhe_types::Note;
use yinhe_pianoroll::instances;
use yinhe_pianoroll::PianoRollView;
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
    let colors = vec![[1.0f32, 1.0, 1.0]; m.tracks.len()];
    let selected = std::collections::HashSet::new();
    let hidden = std::collections::HashSet::new();

    let mut out = Vec::new();
    instances::build_notes(&mut out, 800.0, 600.0, &m, &view, &selected, &hidden, &visible, &colors);
    assert_eq!(out.len(), m.note_count as usize);
}

#[test]
fn build_notes_empty_source() {
    let m = YinModel::default();
    let view = wide_view();
    let visible = vec![];
    let colors = vec![];
    let selected = std::collections::HashSet::new();
    let hidden = std::collections::HashSet::new();

    let mut out = Vec::new();
    instances::build_notes(&mut out, 800.0, 600.0, &m, &view, &selected, &hidden, &visible, &colors);
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
    let hidden = std::collections::HashSet::new();

    let mut out = Vec::new();
    instances::build_notes(&mut out, 800.0, 600.0, &m, &view, &selected, &hidden, &visible, &colors);
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
    let hidden = std::collections::HashSet::new();

    let mut out = Vec::new();
    instances::build_notes(&mut out, 800.0, 600.0, &m, &view, &selected, &hidden, &visible, &colors);
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
    let hidden = std::collections::HashSet::new();

    let mut out = Vec::new();
    instances::build_notes(&mut out, 800.0, 600.0, &m, &view, &selected, &hidden, &visible, &colors);
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
    let hidden = std::collections::HashSet::new();

    let mut out = Vec::new();
    instances::build_notes(&mut out, 800.0, 600.0, &m, &view, &selected, &hidden, &visible, &colors);
    assert!(out.is_empty());
}

#[test]
fn build_notes_stress_many_tracks() {
    let m = make_stress_model(8, 100);
    let mut view = wide_view();
    // Zoom out so all notes (up to tick 12100) are visible
    view.base.pixels_per_tick = 0.01;
    let visible = vec![true; m.tracks.len()];
    let colors = vec![[1.0f32, 1.0, 1.0]; m.tracks.len()];
    let selected = std::collections::HashSet::new();
    let hidden = std::collections::HashSet::new();

    let mut out = Vec::new();
    instances::build_notes(&mut out, 800.0, 600.0, &m, &view, &selected, &hidden, &visible, &colors);
    assert_eq!(out.len(), 800);
}
