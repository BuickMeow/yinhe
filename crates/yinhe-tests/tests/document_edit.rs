use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{UndoAction, UndoEntry};
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_mid2::MidiImportEncoding;
use yinhe_test_helpers::*;

fn doc_with_notes() -> Document {
    make_test_document()
}

fn first_note_key60(doc: &Document) -> (u16, u32, u8, u32) {
    // Find the first note at key 60 and return (track, start_tick, key, end_tick)
    for (key, notes) in doc.data.model.notes.iter().enumerate() {
        for n in notes.iter() {
            if key == 60 {
                return (n.track, n.start_tick, key as u8, n.end_tick);
            }
        }
    }
    panic!("no note at key 60");
}

fn first_note_key48(doc: &Document) -> (u16, u32, u8, u32) {
    for (key, notes) in doc.data.model.notes.iter().enumerate() {
        for n in notes.iter() {
            if key == 48 {
                return (n.track, n.start_tick, key as u8, n.end_tick);
            }
        }
    }
    panic!("no note at key 48");
}

#[test]
fn document_empty_has_conductor_and_16_tracks() {
    let doc = Document::empty();
    assert_eq!(doc.data.model.tracks.len(), 17);
    assert_eq!(doc.track_names()[0], "Conductor");
    assert_eq!(doc.track_names()[1], "A1");
    assert_eq!(doc.track_names()[16], "A16");
}

#[test]
fn document_from_model() {
    let m = make_test_model();
    let doc = Document::from_model("test.mid", m, QuantizePreset::default(), Default::default(), Default::default())
        .expect("from_model failed");
    assert_eq!(doc.data.model.note_count, 4);
    assert_eq!(doc.file_name, "test");
}

#[test]
fn document_detect_conductor() {
    let doc = doc_with_notes();
    // make_test_midi track 0 has notes → detect_conductor returns None → conductor added
    // So conductor_track_idx should be Some(0)
    assert!(doc.edit.conductor_track_idx.is_some());
}

#[test]
fn document_track_info_cache() {
    let doc = doc_with_notes();
    // After from_midi: track 0 = Conductor, track 1+ = original tracks
    assert!(doc.track_info_cache().len() >= 3);
    assert_eq!(doc.track_info_cache()[0].name, "Conductor");
}

#[test]
fn delete_selected_notes() {
    let mut doc = doc_with_notes();
    let (track, start_tick, key, end_tick) = first_note_key60(&mut doc);
    doc.edit.selected.add_rect_track(start_tick, end_tick, key, key, track, track);
    let action = doc.delete_selected();
    assert!(action.is_some());
    // One fewer note at key 60
    let count_after = doc.data.model.notes[60].len();
    assert_eq!(count_after, 1);
    assert!(doc.edit.selected.is_empty());
}

#[test]
fn delete_selected_notes_empty_selection() {
    let mut doc = doc_with_notes();
    assert!(doc.delete_selected().is_none());
}

#[test]
fn duplicate_selected_notes() {
    let mut doc = doc_with_notes();
    let count_before = doc.data.model.notes[60].len();
    let (track, start_tick, key, end_tick) = first_note_key60(&mut doc);
    doc.edit.selected.add_rect_track(start_tick, end_tick, key, key, track, track);
    let action = doc.duplicate_selected();
    assert!(action.is_some());
    assert_eq!(doc.data.model.notes[60].len(), count_before + 1);
}

#[test]
fn duplicate_selected_notes_empty_selection() {
    let mut doc = doc_with_notes();
    assert!(doc.duplicate_selected().is_none());
}

#[test]
fn transpose_selected_notes_up() {
    let mut doc = doc_with_notes();
    let (track, start_tick, key, end_tick) = first_note_key60(&mut doc);
    doc.edit.selected.add_rect_track(start_tick, end_tick, key, key, track, track);
    let action = doc.transpose_selected(12);
    assert!(action.is_some());
    // Note should now be at key 72
    assert_eq!(doc.data.model.notes[72].len(), 1);
}

#[test]
fn transpose_selected_notes_down() {
    let mut doc = doc_with_notes();
    let (track, start_tick, key, end_tick) = first_note_key60(&mut doc);
    doc.edit.selected.add_rect_track(start_tick, end_tick, key, key, track, track);
    let action = doc.transpose_selected(-12);
    assert!(action.is_some());
    // Note should now be at key 48 (60 - 12)
    assert!(doc.data.model.notes[48].len() >= 1);
}

