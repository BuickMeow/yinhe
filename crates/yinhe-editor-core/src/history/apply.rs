//! Undo/redo application logic.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use yinhe_core::Selection;
use yinhe_types::{AutomationEvent, MAX_KEY, Note};

use crate::document::{Document, track_color};

use super::UndoAction;

// ---------------------------------------------------------------------------
// UndoAction apply
// ---------------------------------------------------------------------------

impl UndoAction {
    /// Apply the forward action (used by redo).
    pub fn redo(&self, doc: &mut Document) {
        match self {
            UndoAction::Notes(delta) => apply_note_delta(doc, &delta.before, &delta.after),
            // 操作式 undo：按 rects 重放几何变换（不存数据副本）。
            UndoAction::MoveNotes {
                rects,
                delta_ticks,
                delta_keys,
            } => apply_note_shift(doc, rects, *delta_ticks, *delta_keys),
            UndoAction::FlipNotes {
                rects,
                bounds,
                axis,
            } => apply_note_flip(doc, rects, *bounds, *axis),
            UndoAction::Automation(delta) => apply_automation_delta(
                doc,
                delta.track_idx,
                delta.lane_idx,
                &delta.target,
                &delta.before,
                &delta.after,
            ),
            UndoAction::AutomationLane {
                track_idx,
                lane_idx,
                before: _,
                after,
            } => apply_automation_lane_delta(doc, *track_idx, *lane_idx, after.as_ref()),
            UndoAction::TrackName {
                track_idx,
                old: _,
                new,
            } => {
                let model = Arc::make_mut(&mut doc.data.model);
                if let Some(track) = model.tracks.get_mut(*track_idx) {
                    let track = Arc::make_mut(track);
                    track.name = new.clone();
                    // 同步显示缓存（AR/PR 轨道列表、info panel 读它）。
                    if let Some(ti) = doc.edit.track_info_cache.get_mut(*track_idx) {
                        ti.name = new.clone();
                    }
                    // SMF 标准：track 0 的 TrackName = song title。
                    // 编辑 track 0 name 时同步到 meta.name，保持一致。
                    if *track_idx == 0 {
                        model.meta.name = new.clone();
                    }
                }
            }
            UndoAction::TrackColor {
                track_idx,
                old: _,
                new,
            } => {
                let model = Arc::make_mut(&mut doc.data.model);
                if let Some(track) = model.tracks.get_mut(*track_idx) {
                    let track = Arc::make_mut(track);
                    track.color = *new;
                    // 显示缓存：显式颜色优先，默认占位色回落调色板
                    // （与 document::track_color 一致，重置颜色后 undo/redo 也正确）。
                    if let Some(c) = doc.edit.track_colors_cache.get_mut(*track_idx) {
                        *c = track_color(track, *track_idx, doc.edit.conductor_track_idx);
                    }
                }
                doc.data.bump_revision();
            }
            UndoAction::ProjectName { old: _, new } => {
                let model = Arc::make_mut(&mut doc.data.model);
                model.meta.name = new.clone();
                // SMF 标准：track 0 的 TrackName = song title。
                // 编辑 project name 时同步到 track 0 name，保持一致。
                if let Some(track) = model.tracks.get_mut(0) {
                    Arc::make_mut(track).name = new.clone();
                    if let Some(ti) = doc.edit.track_info_cache.get_mut(0) {
                        ti.name = new.clone();
                    }
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
            UndoAction::ProjectPpq {
                old: _,
                new,
                rescale,
            } => {
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
                    bucket.retain(|n| {
                        note_remap
                            .get(n.track as usize)
                            .copied()
                            .unwrap_or(u16::MAX)
                            != u16::MAX
                    });
                    for note in bucket.iter_mut() {
                        note.track = note_remap
                            .get(note.track as usize)
                            .copied()
                            .unwrap_or(u16::MAX);
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
                    crate::batch_ops::insert_batch(model, by_key);
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
        Arc::make_mut(&mut model.notes[k]).remove_by_ids(to_remove);
        model.mark_dirty(*key);
    }

    // Insert `insert` notes, grouped by key (keeps buckets sorted).
    let mut by_key: HashMap<u8, Vec<Note>> = HashMap::new();
    for (note, key) in insert {
        by_key.entry(*key).or_default().push(*note);
    }
    crate::batch_ops::insert_batch(model, by_key);

    model.rebuild_dirty();
    doc.data.bump_revision();
}

/// 操作式 undo：按 rects 收集音符，统一平移 (delta_ticks, delta_keys)。
///
/// redo 方向：rects = 操作前位置，音符在此，施加 +delta。
/// undo 方向：`reversed()` 已把 rects 平移到操作后位置、delta 取反，
/// 音符在此，施加 −delta——与 redo 共用同一实现。
///
/// 与 `Document::move_selected_notes` 的模型操作一致（收集→变换→删原→插新），
/// 但不碰选区状态。前提：操作不触发 clamp（生成端已检测回退副本制）。
pub(crate) fn apply_note_shift(
    doc: &mut Document,
    rects: &[(u32, u32, u8, u8, u16, u16)],
    delta_ticks: i64,
    delta_keys: i32,
) {
    if delta_ticks == 0 && delta_keys == 0 {
        return;
    }
    let model = Arc::make_mut(&mut doc.data.model);
    let sel = Selection {
        rects: rects.to_vec(),
    };
    let originals = crate::batch_ops::collect_selected(model, &sel);
    if originals.is_empty() {
        return;
    }
    let mut new_by_key: HashMap<u8, Vec<Note>> = HashMap::new();
    for (note, old_key) in &originals {
        let new_key = ((*old_key as i32) + delta_keys).clamp(0, MAX_KEY as i32) as u8;
        let new_start = (note.start_tick as i64 + delta_ticks).max(0) as u32;
        let length = note.end_tick - note.start_tick;
        let moved = Note {
            start_tick: new_start,
            end_tick: new_start + length,
            ..*note
        };
        new_by_key.entry(new_key).or_default().push(moved);
    }
    crate::batch_ops::remove_selected(model, &sel);
    crate::batch_ops::insert_batch(model, new_by_key);
    model.rebuild_dirty();
    doc.data.bump_revision();
}

/// 操作式 undo：按 rects 收集音符，以 bounds 为镜像边界翻转（两次镜像恒等）。
///
/// 前提：所有音符都在选框内（跨出选框的镜像会触发 clamp，生成端已
/// 检测回退副本制）。
pub(crate) fn apply_note_flip(
    doc: &mut Document,
    rects: &[(u32, u32, u8, u8, u16, u16)],
    bounds: (u64, u64, u8, u8),
    axis: crate::document::note_edit::FlipAxis,
) {
    let (t0, t1, kl, kh) = bounds;
    let model = Arc::make_mut(&mut doc.data.model);
    let sel = Selection {
        rects: rects.to_vec(),
    };
    let originals = crate::batch_ops::collect_selected(model, &sel);
    if originals.is_empty() {
        return;
    }
    let mirror_tick = |v: u32| -> u32 { (t0 as i64 + (t1 as i64 - v as i64)).max(0) as u32 };
    let mut new_by_key: HashMap<u8, Vec<Note>> = HashMap::new();
    for (note, old_key) in &originals {
        match axis {
            crate::document::note_edit::FlipAxis::Horizontal => {
                let new_start = mirror_tick(note.end_tick);
                let new_end = mirror_tick(note.start_tick).max(new_start + 1);
                new_by_key.entry(*old_key).or_default().push(Note {
                    start_tick: new_start,
                    end_tick: new_end,
                    ..*note
                });
            }
            crate::document::note_edit::FlipAxis::Vertical => {
                let new_key =
                    (kl as i32 + kh as i32 - *old_key as i32).clamp(0, MAX_KEY as i32) as u8;
                new_by_key.entry(new_key).or_default().push(*note);
            }
        }
    }
    crate::batch_ops::remove_selected(model, &sel);
    crate::batch_ops::insert_batch(model, new_by_key);
    model.rebuild_dirty();
    doc.data.bump_revision();
}

/// 增量应用自动化事件 delta：删除 `remove`（按 tick 匹配），插入 `insert`
/// （排序后归并，保持 lane 按 tick 有序）。
///
/// redo 传 `(before, after)`；undo 经 `reversed()` 交换后同样调用。
/// 兼容旧的全量快照 entry：全量快照的 before/after 覆盖整个 lane，
/// 增量语义退化为“删全部旧 + 插全部新”= 整体替换。
pub(crate) fn apply_automation_delta(
    doc: &mut Document,
    track_idx: usize,
    lane_idx: usize,
    target: &yinhe_types::AutomationTarget,
    remove: &[AutomationEvent],
    insert: &[AutomationEvent],
) {
    let model = Arc::make_mut(&mut doc.data.model);
    if matches!(target, yinhe_types::AutomationTarget::Tempo) {
        let conductor = Arc::make_mut(&mut model.conductor);
        let lane = &mut conductor.tempo;
        apply_event_diff(&mut lane.events, remove, insert);
    } else if let Some(track) = model.tracks.get_mut(track_idx) {
        let track = Arc::make_mut(track);
        if let Some(lane) = track.automation_lanes.get_mut(lane_idx) {
            apply_event_diff(&mut lane.events, remove, insert);
        }
    }
    // Tempo 改了要重建 tempo_map（否则音频引擎和播放光标都用旧 tempo）
    if matches!(target, yinhe_types::AutomationTarget::Tempo) {
        doc.data.rebuild_tempo_map();
    }
    doc.data.bump_revision();
}

/// 应用 automation lane 结构变化（创建/删除整条 lane）。
///
/// after = Some(lane)：在 lane_idx 处插入 lane（越界时 clamp 到末尾），
/// lane.track 校正为 track_idx。after = None：移除 lane_idx 处的 lane
/// （越界时静默忽略，不 panic——规则 17）。
///
/// 不支持 Tempo lane：conductor 的 Tempo 存于 conductor.tempo，不在
/// track.automation_lanes，调用方不会传 Tempo 进来。非 Tempo lane 变化
/// 不影响 tempo_map / 音频，与 apply_automation_delta 的非 Tempo 路径
/// 一致，只需 bump_revision。
fn apply_automation_lane_delta(
    doc: &mut Document,
    track_idx: usize,
    lane_idx: usize,
    after: Option<&yinhe_types::AutomationLane>,
) {
    let model = Arc::make_mut(&mut doc.data.model);
    if let Some(track) = model.tracks.get_mut(track_idx) {
        let track = Arc::make_mut(track);
        match after {
            Some(lane) => {
                let mut lane = lane.clone();
                lane.track = track_idx as u16;
                // 越界防御：clamp 到末尾（调用方保证语义，这里只保证不 panic）。
                let idx = lane_idx.min(track.automation_lanes.len());
                track.automation_lanes.insert(idx, lane);
            }
            None => {
                if lane_idx < track.automation_lanes.len() {
                    track.automation_lanes.remove(lane_idx);
                }
            }
        }
    }
    doc.data.bump_revision();
}

/// 对一个按 tick 排序的事件列表做“删 remove + 插 insert”。
/// 删除按 tick 匹配（lane 内 tick 唯一）；插入排序后双路归并（O(N + K)）。
fn apply_event_diff(
    events: &mut Vec<AutomationEvent>,
    remove: &[AutomationEvent],
    insert: &[AutomationEvent],
) {
    if !remove.is_empty() {
        let remove_ticks: HashSet<u32> = remove.iter().map(|e| e.tick).collect();
        events.retain(|e| !remove_ticks.contains(&e.tick));
    }
    if !insert.is_empty() {
        let mut new_events = insert.to_vec();
        new_events.sort_by_key(|e| e.tick);
        let old = std::mem::take(events);
        let mut merged = Vec::with_capacity(old.len() + new_events.len());
        let (mut i, mut j) = (0usize, 0usize);
        while i < old.len() && j < new_events.len() {
            if old[i].tick <= new_events[j].tick {
                merged.push(old[i]);
                i += 1;
            } else {
                merged.push(new_events[j]);
                j += 1;
            }
        }
        merged.extend_from_slice(&old[i..]);
        merged.extend_from_slice(&new_events[j..]);
        *events = merged;
    }
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
            conductor.time_sig = delta
                .new
                .iter()
                .filter_map(|i| match i {
                    EventListItem::TimeSig(e) => Some(e.clone()),
                    _ => None,
                })
                .collect();
        }
        EventListTarget::KeySig => {
            let conductor = Arc::make_mut(&mut model.conductor);
            conductor.key_sig = delta
                .new
                .iter()
                .filter_map(|i| match i {
                    EventListItem::KeySig(e) => Some(e.clone()),
                    _ => None,
                })
                .collect();
        }
        EventListTarget::Marker => {
            let conductor = Arc::make_mut(&mut model.conductor);
            conductor.markers = delta
                .new
                .iter()
                .filter_map(|i| match i {
                    EventListItem::Marker(e) => Some(e.clone()),
                    _ => None,
                })
                .collect();
        }
        EventListTarget::ConductorLyrics => {
            let conductor = Arc::make_mut(&mut model.conductor);
            conductor.lyrics = delta
                .new
                .iter()
                .filter_map(|i| match i {
                    EventListItem::Lyrics(e) => Some(e.clone()),
                    _ => None,
                })
                .collect();
        }
        EventListTarget::ConductorChord => {
            let conductor = Arc::make_mut(&mut model.conductor);
            conductor.chord = delta
                .new
                .iter()
                .filter_map(|i| match i {
                    EventListItem::Chord(e) => Some(e.clone()),
                    _ => None,
                })
                .collect();
        }
        EventListTarget::Lyrics { track } => {
            if let Some(t) = model.tracks.get_mut(track as usize) {
                let t = Arc::make_mut(t);
                t.lyrics = delta
                    .new
                    .iter()
                    .filter_map(|i| match i {
                        EventListItem::Lyrics(e) => Some(e.clone()),
                        _ => None,
                    })
                    .collect();
            }
        }
        EventListTarget::Chord { track } => {
            if let Some(t) = model.tracks.get_mut(track as usize) {
                let t = Arc::make_mut(t);
                t.chord = delta
                    .new
                    .iter()
                    .filter_map(|i| match i {
                        EventListItem::Chord(e) => Some(e.clone()),
                        _ => None,
                    })
                    .collect();
            }
        }
        EventListTarget::ProgramChange { track } => {
            if let Some(t) = model.tracks.get_mut(track as usize) {
                let t = Arc::make_mut(t);
                t.program_change = delta
                    .new
                    .iter()
                    .filter_map(|i| match i {
                        EventListItem::ProgramChange(e) => Some(*e),
                        _ => None,
                    })
                    .collect();
            }
        }
    }
    if needs_tempo_rebuild {
        doc.data.rebuild_tempo_map();
    }
    doc.data.bump_revision();
}
