//! 自动化事件编辑共享的 undo 工具。
//!
//! 提供 lane events 快照与 push undo 的统一实现，
//! 供 `info_panel/anchor`、`event_browser/edit` 与 `app/layout` 复用，消除重复逻辑。

use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{AutomationDelta, UndoAction, UndoEntry};
use yinhe_types::{AutomationEvent, AutomationTarget};

/// 按 target 取 lane events 快照。
///
/// `Tempo` 走 `conductor.tempo`，其他走 `track.automation_lanes`。
pub fn snapshot_lane_events(
    doc: &Document,
    track_idx: u16,
    lane_idx: usize,
    target: &AutomationTarget,
) -> Vec<AutomationEvent> {
    if matches!(target, AutomationTarget::Tempo) {
        doc.data.model.conductor.tempo.events.clone()
    } else {
        doc.data.model
            .tracks
            .get(track_idx as usize)
            .and_then(|t| t.automation_lanes.get(lane_idx))
            .map(|l| l.events.clone())
            .unwrap_or_default()
    }
}

/// push 一个 AutomationDelta undo entry。
///
/// 比较 `before` / `after`，差异时构造 `UndoEntry` push 到 `doc.history`。
pub fn push_automation_undo(
    doc: &mut Document,
    track_idx: u16,
    lane_idx: usize,
    target: &AutomationTarget,
    before: Vec<AutomationEvent>,
    after: Vec<AutomationEvent>,
    label: &str,
) {
    if before == after {
        return;
    }
    doc.history.push(UndoEntry {
        action: UndoAction::Automation(AutomationDelta {
            track_idx: track_idx as usize,
            lane_idx,
            target: target.clone(),
            before,
            after,
        }),
        label: label.to_string(),
        selected: doc.edit.selected.clone(),
        track_selected: doc.edit.track_selected.clone(),
        sel_rect: doc.edit.sel_rect.clone(),
    });
}

/// 把 `apply_automation_edits` 返回的 `UndoAction` 列表逐个 push 到 history。
///
/// 用于一次性操作（如 `CycleShape`），区别于 [`push_automation_undo`] 的 before/after 模式：
/// - `push_automation_undo`：DragValue 持续编辑，gained 时记录 before，lost 时取 after 对比
/// - `push_automation_actions`：单次操作，`apply_automation_edits` 已返回构造好的 actions
pub fn push_automation_actions(doc: &mut Document, actions: Vec<UndoAction>, label: &str) {
    for action in actions {
        doc.history.push(UndoEntry {
            action,
            label: label.to_string(),
            selected: doc.edit.selected.clone(),
            track_selected: doc.edit.track_selected.clone(),
            sel_rect: doc.edit.sel_rect.clone(),
        });
    }
}