#[test]
fn transpose_selected_notes_empty() {
    let mut doc = doc_with_notes();
    assert!(doc.transpose_selected(12).is_none());
}

#[test]
fn undo_redo_delete() {
    let mut doc = doc_with_notes();
    let note_count_before = doc.data.model.note_count;
    let (track, start_tick, key, end_tick) = first_note_key60(&mut doc);
    doc.edit.selected.add_rect_track(start_tick, end_tick, key, key, track, track);
    let action = doc.delete_selected().unwrap();
    assert_eq!(doc.data.model.note_count, note_count_before - 1);

    // Undo via UndoStack
    doc.history.push(UndoEntry {
        action,
        label: "delete",
        selected: doc.edit.selected.clone(),
        track_selected: doc.edit.track_selected.clone(),
        sel_rect: doc.edit.sel_rect.clone(),
    });
    assert!(doc.undo());
    assert_eq!(doc.data.model.note_count, note_count_before);

    // Redo
    assert!(doc.redo());
    assert_eq!(doc.data.model.note_count, note_count_before - 1);
}

#[test]
fn undo_redo_transpose() {
    let mut doc = doc_with_notes();
    let (track, start_tick, key, end_tick) = first_note_key60(&mut doc);
    doc.edit.selected.add_rect_track(start_tick, end_tick, key, key, track, track);
    let action = doc.transpose_selected(7).unwrap();
    assert_eq!(doc.data.model.notes[67].len(), 1);

    doc.history.push(UndoEntry {
        action,
        label: "transpose",
        selected: doc.edit.selected.clone(),
        track_selected: doc.edit.track_selected.clone(),
        sel_rect: doc.edit.sel_rect.clone(),
    });
    assert!(doc.undo());
    assert!(doc.data.model.notes[67].is_empty());
}

#[test]
fn undo_redo_duplicate() {
    let mut doc = doc_with_notes();
    let count_before = doc.data.model.notes[48].len();
    let (track, start_tick, key, end_tick) = first_note_key48(&mut doc);
    doc.edit.selected.add_rect_track(start_tick, end_tick, key, key, track, track);
    let action = doc.duplicate_selected().unwrap();
    assert_eq!(doc.data.model.notes[48].len(), count_before + 1);

    doc.history.push(UndoEntry {
        action,
        label: "duplicate",
        selected: doc.edit.selected.clone(),
        track_selected: doc.edit.track_selected.clone(),
        sel_rect: doc.edit.sel_rect.clone(),
    });
    assert!(doc.undo());
    assert_eq!(doc.data.model.notes[48].len(), count_before);
}

#[test]
fn undo_stack_push_and_undo_redo() {
    let mut doc = doc_with_notes();

    // Simulate edit 1: rename track
    doc.history.push(UndoEntry {
        action: UndoAction::TrackName {
            track_idx: 1,
            old: "Track 1".into(),
            new: "Edited".into(),
        },
        label: "edit1",
        selected: Default::default(),
        track_selected: Default::default(),
        sel_rect: Default::default(),
    });
    // Apply the edit
    {
        let model = std::sync::Arc::make_mut(&mut doc.data.model);
        std::sync::Arc::make_mut(&mut model.tracks[1]).name = "Edited".into();
    }

    // Simulate edit 2
    doc.history.push(UndoEntry {
        action: UndoAction::TrackName {
            track_idx: 1,
            old: "Edited".into(),
            new: "Edited2".into(),
        },
        label: "edit2",
        selected: Default::default(),
        track_selected: Default::default(),
        sel_rect: Default::default(),
    });
    {
        let model = std::sync::Arc::make_mut(&mut doc.data.model);
        std::sync::Arc::make_mut(&mut model.tracks[1]).name = "Edited2".into();
    }

    assert!(doc.undo());
    assert_eq!(doc.data.model.tracks[1].name, "Edited");
    assert!(doc.undo());
    assert_eq!(doc.data.model.tracks[1].name, "Track 1");
    assert!(doc.redo());
    assert_eq!(doc.data.model.tracks[1].name, "Edited");
}

#[test]
fn document_recode_track_names() {
    let m = make_test_model();
    let mut doc = Document::from_model("test.mid", m, QuantizePreset::default(), Default::default(), Default::default())
        .expect("from_model failed");
    let original_names = doc.data.track_names.clone();
    doc.recode_track_names(MidiImportEncoding::Utf8);
    assert_eq!(doc.data.track_names, original_names);
}

#[test]
fn document_pc_map_cache() {
    let doc = doc_with_notes();
    // After from_midi, track 2 becomes track 3 (Conductor added at 0), channel 16
    assert!(!doc.edit.pc_map_cache.is_empty());
}

