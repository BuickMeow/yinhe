//! Automation event editing: add, move, delete, set-shape, apply-batch.

use std::sync::Arc;

use yinhe_types::AutomationEdit;
use yinhe_types::AutomationTarget;

use crate::history::{AutomationDelta, UndoAction};
use crate::num_expr::{NumOp, apply_ops, apply_ops_round};

use super::Document;

/// AM 锚点批量编辑的字段（Info 面板选框编辑）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorField {
    /// 值。
    Value,
    /// tick。
    Tick,
}

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
            self.data.rebuild_tempo_map();
            self.data.bump_revision();
            return Some((
                0,
                0,
                UndoAction::Automation(AutomationDelta {
                    track_idx: 0,
                    lane_idx: 0,
                    target,
                    before,
                    after,
                }),
            ));
        }
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);

        // 找或创建 lane
        let lane_idx = match track
            .automation_lanes
            .iter()
            .position(|l| l.target == target)
        {
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
        Some((
            track_idx,
            lane_idx,
            UndoAction::Automation(AutomationDelta {
                track_idx,
                lane_idx,
                target,
                before,
                after,
            }),
        ))
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

        if matches!(target, yinhe_types::AutomationTarget::Tempo) {
            self.data.rebuild_tempo_map();
        }
        self.data.bump_revision();
        Some(UndoAction::Automation(AutomationDelta {
            track_idx,
            lane_idx,
            target: target.clone(),
            before,
            after,
        }))
    }

    /// 批量移动多个锚点（一次 undo 快照）。
    ///
    /// `moves = [(old_tick, new_tick, new_value)]`，所有锚点在同一 lane 上。
    /// 先移除所有 old_tick 对应的事件，再按 new_tick 排序后插入，
    /// 避免逐个 move 导致中间状态丢失锚点（如 1→2, 2→3 链式覆盖）。
    pub fn move_automation_events_batch(
        &mut self,
        track_idx: usize,
        lane_idx: usize,
        target: &yinhe_types::AutomationTarget,
        moves: &[(u32, u32, f32)],
    ) -> Option<UndoAction> {
        if moves.is_empty() {
            return None;
        }
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

        // 收集每个 old_tick 对应的 shape，并从 events 移除
        let mut shapes: Vec<yinhe_types::SegmentShape> = Vec::with_capacity(moves.len());
        for (old_tick, _, _) in moves {
            if let Some(idx) = events.iter().position(|e| e.tick == *old_tick) {
                shapes.push(events.remove(idx).shape);
            } else {
                shapes.push(target.default_shape());
            }
        }
        // 按 new_tick 排序后插入（冲突时后者覆盖）
        let mut sorted: Vec<(u32, f32, yinhe_types::SegmentShape)> = moves
            .iter()
            .zip(shapes.iter())
            .map(|((_, new, val), shape)| (*new, *val, *shape))
            .collect();
        sorted.sort_by_key(|(new, _, _)| *new);
        for (new_tick, new_value, shape) in sorted {
            // 移除 new_tick 处可能残留的旧事件
            if let Some(idx) = events.iter().position(|e| e.tick == new_tick) {
                events.remove(idx);
            }
            let insert_idx = events.partition_point(|e| e.tick < new_tick);
            events.insert(
                insert_idx,
                yinhe_types::AutomationEvent {
                    tick: new_tick,
                    value: new_value,
                    shape,
                },
            );
        }

        let after = events.clone();
        if matches!(target, yinhe_types::AutomationTarget::Tempo) {
            self.data.rebuild_tempo_map();
        }
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

        if matches!(target, yinhe_types::AutomationTarget::Tempo) {
            self.data.rebuild_tempo_map();
        }
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

        if matches!(target, yinhe_types::AutomationTarget::Tempo) {
            self.data.rebuild_tempo_map();
        }
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
                AutomationEdit::Add {
                    track_idx,
                    target,
                    tick,
                    value,
                    shape,
                } => {
                    let event = yinhe_types::AutomationEvent { tick, value, shape };
                    match self.add_automation_event(track_idx as usize, target, event) {
                        Some((_, _, action)) => Some(action),
                        None => None,
                    }
                }
                AutomationEdit::Move {
                    track_idx,
                    lane_idx,
                    target,
                    old_tick,
                    new_tick,
                    new_value,
                } => self.move_automation_event(
                    track_idx as usize,
                    lane_idx,
                    &target,
                    old_tick,
                    new_tick,
                    new_value,
                ),
                AutomationEdit::MoveBatch {
                    track_idx,
                    lane_idx,
                    target,
                    moves,
                } => {
                    self.move_automation_events_batch(track_idx as usize, lane_idx, &target, &moves)
                }
                AutomationEdit::CycleShape {
                    track_idx,
                    lane_idx,
                    target,
                    tick,
                } => {
                    // Step ↔ Curve 直线（偏移量 0,0,0,0）
                    let lane = if matches!(target, yinhe_types::AutomationTarget::Tempo) {
                        Some(&self.data.model.conductor.tempo)
                    } else {
                        self.data
                            .model
                            .tracks
                            .get(track_idx as usize)
                            .and_then(|t| t.automation_lanes.get(lane_idx))
                    };
                    let evt = lane.and_then(|l| l.events.iter().find(|e| e.tick == tick));
                    if let Some(evt) = evt {
                        let next = match evt.shape {
                            yinhe_types::SegmentShape::Step => {
                                yinhe_types::SegmentShape::linear_curve()
                            }
                            yinhe_types::SegmentShape::Curve { .. } => {
                                yinhe_types::SegmentShape::Step
                            }
                        };
                        self.set_automation_shape(track_idx as usize, lane_idx, &target, tick, next)
                    } else {
                        None
                    }
                }
                AutomationEdit::SetShape {
                    track_idx,
                    lane_idx,
                    target,
                    tick,
                    shape,
                } => self.set_automation_shape(track_idx as usize, lane_idx, &target, tick, shape),
                AutomationEdit::Delete {
                    track_idx,
                    lane_idx,
                    target,
                    tick,
                } => self.delete_automation_event(track_idx as usize, lane_idx, &target, tick),
            };
            if let Some(action) = action {
                actions.push(action);
            }
        }
        actions
    }

    /// 对面板选框内的锚点批量应用表达式编辑（Info 面板选框编辑）。
    ///
    /// Value 仅改值；Tick 改 tick（保持 value）。加减 uniform 时选框
    /// 跟随平移，乘除/赋值（非 uniform）时选框不动。
    /// 返回单个 UndoAction（AutomationDelta），调用方 push 到 history。
    pub fn apply_anchor_field_edit(
        &mut self,
        panel_idx: usize,
        field: AnchorField,
        ops: &[NumOp],
    ) -> Option<UndoAction> {
        let panel = self.edit.controller_panels.get(panel_idx)?;
        if panel.show_velocity || panel.anchor_sel_rects.is_empty() {
            return None;
        }
        let target = panel.selected_target.clone();
        let rects = panel.anchor_sel_rects.clone();

        // 定位 lane（与 app 层 collect_anchor_ctx 同规则）：
        // Tempo → conductor.tempo；其他 → editing_track 的 target 匹配 lane。
        let (track_idx, lane_idx) = if matches!(target, AutomationTarget::Tempo) {
            (0u16, 0usize)
        } else {
            let track_idx = self
                .edit
                .editing_track
                .filter(|&t| {
                    self.edit
                        .track_visible
                        .get(t as usize)
                        .copied()
                        .unwrap_or(false)
                })
                .filter(|&t| Some(t) != self.edit.conductor_track_idx)?;
            let lane_idx = self
                .data
                .model
                .tracks
                .get(track_idx as usize)?
                .automation_lanes
                .iter()
                .position(|l| l.target == target)?;
            (track_idx, lane_idx)
        };

        // 收集选中锚点 + 计算新值 + uniform 判定
        let events = if matches!(target, AutomationTarget::Tempo) {
            self.data.model.conductor.tempo.events.clone()
        } else {
            self.data.model.tracks[track_idx as usize].automation_lanes[lane_idx]
                .events
                .clone()
        };
        let max_val = target.max_value();
        let mut moves: Vec<(u32, u32, f32)> = Vec::new();
        let mut uniform_tick: Option<i64> = None;
        let mut uniform_value: Option<f32> = None;
        for ev in &events {
            if !rects.iter().any(|r| r.contains(ev.tick, ev.value)) {
                continue;
            }
            let (new_tick, new_value) = match field {
                AnchorField::Value => {
                    let v = apply_ops(ops, ev.value as f64).clamp(0.0, max_val as f64) as f32;
                    (ev.tick, v)
                }
                AnchorField::Tick => {
                    let t = apply_ops_round(ops, ev.tick as f64).clamp(0.0, u32::MAX as f64) as u32;
                    (t, ev.value)
                }
            };
            if new_tick == ev.tick && new_value == ev.value {
                continue;
            }
            moves.push((ev.tick, new_tick, new_value));
            if field == AnchorField::Tick {
                let d = new_tick as i64 - ev.tick as i64;
                match uniform_tick {
                    None => uniform_tick = Some(d),
                    Some(u) if u != d => uniform_tick = None,
                    _ => {}
                }
            } else {
                let d = new_value - ev.value;
                match uniform_value {
                    None => uniform_value = Some(d),
                    Some(u) if (u - d).abs() > 1e-4 => uniform_value = None,
                    _ => {}
                }
            }
        }
        if moves.is_empty() {
            return None;
        }

        let action =
            self.move_automation_events_batch(track_idx as usize, lane_idx, &target, &moves)?;

        // 选框跟随：加减 uniform → tick/value 范围平移
        if let Some(dt) = uniform_tick {
            for r in &mut self.edit.controller_panels[panel_idx].anchor_sel_rects {
                r.tick_start += dt as f64;
                r.tick_end += dt as f64;
            }
        }
        if let Some(dv) = uniform_value {
            for r in &mut self.edit.controller_panels[panel_idx].anchor_sel_rects {
                if let Some((lo, hi)) = &mut r.value_range {
                    *lo += dv;
                    *hi += dv;
                }
            }
        }
        Some(action)
    }

    /// AM 变速：把面板选框时间跨度缩放为 `new_span`，选中锚点相对起点等比缩放。
    ///
    /// `anchor_sel_rects` 的 tick 范围同步缩放（value_range 不动）。
    /// 返回单个 UndoAction（AutomationDelta），调用方 push 到 history。
    pub fn rescale_anchor_span(&mut self, panel_idx: usize, new_span: u64) -> Option<UndoAction> {
        let panel = self.edit.controller_panels.get(panel_idx)?;
        if panel.show_velocity || panel.anchor_sel_rects.is_empty() {
            return None;
        }
        let target = panel.selected_target.clone();
        let rects = panel.anchor_sel_rects.clone();

        let mut t0 = f64::INFINITY;
        let mut t1 = f64::NEG_INFINITY;
        for r in &rects {
            t0 = t0.min(r.tick_start.min(r.tick_end));
            t1 = t1.max(r.tick_start.max(r.tick_end));
        }
        let span = (t1 - t0) as u64;
        if span == 0 || new_span == span || new_span == 0 {
            return None;
        }
        let factor = new_span as f64 / span as f64;

        // 定位 lane（与 apply_anchor_field_edit 同规则）
        let (track_idx, lane_idx) = if matches!(target, AutomationTarget::Tempo) {
            (0u16, 0usize)
        } else {
            let track_idx = self
                .edit
                .editing_track
                .filter(|&t| {
                    self.edit
                        .track_visible
                        .get(t as usize)
                        .copied()
                        .unwrap_or(false)
                })
                .filter(|&t| Some(t) != self.edit.conductor_track_idx)?;
            let lane_idx = self
                .data
                .model
                .tracks
                .get(track_idx as usize)?
                .automation_lanes
                .iter()
                .position(|l| l.target == target)?;
            (track_idx, lane_idx)
        };

        let events = if matches!(target, AutomationTarget::Tempo) {
            self.data.model.conductor.tempo.events.clone()
        } else {
            self.data.model.tracks[track_idx as usize].automation_lanes[lane_idx]
                .events
                .clone()
        };
        let scale_tick = |t: u32| -> u32 {
            let s = (t0 + (t as f64 - t0) * factor).round();
            if s > u32::MAX as f64 {
                u32::MAX
            } else if s < 0.0 {
                0
            } else {
                s as u32
            }
        };
        let mut moves: Vec<(u32, u32, f32)> = Vec::new();
        for ev in &events {
            if !rects.iter().any(|r| r.contains(ev.tick, ev.value)) {
                continue;
            }
            let new_tick = scale_tick(ev.tick);
            if new_tick != ev.tick {
                moves.push((ev.tick, new_tick, ev.value));
            }
        }
        if moves.is_empty() {
            return None;
        }
        let action =
            self.move_automation_events_batch(track_idx as usize, lane_idx, &target, &moves)?;

        // 选框 rect 缩放（tick 范围）
        for r in &mut self.edit.controller_panels[panel_idx].anchor_sel_rects {
            let ts = r.tick_start.min(r.tick_end);
            let te = r.tick_start.max(r.tick_end);
            let nts = (t0 + (ts - t0) * factor).round();
            let nte = (t0 + (te - t0) * factor).round().max(nts + 1.0);
            r.tick_start = nts;
            r.tick_end = nte;
        }
        Some(action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use yinhe_core::{ConductorData, TrackData, YinModel};
    use yinhe_types::{
        AnchorSelRect, AutomationEvent, AutomationLane, AutomationPanelView, AutomationTarget,
        SegmentShape, TimeSigEvent,
    };

    fn make_doc_with_anchor() -> Document {
        let model = YinModel {
            conductor: Arc::new(ConductorData {
                tempo: AutomationLane {
                    target: AutomationTarget::Tempo,
                    track: 0,
                    events: vec![AutomationEvent {
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
                t.automation_lanes.push(AutomationLane {
                    target: AutomationTarget::CC { controller: 7 },
                    track: 0,
                    events: vec![
                        AutomationEvent {
                            tick: 100,
                            value: 64.0,
                            shape: SegmentShape::Step,
                        },
                        AutomationEvent {
                            tick: 200,
                            value: 96.0,
                            shape: SegmentShape::Step,
                        },
                    ],
                });
                t
            })],
            ..Default::default()
        };
        let doc = Document {
            data: crate::project_data::ProjectData::new(
                Arc::new(model),
                vec!["t".to_string()],
                Default::default(),
                Default::default(),
            ),
            edit: crate::edit_state::EditState {
                track_visible: vec![true],
                track_pianoroll_visible: vec![true],
                editing_track: Some(0),
                controller_panels: vec![AutomationPanelView {
                    show_velocity: false,
                    selected_target: AutomationTarget::CC { controller: 7 },
                    anchor_sel_rects: vec![AnchorSelRect {
                        tick_start: 0.0,
                        tick_end: 250.0,
                        value_range: Some((0.0, 127.0)),
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            },
            history: crate::history::UndoStack::new(),
            file_name: "test".into(),
            file_path: None,
        };
        doc
    }

    #[test]
    fn apply_anchor_field_edit_value_add() {
        let mut doc = make_doc_with_anchor();
        let ops = crate::num_expr::parse_num_expr("+10").unwrap();
        let action = doc
            .apply_anchor_field_edit(0, AnchorField::Value, &ops)
            .expect("should edit");
        let lane = &doc.data.model.tracks[0].automation_lanes[0];
        assert_eq!(lane.events[0].value, 74.0);
        assert_eq!(lane.events[1].value, 106.0);
        assert!(matches!(action, UndoAction::Automation(_)));
    }

    #[test]
    fn apply_anchor_field_edit_value_uniform_moves_rect() {
        let mut doc = make_doc_with_anchor();
        let ops = crate::num_expr::parse_num_expr("-4").unwrap();
        doc.apply_anchor_field_edit(0, AnchorField::Value, &ops);
        let rect = doc.edit.controller_panels[0].anchor_sel_rects[0];
        assert_eq!(rect.value_range, Some((-4.0, 123.0)));
    }

    #[test]
    fn apply_anchor_field_edit_tick_add_moves_rect() {
        let mut doc = make_doc_with_anchor();
        let ops = crate::num_expr::parse_num_expr("+50").unwrap();
        doc.apply_anchor_field_edit(0, AnchorField::Tick, &ops);
        let lane = &doc.data.model.tracks[0].automation_lanes[0];
        assert_eq!(lane.events[0].tick, 150);
        assert_eq!(lane.events[1].tick, 250);
        let rect = doc.edit.controller_panels[0].anchor_sel_rects[0];
        assert_eq!(rect.tick_start, 50.0);
        assert_eq!(rect.tick_end, 300.0);
    }

    #[test]
    fn apply_anchor_field_edit_tick_mul_keeps_rect() {
        let mut doc = make_doc_with_anchor();
        let ops = crate::num_expr::parse_num_expr("x2").unwrap();
        doc.apply_anchor_field_edit(0, AnchorField::Tick, &ops);
        // 100→200、200→400：delta 不同 → 选框不动
        let lane = &doc.data.model.tracks[0].automation_lanes[0];
        assert_eq!(lane.events[0].tick, 200);
        assert_eq!(lane.events[1].tick, 400);
        let rect = doc.edit.controller_panels[0].anchor_sel_rects[0];
        assert_eq!(rect.tick_start, 0.0);
        assert_eq!(rect.tick_end, 250.0);
    }

    #[test]
    fn apply_anchor_field_edit_no_selection_returns_none() {
        let mut doc = make_doc_with_anchor();
        doc.edit.controller_panels[0].anchor_sel_rects.clear();
        let ops = crate::num_expr::parse_num_expr("+10").unwrap();
        assert!(
            doc.apply_anchor_field_edit(0, AnchorField::Value, &ops)
                .is_none()
        );
    }

    #[test]
    fn rescale_anchor_span_doubles_ticks() {
        let mut doc = make_doc_with_anchor();
        // 跨度 250 → 500（×2）：锚点 100→200，200→400
        doc.rescale_anchor_span(0, 500).expect("should edit");
        let lane = &doc.data.model.tracks[0].automation_lanes[0];
        assert_eq!(lane.events[0].tick, 200);
        assert_eq!(lane.events[1].tick, 400);
        let rect = doc.edit.controller_panels[0].anchor_sel_rects[0];
        assert_eq!(rect.tick_start, 0.0);
        assert_eq!(rect.tick_end, 500.0);
    }

    #[test]
    fn rescale_anchor_span_same_span_returns_none() {
        let mut doc = make_doc_with_anchor();
        assert!(doc.rescale_anchor_span(0, 250).is_none());
    }
}
