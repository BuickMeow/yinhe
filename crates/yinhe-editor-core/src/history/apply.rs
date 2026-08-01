//! Undo/redo application logic.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use yinhe_types::{AutomationEvent, Note};

use crate::document::Document;

use super::UndoAction;

// ---------------------------------------------------------------------------
// UndoAction apply
// ---------------------------------------------------------------------------

impl UndoAction {
    /// Apply the forward action (used by redo).
    pub fn redo(&self, doc: &mut Document) {
        match self {
            UndoAction::Notes(delta) => apply_note_delta(doc, &delta.before, &delta.after),
            UndoAction::Automation(delta) => apply_automation_delta(
                doc,
                delta.track_idx,
                delta.lane_idx,
                &delta.target,
                &delta.after,
            ),
            UndoAction::TrackName { track_idx, old: _, new } => {
                let model = Arc::make_mut(&mut doc.data.model);
                if let Some(track) = model.tracks.get_mut(*track_idx) {
                    let track = Arc::make_mut(track);
                    track.name = new.clone();
                    // SMF 标准：track 0 的 TrackName = song title。
                    // 编辑 track 0 name 时同步到 meta.name，保持一致。
                    if *track_idx == 0 {
                        model.meta.name = new.clone();
                    }
                }
            }
            UndoAction::ProjectName { old: _, new } => {
                let model = Arc::make_mut(&mut doc.data.model);
                model.meta.name = new.clone();
                // SMF 标准：track 0 的 TrackName = song title。
                // 编辑 project name 时同步到 track 0 name，保持一致。
                if let Some(track) = model.tracks.get_mut(0) {
                    Arc::make_mut(track).name = new.clone();
                }
            }
            UndoAction::ProjectArtist { old: _, new } => {
                let model = Arc::make_mut(&mut doc.data.model);
                model.meta.artist = new.clone();
            }
            UndoAction::ProjectDescription { old: _, new } => {
                let model = Arc::make_mut(&mut doc.data.model);
                model.meta.description = new.clone();
            }
            UndoAction::ProjectPpq { old: _, new, rescale } => {
                let model = Arc::make_mut(&mut doc.data.model);
                if *rescale {
                    model.rescale_ppq(*new);
                    doc.data.bump_revision();
                } else {
                    model.meta.ppq = *new;
                    model.rebuild_tempo_map();
                }
            }
            UndoAction::CompressionLevel { old: _, new } => {
                let model = Arc::make_mut(&mut doc.data.model);
                model.meta.compression_level = *new;
            }
            UndoAction::EventList(delta) => apply_event_list_delta(doc, delta),
            UndoAction::TrackStructure {
                tracks_before,
                tracks_after,
                note_remap,
                note_remap_inverse: _,
                deleted_notes,
            } => {
                let model = Arc::make_mut(&mut doc.data.model);
                model.tracks = tracks_after.clone();
                for bucket in model.notes.iter_mut() {
                    let bucket = Arc::make_mut(bucket);
                    // 越界音符（track 字段 >= note_remap.len()）按删除处理，避免 panic（规则 17）。
                    bucket.retain(|n| note_remap.get(n.track as usize).copied().unwrap_or(u16::MAX) != u16::MAX);
                    for note in bucket.iter_mut() {
                        note.track = note_remap.get(note.track as usize).copied().unwrap_or(u16::MAX);
                    }
                }
                // undo remove_track 时 tracks_after 比 tracks_before 长，
                // 此时需要把先前被物理删除的音符插回模型。
                // add_track 的 undo（tracks_after 更短）与 move_track（等长）
                // 都不会进入此分支，deleted_notes 为空时也安全。
                if tracks_after.len() > tracks_before.len() && !deleted_notes.is_empty() {
                    let mut by_key: HashMap<u8, Vec<Note>> = HashMap::new();
                    for (note, key) in deleted_notes {
                        by_key.entry(*key).or_default().push(*note);
                    }
                    for (key, notes) in by_key {
                        let k = key as usize;
                        Arc::make_mut(&mut model.notes[k]).extend(notes);
                    }
                }
                model.rebuild();
                doc.data.bump_revision();
                doc.sync_track_caches();
            }
            UndoAction::Composite(actions) => {
                for action in actions {
                    action.redo(doc);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Apply helpers
// ---------------------------------------------------------------------------

/// Remove `remove` notes and insert `insert` notes into the model.
///
/// Notes in `remove` are matched by their全局唯一 `id`。
pub(crate) fn apply_note_delta(doc: &mut Document, remove: &[(Note, u8)], insert: &[(Note, u8)]) {
    if remove.is_empty() && insert.is_empty() {
        return;
    }
    let model = Arc::make_mut(&mut doc.data.model);

    // Remove notes matching `remove`, grouped by key for a single retain per bucket.
    let mut remove_by_key: HashMap<u8, HashSet<u32>> = HashMap::new();
    for (note, key) in remove {
        remove_by_key.entry(*key).or_default().insert(note.id);
    }
    for (key, to_remove) in &remove_by_key {
        let k = *key as usize;
        Arc::make_mut(&mut model.notes[k]).retain(|n| !to_remove.contains(&n.id));
        model.mark_dirty(*key);
    }

    // Insert `insert` notes, grouped by key.
    let mut by_key: HashMap<u8, Vec<Note>> = HashMap::new();
    for (note, key) in insert {
        by_key.entry(*key).or_default().push(*note);
    }
    for (key, notes) in by_key {
        let k = key as usize;
        Arc::make_mut(&mut model.notes[k]).extend(notes);
        model.mark_dirty(key);
    }

    model.rebuild_dirty();
    doc.data.bump_revision();
}

/// Replace the event list of `track_idx`'s `lane_idx` with `events`.
///
/// Tempo 走 `conductor.tempo` 路径（`track_idx`/`lane_idx` 被忽略）；
/// 其他 target 走 `track.automation_lanes[lane_idx]` 路径。
pub(crate) fn apply_automation_delta(
    doc: &mut Document,
    track_idx: usize,
    lane_idx: usize,
    target: &yinhe_types::AutomationTarget,
    events: &[AutomationEvent],
) {
    let model = Arc::make_mut(&mut doc.data.model);
    if matches!(target, yinhe_types::AutomationTarget::Tempo) {
        let conductor = Arc::make_mut(&mut model.conductor);
        let lane = &mut conductor.tempo;
        lane.events.clear();
        lane.events.extend_from_slice(events);
        lane.events.sort_by_key(|e| e.tick);
    } else if let Some(track) = model.tracks.get_mut(track_idx) {
        let track = Arc::make_mut(track);
        if let Some(lane) = track.automation_lanes.get_mut(lane_idx) {
            lane.events.clear();
            lane.events.extend_from_slice(events);
            // 保持有序（编辑操作应已保证，但防御性排序）
            lane.events.sort_by_key(|e| e.tick);
        }
    }
    // Tempo 改了要重建 tempo_map（否则音频引擎和播放光标都用旧 tempo）
    if matches!(target, yinhe_types::AutomationTarget::Tempo) {
        doc.data.rebuild_tempo_map();
    }
    doc.data.bump_revision();
}

/// 把 `EventListDelta::new` 写回到 `target` 指定的事件列表字段。
///
/// 8 种事件列表共用一个 undo 变体后，所有"写到哪"的逻辑集中在这里。
/// 写入后：
/// - TimeSig 需要重建 TempoMap（tempo 依赖 time_sig）。
/// - 其他事件列表只 bump_revision。
fn apply_event_list_delta(doc: &mut Document, delta: &super::EventListDelta) {
    use super::{EventListItem, EventListTarget};
    let model = Arc::make_mut(&mut doc.data.model);
    let needs_tempo_rebuild = matches!(delta.target, EventListTarget::TimeSig);
    match delta.target {
        EventListTarget::TimeSig => {
            let conductor = Arc::make_mut(&mut model.conductor);
            conductor.time_sig = delta.new.iter().filter_map(|i| match i {
                EventListItem::TimeSig(e) => Some(e.clone()),
                _ => None,
            }).collect();
        }
        EventListTarget::KeySig => {
            let conductor = Arc::make_mut(&mut model.conductor);
            conductor.key_sig = delta.new.iter().filter_map(|i| match i {
                EventListItem::KeySig(e) => Some(e.clone()),
                _ => None,
            }).collect();
        }
        EventListTarget::Marker => {
            let conductor = Arc::make_mut(&mut model.conductor);
            conductor.markers = delta.new.iter().filter_map(|i| match i {
                EventListItem::Marker(e) => Some(e.clone()),
                _ => None,
            }).collect();
        }
        EventListTarget::ConductorLyrics => {
            let conductor = Arc::make_mut(&mut model.conductor);
            conductor.lyrics = delta.new.iter().filter_map(|i| match i {
                EventListItem::Lyrics(e) => Some(e.clone()),
                _ => None,
            }).collect();
        }
        EventListTarget::ConductorChord => {
            let conductor = Arc::make_mut(&mut model.conductor);
            conductor.chord = delta.new.iter().filter_map(|i| match i {
                EventListItem::Chord(e) => Some(e.clone()),
                _ => None,
            }).collect();
        }
        EventListTarget::Lyrics { track } => {
            if let Some(t) = model.tracks.get_mut(track as usize) {
                let t = Arc::make_mut(t);
                t.lyrics = delta.new.iter().filter_map(|i| match i {
                    EventListItem::Lyrics(e) => Some(e.clone()),
                    _ => None,
                }).collect();
            }
        }
        EventListTarget::Chord { track } => {
            if let Some(t) = model.tracks.get_mut(track as usize) {
                let t = Arc::make_mut(t);
                t.chord = delta.new.iter().filter_map(|i| match i {
                    EventListItem::Chord(e) => Some(e.clone()),
                    _ => None,
                }).collect();
            }
        }
        EventListTarget::ProgramChange { track } => {
            if let Some(t) = model.tracks.get_mut(track as usize) {
                let t = Arc::make_mut(t);
                t.program_change = delta.new.iter().filter_map(|i| match i {
                    EventListItem::ProgramChange(e) => Some(*e),
                    _ => None,
                }).collect();
            }
        }
    }
    if needs_tempo_rebuild {
        doc.data.rebuild_tempo_map();
    }
    doc.data.bump_revision();
}
