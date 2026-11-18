//! 自动化冲突检测（方案 C 最新决策 #13）。
//!
//! 同一 `AutomationTarget` 被多条轨道的 lane 在同一 tick 写成不同值时，
//! 产生冲突。默认「最后编辑优先」（回放按 track 顺序发射事件，末条生效）；
//! UI 可展示标记并让用户选「以哪条为准」，选择结果存在编辑态里。

use std::collections::HashMap;

use yinhe_core::YinModel;
use yinhe_types::AutomationTarget;

/// 一个冲突点：同一 target/tick 上有多个不同的写入值。
#[derive(Clone, Debug, PartialEq)]
pub struct AutomationConflict {
    pub target: AutomationTarget,
    pub tick: u32,
    /// 候选写入：`(轨道索引, lane 下标, 值)`，按轨道索引升序。
    pub candidates: Vec<(u16, usize, f32)>,
}

/// 扫描模型里全部轨道的自动化 lane，找出冲突点。
///
/// 只比较「同一 target、同一 tick、不同值」且来自不同轨道的写入；
/// 同一轨道的多条 lane 不算冲突（它们本就按 lane 顺序叠加）。
pub fn find_conflicts(model: &YinModel) -> Vec<AutomationConflict> {
    // (target, tick) -> track -> (lane, value)
    let mut grouped: HashMap<(AutomationTarget, u32), HashMap<u16, (usize, f32)>> = HashMap::new();
    for (track_idx, track) in model.tracks.iter().enumerate() {
        for (lane_idx, lane) in track.automation_lanes.iter().enumerate() {
            for event in &lane.events {
                grouped
                    .entry((lane.target.clone(), event.tick))
                    .or_default()
                    .entry(track_idx as u16)
                    .or_insert((lane_idx, event.value));
            }
        }
    }

    let mut out: Vec<AutomationConflict> = grouped
        .into_iter()
        .filter_map(|((target, tick), by_track)| {
            let mut candidates: Vec<_> = by_track
                .into_iter()
                .map(|(track, (lane, value))| (track, lane, value))
                .collect();
            candidates.sort_unstable_by_key(|&(track, _, _)| track);
            let distinct: std::collections::HashSet<u32> = candidates
                .iter()
                .map(|(_, _, value)| value.to_bits())
                .collect();
            if candidates.len() > 1 && distinct.len() > 1 {
                Some(AutomationConflict {
                    target,
                    tick,
                    candidates,
                })
            } else {
                None
            }
        })
        .collect();
    out.sort_by(|a, b| a.target.cmp(&b.target).then_with(|| a.tick.cmp(&b.tick)));
    out
}

/// 某 (target, tick) 上存在冲突且候选里包含 `track` 时，返回该冲突。
pub fn conflict_at<'a>(
    conflicts: &'a [AutomationConflict],
    target: &AutomationTarget,
    tick: u32,
    track: u16,
) -> Option<&'a AutomationConflict> {
    conflicts.iter().find(|conflict| {
        conflict.target == *target
            && conflict.tick == tick
            && conflict.candidates.iter().any(|&(t, _, _)| t == track)
    })
}

use crate::document::Document;
use crate::history::{AutomationDelta, UndoAction};

