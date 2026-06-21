use yinhe_editor_core::document::Document;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_mid2::MidiImportEncoding;
use yinhe_test_helpers::*;

fn doc_with_notes() -> Document {
    make_test_document()
}

fn first_note_key60(doc: &Document) -> (u16, u32, u8) {
    // Find the first note at key 60 and return its (track, start_tick, key) tuple
    for (key, notes) in doc.data.model.key_notes_cache.iter().enumerate() {
        for n in notes {
            if key == 60 {
                return (n.track, n.start_tick, key as u8);
            }
        }
    }
    panic!("no note at key 60");
}

fn first_note_key48(doc: &Document) -> (u16, u32, u8) {
    for (key, notes) in doc.data.model.key_notes_cache.iter().enumerate() {
        for n in notes {
            if key == 48 {
                return (n.track, n.start_tick, key as u8);
            }
        }
    }
    panic!("no note at key 48");
}

#[test]
fn document_empty_has_one_track() {
    let doc = Document::empty();
    assert_eq!(doc.data.model.tracks.len(), 1);
    assert_eq!(doc.track_names(), &["Track 1"]);
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
    let sel = first_note_key60(&mut doc);
    doc.edit.selected.insert(sel);
    let deleted = doc.delete_selected();
    assert!(deleted);
    // One fewer note at key 60
    let count_after = doc.data.model.key_notes_cache[60].len();
    assert_eq!(count_after, 1);
    assert!(doc.edit.selected.is_empty());
}

#[test]
fn delete_selected_notes_empty_selection() {
    let mut doc = doc_with_notes();
    assert!(!doc.delete_selected());
}

#[test]
fn duplicate_selected_notes() {
    let mut doc = doc_with_notes();
    let count_before = doc.data.model.key_notes_cache[60].len();
    let sel = first_note_key60(&mut doc);
    doc.edit.selected.insert(sel);
    let duplicated = doc.duplicate_selected();
    assert!(duplicated);
    assert_eq!(doc.data.model.key_notes_cache[60].len(), count_before + 1);
}

#[test]
fn duplicate_selected_notes_empty_selection() {
    let mut doc = doc_with_notes();
    assert!(!doc.duplicate_selected());
}

#[test]
fn transpose_selected_notes_up() {
    let mut doc = doc_with_notes();
    let sel = first_note_key60(&mut doc);
    doc.edit.selected.insert(sel);
    let transposed = doc.transpose_selected(12);
    assert!(transposed);
    // Note should now be at key 72
    assert_eq!(doc.data.model.key_notes_cache[72].len(), 1);
}

#[test]
fn transpose_selected_notes_down() {
    let mut doc = doc_with_notes();
    let sel = first_note_key60(&mut doc);
    doc.edit.selected.insert(sel);
    let transposed = doc.transpose_selected(-12);
    assert!(transposed);
    // Note should now be at key 48 (60 - 12)
    assert!(doc.data.model.key_notes_cache[48].len() >= 1);
}

#[test]
fn transpose_selected_notes_empty() {
    let mut doc = doc_with_notes();
    assert!(!doc.transpose_selected(12));
}

#[test]
fn undo_redo_delete() {
    let mut doc = doc_with_notes();
    let note_count_before = doc.data.model.note_count;
    let snap = doc.data.snapshot("delete");
    let sel = first_note_key60(&mut doc);
    doc.edit.selected.insert(sel);
    doc.delete_selected();
    assert_eq!(doc.data.model.note_count, note_count_before - 1);
    doc.apply_undo_snapshot(snap);
    assert_eq!(doc.data.model.note_count, note_count_before);
}

#[test]
fn undo_redo_transpose() {
    let mut doc = doc_with_notes();
    let snap = doc.data.snapshot("transpose");
    let sel = first_note_key60(&mut doc);
    doc.edit.selected.insert(sel);
    doc.transpose_selected(7);
    assert_eq!(doc.data.model.key_notes_cache[67].len(), 1);
    doc.apply_undo_snapshot(snap);
    assert!(doc.data.model.key_notes_cache[67].is_empty());
}

