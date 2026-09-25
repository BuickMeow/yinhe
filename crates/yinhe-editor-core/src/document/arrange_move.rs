//! Arrange-view drag: move notes + automation across tracks.

use std::sync::Arc;

use yinhe_types::AutomationEvent;

use crate::batch_ops;
use crate::history::{AutomationDelta, NoteDelta, UndoAction};

use super::Document;

/// 把轨道索引偏移 `delta`（clamp 到合法范围），并跳过 conductor 轨
/// （音符/自动化事件不能落在它上面：向上移动时夹到第一条普通轨，向下时夹回前一条）。
pub(crate) fn offset_track_skip_conductor(
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
        let num_tracks = self.data.model.tracks.len() as i32;
        let selection = self.edit.selected.clone();
        let allow_overlap = self.edit.allow_overlapping_notes;
        let conductor = self.edit.conductor_track_idx;

        // ── 0. 快速路径判定（流式只读，不物化）──
        // 条件：无 tick clamp、轨道未被夹取/跳过 conductor、目标无重叠。
        // 满足时音符部分逐桶原地改（1.64 亿全选拖动峰值从 ~6.5GB 降到桶级）。
        let mut hit_any = false;
        let fast_ok = {
            let model = &self.data.model;
            let mut ok = true;
            batch_ops::for_each_selected(model, &selection, |n, _| {
                hit_any = true;
                let ns = n.start_tick as i64 + delta_ticks;
                let ne = n.end_tick as i64 + delta_ticks;
                if ns < 0 || ne > u32::MAX as i64 {
                    ok = false;
                    return;
                }
                let nt = offset_track_skip_conductor(
                    n.track as i32 + delta_tracks,
                    delta_tracks,
                    num_tracks,
                    conductor,
                );
                if nt as i32 != n.track as i32 + delta_tracks {
                    ok = false;
                }
            });
            if !ok {
                false
            } else if allow_overlap {
                true
            } else {
                // 目标区域（tick + track 平移后的选区）是否有「不属于本次移动集合」的音符。
                let dest_sel =
                    selection.dest_overlap_selection(delta_ticks, 0, delta_tracks, num_tracks - 1);
                !batch_ops::compute_dest_overlap(model, &dest_sel, Some(&selection))
            }
        };

        let model = Arc::make_mut(&mut self.data.model);

        // ── 1. Move notes (tick + track in one pass) ──
        if fast_ok {
            // 原地改：track 只是 Note 字段，不换桶；start 变化后重排桶。
            batch_ops::update_selected_in_place(model, &selection, |n, _k| {
                let length = n.end_tick - n.start_tick;
                n.start_tick = (n.start_tick as i64 + delta_ticks) as u32;
                n.end_tick = n.start_tick + length;
                n.track = offset_track_skip_conductor(
                    n.track as i32 + delta_tracks,
                    delta_tracks,
                    num_tracks,
                    conductor,
                );
            });
            if hit_any {
                sub_actions.push(UndoAction::ArrangeMoveNotes {
                    selection: selection.clone(),
                    delta_ticks,
                    delta_tracks,
                });
            }
        } else {
            // Collect originals, remove from model, re-insert at new positions.
            let originals = batch_ops::remove_selected(model, &self.edit.selected);
            if !originals.is_empty() {
                let behavior = self.edit.overlap_blocked_behavior;
                let mut tally = batch_ops::MoveTally::default();
                for (note, old_key) in &originals {
                    let new_tick = (note.start_tick as i64 + delta_ticks).max(0) as u32;
                    // Skip over conductor track: notes cannot land on it.
                    let new_track = offset_track_skip_conductor(
                        note.track as i32 + delta_tracks,
                        delta_tracks,
                        num_tracks,
                        self.edit.conductor_track_idx,
                    );
                    let original = (*note, *old_key);
                    // 「允许新重叠音符」关闭：目标位置与非本次移动的已有音符重叠
                    // （移动集合已先移除，检查看不到它们）→ 按 behavior 处理。
                    if !allow_overlap
                        && batch_ops::move_one_note_blocked(
                            model, &original, *old_key, new_tick, new_track, behavior, &mut tally,
                        )
                    {
                        continue;
                    }
                    tally.push_moved(&original, *old_key, new_tick, new_track);
                }
                batch_ops::insert_batch(model, tally.new_by_key);
                let delta = batch_ops::compose_move_delta(
                    tally.moved_before,
                    tally.deleted_before,
                    tally.replaced_before,
                    tally.moved_after,
                );
                // KeepOriginal 全拦 → before 为空（无 moved/deleted/replaced），无需 undo。
                if !delta.before.is_empty() {
                    sub_actions.push(UndoAction::Notes(delta));
                }
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
        // 判定（accepts_automation_event）本身覆盖整个选区，每个 lane 只需处理
        // 一次——多个重叠 rect 命中同一 lane 时不得重复搬运/重复记 undo。
        let mut seen_lanes: std::collections::HashSet<(usize, usize)> =
            std::collections::HashSet::new();

        for &(_tick_start, _tick_end, _key_lo, _key_hi, track_lo, track_hi) in &selection.rects {
            for track_idx in track_lo..=track_hi {
                let track_idx = track_idx as usize;
                if track_idx >= model.tracks.len() {
                    continue;
                }
                let num_lanes = model.tracks[track_idx].automation_lanes.len();
                for lane_idx in 0..num_lanes {
                    if !seen_lanes.insert((track_idx, lane_idx)) {
                        continue;
                    }
                    let track = Arc::make_mut(&mut model.tracks[track_idx]);
                    let lane = &track.automation_lanes[lane_idx];
                    let mut in_range: Vec<AutomationEvent> = Vec::new();
                    let mut out_of_range: Vec<AutomationEvent> = Vec::new();
                    for evt in lane.events.iter() {
                        // 成员态按事件 id 判定（落点锚点不被重收集），
                        // 矩形态按 rect + 属性筛选判定；rect 循环只提供扫描范围。
                        if selection.accepts_automation_event(track_idx as u16, &lane.target, evt) {
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

            if delta_tracks == 0 {
                // Same lane: add moved events back with offset ticks（整体保持有序）
                let mut merged = lm.remaining.clone();
                merged.extend(lm.events.iter().copied());
                src_lane.replace_all(merged);
            } else {
                src_lane.events = lm.remaining.clone();
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
                    let mut merged = std::mem::take(&mut src_lane.events);
                    merged.extend(lm.events.iter().copied());
                    src_lane.replace_all(merged);
                    sub_actions.push(UndoAction::Automation(AutomationDelta {
                        track_idx: lm.src_track,
                        lane_idx: lm.lane_idx,
                        target: lm.target.clone(),
                        before: before_readd,
                        after: src_lane.events.clone(),
                    }));
                    continue;
                }
                let Some((dst_lane, dst_lane_idx)) =
                    model.ensure_automation_lane_mut(dst_track_idx, lm.target.clone())
                else {
                    continue; // 轨道不存在（防御，不 panic）
                };
                let before_dst = dst_lane.events.clone();
                let mut merged = dst_lane.events.clone();
                merged.extend(lm.events.iter().copied());
                dst_lane.replace_all(merged);
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
        let selection = self.edit.selected.clone();

        // ── 1. 复制音符（原音符保留，副本平移到新 tick/新轨）──
        let selected_data = batch_ops::collect_selected(model, &self.edit.selected);
        let mut dup_note_ids: Vec<u32> = Vec::new();
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
                let after = batch_ops::flatten_by_key(&new_by_key);
                dup_note_ids.extend(after.iter().map(|(n, _)| n.id));
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
        for &(_ts, _te, _kl, _kh, track_lo, track_hi) in &selection.rects {
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
                        .filter(|evt| {
                            // 成员态按事件 id 判定；矩形态按 rect + 属性筛选。
                            selection.accepts_automation_event(track_idx as u16, &lane.target, evt)
                        })
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

        let mut dup_event_ids: Vec<u32> = Vec::new();
        for lc in &lane_collects {
            let copies: Vec<AutomationEvent> = lc
                .events
                .iter()
                .map(|e| AutomationEvent {
                    // 副本发新 id（会话内身份，选择集跟随副本）。
                    id: model.alloc_automation_id(),
                    tick: (e.tick as i64 + delta_ticks).max(0) as u32,
                    ..*e
                })
                .collect();
            dup_event_ids.extend(copies.iter().map(|e| e.id));
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
        // 选区精确跟随副本音符（新 id），落点处的其他音符不纳入。
        if !dup_note_ids.is_empty() {
            self.edit.selected.set_members(dup_note_ids);
        }
        // 自动化同理：选区跟随副本事件（新 id）。
        if !dup_event_ids.is_empty() {
            self.edit.selected.set_automation_members(dup_event_ids);
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
            doc_id: 0,
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
        doc.edit.overlap_blocked_behavior =
            crate::audio_settings::OverlapBlockedBehavior::KeepOriginal;

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

    /// AR 跨轨 ReplaceTarget：落点轨的目标音符被替换删除、音符搬到目标轨，
    /// undo/redo 精确（同时覆盖 helper 的 new_track 传参路径）。
    #[test]
    fn move_selected_arrange_cross_track_replace_target() {
        let mut doc = make_doc_tracks(2);
        add(&mut doc, 100, 200, 60); // A track0（选中）
        doc.add_note(
            1,
            NoteEvent {
                id: 0,
                start_tick: 100,
                end_tick: 150,
                key: 60,
                velocity: 100,
            },
        ); // C track1 落点
        doc.edit.selected.add_rect_track(100, 201, 60, 60, 0, 0);
        doc.edit.allow_overlapping_notes = false;
        doc.edit.overlap_blocked_behavior =
            crate::audio_settings::OverlapBlockedBehavior::ReplaceTarget;
        let before_snap = doc.capture_snapshot();
        let action = doc.move_selected_arrange(0, 1).expect("应产生 undo");

        match &action {
            UndoAction::Notes(delta) => {
                assert_eq!(delta.before.len(), 2, "before = A + 被替换的 C");
                assert_eq!(delta.after.len(), 1);
            }
            other => panic!("期望副本制 Notes，实际 {other:?}"),
        }
        assert_eq!(doc.data.model.notes[60].len(), 1, "只剩搬过去的 A");
        let n = doc.data.model.notes[60][0];
        assert_eq!((n.track, n.start_tick, n.end_tick), (1, 100, 200));

        doc.push_undo(action, "arrange-replace", before_snap);
        assert!(doc.undo(), "undo 应恢复 A(track0) 与 C(track1)");
        assert_eq!(doc.data.model.notes[60].len(), 2);
        assert!(
            doc.data.model.notes[60]
                .iter()
                .any(|n| n.track == 0 && n.start_tick == 100 && n.end_tick == 200)
        );
        assert!(
            doc.data.model.notes[60]
                .iter()
                .any(|n| n.track == 1 && n.start_tick == 100 && n.end_tick == 150)
        );
        assert!(doc.redo(), "redo 应再次替换");
        assert_eq!(doc.data.model.notes[60].len(), 1);
    }

    /// 无重叠/无 clamp 时走操作式 ArrangeMoveNotes（原地路径），undo/redo 精确。
    #[test]
    fn move_selected_arrange_fast_path_roundtrips() {
        let mut doc = make_doc();
        add(&mut doc, 100, 200, 60);
        add(&mut doc, 300, 400, 62);
        doc.edit.selected.add_rect_track(100, 401, 60, 62, 0, 0);
        doc.edit.allow_overlapping_notes = false;

        let before_snap = doc.capture_snapshot();
        let action = doc.move_selected_arrange(50, 0).expect("应产生 undo");
        assert!(
            matches!(action, UndoAction::ArrangeMoveNotes { .. }),
            "无重叠/无 clamp 应走操作式，实际 {action:?}"
        );
        assert!(
            doc.data.model.notes[60]
                .iter()
                .any(|n| n.start_tick == 150 && n.end_tick == 250)
        );
        assert!(doc.data.model.notes[60].is_sorted());

        doc.push_undo(action, "arrange-move", before_snap);
        assert!(doc.undo());
        assert!(
            doc.data.model.notes[60]
                .iter()
                .any(|n| n.start_tick == 100 && n.end_tick == 200)
        );
        assert!(doc.redo());
        assert!(
            doc.data.model.notes[60]
                .iter()
                .any(|n| n.start_tick == 150 && n.end_tick == 250)
        );
    }

    /// 轨道被夹取（越界回不来）时回退副本制，undo/redo 仍精确。
    #[test]
    fn move_selected_arrange_track_clamp_falls_back() {
        let mut doc = make_doc();
        add(&mut doc, 100, 200, 60);
        doc.edit.selected.add_rect_track(100, 201, 60, 60, 0, 0);
        doc.edit.allow_overlapping_notes = false;

        // 单轨模型 + delta_tracks = -1：track 0 被夹回 0，不可逆 → 副本制。
        let before_snap = doc.capture_snapshot();
        let action = doc.move_selected_arrange(0, -1).expect("应产生 undo");
        assert!(
            matches!(action, UndoAction::Notes(_)),
            "轨道夹取应回退副本制，实际 {action:?}"
        );
        doc.push_undo(action, "arrange-clamp", before_snap);
        assert!(doc.undo());
        assert_eq!(doc.data.model.notes[60][0].start_tick, 100);
    }

    /// move_selected_arrange：全部被拦时音符不动、不产生 undo。
    #[test]
    fn move_selected_arrange_all_blocked() {
        let mut doc = make_doc();
        add(&mut doc, 100, 200, 60); // A（选中）
        add(&mut doc, 500, 600, 60); // C 占位
        doc.edit.selected.add_rect_track(100, 201, 60, 60, 0, 0);
        doc.edit.allow_overlapping_notes = false;
        doc.edit.overlap_blocked_behavior =
            crate::audio_settings::OverlapBlockedBehavior::KeepOriginal;

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

    /// 回归：AR 框选物化后，移动提交再拖动不会把落点的路人音符吸进选区。
    /// （框选提交在 UI 层物化，这里直接构造物化态 Selection。）
    #[test]
    fn materialized_arrange_selection_stable_across_moves() {
        let mut doc = make_doc();
        add(&mut doc, 100, 200, 60); // A（被框选）
        add(&mut doc, 300, 400, 60); // B（落点路人）
        doc.edit.allow_overlapping_notes = true;

        // 模拟 AR 框选提交：[100,201) 全 key，物化成员。
        doc.edit
            .selected
            .add_rect_track(100, 201, 0, yinhe_types::MAX_KEY, 0, 0);
        doc.edit.selected.materialize_pending(&*doc.data.model);
        assert_eq!(doc.edit.selected.explicit_member_count(), Some(1));

        // 第一次移动 +200：A 到 [300,400)，与 B 重叠（允许重叠）。
        doc.move_selected_arrange(200, 0)
            .expect("第一次移动应产生 undo");
        // 第二次移动 +100：只应搬 A（成员），不得带上 B。
        doc.move_selected_arrange(100, 0)
            .expect("第二次移动应产生 undo");

        let k60: Vec<(u32, u32)> = doc.data.model.notes[60]
            .iter()
            .map(|n| (n.start_tick, n.end_tick))
            .collect();
        assert!(k60.contains(&(400, 500)), "A 应移到 [400,500)");
        assert!(k60.contains(&(300, 400)), "B 应留在 [300,400)");
        assert_eq!(
            doc.edit.selected.explicit_member_count(),
            Some(1),
            "成员集合保持只有 A"
        );
    }

    /// AR Alt 拖动复制后选区精确跟随副本，不吸收落点处的其他音符。
    #[test]
    fn duplicate_selected_arrange_selection_follows_copies() {
        let mut doc = make_doc();
        add(&mut doc, 100, 200, 60); // A（被框选）
        add(&mut doc, 500, 600, 60); // C（落点已有音符）
        doc.edit.allow_overlapping_notes = true;
        doc.edit
            .selected
            .add_rect_track(100, 201, 0, yinhe_types::MAX_KEY, 0, 0);
        doc.edit.selected.materialize_pending(&*doc.data.model);

        // Alt+拖动复制 +400：副本落点 [500,600) 与 C 重叠。
        doc.duplicate_selected_arrange(400, 0)
            .expect("复制应产生 undo");

        let collected = batch_ops::collect_selected(&doc.data.model, &doc.edit.selected);
        assert_eq!(collected.len(), 1, "落点已有音符 C 不得被选区吸收");
        assert_eq!(collected[0].0.start_tick, 500, "选区应指向副本");
        assert!(
            doc.data.model.notes[60].iter().any(|n| n.start_tick == 100),
            "原件应保留"
        );
    }

    /// 回归：AR 自动化成员态下移动两次，不吃落点处的锚点。
    #[test]
    fn materialized_arrange_automation_stable_across_moves() {
        let mut doc = make_doc();
        // track 0 加一条 CC lane：id1@100（被框选）、id2@300（落点路人）。
        {
            let model = Arc::make_mut(&mut doc.data.model);
            let track = Arc::make_mut(&mut model.tracks[0]);
            track.automation_lanes.push(yinhe_types::AutomationLane {
                target: yinhe_types::AutomationTarget::CC { controller: 7 },
                track: 0,
                events: vec![
                    yinhe_types::AutomationEvent {
                        id: 1,
                        tick: 100,
                        value: 0.5,
                        shape: yinhe_types::SegmentShape::Step,
                    },
                    yinhe_types::AutomationEvent {
                        id: 2,
                        tick: 300,
                        value: 0.7,
                        shape: yinhe_types::SegmentShape::Step,
                    },
                ],
            });
            model.next_automation_id = 10;
        }

        // AR 框选 [100,200) 全 key：物化自动化成员 = {id1}。
        doc.edit
            .selected
            .add_rect_track(100, 200, 0, yinhe_types::MAX_KEY, 0, 0);
        let tracks = doc.data.model.tracks.clone();
        doc.edit.selected.materialize_automation_pending(&tracks);
        assert_eq!(doc.edit.selected.automation_member_count(), Some(1));

        // 第一次移动 +150：id1 → 250，选框平移到 [250,350)（覆盖 id2@300）。
        doc.move_selected_arrange(150, 0)
            .expect("第一次移动应产生 undo");
        // 第二次移动 +40：成员态只搬 id1（250→290），不得带上 id2@300。
        doc.move_selected_arrange(40, 0)
            .expect("第二次移动应产生 undo");

        let lane = &doc.data.model.tracks[0].automation_lanes[0];
        let ticks: Vec<(u32, u32)> = lane.events.iter().map(|e| (e.id, e.tick)).collect();
        assert!(ticks.contains(&(1, 290)), "id1 应移到 290，实际 {ticks:?}");
        assert!(ticks.contains(&(2, 300)), "id2 应留在 300，实际 {ticks:?}");
    }

    /// 多轨测试文档（track_visible 长度与轨道数一致）。
    fn make_doc_tracks(n: usize) -> Document {
        let mut doc = make_doc();
        let model = Arc::make_mut(&mut doc.data.model);
        while model.tracks.len() < n {
            let i = model.tracks.len();
            model.tracks.push(Arc::new(TrackData::new(0, i as u8)));
        }
        doc.edit.track_visible = vec![true; n];
        doc.edit.track_pianoroll_visible = vec![true; n];
        doc
    }

    /// 往指定轨塞一条 CC7 lane（事件 id/tick 给定）。
    fn add_cc_lane(doc: &mut Document, track_idx: usize, evts: Vec<(u32, u32)>) {
        let model = Arc::make_mut(&mut doc.data.model);
        let track = Arc::make_mut(&mut model.tracks[track_idx]);
        track.automation_lanes.push(yinhe_types::AutomationLane {
            target: yinhe_types::AutomationTarget::CC { controller: 7 },
            track: track_idx as u16,
            events: evts
                .into_iter()
                .map(|(id, tick)| yinhe_types::AutomationEvent {
                    id,
                    tick,
                    value: 0.5,
                    shape: yinhe_types::SegmentShape::Step,
                })
                .collect(),
        });
        model.next_automation_id = 1000;
    }

    fn cc7_lane(doc: &Document, track_idx: usize) -> Option<&yinhe_types::AutomationLane> {
        doc.data.model.tracks[track_idx]
            .automation_lanes
            .iter()
            .find(|l| {
                matches!(
                    l.target,
                    yinhe_types::AutomationTarget::CC { controller: 7 }
                )
            })
    }

    /// AR 跨轨拖动：源 lane 事件搬到目标轨的同名 lane（不存在则创建）；
    /// undo 撤销目标轨写入并恢复源轨，redo 再现。
    #[test]
    fn arrange_automation_cross_track_move_and_undo() {
        let mut doc = make_doc_tracks(2);
        add_cc_lane(&mut doc, 0, vec![(1, 100), (2, 300)]);
        // 框选 [100,101) track 0：只选中 tick=100 的锚点。
        doc.edit
            .selected
            .add_rect_track(100, 101, 0, yinhe_types::MAX_KEY, 0, 0);
        let tracks = doc.data.model.tracks.clone();
        doc.edit.selected.materialize_automation_pending(&tracks);

        let before = doc.capture_snapshot();
        let action = doc
            .move_selected_arrange(0, 1)
            .expect("跨轨搬运应产生 undo");
        doc.push_undo(action, "arrange_move", before);

        let dst = cc7_lane(&doc, 1).expect("目标轨应懒创建 CC7 lane");
        assert_eq!(dst.events.len(), 1);
        assert_eq!(dst.events[0].tick, 100, "被选锚点搬到目标轨");
        let src = &doc.data.model.tracks[0].automation_lanes[0];
        assert_eq!(src.events.len(), 1, "源轨只剩未选锚点");
        assert_eq!(src.events[0].tick, 300);

        assert!(doc.undo(), "undo 应成功");
        let dst = cc7_lane(&doc, 1).expect("目标 lane 结构不随事件 delta 撤销");
        assert!(dst.events.is_empty(), "undo 应移除目标轨事件");
        assert_eq!(
            doc.data.model.tracks[0].automation_lanes[0].events.len(),
            2,
            "undo 应恢复源轨两条锚点"
        );

        assert!(doc.redo(), "redo 应成功");
        assert_eq!(
            cc7_lane(&doc, 1).expect("目标 lane").events.len(),
            1,
            "redo 再现目标轨写入"
        );
    }

    /// 回归：两个重叠 rect 命中同一 lane 时，同一锚点只搬一次
    /// （曾会按 rect 重复收集 LaneMove，跨轨时把事件重复插入目标轨）。
    #[test]
    fn arrange_automation_overlapping_rects_move_once() {
        let mut doc = make_doc_tracks(2);
        add_cc_lane(&mut doc, 0, vec![(1, 100)]);
        // 两个重叠 rect 均覆盖 tick=100。
        doc.edit
            .selected
            .add_rect_track(50, 150, 0, yinhe_types::MAX_KEY, 0, 0);
        doc.edit
            .selected
            .add_rect_track(100, 200, 0, yinhe_types::MAX_KEY, 0, 0);

        let before = doc.capture_snapshot();
        let action = doc.move_selected_arrange(0, 1).expect("搬运应产生 undo");
        doc.push_undo(action, "arrange_move", before);

        let dst = cc7_lane(&doc, 1).expect("目标轨应有 CC7 lane");
        assert_eq!(
            dst.events.len(),
            1,
            "同一锚点只能搬一次，实际 {:?}",
            dst.events
        );
        assert_eq!(dst.events[0].tick, 100);
    }
}
