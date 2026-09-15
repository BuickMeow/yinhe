//! 插件参数自动化的 GUI 录制。
//!
//! 插件原生 GUI（VST3 `performEdit`）改参 → 每帧从乐器机架取出归一化值，
//! 写入该参数**已存在的** AM lane（光标位置）；插件没有 lane 时不写
//!（避免用户只想临时调音色却被记录）。
//!
//! 一次 GUI 拖动（`beginEdit`/`endEdit` 界定）合并为一条 undo：
//! 首个变化捕获快照，编辑结束（endEdit 到达）且当帧无新变化时提交。

use rust_i18n::t;
use yinhe_types::{AutomationEvent, AutomationTarget};

use crate::app::App;

/// 本次拖动写过的单个 lane（undo 提交用）：写入前事件 + 定位。
struct PluginParamWrite {
    track_idx: u16,
    lane_idx: usize,
    target: AutomationTarget,
    before: Vec<AutomationEvent>,
}

/// 一次插件参数 GUI 拖动会话（undo 合并边界）。
pub(crate) struct PluginParamDrag {
    writes: Vec<PluginParamWrite>,
    /// 编辑前捕获的界面快照。
    snapshot: yinhe_editor_core::history::EditSnapshot,
}

impl App {
    /// 每帧处理插件 GUI 的改参队列（归一化值）与编辑状态。
    pub(crate) fn poll_plugin_param_automation(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let (changes, editing) = {
            let Some(rack) = self.instrument_racks.get_mut(idx) else {
                return;
            };
            (
                std::mem::take(&mut rack.gui_param_changes),
                rack.gui_param_editing.clone(),
            )
        };
        let tick = {
            let doc = &self.workspace.documents[idx];
            doc.edit.cursor_tick.unwrap_or(0.0).max(0.0) as u32
        };
        let mut any_change = false;
        for (ich, param_id, value) in changes {
            let Some((track_idx, lane_idx, target)) = self.plugin_param_lane(idx, ich, param_id)
            else {
                continue;
            };
            // 首个变化：捕获快照（一次拖动 = 一条 undo）。
            if self.plugin_param_drag.is_none() {
                let doc = &self.workspace.documents[idx];
                self.plugin_param_drag = Some(PluginParamDrag {
                    writes: Vec::new(),
                    snapshot: doc.capture_snapshot(),
                });
            }
            // 本拖动首次写该 lane：先记录写入前事件（undo 用）。
            let already = self.plugin_param_drag.as_ref().is_some_and(|d| {
                d.writes
                    .iter()
                    .any(|w| w.track_idx == track_idx && w.target == target)
            });
            if !already {
                let doc = &self.workspace.documents[idx];
                let before = crate::right_panel::automation_undo::snapshot_lane_events(
                    doc, track_idx, lane_idx, &target,
                );
                if let Some(drag) = self.plugin_param_drag.as_mut() {
                    drag.writes.push(PluginParamWrite {
                        track_idx,
                        lane_idx,
                        target: target.clone(),
                        before,
                    });
                }
            }
            // 写入光标位置（同 tick 覆盖）。
            let value = value.clamp(0.0, 1.0) as f32;
            {
                let doc = &mut self.workspace.documents[idx];
                let has_event = doc.data.model.tracks[track_idx as usize].automation_lanes
                    [lane_idx]
                    .events
                    .iter()
                    .any(|e| e.tick == tick);
                if has_event {
                    doc.move_automation_event(
                        track_idx as usize,
                        lane_idx,
                        &target,
                        tick,
                        tick,
                        value,
                    );
                } else {
                    doc.add_automation_event(
                        track_idx as usize,
                        target.clone(),
                        AutomationEvent {
                            tick,
                            value,
                            shape: target.default_shape(),
                        },
                    );
                }
            }
            any_change = true;
        }
        if any_change {
            self.notify_audio_model_changed();
        }
        // 提交条件：编辑结束（endEdit 已到或插件不发 begin/end）+ 当帧无新变化。
        // 鼠标停顿时插件可能不再发 performEdit，但 editing 仍为 true → 不提前提交。
        if !any_change
            && editing.is_empty()
            && let Some(drag) = self.plugin_param_drag.take()
        {
            let doc = &mut self.workspace.documents[idx];
            for w in drag.writes {
                let after = crate::right_panel::automation_undo::snapshot_lane_events(
                    doc,
                    w.track_idx,
                    w.lane_idx,
                    &w.target,
                );
                crate::right_panel::automation_undo::push_automation_undo(
                    doc,
                    w.track_idx,
                    w.lane_idx,
                    &w.target,
                    w.before,
                    after,
                    t!("undo.edit_plugin_param").as_ref(),
                    drag.snapshot.clone(),
                );
            }
        }
    }

