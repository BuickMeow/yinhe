//! 参数旋钮：绘制、拖动会话与自动化写入。

use rust_i18n::t;
use yinhe_editor_core::document::Document;

use super::*;

/// 写入目标轨：优先已有该 target lane 的轨（保持导入的 "CC xx" 轨结构），
/// 都没有时用第一条轨（lane 懒创建）。
fn lane_write_track(
    model: &yinhe_core::YinModel,
    track_tis: &[usize],
    target: &AutomationTarget,
) -> usize {
    track_tis
        .iter()
        .copied()
        .find(|&ti| {
            model
                .tracks
                .get(ti)
                .is_some_and(|t| t.automation_lanes.iter().any(|l| l.target == *target))
        })
        .or_else(|| track_tis.first().copied())
        .unwrap_or(0)
}

/// 参数纵向列表：每项「旋钮 + 右侧两行（名称、数值）」。
pub(super) fn param_knobs(ui: &mut egui::Ui, values: &[DockParam], actions: &mut Vec<KnobAction>) {
    for param in values {
        knob_row(ui, param, actions);
    }
}

/// 单个参数行：左旋钮 + 右两行（第一行名称、第二行数值）。
pub(super) fn knob_row(ui: &mut egui::Ui, param: &DockParam, actions: &mut Vec<KnobAction>) {
    // 统一参数模型：lane 存归一化值，旋钮直接用（显示换算只在文本处）。
    let mut norm = param.current.unwrap_or(param.default).clamp(0.0, 1.0);
    let target = &param.target;

    ui.horizontal(|ui| {
        let resp = if param.centered {
            crate::widgets::knob::knob_centered(ui, &mut norm, 28.0)
        } else {
            crate::widgets::knob::knob(ui, &mut norm, 28.0)
        };
        crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.dock.knob"));
        if resp.drag_started() {
            actions.push(KnobAction::DragStart(target.clone()));
        }
        if resp.dragged() {
            actions.push(KnobAction::Drag(target.clone(), norm));
        }
        if resp.drag_stopped() {
            actions.push(KnobAction::DragStop(target.clone()));
        }

        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(&param.name)
                    .size(crate::theme::SMALL_FONT)
                    .color(crate::theme::text_primary()),
            );
            let (text, color) = match param.current {
                Some(v) => (
                    crate::piano_view::automation_panel::format_display_value(&param.target, v),
                    crate::theme::accent_active(),
                ),
                None => (
                    format!(
                        "{}（默认）",
                        crate::piano_view::automation_panel::format_display_value(
                            &param.target,
                            param.default,
                        )
                    ),
                    crate::theme::text_muted(),
                ),
            };
            ui.label(
                egui::RichText::new(text)
                    .size(crate::theme::SMALL_FONT)
                    .color(color),
            );
        });
    });
    ui.add_space(6.0);
}

/// 应用单帧旋钮动作：拖动中记录会话，松手写事件并 push 一条 undo。
pub(super) fn apply_knob_action(
    app: &mut App,
    idx: usize,
    track_tis: &[usize],
    tick: u32,
    action: KnobAction,
) {
    match action {
        KnobAction::DragStart(target) => {
            // 这里只记录会话（松手才可能落 lane）。
            let doc = &mut app.workspace.documents[idx];
            if let Some(drag) = begin_knob_drag(doc, track_tis, target, tick) {
                app.knob_drag = Some(drag);
            }
        }
        KnobAction::Drag(target, norm) => {
            // 无实时预览通道，拖动中不写模型（松手落 lane，避免每帧重 flatten 卡顿）。
            let Some(mut drag) = app.knob_drag.take() else {
                return;
            };
            if drag.target == target {
                // 仅更新本地快照位置（松手时用），不写模型。
                drag.tick = tick;
                drag.last_norm = norm;
                drag.moved = true;
            }
            app.knob_drag = Some(drag);
        }
        KnobAction::DragStop(target) => {
            let Some(drag) = app.knob_drag.take_if(|d| d.target == target) else {
                return;
            };
            // 用松手时的最终值写一条事件（拖动过程不写模型，避免每帧重 flatten）。
            let doc = &mut app.workspace.documents[idx];
            finish_knob_drag(doc, drag);
            app.notify_audio_model_changed();
        }
    }
}

/// 开始旋钮拖动会话：记录目标轨与 lane 编辑前状态（undo 用）。
pub(super) fn begin_knob_drag(
    doc: &mut Document,
    track_tis: &[usize],
    target: AutomationTarget,
    tick: u32,
) -> Option<KnobDrag> {
    // 目标轨：优先已有该 target lane 的轨（保持导入的 "CC xx" 轨结构）。
    let track_idx = lane_write_track(&doc.data.model, track_tis, &target);
    if track_idx >= doc.data.model.tracks.len() {
        return None;
    }
    let lane_pos = doc.data.model.tracks[track_idx]
        .automation_lanes
        .iter()
        .position(|l| l.target == target);
    let before = lane_pos
        .map(|li| {
            doc.data.model.tracks[track_idx].automation_lanes[li]
                .events
                .clone()
        })
        .unwrap_or_default();
    Some(KnobDrag {
        last_norm: 0.0,
        moved: false,
        track_idx,
        target,
        tick,
        snapshot: doc.capture_snapshot(),
        before,
        lane_idx: lane_pos,
    })
}

/// 结束拖动会话：按 `drag.last_norm` 写事件并 push 一条 undo。
pub(super) fn finish_knob_drag(doc: &mut Document, mut drag: KnobDrag) {
    let norm = drag.last_norm;
    upsert_automation_event(doc, &mut drag, norm);
    let Some(lane_idx) = drag.lane_idx else {
        return;
    };
    let after = crate::right_panel::automation_undo::snapshot_lane_events(
        doc,
        drag.track_idx as u16,
        lane_idx,
        &drag.target,
    );
    crate::right_panel::automation_undo::push_automation_undo(
        doc,
        drag.track_idx as u16,
        lane_idx,
        &drag.target,
        drag.before,
        after,
        t!("undo.edit_automation").as_ref(),
        drag.snapshot,
    );
}

/// 在拖动会话的 tick 处写入/覆盖自动化事件（lane 懒创建）。
fn upsert_automation_event(doc: &mut Document, drag: &mut KnobDrag, norm: f32) {
    let lane_idx = doc.data.model.tracks[drag.track_idx]
        .automation_lanes
        .iter()
        .position(|l| l.target == drag.target);
    // 已有同 tick 事件时只更新其值（保留 id/shape），否则新增。
    let has_event = lane_idx.is_some_and(|li| {
        doc.data.model.tracks[drag.track_idx].automation_lanes[li]
            .events
            .iter()
            .any(|e| e.tick == drag.tick)
    });
    match lane_idx {
        Some(li) if has_event => {
            drag.lane_idx = Some(li);
            doc.move_automation_event(drag.track_idx, li, &drag.target, drag.tick, drag.tick, norm);
        }
        _ => {
            doc.add_automation_event(
                drag.track_idx,
                drag.target.clone(),
                AutomationEvent {
                    id: 0,
                    tick: drag.tick,
                    value: norm,
                    shape: drag.target.default_shape(),
                },
            );
            drag.lane_idx = doc.data.model.tracks[drag.track_idx]
                .automation_lanes
                .iter()
                .position(|l| l.target == drag.target);
        }
    }
}
