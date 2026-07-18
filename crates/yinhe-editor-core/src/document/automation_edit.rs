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
    ///
    /// 如果 `target` 是 `Tempo`，忽略 `track_idx`，直接操作 `conductor.tempo`。
    pub fn add_automation_event(
        &mut self,
        track_idx: usize,
        target: yinhe_types::AutomationTarget,
        event: yinhe_types::AutomationEvent,
    ) -> Option<(usize, usize, UndoAction)> {
        if matches!(target, yinhe_types::AutomationTarget::Tempo) {
            let model = Arc::make_mut(&mut self.data.model);
            let conductor = Arc::make_mut(&mut model.conductor);
            let lane = &mut conductor.tempo;
            let before = lane.events.clone();
            let insert_pos = lane.events.partition_point(|e| e.tick < event.tick);
            lane.events.insert(insert_pos, event);
            let after = lane.events.clone();
            self.data.bump_revision();
            return Some((0, 0, UndoAction::Automation(AutomationDelta {
                track_idx: 0,
                lane_idx: 0,
                target,
                before,
                after,
            })));
        }
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
            target,
            before,
            after,
        })))
    }

    /// 移动指定 lane 上 tick=`old_tick` 的事件到 `(new_tick, new_value)`。
    /// 如果 `new_tick` 与同 lane 已有事件冲突，会先移除冲突项。
    ///
    /// 如果 `target` 是 `Tempo`，忽略 `track_idx`/`lane_idx`，直接操作
    /// `conductor.tempo`。
    pub fn move_automation_event(
        &mut self,
        track_idx: usize,
        lane_idx: usize,
        target: &yinhe_types::AutomationTarget,
        old_tick: u32,
        new_tick: u32,
        new_value: f32,
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let events = if matches!(target, yinhe_types::AutomationTarget::Tempo) {
            let conductor = Arc::make_mut(&mut model.conductor);
            &mut conductor.tempo.events
        } else {
            let track = model.tracks.get_mut(track_idx)?;
            let track = Arc::make_mut(track);
            &mut track.automation_lanes.get_mut(lane_idx)?.events
        };

        let before = events.clone();
        // 验证原事件存在
        events.iter().position(|e| e.tick == old_tick)?;

        if old_tick == new_tick {
            // 只改 value，不改 tick：直接原地修改，避免 retain 误删原事件
            let evt = events.iter_mut().find(|e| e.tick == old_tick)?;
            evt.value = new_value;
        } else {
            // 移除目标 tick 上已有的事件（避免重复 tick）
            events.retain(|e| e.tick != new_tick);
            // 找到原事件并修改
            let evt = events.iter_mut().find(|e| e.tick == old_tick)?;
            evt.tick = new_tick;
            evt.value = new_value;
            events.sort_by_key(|e| e.tick);
        }
        let after = events.clone();

        self.data.bump_revision();
        Some(UndoAction::Automation(AutomationDelta {
            track_idx,
            lane_idx,
            target: target.clone(),
            before,
            after,
        }))
    }

    /// 删除指定 lane 上 tick=`tick` 的事件。
    ///
    /// 如果 `target` 是 `Tempo`，忽略 `track_idx`/`lane_idx`，直接操作
    /// `conductor.tempo`。
    pub fn delete_automation_event(
        &mut self,
        track_idx: usize,
        lane_idx: usize,
        target: &yinhe_types::AutomationTarget,
        tick: u32,
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let events = if matches!(target, yinhe_types::AutomationTarget::Tempo) {
            let conductor = Arc::make_mut(&mut model.conductor);
            &mut conductor.tempo.events
        } else {
            let track = model.tracks.get_mut(track_idx)?;
            let track = Arc::make_mut(track);
            &mut track.automation_lanes.get_mut(lane_idx)?.events
        };

        let before = events.clone();
        events.retain(|e| e.tick != tick);
        if before.len() == events.len() {
            return None;
        }
        let after = events.clone();

        self.data.bump_revision();
        Some(UndoAction::Automation(AutomationDelta {
            track_idx,
            lane_idx,
            target: target.clone(),
            before,
            after,
        }))
    }

    /// 修改指定 lane 上 tick=`tick` 的事件的 shape。
    ///
    /// 如果 `target` 是 `Tempo`，忽略 `track_idx`/`lane_idx`，直接操作
    /// `conductor.tempo`。
    pub fn set_automation_shape(
        &mut self,
        track_idx: usize,
        lane_idx: usize,
        target: &yinhe_types::AutomationTarget,
        tick: u32,
        shape: yinhe_types::SegmentShape,
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let events = if matches!(target, yinhe_types::AutomationTarget::Tempo) {
            let conductor = Arc::make_mut(&mut model.conductor);
            &mut conductor.tempo.events
        } else {
            let track = model.tracks.get_mut(track_idx)?;
            let track = Arc::make_mut(track);
            &mut track.automation_lanes.get_mut(lane_idx)?.events
        };

        let before = events.clone();
        let evt = events.iter_mut().find(|e| e.tick == tick)?;
        if evt.shape == shape {
            return None;
        }
        evt.shape = shape;
        let after = events.clone();

        self.data.bump_revision();
        Some(UndoAction::Automation(AutomationDelta {
            track_idx,
            lane_idx,
            target: target.clone(),
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
                AutomationEdit::Move { track_idx, lane_idx, target, old_tick, new_tick, new_value } => {
                    self.move_automation_event(track_idx as usize, lane_idx, &target, old_tick, new_tick, new_value)
                }
                AutomationEdit::CycleShape { track_idx, lane_idx, target, tick } => {
                    // Step ↔ Curve{tension:0}
                    let lane = if matches!(target, yinhe_types::AutomationTarget::Tempo) {
                        Some(&self.data.model.conductor.tempo)
                    } else {
                        self.data.model.tracks.get(track_idx as usize)
                            .and_then(|t| t.automation_lanes.get(lane_idx))
                    };
                    let evt = lane.and_then(|l| l.events.iter().find(|e| e.tick == tick));
                    if let Some(evt) = evt {
                        let next = match evt.shape {
                            yinhe_types::SegmentShape::Step => yinhe_types::SegmentShape::Curve { tension: 0.0 },
                            yinhe_types::SegmentShape::Curve { .. } => yinhe_types::SegmentShape::Step,
                        };
                        self.set_automation_shape(track_idx as usize, lane_idx, &target, tick, next)
                    } else {
                        None
                    }
                }
                AutomationEdit::Delete { track_idx, lane_idx, target, tick } => {
                    self.delete_automation_event(track_idx as usize, lane_idx, &target, tick)
                }
            };
            if let Some(action) = action {
                actions.push(action);
            }
        }
        actions
    }
}
