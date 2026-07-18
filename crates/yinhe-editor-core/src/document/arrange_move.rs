//! Arrange-view drag: move notes + automation across tracks.

use std::sync::Arc;

use yinhe_types::AutomationEvent;

use crate::batch_ops;
use crate::history::{AutomationDelta, NoteDelta, UndoAction};

use super::Document;

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
    pub fn move_selected_arrange(&mut self, delta_ticks: i64, delta_tracks: i32) -> Option<UndoAction> {
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
            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> = std::collections::HashMap::new();
            for (note, old_key) in &originals {
                let new_tick = (note.start_tick as i64 + delta_ticks).max(0) as u32;
                let raw_track = (note.track as i32 + delta_tracks).clamp(0, num_tracks - 1);
                // Skip over conductor track: notes cannot land on it.
                let new_track = if Some(raw_track as u16) == self.edit.conductor_track_idx {
                    if delta_tracks < 0 {
                        // Moving up: clamp to first non-conductor track
                        (raw_track + 1).min(num_tracks - 1) as u16
                    } else {
                        // Moving down: clamp to last track before conductor (or stay)
                        (raw_track - 1).max(0) as u16
                    }
                } else {
                    raw_track as u16
                };
                let length = note.end_tick - note.start_tick;
                let moved = yinhe_types::Note {
                    id: note.id,
                    start_tick: new_tick,
                    end_tick: new_tick + length,
                    velocity: note.velocity,
                    track: new_track,
                };
                new_by_key.entry(*old_key).or_default().push(moved);
            }
            let after: Vec<(yinhe_types::Note, u8)> = new_by_key
                .iter()
                .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
                .collect();
            batch_ops::insert_batch(model, new_by_key);
            sub_actions.push(UndoAction::Notes(NoteDelta {
                before: originals,
                after,
            }));
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
                let raw_dst = (lm.src_track as i32 + delta_tracks)
                    .clamp(0, num_tracks - 1);
                // Skip over conductor track: automation cannot land on it.
                let dst_track_idx = if Some(raw_dst as u16) == self.edit.conductor_track_idx {
                    if delta_tracks < 0 {
                        (raw_dst + 1).min(num_tracks - 1) as usize
                    } else {
                        (raw_dst - 1).max(0) as usize
                    }
                } else {
                    raw_dst as usize
                };
                if dst_track_idx == lm.src_track {
                    continue; // clamped to same track, events already in source lane
                }
                let dst_track = Arc::make_mut(&mut model.tracks[dst_track_idx]);
                let dst_lane_idx = match dst_track.automation_lanes.iter().position(|l| l.target == lm.target) {
                    Some(idx) => idx,
                    None => {
                        dst_track.automation_lanes.push(yinhe_types::AutomationLane {
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
}
