//! Arrange-view drag: move notes + automation across tracks.

use std::sync::Arc;

use yinhe_types::AutomationEvent;

use crate::batch_ops;
use crate::history::{AutomationDelta, NoteDelta, UndoAction};

use super::Document;

/// 把轨道索引偏移 `delta`（clamp 到合法范围），并跳过 conductor 轨
/// （音符/自动化事件不能落在它上面：向上移动时夹到第一条普通轨，向下时夹回前一条）。
fn offset_track_skip_conductor(
    raw: i32,
    delta: i32,
    num_tracks: i32,
    conductor_track_idx: Option<u16>,
) -> u16 {
    let raw_track = raw.clamp(0, num_tracks - 1);
    if Some(raw_track as u16) == conductor_track_idx {
        if delta < 0 {
            (raw_track + 1).min(num_tracks - 1) as u16
        } else {
            (raw_track - 1).max(0) as u16
        }
    } else {
        raw_track as u16
    }
}

impl Document {
    /// Move all selected notes and automation events by `(delta_ticks, delta_tracks)`.
    ///
    /// This is the single atomic operation for AR arrange drag. It:
    /// 1. Collects all notes in the selection (using original selection rects)
    /// 2. Removes them from the model
    /// 3. Re-inserts them at new tick + new track
    /// 4. Moves automation events (same track or cross-track)
    /// 5. Offsets the selection rects to follow
    ///
    /// Returns a single `Composite` UndoAction (or None if nothing moved).
    pub fn move_selected_arrange(
        &mut self,
        delta_ticks: i64,
        delta_tracks: i32,
    ) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        if delta_ticks == 0 && delta_tracks == 0 {
            return None;
        }

        let mut sub_actions: Vec<UndoAction> = Vec::new();
        let model = Arc::make_mut(&mut self.data.model);
        let num_tracks = model.tracks.len() as i32;
        let rects = self.edit.selected.rects.clone();

        // ── 1. Move notes (tick + track in one pass) ──
        // Collect originals, remove from model, re-insert at new positions.
        let originals = batch_ops::remove_selected(model, &self.edit.selected);
        if !originals.is_empty() {
            let allow_overlap = self.edit.allow_overlapping_notes;
            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
                std::collections::HashMap::new();
            // 被「禁止重叠」拦下而留在原处的音符不进 undo delta（位置没变），
            // before/after 只含真正移动的音符。
            let mut moved_before: Vec<(yinhe_types::Note, u8)> = Vec::new();
            let mut moved_after: Vec<(yinhe_types::Note, u8)> = Vec::new();
            for (note, old_key) in &originals {
                let new_tick = (note.start_tick as i64 + delta_ticks).max(0) as u32;
                // Skip over conductor track: notes cannot land on it.
                let new_track = offset_track_skip_conductor(
                    note.track as i32 + delta_tracks,
                    delta_tracks,
                    num_tracks,
                    self.edit.conductor_track_idx,
                );
                let length = note.end_tick - note.start_tick;
                // 「允许新重叠音符」关闭：目标位置与非本次移动的已有音符重叠
                // （移动集合已先移除，检查看不到它们）→ 该音符留在原处。
                if !allow_overlap
                    && batch_ops::has_overlapping_note(
                        model,
                        new_track,
                        *old_key,
                        new_tick,
                        new_tick + length,
                    )
                {
                    new_by_key.entry(*old_key).or_default().push(*note);
                    continue;
                }
                let moved = yinhe_types::Note {
                    id: note.id,
                    start_tick: new_tick,
                    end_tick: new_tick + length,
                    velocity: note.velocity,
                    track: new_track,
                };
                moved_before.push((*note, *old_key));
                moved_after.push((moved, *old_key));
                new_by_key.entry(*old_key).or_default().push(moved);
            }
            batch_ops::insert_batch(model, new_by_key);
            if !moved_before.is_empty() {
                sub_actions.push(UndoAction::Notes(NoteDelta {
                    before: moved_before,
                    after: moved_after,
                }));
            }
        }