    /// 添加/移除一条 AM lane（「添加自动化」窗口的条目切换）。
    /// 返回操作后该 lane 是否存在（供窗口回写对勾）。
    pub(crate) fn apply_automation_toggle(
        &mut self,
        idx: usize,
        track_idx: usize,
        target: &AutomationTarget,
        add: bool,
    ) -> bool {
        let (snapshot, action, exists_after) = {
            let doc = &mut self.workspace.documents[idx];
            if track_idx >= doc.data.model.tracks.len() {
                return false;
            }
            let lane_idx = doc.data.model.tracks[track_idx]
                .automation_lanes
                .iter()
                .position(|l| &l.target == target);
            if add {
                if lane_idx.is_some() {
                    return true;
                }
                let snapshot = doc.capture_snapshot();
                let action = doc.add_automation_lane(track_idx, target.clone());
                (Some(snapshot), action, true)
            } else {
                let Some(li) = lane_idx else {
                    return false;
                };
                let snapshot = doc.capture_snapshot();
                let action = doc.remove_automation_lane(track_idx, li).map(|a| (li, a));
                (Some(snapshot), action, false)
            }
        };
        // 添加后自动展开该轨的自动化面板（移除时保持现状）。
        {
            let doc = &mut self.workspace.documents[idx];
            if let Some(e) = doc.edit.arr_am_expanded.get_mut(track_idx) {
                *e = true;
            }
        }
        if let (Some(snapshot), Some((_li, action))) = (snapshot, action) {
            let label = if add {
                t!("undo.create_automation")
            } else {
                t!("undo.delete_automation_lane")
            };
            let doc = &mut self.workspace.documents[idx];
            doc.push_undo(action, label.as_ref(), snapshot);
            self.notify_audio_model_changed();
        }
        exists_after
    }

    /// ParamPanel 的「显示自动化」按钮：确保该插件参数的 AM lane 存在并展开轨道。
    /// 已存在时只展开（不重复创建、不产生 undo）。
    pub(crate) fn toggle_plugin_param_lane(
        &mut self,
        idx: usize,
        channel: u8,
        param_id: u32,
        name: &str,
    ) {
        let target = AutomationTarget::PluginParam {
            channel,
            param_id,
            name: name.to_string(),
        };
        let Some(track_idx) = ({
            let doc = &self.workspace.documents[idx];
            doc.data
                .model
                .tracks
                .iter()
                .position(|t| t.global_channel() == channel)
        }) else {
            return;
        };
        let (snapshot, action) = {
            let doc = &mut self.workspace.documents[idx];
            let exists = doc.data.model.tracks[track_idx]
                .automation_lanes
                .iter()
                .any(|l| l.target == target);
            if exists {
                (None, None)
            } else {
                let snapshot = doc.capture_snapshot();
                let action = doc.add_automation_lane(track_idx, target);
                (Some(snapshot), action)
            }
        };
        // 创建后自动展开该轨的自动化面板（已存在则只展开）。
        {
            let doc = &mut self.workspace.documents[idx];
            if let Some(e) = doc.edit.arr_am_expanded.get_mut(track_idx) {
                *e = true;
            }
        }
        if let (Some(snapshot), Some((_, action))) = (snapshot, action) {
            let doc = &mut self.workspace.documents[idx];
            doc.push_undo(action, t!("undo.create_automation").as_ref(), snapshot);
            self.notify_audio_model_changed();
        }
    }

    /// 找插件 GUI 改参对应的 AM lane：`(track_idx, lane_idx, target)`。
    ///
    /// 匹配规则：轨道归属该乐器通道；lane 的 target 是同通道同 param_id 的
    /// `PluginParam`（name 不参与匹配，换插件显示名变化不影响）。
    fn plugin_param_lane(
        &self,
        idx: usize,
        channel: u8,
        param_id: u32,
    ) -> Option<(u16, usize, AutomationTarget)> {
        let doc = &self.workspace.documents[idx];
        let track_idx = doc
            .data
            .model
            .tracks
            .iter()
            .position(|t| t.global_channel() == channel)?;
        let track = doc.data.model.tracks.get(track_idx)?;
        let lane_idx = track.automation_lanes.iter().position(|l| {
            matches!(
                &l.target,
                AutomationTarget::PluginParam {
                    channel: c,
                    param_id: pid,
                    ..
                } if *c == channel && *pid == param_id
            )
        })?;
        Some((
            track_idx as u16,
            lane_idx,
            track.automation_lanes[lane_idx].target.clone(),
        ))
    }
}