impl Document {
    /// 裁决冲突：把 `(target, tick)` 上其它轨道的事件值改成获胜轨道的值。
    /// 返回每条被修改轨道的 undo（调用方逐个 push，保持与其他 AM 编辑一致）。
    pub fn resolve_automation_conflict(
        &mut self,
        target: &AutomationTarget,
        tick: u32,
        winner_track: u16,
    ) -> Vec<UndoAction> {
        let conflicts = find_conflicts(&self.data.model);
        let Some(conflict) = conflict_at(&conflicts, target, tick, winner_track) else {
            return Vec::new();
        };
        let Some(&(_, _, winning_value)) = conflict
            .candidates
            .iter()
            .find(|&&(track, _, _)| track == winner_track)
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for &(track, lane_idx, value) in &conflict.candidates {
            if track == winner_track || (value - winning_value).abs() < f32::EPSILON {
                continue;
            }
            let model = std::sync::Arc::make_mut(&mut self.data.model);
            let Some(lane) = model.tracks.get(track as usize).and_then(|t| {
                t.automation_lanes
                    .get(lane_idx)
                    .filter(|l| l.target == *target)
            }) else {
                continue;
            };
            let Some(pos) = lane.events.iter().position(|e| e.tick == tick) else {
                continue;
            };
            let before_event = lane.events[pos];
            let mut after_event = before_event;
            after_event.value = winning_value;
            let Some(lane) = std::sync::Arc::make_mut(&mut model.tracks[track as usize])
                .automation_lanes
                .get_mut(lane_idx)
            else {
                continue;
            };
            lane.events[pos] = after_event;
            out.push(UndoAction::Automation(AutomationDelta {
                track_idx: track as usize,
                lane_idx,
                target: target.clone(),
                before: vec![before_event],
                after: vec![after_event],
            }));
        }
        if !out.is_empty() {
            self.data.bump_revision();
            self.edit
                .am_conflict_choice
                .insert((target.clone(), tick), winner_track);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use yinhe_core::TrackKind;
    use yinhe_types::{AutomationEvent, AutomationLane, SegmentShape};

    fn lane(target: AutomationTarget, tick: u32, value: f32) -> AutomationLane {
        AutomationLane {
            target,
            track: 0,
            events: vec![AutomationEvent {
                id: 0,
                tick,
                value,
                shape: SegmentShape::Step,
            }],
        }
    }

    #[test]
    fn detects_same_target_tick_different_values() {
        let target = AutomationTarget::CC { controller: 7 };
        let mut model = YinModel::new_empty_with_16_tracks();
        {
            let t = Arc::make_mut(&mut model.tracks[1]);
            t.kind = TrackKind::Midi;
            t.automation_lanes = vec![lane(target.clone(), 100, 0.2)];
        }
        {
            let t = Arc::make_mut(&mut model.tracks[2]);
            t.kind = TrackKind::Midi;
            t.automation_lanes = vec![lane(target.clone(), 100, 0.8)];
        }
        let conflicts = find_conflicts(&model);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].tick, 100);
        assert_eq!(conflicts[0].candidates.len(), 2);
    }

    #[test]
    fn resolution_writes_winning_value_to_other_tracks() {
        use crate::document::Document;
        let target = AutomationTarget::CC { controller: 7 };
        let mut doc = Document::empty();
        {
            let model = std::sync::Arc::make_mut(&mut doc.data.model);
            {
                let t = std::sync::Arc::make_mut(&mut model.tracks[1]);
                t.automation_lanes = vec![lane(target.clone(), 100, 0.2)];
            }
            {
                let t = std::sync::Arc::make_mut(&mut model.tracks[2]);
                t.automation_lanes = vec![lane(target.clone(), 100, 0.8)];
            }
        }
        let actions = doc.resolve_automation_conflict(&target, 100, 1);
        assert_eq!(actions.len(), 1, "只有另一条轨被改写");
        let value_of =
            |track: usize| doc.data.model.tracks[track].automation_lanes[0].events[0].value;
        assert!((value_of(1) - 0.2).abs() < 1e-6);
        assert!((value_of(2) - 0.2).abs() < 1e-6, "应以 Track 1 为准");
        assert!(find_conflicts(&doc.data.model).is_empty());
    }

    #[test]
    fn same_value_is_not_a_conflict() {
        let target = AutomationTarget::Tempo;
        let mut model = YinModel::new_empty_with_16_tracks();
        {
            let t = Arc::make_mut(&mut model.tracks[0]);
            t.automation_lanes = vec![lane(target.clone(), 50, 120.0)];
        }
        {
            let t = Arc::make_mut(&mut model.tracks[1]);
            t.automation_lanes = vec![lane(target.clone(), 50, 120.0)];
        }
        assert!(find_conflicts(&model).is_empty());
    }
}