        // ── 2. Move automation events (tick + track in one pass) ──
        // Collect per-lane: (src_track, lane_idx, target, moved_events, remaining_events)
        struct LaneMove {
            src_track: usize,
            lane_idx: usize,
            target: yinhe_types::AutomationTarget,
            events: Vec<AutomationEvent>,
            remaining: Vec<AutomationEvent>,
        }
        let mut lane_moves: Vec<LaneMove> = Vec::new();

        for &(tick_start, tick_end, _key_lo, _key_hi, track_lo, track_hi) in &rects {
            for track_idx in track_lo..=track_hi {
                let track_idx = track_idx as usize;
                if track_idx >= model.tracks.len() {
                    continue;
                }
                let track = Arc::make_mut(&mut model.tracks[track_idx]);
                for lane_idx in 0..track.automation_lanes.len() {
                    let lane = &track.automation_lanes[lane_idx];
                    let mut in_range: Vec<AutomationEvent> = Vec::new();
                    let mut out_of_range: Vec<AutomationEvent> = Vec::new();
                    for evt in lane.events.iter() {
                        if evt.tick >= tick_start && evt.tick < tick_end {
                            let mut moved = *evt;
                            moved.tick = (moved.tick as i64 + delta_ticks).max(0) as u32;
                            in_range.push(moved);
                        } else {
                            out_of_range.push(*evt);
                        }
                    }
                    if !in_range.is_empty() {
                        lane_moves.push(LaneMove {
                            src_track: track_idx,
                            lane_idx,
                            target: lane.target.clone(),
                            events: in_range,
                            remaining: out_of_range,
                        });
                    }
                }
            }
        }

        for lm in &lane_moves {
            // Source lane: replace with remaining
            let src_track = Arc::make_mut(&mut model.tracks[lm.src_track]);
            let src_lane = &mut src_track.automation_lanes[lm.lane_idx];
            let before_src = src_lane.events.clone();
            src_lane.events = lm.remaining.clone();

            if delta_tracks == 0 {
                // Same lane: add moved events back with offset ticks
                src_lane.events.extend(lm.events.iter().copied());
                src_lane.events.sort_by_key(|e| e.tick);
            }
            sub_actions.push(UndoAction::Automation(AutomationDelta {
                track_idx: lm.src_track,
                lane_idx: lm.lane_idx,
                target: lm.target.clone(),
                before: before_src,
                after: src_lane.events.clone(),
            }));
        }

        if delta_tracks != 0 {
            // Cross-track: add moved events to destination tracks
            for lm in &lane_moves {
                // Skip over conductor track: automation cannot land on it.
                let dst_track_idx = offset_track_skip_conductor(
                    lm.src_track as i32 + delta_tracks,
                    delta_tracks,
                    num_tracks,
                    self.edit.conductor_track_idx,
                ) as usize;
                if dst_track_idx == lm.src_track {
                    // 被夹回原轨：phase 1 已把被拖事件从源 lane 剔除（换成 remaining），
                    // 必须把它们加回源 lane，否则事件蒸发。补一个 AutomationDelta
                    // 记录这次"加回"，使 undo/redo 双向一致。
                    let src_track = Arc::make_mut(&mut model.tracks[lm.src_track]);
                    let src_lane = &mut src_track.automation_lanes[lm.lane_idx];
                    let before_readd = src_lane.events.clone();
                    src_lane.events.extend(lm.events.iter().copied());
                    src_lane.events.sort_by_key(|e| e.tick);
                    sub_actions.push(UndoAction::Automation(AutomationDelta {
                        track_idx: lm.src_track,
                        lane_idx: lm.lane_idx,
                        target: lm.target.clone(),
                        before: before_readd,
                        after: src_lane.events.clone(),
                    }));
                    continue;
                }
                let dst_track = Arc::make_mut(&mut model.tracks[dst_track_idx]);
                let dst_lane_idx = match dst_track
                    .automation_lanes
                    .iter()
                    .position(|l| l.target == lm.target)
                {
                    Some(idx) => idx,
                    None => {
                        dst_track
                            .automation_lanes
                            .push(yinhe_types::AutomationLane {
                                target: lm.target.clone(),
                                track: dst_track_idx as u16,
                                events: Vec::new(),
                            });
                        dst_track.automation_lanes.len() - 1
                    }
                };
                let dst_lane = &mut dst_track.automation_lanes[dst_lane_idx];
                let before_dst = dst_lane.events.clone();
                dst_lane.events.extend(lm.events.iter().copied());
                dst_lane.events.sort_by_key(|e| e.tick);
                sub_actions.push(UndoAction::Automation(AutomationDelta {
                    track_idx: dst_track_idx,
                    lane_idx: dst_lane_idx,
                    target: lm.target.clone(),
                    before: before_dst,
                    after: dst_lane.events.clone(),
                }));
            }
        }

