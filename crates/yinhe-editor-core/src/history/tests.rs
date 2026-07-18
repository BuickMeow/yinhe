use std::collections::HashSet;
use std::sync::Arc;

use yinhe_core::{ConductorData, NoteEvent, TempoEvent, TrackData, YinModel};
use yinhe_types::TimeSigEvent;

use crate::edit_state::SelRectState;
use crate::document::Document;

use super::*;
use yinhe_core::Selection;

fn make_doc(name: &str) -> Document {
    let model = YinModel {
        conductor: Arc::new(ConductorData {
            tempo: vec![TempoEvent { tick: 0, bpm: 120.0 }],
            time_sig: vec![TimeSigEvent {
                tick: 0,
                numerator: 4,
                denominator: 2,
            }],
        }),
        tracks: vec![Arc::new({
            let mut t = TrackData::new(0, 0);
            t.name = name.to_string();
            t
        })],
        ..Default::default()
    };
    Document {
        data: crate::project_data::ProjectData::new(
            Arc::new(model),
            vec![name.to_string()],
            Default::default(),
            Default::default(),
        ),
        edit: crate::edit_state::EditState {
            track_visible: vec![true],
            track_pianoroll_visible: vec![true],
            ..Default::default()
        },
        history: UndoStack::new(),
        file_name: "test".into(),
        file_path: None,
    }
}

#[test]
fn push_stores_and_clears_redo() {
    let mut doc = make_doc("a");
    doc.history.push(UndoEntry {
        action: UndoAction::TrackName {
            track_idx: 0,
            old: "a".into(),
            new: "b".into(),
        },
        label: "rename",
        selected: Selection::default(),
        track_selected: HashSet::new(),
        sel_rect: SelRectState::default(),
    });
    assert!(doc.history.can_undo());
    assert!(!doc.history.can_redo());

    doc.undo();
    assert!(!doc.history.can_undo());
    assert!(doc.history.can_redo());

    doc.history.push(UndoEntry {
        action: UndoAction::TrackName {
            track_idx: 0,
            old: "c".into(),
            new: "d".into(),
        },
        label: "rename2",
        selected: Selection::default(),
        track_selected: HashSet::new(),
        sel_rect: SelRectState::default(),
    });
    assert!(!doc.history.can_redo());
    assert!(doc.history.can_undo());
}

#[test]
fn undo_restores_track_name() {
    let mut doc = make_doc("old");
    doc.history.push(UndoEntry {
        action: UndoAction::TrackName {
            track_idx: 0,
            old: "old".into(),
            new: "new".into(),
        },
        label: "rename",
        selected: Selection::default(),
        track_selected: HashSet::new(),
        sel_rect: SelRectState::default(),
    });
    // Apply the forward action manually (simulating the edit)
    {
        let model = Arc::make_mut(&mut doc.data.model);
        let track = Arc::make_mut(&mut model.tracks[0]);
        track.name = "new".into();
    }
    assert_eq!(doc.data.model.tracks[0].name, "new");

    // Undo
    assert!(doc.undo());
    assert_eq!(doc.data.model.tracks[0].name, "old");
    assert!(doc.history.can_redo());

    // Redo
    assert!(doc.redo());
    assert_eq!(doc.data.model.tracks[0].name, "new");
    assert!(doc.history.can_undo());
}

#[test]
fn note_delta_undo_redo() {
    let mut doc = make_doc("test");
    // Add a note
    let note = NoteEvent {
        id: 0,
        start_tick: 0,
        end_tick: 480,
        key: 60,
        velocity: 100,
    };
    let key = 60;
    {
        let model = Arc::make_mut(&mut doc.data.model);
        Arc::make_mut(&mut model.notes[key as usize]).push(yinhe_types::Note {
            id: 0,
            start_tick: note.start_tick,
            end_tick: note.end_tick,
            velocity: note.velocity,
            track: 0,
        });
        model.mark_dirty(key);
        model.rebuild_dirty();
    }

    let removed = {
        let model = Arc::make_mut(&mut doc.data.model);
        let mut sel = Selection::default();
        sel.add_rect_track(0, 480, 60, 60, 0, u16::MAX);
        let r = crate::batch_ops::remove_selected(
            model,
            &sel,
        );
        model.rebuild_dirty();
        r
    };
    assert_eq!(removed.len(), 1);

    doc.history.push(UndoEntry {
        action: UndoAction::Notes(NoteDelta {
            before: removed,
            after: vec![],
        }),
        label: "delete",
        selected: Selection::default(),
        track_selected: HashSet::new(),
        sel_rect: SelRectState::default(),
    });

    // Note should be gone
    assert!(doc.data.model.notes[60].is_empty());

    // Undo
    assert!(doc.undo());
    assert_eq!(doc.data.model.notes[60].len(), 1);
    assert_eq!(doc.data.model.notes[60][0].start_tick, 0);

    // Redo
    assert!(doc.redo());
    assert!(doc.data.model.notes[60].is_empty());
}

#[test]
fn undo_returns_none_when_empty() {
    let mut doc = make_doc("x");
    assert!(!doc.undo());
}

#[test]
fn redo_returns_none_when_empty() {
    let mut doc = make_doc("x");
    assert!(!doc.redo());
}

#[test]
fn clear_wipes_everything() {
    let mut doc = make_doc("a");
    doc.history.push(UndoEntry {
        action: UndoAction::TrackName {
            track_idx: 0,
            old: "a".into(),
            new: "b".into(),
        },
        label: "rename",
        selected: Selection::default(),
        track_selected: HashSet::new(),
        sel_rect: SelRectState::default(),
    });
    doc.undo();
    assert!(doc.history.can_undo() || doc.history.can_redo());

    doc.history.clear();
    assert!(!doc.history.can_undo());
    assert!(!doc.history.can_redo());
    assert_eq!(doc.history.past.len(), 0);
    assert_eq!(doc.history.future.len(), 0);
}
