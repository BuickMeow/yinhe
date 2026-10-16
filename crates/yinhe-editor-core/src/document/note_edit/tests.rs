use std::sync::Arc;

use yinhe_core::{ConductorData, NoteEvent, TrackData, YinModel};
use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape, TimeSigEvent};

use crate::document::Document;
use crate::history::UndoAction;

use super::*;

mod add_delete;
mod duplicate;
mod field_edit;
mod humanize_cleanup;
mod move_resize;
mod tools;
mod transform;

fn make_doc_with_note() -> Document {
    let model = YinModel {
        conductor: Arc::new(ConductorData {
            tempo: AutomationLane {
                target: AutomationTarget::Tempo,
                track: 0,
                events: vec![AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 120.0,
                    shape: SegmentShape::Step,
                }],
            },
            time_sig: vec![TimeSigEvent {
                tick: 0,
                numerator: 4,
                denominator: 2,
            }],
            key_sig: Vec::new(),
            markers: Vec::new(),
            lyrics: Vec::new(),
            chord: Vec::new(),
        }),
        tracks: vec![Arc::new({
            let mut t = TrackData::new(0, 0);
            t.name = "t".to_string();
            t
        })],
        ..Default::default()
    };
    let mut doc = Document {
        data: crate::project_data::ProjectData::new(
            Arc::new(model),
            Default::default(),
            Default::default(),
        ),
        edit: crate::edit_state::EditState {
            track_visible: vec![true],
            track_pianoroll_visible: vec![true],
            ..Default::default()
        },
        history: crate::history::UndoStack::new(),
        file_name: "test".into(),
        file_path: None,
        mixer: Default::default(),
        mixer_dirty: false,
        doc_id: 0,
    };
    // 加一个音符 (tick 100~200, key 60)
    doc.add_note(
        0,
        NoteEvent {
            id: 0,
            start_tick: 100,
            end_tick: 200,
            key: 60,
            velocity: 100,
        },
    );
    // 选中它
    doc.edit.selected.add_rect_track(100, 201, 60, 60, 0, 0);
    doc
}

fn ev(start: u32, end: u32, key: u8) -> NoteEvent {
    NoteEvent {
        id: 0,
        start_tick: start,
        end_tick: end,
        key,
        velocity: 100,
    }
}

/// 多轨测试文档（track_visible 与轨道数同步；含 make_doc_with_note 的初始音符）。
fn make_doc_tracks(n: usize) -> Document {
    let mut doc = make_doc_with_note();
    let model = Arc::make_mut(&mut doc.data.model);
    while model.tracks.len() < n {
        let i = model.tracks.len();
        model.tracks.push(Arc::new(TrackData::new(0, i as u8)));
    }
    doc.edit.track_visible = vec![true; n];
    doc.edit.track_pianoroll_visible = vec![true; n];
    doc
}

/// 音符字段快照：(id, start, end, velocity, track)。
type NoteTuple = (u32, u32, u32, u8, u16);

/// 全桶字段快照（Note 无 PartialEq，用元组比较）。
fn note_tuples(doc: &Document) -> Vec<Vec<NoteTuple>> {
    (0..yinhe_types::KEY_COUNT)
        .map(|k| {
            doc.data.model.notes[k]
                .iter()
                .map(|n| (n.id, n.start_tick, n.end_tick, n.velocity, n.track))
                .collect()
        })
        .collect()
}

/// 某 key 桶的 (start, end, track) 快照（桶内顺序）。
fn spans(doc: &Document, key: u8) -> Vec<(u32, u32, u16)> {
    doc.data.model.notes[key as usize]
        .iter()
        .map(|n| (n.start_tick, n.end_tick, n.track))
        .collect()
}

/// 设置音符力度（测试用）。
fn set_velocity(doc: &mut Document, key: u8, id: u32, velocity: u8) {
    let model = Arc::make_mut(&mut doc.data.model);
    let bucket = Arc::make_mut(&mut model.notes[key as usize]);
    if let Some(n) = bucket.find_mut(id) {
        n.velocity = velocity;
    }
}
