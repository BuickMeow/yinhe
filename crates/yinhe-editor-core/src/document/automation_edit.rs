//! Automation event editing: add, move, delete, set-shape, apply-batch.

use std::sync::Arc;

use yinhe_types::AutomationEdit;
use yinhe_types::AutomationEvent;
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
        mut event: yinhe_types::AutomationEvent,
    ) -> Option<(usize, usize, UndoAction)> {
        // id 0 = 未分配：所有新增事件统一在此发号（UI 构造点传 0 即可）。
        if event.id == 0 {
            event.id = Arc::make_mut(&mut self.data.model).alloc_automation_id();
        }
        let is_tempo = matches!(target, yinhe_types::AutomationTarget::Tempo);
        // Tempo 无视 track_idx（conductor 持有），delta 里统一记 0/0。
        let result_track = if is_tempo { 0 } else { track_idx };
        let model = Arc::make_mut(&mut self.data.model);
        let (lane, lane_idx) = model.ensure_automation_lane_mut(track_idx, target.clone())?;
        lane.insert_sorted(event);
        model.commit_automation(&target);
        self.data.bump_revision();
        Some((
            result_track,
            lane_idx,
            UndoAction::Automation(AutomationDelta {
                track_idx: result_track,
                lane_idx,
                target,
                // 增量 delta：只存被编辑的事件（before 空 = 纯新增）。
                before: vec![],
                after: vec![event],
            }),
        ))
    }

    /// 创建空的 automation lane（同 target 已存在时返回 None）。
    /// 返回 (lane_idx, UndoAction)。lane 插入到该轨 lanes 末尾。
    ///
    /// 不支持 Tempo：Tempo lane 由 conductor 持有（conductor.tempo），
    /// 不在 track.automation_lanes，传 Tempo 返回 None。
    pub fn add_automation_lane(
        &mut self,
        track_idx: usize,
        target: AutomationTarget,
    ) -> Option<(usize, UndoAction)> {
        if matches!(target, AutomationTarget::Tempo) {
            return None;
        }
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);
        // 同 target 的 lane 每轨至多一条（与 add_automation_event 的懒创建一致）。
        if track.automation_lanes.iter().any(|l| l.target == target) {
            return None;
        }
        let lane = yinhe_types::AutomationLane {
            target,
            track: track_idx as u16,
            events: Vec::new(),
        };
        track.automation_lanes.push(lane.clone());
        let lane_idx = track.automation_lanes.len() - 1;
        self.data.bump_revision();
        Some((
            lane_idx,
            UndoAction::AutomationLane {
                track_idx,
                lane_idx,
                before: None,
                after: Some(lane),
            },
        ))
    }

    /// 删除整条 automation lane。返回 UndoAction（before 含完整 lane 供 undo 恢复）。
    pub fn remove_automation_lane(
        &mut self,
        track_idx: usize,
        lane_idx: usize,
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);
        if lane_idx >= track.automation_lanes.len() {
            return None;
        }
        let lane = track.automation_lanes.remove(lane_idx);
        self.data.bump_revision();
        Some(UndoAction::AutomationLane {
            track_idx,
            lane_idx,
            before: Some(lane),
            after: None,
        })
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
        let (lane, _) = model.automation_lane_mut(track_idx, target)?;
        let events = &mut lane.events;

        let mut before = Vec::with_capacity(2);
        // 原事件（操作前值）必须进 before，undo 才能恢复。
        let orig = *events.iter().find(|e| e.tick == old_tick)?;
        before.push(orig);

        if old_tick == new_tick {
            // 只改 value，不改 tick：直接原地修改，避免 retain 误删原事件
            let evt = events.iter_mut().find(|e| e.tick == old_tick)?;
            evt.value = new_value;
        } else {
            // 目标 tick 上已有的事件（冲突项）会被移除——必须进 before，
            // 否则 undo 丢失它（增量 delta 的前提：before 覆盖全部被移除项）。
            if let Some(conflict) = events.iter().find(|e| e.tick == new_tick) {
                before.push(*conflict);
            }
            events.retain(|e| e.tick != new_tick);
            // 找到原事件并修改
            let evt = events.iter_mut().find(|e| e.tick == old_tick)?;
            evt.tick = new_tick;
            evt.value = new_value;
            events.sort_by_key(|e| e.tick);
        }
        let after = vec![AutomationEvent {
            tick: new_tick,
            value: new_value,
            ..before[0] // 保留原 id/shape
        }];

        model.commit_automation(target);
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
        let (lane, _) = model.automation_lane_mut(track_idx, target)?;
        let events = &mut lane.events;

        // 增量 before：原事件（old_tick 处）+ 冲突项（new_tick 处非本次移动的事件）。
        // 冲突项定义：插入时会移除 new_tick 残留事件——残留 = tick ∈ new_ticks
        // 且 tick ∉ old_ticks 的既有事件（本次移动的事件已被 remove）。
        let old_ticks: std::collections::HashSet<u32> = moves.iter().map(|(o, _, _)| *o).collect();
        let mut before: Vec<AutomationEvent> = Vec::with_capacity(moves.len() * 2);
        for (old_tick, new_tick, _) in moves {
            if let Some(ev) = events.iter().find(|e| e.tick == *old_tick) {
                before.push(*ev);
            }
            if !old_ticks.contains(new_tick)
                && let Some(ev) = events.iter().find(|e| e.tick == *new_tick)
            {
                before.push(*ev);
            }
        }

        // 收集每个 old_tick 对应的 (shape, id)，并从 events 移除。
        // id 随事件移动保留（选择集成员身份不变）。
        let mut bases: Vec<(yinhe_types::SegmentShape, u32)> = Vec::with_capacity(moves.len());
        for (old_tick, _, _) in moves {
            if let Some(idx) = events.iter().position(|e| e.tick == *old_tick) {
                let e = events.remove(idx);
                bases.push((e.shape, e.id));
            } else {
                bases.push((target.default_shape(), 0));
            }
        }
        // 按 new_tick 排序后插入（冲突时后者覆盖）
        let mut sorted: Vec<(u32, f32, yinhe_types::SegmentShape, u32)> = moves
            .iter()
            .zip(bases.iter())
            .map(|((_, new, val), (shape, id))| (*new, *val, *shape, *id))
            .collect();
        sorted.sort_by_key(|(new, _, _, _)| *new);
        let mut after: Vec<AutomationEvent> = Vec::with_capacity(sorted.len());
        for (new_tick, new_value, shape, id) in sorted {
            // 移除 new_tick 处可能残留的旧事件
            if let Some(idx) = events.iter().position(|e| e.tick == new_tick) {
                events.remove(idx);
            }
            let insert_idx = events.partition_point(|e| e.tick < new_tick);
            let evt = AutomationEvent {
                id,
                tick: new_tick,
                value: new_value,
                shape,
            };
            events.insert(insert_idx, evt);
            after.push(evt);
        }
        model.commit_automation(target);
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
        let (lane, _) = model.automation_lane_mut(track_idx, target)?;

        // 增量 delta：before = 被删事件（undo 恢复的唯一信息源）。
        let before: Vec<AutomationEvent> = lane
            .events
            .iter()
            .filter(|e| e.tick == tick)
            .copied()
            .collect();
        if before.is_empty() {
            return None;
        }
        lane.events.retain(|e| e.tick != tick);
        let after = Vec::new();

        model.commit_automation(target);
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
        let (lane, _) = model.automation_lane_mut(track_idx, target)?;

        // 增量 delta：before = 原事件，after = 改后事件（单事件对）。
        let before: Vec<AutomationEvent> = lane
            .events
            .iter()
            .filter(|e| e.tick == tick)
            .copied()
            .collect();
        if before.is_empty() {
            return None;
        }
        let evt = lane.events.iter_mut().find(|e| e.tick == tick)?;
        if evt.shape == shape {
            return None;
        }
        evt.shape = shape;
        let after = vec![*evt];

        model.commit_automation(target);
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
                    let event = yinhe_types::AutomationEvent {
                        id: 0,
                        tick,
                        value,
                        shape,
                    };
                    self.add_automation_event(track_idx as usize, target, event)
                        .map(|(_, _, action)| action)
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
    /// `ops` 在显示值域输入（Tempo 为 BPM，其余为绑定表上限内的原始值），
    /// 写回 lane 时归一化（除 Tempo）。
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

        // 定位 lane（与 app 层 collect_anchor_ctx 同规则）：
        // Tempo → conductor.tempo；其他 → 主音轨的 target 匹配 lane。
        let (track_idx, lane_idx) = if matches!(target, AutomationTarget::Tempo) {
            (0u16, 0usize)
        } else {
            // 主音轨在 PR 强制可见（layout pr_visible），不要求 track_visible 勾选。
            let track_idx = self
                .edit
                .main_track()
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

        // 收集选中锚点 + 计算新值 + uniform 判定（只读借用，无需克隆）
        let events = if matches!(target, AutomationTarget::Tempo) {
            &self.data.model.conductor.tempo.events
        } else {
            &self.data.model.tracks[track_idx as usize].automation_lanes[lane_idx].events
        };
        // lane 存归一化值（Tempo 为 BPM 原值，不归一化）：Info 面板按
        // 显示值输入，这里换算。
        let display_max = match target.display_max() {
            Some(max) => max,
            // Tempo 的显示值就是 BPM（历史上限）；第三方插件参数按归一化值显示。
            None if matches!(target, AutomationTarget::Tempo) => 60_000_000.0,
            None => 1.0,
        };
        let mut moves: Vec<(u32, u32, f32)> = Vec::new();
        let mut uniform_tick: Option<i64> = None;
        let mut uniform_value: Option<f32> = None;
        for ev in events {
            // 成员态按 id 判定（落点锚点不纳入）；矩形态回退选框几何。
            if !panel.accepts_anchor(ev) {
                continue;
            }
            let (new_tick, new_value) = match field {
                AnchorField::Value => {
                    let cur = target.to_display_value(ev.value) as f64;
                    let display = apply_ops(ops, cur).clamp(0.0, display_max as f64) as f32;
                    (ev.tick, target.from_display_value(display))
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
            self.edit.offset_anchor_ticks(panel_idx, dt);
        }
        if let Some(dv) = uniform_value {
            self.edit.offset_anchor_values(panel_idx, dv);
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
            // 主音轨在 PR 强制可见（layout pr_visible），不要求 track_visible 勾选。
            let track_idx = self
                .edit
                .main_track()
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
            &self.data.model.conductor.tempo.events
        } else {
            &self.data.model.tracks[track_idx as usize].automation_lanes[lane_idx].events
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
        for ev in events {
            // 成员态按 id 判定；矩形态回退选框几何。
            if !panel.accepts_anchor(ev) {
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
        self.edit.scale_anchor_ticks(panel_idx, t0, factor);
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
                t.automation_lanes.push(AutomationLane {
                    target: AutomationTarget::CC { controller: 7 },
                    track: 0,
                    events: vec![
                        AutomationEvent {
                            id: 0,
                            tick: 100,
                            value: 64.0 / 127.0,
                            shape: SegmentShape::Step,
                        },
                        AutomationEvent {
                            id: 0,
                            tick: 200,
                            value: 96.0 / 127.0,
                            shape: SegmentShape::Step,
                        },
                    ],
                });
                t
            })],
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
                track_selected: [0u16].into_iter().collect(),
                controller_panels: vec![AutomationPanelView {
                    show_velocity: false,
                    selected_target: AutomationTarget::CC { controller: 7 },
                    anchor_sel_rects: vec![AnchorSelRect {
                        tick_start: 0.0,
                        tick_end: 250.0,
                        value_range: Some((0.0, 1.0)),
                    }],
                    ..Default::default()
                }],
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

    #[test]
    fn apply_anchor_field_edit_value_add() {
        let mut doc = make_doc_with_anchor();
        let ops = crate::num_expr::parse_num_expr("+10").unwrap();
        let action = doc
            .apply_anchor_field_edit(0, AnchorField::Value, &ops)
            .expect("should edit");
        let lane = &doc.data.model.tracks[0].automation_lanes[0];
        // Info 面板按显示值输入（+10 个 CC 单位），lane 存归一化值。
        assert!((lane.events[0].value - 74.0 / 127.0).abs() < 1e-6);
        assert!((lane.events[1].value - 106.0 / 127.0).abs() < 1e-6);
        assert!(matches!(action, UndoAction::Automation(_)));
    }

    #[test]
    fn apply_anchor_field_edit_value_uniform_moves_rect() {
        let mut doc = make_doc_with_anchor();
        let ops = crate::num_expr::parse_num_expr("-4").unwrap();
        doc.apply_anchor_field_edit(0, AnchorField::Value, &ops);
        let rect = doc.edit.controller_panels[0].anchor_sel_rects[0];
        let dv = -4.0 / 127.0;
        let (lo, hi) = rect.value_range.expect("rect 应有 value_range");
        assert!((lo - dv).abs() < 1e-6);
        assert!((hi - (1.0 + dv)).abs() < 1e-6);
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

    #[test]
    fn rescale_anchor_undo_restores_rect() {
        let mut doc = make_doc_with_anchor();
        let before = doc.capture_snapshot();
        let action = doc.rescale_anchor_span(0, 500).expect("should edit");
        doc.push_undo(action, "rescale", before);
        assert!(doc.undo(), "undo 应成功");
        let lane = &doc.data.model.tracks[0].automation_lanes[0];
        assert_eq!(lane.events[0].tick, 100, "锚点 tick 恢复");
        let rect = doc.edit.controller_panels[0].anchor_sel_rects[0];
        assert_eq!(rect.tick_start, 0.0, "AM 选框恢复");
        assert_eq!(rect.tick_end, 250.0);
    }

    /// 给 lane 事件分配 id（生产路径由加载/编辑发号；直接构造的模型手动指定）。
    fn assign_lane_ids(doc: &mut Document) {
        let model = Arc::make_mut(&mut doc.data.model);
        let track = Arc::make_mut(&mut model.tracks[0]);
        for (i, e) in track.automation_lanes[0].events.iter_mut().enumerate() {
            e.id = i as u32 + 1;
        }
        model.next_automation_id = 10;
    }

    /// 回归：锚点成员态下选框平移到落点后再拖动，不会吸收落点处的锚点。
    /// （修复前逐次按选框几何重收集，落点路人锚点会被一起搬走。）
    #[test]
    fn materialized_anchor_selection_stable_across_moves() {
        let mut doc = make_doc_with_anchor();
        assign_lane_ids(&mut doc);
        // lane: id1@100（被框选）、id2@200（落点路人）。
        // 框选 [100,150) → 物化成员 = {id1}。
        doc.edit.controller_panels[0].anchor_sel_rects = vec![AnchorSelRect {
            tick_start: 100.0,
            tick_end: 150.0,
            value_range: None,
        }];
        let lane = &doc.data.model.tracks[0].automation_lanes[0];
        doc.edit.controller_panels[0].materialize_anchor_pending(lane);
        assert_eq!(doc.edit.controller_panels[0].anchor_member_count(), Some(1));

        // 拖动提交：id1 100→250（避免与 id2@200 同 tick 覆盖），选框平移到落点。
        let target = AutomationTarget::CC { controller: 7 };
        doc.move_automation_events_batch(0, 0, &target, &[(100, 250, 64.0 / 127.0)])
            .expect("移动应成功");
        // 落点选框扩大覆盖到 id2@200（模拟第二轮拖动时的选框范围）。
        doc.edit.controller_panels[0].anchor_sel_rects[0].tick_start = 150.0;
        doc.edit.controller_panels[0].anchor_sel_rects[0].tick_end = 320.0;

        // 第二次拖动收集：成员态只认 id1，落点选框内的 id2 不得被吸收。
        let lane = &doc.data.model.tracks[0].automation_lanes[0];
        let selected: Vec<u32> = lane
            .events
            .iter()
            .filter(|e| doc.edit.controller_panels[0].accepts_anchor(e))
            .map(|e| e.id)
            .collect();
        assert_eq!(selected, vec![1], "落点选框内的 id2 不得被选中");
        assert!(
            lane.events.iter().any(|e| e.id == 2 && e.tick == 200),
            "id2 应留在原处"
        );
    }
}
