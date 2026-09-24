//! 自动化 lane 的模型级存取：Tempo 特例统一、lane 懒创建、tempo_map 收尾。
//!
//! editor-core 原先在 7 处手写「Tempo → conductor.tempo，其余 → 轨道 lane」
//! 的分派，并在 12+ 处手工调用 `rebuild_tempo_map`（漏一处播放就用旧 tempo）。
//! 全部收进本模块。

use std::sync::Arc;

use yinhe_types::{AutomationLane, AutomationTarget};

use crate::model::YinModel;

impl YinModel {
    /// 定位 target 对应的事件 lane（可变）。
    ///
    /// - `Tempo` → `conductor.tempo`（`lane_idx` 恒为 0，与 `AutomationDelta` 约定一致）；
    /// - 其余 → `tracks[track_idx]` 中 `target` 匹配的 lane（不存在返回 `None`）。
    pub fn automation_lane_mut(
        &mut self,
        track_idx: usize,
        target: &AutomationTarget,
    ) -> Option<(&mut AutomationLane, usize)> {
        if matches!(target, AutomationTarget::Tempo) {
            return Some((&mut Arc::make_mut(&mut self.conductor).tempo, 0));
        }
        let track = Arc::make_mut(self.tracks.get_mut(track_idx)?);
        let idx = track
            .automation_lanes
            .iter()
            .position(|l| l.target == *target)?;
        Some((&mut track.automation_lanes[idx], idx))
    }

    /// 定位或懒创建 target 的 lane（可变）。
    ///
    /// `Tempo` 恒存在；轨道不存在返回 `None`。同一轨同一 target 至多一条 lane
    /// 的模型不变量由本方法维护（不存在则追加）。
    pub fn ensure_automation_lane_mut(
        &mut self,
        track_idx: usize,
        target: AutomationTarget,
    ) -> Option<(&mut AutomationLane, usize)> {
        if matches!(target, AutomationTarget::Tempo) {
            return Some((&mut Arc::make_mut(&mut self.conductor).tempo, 0));
        }
        let track = Arc::make_mut(self.tracks.get_mut(track_idx)?);
        let idx = match track
            .automation_lanes
            .iter()
            .position(|l| l.target == target)
        {
            Some(idx) => idx,
            None => {
                track.automation_lanes.push(AutomationLane {
                    target,
                    track: track_idx as u16,
                    events: Vec::new(),
                });
                track.automation_lanes.len() - 1
            }
        };
        Some((&mut track.automation_lanes[idx], idx))
    }

    /// 事件列表变更收尾：`Tempo` 影响 `tempo_map`，统一在此重建。
    ///
    /// 所有自动化写入路径（含 undo 回放）必须在结束对 lane 的借用后调用，
    /// 否则播放/光标换算会使用旧 tempo。
    pub fn commit_automation(&mut self, target: &AutomationTarget) {
        if matches!(target, AutomationTarget::Tempo) {
            self.rebuild_tempo_map();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TrackData;
    use yinhe_types::{AutomationEvent, SegmentShape};

    fn evt(id: u32, tick: u32) -> AutomationEvent {
        AutomationEvent {
            id,
            tick,
            value: 0.5,
            shape: SegmentShape::Step,
        }
    }

    fn model() -> YinModel {
        YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        }
    }

    /// 懒创建 lane 与 Tempo 特例：同一 target 至多一条 lane。
    #[test]
    fn ensure_lane_creates_once_and_tempo_is_special() {
        let mut m = model();
        let target = AutomationTarget::CC { controller: 7 };
        let (lane, idx) = m
            .ensure_automation_lane_mut(0, target.clone())
            .expect("lane");
        assert_eq!(idx, 0);
        assert_eq!(lane.track, 0);
        lane.upsert(evt(1, 100));

        let (_, idx2) = m.ensure_automation_lane_mut(0, target).expect("lane");
        assert_eq!(idx2, 0, "同 target 复用同一 lane");
        assert_eq!(m.tracks[0].automation_lanes.len(), 1);

        let (tempo, tidx) = m
            .automation_lane_mut(0, &AutomationTarget::Tempo)
            .expect("tempo");
        assert_eq!(tidx, 0);
        assert!(tempo.events.is_empty());

        assert!(
            m.automation_lane_mut(9, &AutomationTarget::Tempo).is_some(),
            "Tempo 不依赖 track"
        );
        assert!(
            m.automation_lane_mut(9, &AutomationTarget::CC { controller: 7 })
                .is_none(),
            "越界轨道返回 None"
        );
    }

    /// commit_automation：Tempo 触发 tempo_map 重建，非 Tempo 不动。
    #[test]
    fn commit_automation_rebuilds_tempo_map_only_for_tempo() {
        let mut m = model();
        let before = Arc::as_ptr(&m.tempo_map);
        let (lane, _) = m
            .ensure_automation_lane_mut(0, AutomationTarget::CC { controller: 7 })
            .expect("lane");
        lane.upsert(evt(1, 0));
        m.commit_automation(&AutomationTarget::CC { controller: 7 });
        assert_eq!(
            Arc::as_ptr(&m.tempo_map),
            before,
            "非 Tempo 变更不应重建 tempo_map"
        );

        let (tempo, _) = m
            .automation_lane_mut(0, &AutomationTarget::Tempo)
            .expect("tempo");
        tempo.upsert(AutomationEvent {
            value: 140.0,
            ..evt(2, 0)
        });
        m.commit_automation(&AutomationTarget::Tempo);
        assert_ne!(
            Arc::as_ptr(&m.tempo_map),
            before,
            "Tempo 变更必须重建 tempo_map"
        );
    }
}