#[test]
fn undo_redo_duplicate() {
    let mut doc = doc_with_notes();
    let count_before = doc.data.model.key_notes_cache[60].len();
    let snap = doc.data.snapshot("duplicate");
    let sel = first_note_key60(&mut doc);
    doc.edit.selected.insert(sel);
    doc.duplicate_selected();
    assert_eq!(doc.data.model.key_notes_cache[60].len(), count_before + 1);
    doc.apply_undo_snapshot(snap);
    assert_eq!(doc.data.model.key_notes_cache[60].len(), count_before);
}

#[test]
fn undo_stack_push_and_undo_redo() {
    let mut doc = doc_with_notes();
    let snap1 = doc.data.snapshot("edit1");
    doc.history.push(snap1);
    doc.data.bump_version();
    let snap2 = doc.data.snapshot("edit2");
    doc.history.push(snap2);
    doc.data.bump_version();
    let current = doc.data.snapshot("current");
    let restored = doc.history.undo(current);
    assert!(restored.is_some());
    let current2 = doc.data.snapshot("current");
    let re_restored = doc.history.redo(current2);
    assert!(re_restored.is_some());
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
    let to_delete: Vec<(u16, u32, u8)> = doc.data.model.key_notes_cache.iter().enumerate()
        .flat_map(|(key, notes)| notes.iter().map(move |n| (n.track, n.start_tick, key as u8)))
        .take(2)
        .collect();
    let note_count_before = doc.data.model.note_count;
    for sel in &to_delete {
        doc.edit.selected.insert(*sel);
    }
    doc.delete_selected();
    assert_eq!(doc.data.model.note_count, note_count_before - 2);
}

#[test]
fn duplicate_multiple_notes() {
    let mut doc = doc_with_notes();
    let to_dup: Vec<(u16, u32, u8)> = doc.data.model.key_notes_cache.iter().enumerate()
        .flat_map(|(key, notes)| notes.iter().map(move |n| (n.track, n.start_tick, key as u8)))
        .take(2)
        .collect();
    let note_count_before = doc.data.model.note_count;
    for sel in &to_dup {
        doc.edit.selected.insert(*sel);
    }
    doc.duplicate_selected();
    assert_eq!(doc.data.model.note_count, note_count_before + 2);
}

#[test]
fn transpose_clamps_to_valid_range() {
    let mut doc = doc_with_notes();
    let sel = first_note_key60(&mut doc);
    doc.edit.selected.insert(sel);
    doc.transpose_selected(-100);
    // Should clamp to key 0
    assert_eq!(doc.data.model.key_notes_cache[0].len(), 1);
}

#[test]
fn transpose_clamps_upper_bound() {
    let mut doc = doc_with_notes();
    let sel = first_note_key60(&mut doc);
    doc.edit.selected.insert(sel);
    doc.transpose_selected(100);
    // Should clamp to key 127
    assert_eq!(doc.data.model.key_notes_cache[127].len(), 1);
}

#[test]
fn delete_then_undo_restores_notes() {
    let mut doc = doc_with_notes();
    let snap = doc.data.snapshot("before");
    let note_count_before = doc.data.model.note_count;
    let sel = first_note_key60(&mut doc);
    doc.edit.selected.insert(sel);
    doc.delete_selected();
    assert_eq!(doc.data.model.note_count, note_count_before - 1);
    doc.apply_undo_snapshot(snap);
    assert_eq!(doc.data.model.note_count, note_count_before);
}

#[test]
fn consecutive_operations() {
    let mut doc = doc_with_notes();
    let snap1 = doc.data.snapshot("del");
    let note_count_before = doc.data.model.note_count;
    let sel = first_note_key60(&mut doc);
    doc.edit.selected.insert(sel);
    doc.delete_selected();
    assert_eq!(doc.data.model.note_count, note_count_before - 1);

    let sel2 = first_note_key48(&mut doc);
    doc.edit.selected.insert(sel2);
    doc.transpose_selected(12);

    doc.apply_undo_snapshot(snap1);
    assert_eq!(doc.data.model.note_count, note_count_before);
}
