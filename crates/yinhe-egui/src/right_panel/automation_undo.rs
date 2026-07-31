//! 自动化事件编辑共享的 undo 工具。
//!
//! 提供 lane events 快照与 push undo 的统一实现，
//! 供 `info_panel/anchor`、`event_browser/edit` 与 `app/layout` 复用，消除重复逻辑。

use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{AutomationDelta, EditSnapshot, UndoAction};
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
        doc.data
            .model
            .tracks
            .get(track_idx as usize)
            .and_then(|t| t.automation_lanes.get(lane_idx))
            .map(|l| l.events.clone())
            .unwrap_or_default()
    }
}

/// push 一个 AutomationDelta undo entry。
///
/// 比较 `before` / `after`，差异时构造 `UndoAction` push 到 `doc.history`。
/// `snapshot` 必须是编辑**前**捕获的界面状态快照。
pub fn push_automation_undo(
    doc: &mut Document,
    track_idx: u16,
    lane_idx: usize,
    target: &AutomationTarget,
    before: Vec<AutomationEvent>,
    after: Vec<AutomationEvent>,
    label: &str,
    snapshot: EditSnapshot,
) {
    if before == after {
        return;
    }
    doc.push_undo(
        UndoAction::Automation(AutomationDelta {
            track_idx: track_idx as usize,
            lane_idx,
            target: target.clone(),
            before,
            after,
        }),
        label,
        snapshot,
    );
}

/// 把 `apply_automation_edits` 返回的 `UndoAction` 列表作为一个 undo entry push 到 history。
///
/// 多个 action 用 `Composite` 合并，一次操作 = 一次 undo（避免逐个 undo）。
/// `snapshot` 必须是编辑**前**捕获的界面状态快照。
pub fn push_automation_actions(
    doc: &mut Document,
    actions: Vec<UndoAction>,
    label: &str,
    snapshot: EditSnapshot,
) {
    if actions.is_empty() {
        return;
    }
    // len==1 时直接取唯一 action（不 unwrap：用 match 防御空迭代器）；否则 Composite 合并。
    let action = match actions.len() {
        1 => match actions.into_iter().next() {
            Some(a) => a,
            None => return,
        },
        _ => UndoAction::Composite(actions),
    };
    doc.push_undo(action, label, snapshot);
}