#[test]
fn delete_multiple_notes() {
    let mut doc = doc_with_notes();
    // Collect note references first to avoid borrow conflict
    let to_delete: Vec<(u16, u32, u8, u32)> = doc.data.model.notes.iter().enumerate()
        .flat_map(|(key, notes)| notes.iter().map(move |n| (n.track, n.start_tick, key as u8, n.end_tick)))
        .take(2)
        .collect();
    let note_count_before = doc.data.model.note_count;
    for (track, start_tick, key, end_tick) in &to_delete {
        doc.edit.selected.add_rect_track(*start_tick, *end_tick, *key, *key, *track, *track);
    }
    doc.delete_selected();
    assert_eq!(doc.data.model.note_count, note_count_before - 2);
}

#[test]
fn duplicate_multiple_notes() {
    let mut doc = doc_with_notes();
    let to_dup: Vec<(u16, u32, u8, u32)> = doc.data.model.notes.iter().enumerate()
        .flat_map(|(key, notes)| notes.iter().map(move |n| (n.track, n.start_tick, key as u8, n.end_tick)))
        .take(2)
        .collect();
    let note_count_before = doc.data.model.note_count;
    for (track, start_tick, key, end_tick) in &to_dup {
        doc.edit.selected.add_rect_track(*start_tick, *end_tick, *key, *key, *track, *track);
    }
    doc.duplicate_selected();
    assert_eq!(doc.data.model.note_count, note_count_before + 2);
}

#[test]
fn transpose_clamps_to_valid_range() {
    let mut doc = doc_with_notes();
    let (track, start_tick, key, end_tick) = first_note_key60(&mut doc);
    doc.edit.selected.add_rect_track(start_tick, end_tick, key, key, track, track);
    doc.transpose_selected(-100);
    // Should clamp to key 0
    assert_eq!(doc.data.model.notes[0].len(), 1);
}

#[test]
fn transpose_clamps_upper_bound() {
    let mut doc = doc_with_notes();
    let (track, start_tick, key, end_tick) = first_note_key60(&mut doc);
    doc.edit.selected.add_rect_track(start_tick, end_tick, key, key, track, track);
    doc.transpose_selected(100);
    // Should clamp to key 127
    assert_eq!(doc.data.model.notes[127].len(), 1);
}

#[test]
fn delete_then_undo_restores_notes() {
    let mut doc = doc_with_notes();
    let note_count_before = doc.data.model.note_count;
    let (track, start_tick, key, end_tick) = first_note_key60(&mut doc);
    doc.edit.selected.add_rect_track(start_tick, end_tick, key, key, track, track);
    let action = doc.delete_selected().unwrap();
    assert_eq!(doc.data.model.note_count, note_count_before - 1);

    doc.history.push(UndoEntry {
        action,
        label: "delete",
        selected: doc.edit.selected.clone(),
        track_selected: doc.edit.track_selected.clone(),
        sel_rect: doc.edit.sel_rect.clone(),
    });
    assert!(doc.undo());
    assert_eq!(doc.data.model.note_count, note_count_before);
}

#[test]
fn consecutive_operations() {
    let mut doc = doc_with_notes();
    let note_count_before = doc.data.model.note_count;
    let (track, start_tick, key, end_tick) = first_note_key60(&mut doc);
    doc.edit.selected.add_rect_track(start_tick, end_tick, key, key, track, track);
    let action1 = doc.delete_selected().unwrap();
    assert_eq!(doc.data.model.note_count, note_count_before - 1);

    let (track2, start_tick2, key2, end_tick2) = first_note_key48(&mut doc);
    doc.edit.selected.add_rect_track(start_tick2, end_tick2, key2, key2, track2, track2);
    let action2 = doc.transpose_selected(12).unwrap();

    // Undo both
    doc.history.push(UndoEntry {
        action: action1,
        label: "delete",
        selected: doc.edit.selected.clone(),
        track_selected: doc.edit.track_selected.clone(),
        sel_rect: doc.edit.sel_rect.clone(),
    });
    doc.history.push(UndoEntry {
        action: action2,
        label: "transpose",
        selected: doc.edit.selected.clone(),
        track_selected: doc.edit.track_selected.clone(),
        sel_rect: doc.edit.sel_rect.clone(),
    });

    assert!(doc.undo());
    assert!(doc.undo());
    assert_eq!(doc.data.model.note_count, note_count_before);
}
