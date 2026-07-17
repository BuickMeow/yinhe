//! Automation event editing: add, move, delete, set-shape, apply-batch.

use std::sync::Arc;

use yinhe_types::AutomationEdit;

use crate::history::{AutomationDelta, UndoAction};

use super::Document;

impl Document {
    /// 在指定 track 的指定 lane 上添加一个 automation 事件。
    ///
    /// 如果该 track 没有 target 对应的 lane，会先创建。
    /// 返回 (track_idx, lane_idx, UndoAction)，调用方需把 UndoAction push 到 history。
    pub fn add_automation_event(
        &mut self,
        track_idx: usize,
        target: yinhe_types::AutomationTarget,
        event: yinhe_types::AutomationEvent,
    ) -> Option<(usize, usize, UndoAction)> {
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);

        // 找或创建 lane
        let lane_idx = match track.automation_lanes.iter().position(|l| l.target == target) {
            Some(idx) => idx,
            None => {
                track.automation_lanes.push(yinhe_types::AutomationLane {
                    target: target.clone(),
                    track: track_idx as u16,
                    events: Vec::new(),
                });
                track.automation_lanes.len() - 1
            }
        };
        let lane = &mut track.automation_lanes[lane_idx];

        let before = lane.events.clone();
        let insert_pos = lane.events.partition_point(|e| e.tick < event.tick);
        lane.events.insert(insert_pos, event);
        let after = lane.events.clone();

        self.data.bump_revision();
        Some((track_idx, lane_idx, UndoAction::Automation(AutomationDelta {
            track_idx,
            lane_idx,
            before,
            after,
        })))
    }

    /// 移动指定 lane 上 tick=`old_tick` 的事件到 `(new_tick, new_value)`。
    /// 如果 `new_tick` 与同 lane 已有事件冲突，会先移除冲突项。
    pub fn move_automation_event(
        &mut self,
        track_idx: usize,
        lane_idx: usize,
        old_tick: u32,
        new_tick: u32,
        new_value: u16,
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);
        let lane = track.automation_lanes.get_mut(lane_idx)?;

        let before = lane.events.clone();
        // 验证原事件存在
        lane.events.iter().position(|e| e.tick == old_tick)?;

        if old_tick == new_tick {
            // 只改 value，不改 tick：直接原地修改，避免 retain 误删原事件
            let evt = lane.events.iter_mut().find(|e| e.tick == old_tick)?;
            evt.value = new_value;
        } else {
            // 移除目标 tick 上已有的事件（避免重复 tick）
            lane.events.retain(|e| e.tick != new_tick);
            // 找到原事件并修改
            let evt = lane.events.iter_mut().find(|e| e.tick == old_tick)?;
            evt.tick = new_tick;
            evt.value = new_value;
            lane.events.sort_by_key(|e| e.tick);
        }
        let after = lane.events.clone();

        self.data.bump_revision();
        Some(UndoAction::Automation(AutomationDelta {
            track_idx,
            lane_idx,
            before,
            after,
        }))
    }

    /// 删除指定 lane 上 tick=`tick` 的事件。
    pub fn delete_automation_event(
        &mut self,
        track_idx: usize,
        lane_idx: usize,
        tick: u32,
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);
        let lane = track.automation_lanes.get_mut(lane_idx)?;

        let before = lane.events.clone();
        lane.events.retain(|e| e.tick != tick);
        if before.len() == lane.events.len() {
            return None;
        }
        let after = lane.events.clone();

        self.data.bump_revision();
        Some(UndoAction::Automation(AutomationDelta {
            track_idx,
            lane_idx,
            before,
            after,
        }))
    }

    /// 修改指定 lane 上 tick=`tick` 的事件的 shape。
    pub fn set_automation_shape(
        &mut self,
        track_idx: usize,
        lane_idx: usize,
        tick: u32,
        shape: yinhe_types::SegmentShape,
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);
        let lane = track.automation_lanes.get_mut(lane_idx)?;

        let before = lane.events.clone();
        let evt = lane.events.iter_mut().find(|e| e.tick == tick)?;
        if evt.shape == shape {
            return None;
        }
        evt.shape = shape;
        let after = lane.events.clone();

        self.data.bump_revision();
        Some(UndoAction::Automation(AutomationDelta {
            track_idx,
            lane_idx,
            before,
            after,
        }))
    }

    /// Apply a batch of automation edits (add / move / cycle-shape).
    ///
    /// Returns a `Vec<UndoAction>` for all successfully applied edits.
    /// The caller is responsible for pushing them to the history stack,
    /// marking the view dirty, and sending `AudioCommand::ReloadNotes`.
    pub fn apply_automation_edits(&mut self, edits: Vec<AutomationEdit>) -> Vec<UndoAction> {
        let mut actions = Vec::new();
        for edit in edits {
            let action = match edit {
                AutomationEdit::Add { track_idx, target, tick, value, shape } => {
                    let event = yinhe_types::AutomationEvent { tick, value, shape };
                    match self.add_automation_event(track_idx as usize, target, event) {
                        Some((_, _, action)) => Some(action),
                        None => None,
                    }
                }
                AutomationEdit::Move { track_idx, lane_idx, old_tick, new_tick, new_value } => {
                    self.move_automation_event(track_idx as usize, lane_idx, old_tick, new_tick, new_value)
                }
                AutomationEdit::CycleShape { track_idx, lane_idx, tick } => {
                    // Step ↔ Curve{tension:0}
                    let track = self.data.model.tracks.get(track_idx as usize);
                    let lane = track.and_then(|t| t.automation_lanes.get(lane_idx));
                    let evt = lane.and_then(|l| l.events.iter().find(|e| e.tick == tick));
                    if let Some(evt) = evt {
                        let next = match evt.shape {
                            yinhe_types::SegmentShape::Step => yinhe_types::SegmentShape::Curve { tension: 0 },
                            yinhe_types::SegmentShape::Curve { .. } => yinhe_types::SegmentShape::Step,
                        };
                        self.set_automation_shape(track_idx as usize, lane_idx, tick, next)
                    } else {
                        None
                    }
                }
                AutomationEdit::Delete { track_idx, lane_idx, tick } => {
                    self.delete_automation_event(track_idx as usize, lane_idx, tick)
                }
            };
            if let Some(action) = action {
                actions.push(action);
            }
        }
        actions
    }
}