        // ── 3. Offset selection rects to follow ──
        self.edit.selected.offset_ticks(delta_ticks);
        if delta_tracks != 0 {
            self.edit.selected.offset_tracks(delta_tracks);
        }

        model.rebuild_dirty();
        self.data.bump_revision();

        if sub_actions.is_empty() {
            None
        } else if sub_actions.len() == 1 {
            sub_actions.into_iter().next()
        } else {
            Some(UndoAction::Composite(sub_actions))
        }
    }

    /// Duplicate all selected notes and automation events, offsetting the copies
    /// by `(delta_ticks, delta_tracks)`. Originals stay untouched.
    ///
    /// AR Alt+拖动复制：原音符/原自动化事件保留，副本平移到新位置；
    /// 选区同步移到副本范围，便于连续 Alt+拖动。一步操作，一个 undo entry。
    pub fn duplicate_selected_arrange(
        &mut self,
        delta_ticks: i64,
        delta_tracks: i32,
    ) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        if delta_ticks == 0 && delta_tracks == 0 {
            return None;
        }

        let mut sub_actions: Vec<UndoAction> = Vec::new();
        let model = Arc::make_mut(&mut self.data.model);
        let num_tracks = model.tracks.len() as i32;
        let rects = self.edit.selected.rects.clone();

        // ── 1. 复制音符（原音符保留，副本平移到新 tick/新轨）──
        let selected_data = batch_ops::collect_selected(model, &self.edit.selected);
        if !selected_data.is_empty() {
            let allow_overlap = self.edit.allow_overlapping_notes;
            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
                std::collections::HashMap::new();
            for (note, old_key) in &selected_data {
                let new_tick = (note.start_tick as i64 + delta_ticks).max(0) as u32;
                // Skip over conductor track: notes cannot land on it.
                let new_track = offset_track_skip_conductor(
                    note.track as i32 + delta_tracks,
                    delta_tracks,
                    num_tracks,
                    self.edit.conductor_track_idx,
                );
                let length = note.end_tick - note.start_tick;
                // 「允许新重叠音符」关闭：副本与已有音符重叠 → 跳过该副本。
                if !allow_overlap
                    && batch_ops::has_overlapping_note(
                        model,
                        new_track,
                        *old_key,
                        new_tick,
                        new_tick + length,
                    )
                {
                    continue;
                }
                new_by_key
                    .entry(*old_key)
                    .or_default()
                    .push(yinhe_types::Note {
                        id: model.alloc_note_id(),
                        start_tick: new_tick,
                        end_tick: new_tick + length,
                        velocity: note.velocity,
                        track: new_track,
                    });
            }
            if !new_by_key.is_empty() {
                let after: Vec<(yinhe_types::Note, u8)> = new_by_key
                    .iter()
                    .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
                    .collect();
                batch_ops::insert_batch(model, new_by_key);
                sub_actions.push(UndoAction::Notes(NoteDelta {
                    before: vec![],
                    after,
                }));
            }
        }

        // ── 2. 复制自动化事件（原事件保留，副本平移到新 tick/新轨）──
        // 收集每个 lane 在选区内的原始事件（只读）。
        struct LaneCollect {
            src_track: usize,
            lane_idx: usize,
            target: yinhe_types::AutomationTarget,
            events: Vec<AutomationEvent>,
        }
        let mut lane_collects: Vec<LaneCollect> = Vec::new();
        for &(tick_start, tick_end, _key_lo, _key_hi, track_lo, track_hi) in &rects {
            for track_idx in track_lo..=track_hi {
                let track_idx = track_idx as usize;
                if track_idx >= model.tracks.len() {
                    continue;
                }
                let track = &model.tracks[track_idx];
                for lane_idx in 0..track.automation_lanes.len() {
                    let lane = &track.automation_lanes[lane_idx];
                    let in_range: Vec<AutomationEvent> = lane
                        .events
                        .iter()
                        .filter(|evt| evt.tick >= tick_start && evt.tick < tick_end)
                        .copied()
                        .collect();
                    if !in_range.is_empty() {
                        lane_collects.push(LaneCollect {
                            src_track: track_idx,
                            lane_idx,
                            target: lane.target.clone(),
                            events: in_range,
                        });
                    }
                }
            }
        }

        for lc in &lane_collects {
            let copies: Vec<AutomationEvent> = lc
                .events
                .iter()
                .map(|e| AutomationEvent {
                    tick: (e.tick as i64 + delta_ticks).max(0) as u32,
                    ..*e
                })
                .collect();
            if delta_tracks == 0 {
                // Same track: append copies to the source lane.
                let src_track = Arc::make_mut(&mut model.tracks[lc.src_track]);
                let lane = &mut src_track.automation_lanes[lc.lane_idx];
                let before = lane.events.clone();
                lane.events.extend(copies.iter().copied());
                lane.events.sort_by_key(|e| e.tick);
                sub_actions.push(UndoAction::Automation(AutomationDelta {
                    track_idx: lc.src_track,
                    lane_idx: lc.lane_idx,
                    target: lc.target.clone(),
                    before,
                    after: lane.events.clone(),
                }));
            } else {
                // Cross-track: append copies to the destination lane (create if missing).
                let dst_track_idx = offset_track_skip_conductor(
                    lc.src_track as i32 + delta_tracks,
                    delta_tracks,
                    num_tracks,
                    self.edit.conductor_track_idx,
                ) as usize;
                let dst_track = Arc::make_mut(&mut model.tracks[dst_track_idx]);
                let dst_lane_idx = match dst_track
                    .automation_lanes
                    .iter()
                    .position(|l| l.target == lc.target)
                {
                    Some(idx) => idx,
                    None => {
                        dst_track
                            .automation_lanes
                            .push(yinhe_types::AutomationLane {
                                target: lc.target.clone(),
                                track: dst_track_idx as u16,
                                events: Vec::new(),
                            });
                        dst_track.automation_lanes.len() - 1
                    }
                };
                let dst_lane = &mut dst_track.automation_lanes[dst_lane_idx];
                let before_dst = dst_lane.events.clone();
                dst_lane.events.extend(copies.iter().copied());
                dst_lane.events.sort_by_key(|e| e.tick);
                sub_actions.push(UndoAction::Automation(AutomationDelta {
                    track_idx: dst_track_idx,
                    lane_idx: dst_lane_idx,
                    target: lc.target.clone(),
                    before: before_dst,
                    after: dst_lane.events.clone(),
                }));
            }
        }

        if sub_actions.is_empty() {
            // 全部被「禁止重叠」拦下：不移动选区
            model.rebuild_dirty();
            return None;
        }

        // ── 3. Offset selection rects to follow ──
        self.edit.selected.offset_ticks(delta_ticks);
        if delta_tracks != 0 {
            self.edit.selected.offset_tracks(delta_tracks);
        }

        model.rebuild_dirty();
        self.data.bump_revision();

        if sub_actions.len() == 1 {
            sub_actions.into_iter().next()
        } else {
            Some(UndoAction::Composite(sub_actions))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_core::{ConductorData, NoteEvent, TrackData, YinModel};

    fn make_doc() -> Document {
        let model = YinModel {
            conductor: Arc::new(ConductorData::default()),
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        Document {
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
        }
    }

    fn add(doc: &mut Document, start: u32, end: u32, key: u8) {
        doc.add_note(
            0,
            NoteEvent {
                id: 0,
                start_tick: start,
                end_tick: end,
                key,
                velocity: 100,
            },
        );
    }

    /// move_selected_arrange：目标被非移动集合的已有音符占据的音符留在原处，
    /// 其余正常移动；undo delta 只含真正移动的音符。
    #[test]
    fn move_selected_arrange_partially_blocked_when_disallowed() {
        let mut doc = make_doc();
        add(&mut doc, 100, 200, 60); // A（选中）
        add(&mut doc, 100, 150, 62); // B（选中）
        add(&mut doc, 500, 600, 60); // C k60 占位（不选中）
        doc.edit.selected.add_rect_track(100, 201, 60, 62, 0, 0);
        doc.edit.allow_overlapping_notes = false;

        // +400 tick：A 目标 [500,600) 与 C 重叠 → 留原处；B 目标 k62 [500,550) → 移动
        let before_snap = doc.capture_snapshot();
        let action = doc
            .move_selected_arrange(400, 0)
            .expect("部分移动应产生 undo");
        match &action {
            UndoAction::Notes(delta) => {
                assert_eq!(delta.before.len(), 1, "delta 只含真正移动的 B");
                assert_eq!(delta.after.len(), 1);
                assert_eq!(delta.after[0].0.start_tick, 500);
            }
            other => panic!("期望 UndoAction::Notes，实际 {other:?}"),
        }
        assert_eq!(doc.data.model.notes[60].len(), 2, "A 留原处、C 不动");
        assert!(
            doc.data.model.notes[60]
                .iter()
                .any(|n| n.start_tick == 100 && n.end_tick == 200),
            "A 应留在原处"
        );
        assert!(
            doc.data.model.notes[62]
                .iter()
                .any(|n| n.start_tick == 500 && n.end_tick == 550),
            "B 应移到 [500,550)"
        );

        // undo/redo 回放不受开关拦截
        doc.push_undo(action, "move", before_snap);
        assert!(doc.undo(), "undo 应成功");
        assert!(
            doc.data.model.notes[62]
                .iter()
                .any(|n| n.start_tick == 100 && n.end_tick == 150),
            "undo 后 B 应回到 [100,150)"
        );
    }

    /// move_selected_arrange：全部被拦时音符不动、不产生 undo。
    #[test]
    fn move_selected_arrange_all_blocked() {
        let mut doc = make_doc();
        add(&mut doc, 100, 200, 60); // A（选中）
        add(&mut doc, 500, 600, 60); // C 占位
        doc.edit.selected.add_rect_track(100, 201, 60, 60, 0, 0);
        doc.edit.allow_overlapping_notes = false;

        assert!(
            doc.move_selected_arrange(400, 0).is_none(),
            "目标全被占据且无自动化移动时应返回 None",
        );
        assert_eq!(doc.data.model.notes[60].len(), 2);
        assert!(
            doc.data.model.notes[60]
                .iter()
                .any(|n| n.start_tick == 100 && n.end_tick == 200),
            "A 应留在原处",
        );
    }

    /// 默认允许重叠：AR 移动照常（现状行为）。
    #[test]
    fn move_selected_arrange_allows_overlap_by_default() {
        let mut doc = make_doc();
        add(&mut doc, 100, 200, 60);
        add(&mut doc, 500, 600, 60);
        doc.edit.selected.add_rect_track(100, 201, 60, 60, 0, 0);
        assert!(
            doc.move_selected_arrange(400, 0).is_some(),
            "默认应允许重叠移动"
        );
        assert!(
            doc.data.model.notes[60]
                .iter()
                .any(|n| n.start_tick == 500 && n.end_tick == 600),
        );
    }
}
