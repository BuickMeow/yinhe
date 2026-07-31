//! Automation 事件的 value / shape / tick 编辑 popup。
//!
//! popup 打开期间不修改 Document，pending 写到 egui memory。
//! 关闭时（Closed）一次性 apply + push AutomationDelta undo；取消（Cancelled）仅清理。
//! 注意：automation 用 `UndoAction::Automation(AutomationDelta)`，before/after 是
//! `Vec<AutomationEvent>`，不是 EventListItem。

use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_types::{AutomationTarget, SegmentShape};

use crate::right_panel::automation_undo::{push_automation_undo, snapshot_lane_events};

use super::super::bar_lookup::BarLookup;
use super::super::state::EditRequest;
use super::super::table::{peek_edit_request, peek_pos_edit_request, remove_pos_edit_request};
use super::{PopupAction, PopupConfig, cleanup_edit_request, show_number_popup, show_tick_popup};

/// Automation 编辑上下文：把 lane 寻址所需的 3 个字段打包，
/// 避免 popup 函数超过 7 个参数（clippy `too_many_arguments`）。
struct AutoCtx<'a> {
    track_idx: u16,
    lane_idx: usize,
    target: &'a AutomationTarget,
}

/// 处理 automation 的 value / shape / tick 编辑 popup。
///
/// 与 `cell_editable` 共用同一个 `salt`：同一时间只有一个 EditRequest
/// （用户右键的瞬间只有一个 cell），按 EditRequest 类型 match 分派。
/// 优先响应位置编辑请求（`(salt, "edit_pos")` key），再响应普通编辑请求。
pub fn apply_automation_popups(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    track_idx: u16,
    lane_idx: usize,
    target: &AutomationTarget,
    bar_lookup: &BarLookup,
) {
    let ctx = AutoCtx {
        track_idx,
        lane_idx,
        target,
    };
    if let Some(req) = peek_pos_edit_request(ui, salt) {
        match req {
            EditRequest::AutoTick { tick, value } => {
                show_auto_tick_popup(ui, doc, salt, tick, value, &ctx, Some(bar_lookup));
            }
            _ => remove_pos_edit_request(ui, salt),
        }
        return;
    }
    let Some(req) = peek_edit_request(ui, salt) else {
        return;
    };
    match req {
        EditRequest::AutoValue { tick, value } => {
            show_auto_value_popup(ui, doc, salt, tick, value, &ctx);
        }
        EditRequest::AutoShape { tick, shape } => {
            show_auto_shape_popup(ui, doc, salt, tick, shape, &ctx);
        }
        EditRequest::AutoTick { tick, value } => {
            show_auto_tick_popup(ui, doc, salt, tick, value, &ctx, None);
        }
        // 音符的 EditRequest 不在这里处理
        _ => {}
    }
}

fn show_auto_value_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
    value: f32,
    ctx: &AutoCtx,
) {
    let action = show_number_popup(
        ui,
        PopupConfig {
            salt,
            title: t!("event_browser.edit_value").as_ref(),
            initial: value as f64,
            range_min: 0.0,
            range_max: ctx.target.max_value() as f64,
            speed: 1.0,
            fixed_decimals: None,
        },
    );
    match action {
        PopupAction::Closed(new_val_f) => {
            let new_val = new_val_f as f32;
            let snapshot = doc.capture_snapshot();
            let before = snapshot_lane_events(doc, ctx.track_idx, ctx.lane_idx, ctx.target);
            doc.apply_automation_edits(vec![yinhe_types::AutomationEdit::Move {
                track_idx: ctx.track_idx,
                lane_idx: ctx.lane_idx,
                target: ctx.target.clone(),
                old_tick: tick,
                new_tick: tick,
                new_value: new_val,
            }]);
            let after = snapshot_lane_events(doc, ctx.track_idx, ctx.lane_idx, ctx.target);
            push_automation_undo(
                doc,
                ctx.track_idx,
                ctx.lane_idx,
                ctx.target,
                before,
                after,
                t!("undo.edit_anchor_value").as_ref(),
                snapshot,
            );
            cleanup_edit_request(ui, salt);
        }
        PopupAction::Cancelled => cleanup_edit_request(ui, salt),
        PopupAction::None => {}
    }
}

fn show_auto_tick_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
    value: f32,
    ctx: &AutoCtx,
    bar_lookup: Option<&BarLookup>,
) {
    let action = show_tick_popup(
        ui,
        salt,
        t!("event_browser.edit_tick").as_ref(),
        tick,
        0,
        bar_lookup,
    );
    match action {
        PopupAction::Closed(new_tick_f) => {
            let new_tick = new_tick_f as u32;
            let snapshot = doc.capture_snapshot();
            let before = snapshot_lane_events(doc, ctx.track_idx, ctx.lane_idx, ctx.target);
            doc.apply_automation_edits(vec![yinhe_types::AutomationEdit::Move {
                track_idx: ctx.track_idx,
                lane_idx: ctx.lane_idx,
                target: ctx.target.clone(),
                old_tick: tick,
                new_tick,
                new_value: value,
            }]);
            let after = snapshot_lane_events(doc, ctx.track_idx, ctx.lane_idx, ctx.target);
            push_automation_undo(
                doc,
                ctx.track_idx,
                ctx.lane_idx,
                ctx.target,
                before,
                after,
                t!("undo.edit_anchor_tick").as_ref(),
                snapshot,
            );
            cleanup_edit_request(ui, salt);
        }
        PopupAction::Cancelled => cleanup_edit_request(ui, salt),
        PopupAction::None => {}
    }
}

fn show_auto_shape_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
    shape: SegmentShape,
    ctx: &AutoCtx,
) {
    let popup_id = ui.id().with((salt, "popup"));
    let work_id = popup_id.with("work");
    let work_shape: SegmentShape = ui.memory(|m| m.data.get_temp(work_id).unwrap_or(shape));
    let mut open = true;
    let mut cancelled = false;
    let popup_pos = ui.clip_rect().min + egui::vec2(20.0, 20.0);

    egui::Area::new(popup_id)
        .order(egui::Order::Foreground)
        .fixed_pos(popup_pos)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(220.0);
                ui.label(
                    egui::RichText::new(t!("event_browser.edit_shape").as_ref())
                        .strong()
                        .size(11.0),
                );
                ui.add_space(2.0);

                let mut is_step = matches!(work_shape, SegmentShape::Step);
                let was_step = is_step;
                ui.checkbox(&mut is_step, t!("event_browser.shape_step").as_ref());
                // 用直接比较替代 checkbox.changed()——Area 中 response 标记不可靠
                // 只更新 pending（work_id），不调 doc.set_automation_shape
                if is_step != was_step {
                    let new_shape = if is_step {
                        SegmentShape::Step
                    } else {
                        SegmentShape::linear_curve()
                    };
                    ui.ctx()
                        .memory_mut(|m| m.data.insert_temp(work_id, new_shape));
                }

                if let SegmentShape::Curve { x1, y1, x2, y2 } = work_shape {
                    ui.add_space(2.0);
                    // ranges 与 anchor.rs 一致：x1 ∈ [0, 0.25], y1/y2 ∈ [-0.5, 0.5], x2 ∈ [-0.25, 0]
                    let ranges: [(f32, f32); 4] =
                        [(0.0, 0.25), (-0.5, 0.5), (-0.25, 0.0), (-0.5, 0.5)];
                    let labels = ["X1", "Y1", "X2", "Y2"];
                    let mut vals = [x1, y1, x2, y2];
                    let old_vals = vals;
                    for i in 0..4 {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(labels[i])
                                    .size(11.0)
                                    .color(egui::Color32::GRAY),
                            );
                            ui.add(
                                crate::widgets::numeric_input::decimal_drag_value(&mut vals[i])
                                    .range(ranges[i].0 as f64..=ranges[i].1 as f64)
                                    .speed(0.01)
                                    .fixed_decimals(2),
                            );
                        });
                    }
                    // 用直接比较替代 resp.changed()——后者在 Area 中不可靠
                    // 只更新 pending（work_id），不调 doc.set_automation_shape
                    if vals != old_vals {
                        let ns = SegmentShape::Curve {
                            x1: vals[0],
                            y1: vals[1],
                            x2: vals[2],
                            y2: vals[3],
                        };
                        ui.ctx().memory_mut(|m| m.data.insert_temp(work_id, ns));
                    }
                }

                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    if ui.button(t!("common.confirm").as_ref()).clicked() {
                        open = false;
                    }
                    if ui.button(t!("common.cancel").as_ref()).clicked() {
                        open = false;
                        cancelled = true;
                    }
                });
            });
        });

    if !open {
        if !cancelled {
            // 读 pending shape，一次性 apply + push undo
            let pending_shape =
                ui.memory(|m| m.data.get_temp::<SegmentShape>(work_id).unwrap_or(shape));
            let snapshot = doc.capture_snapshot();
            let before = snapshot_lane_events(doc, ctx.track_idx, ctx.lane_idx, ctx.target);
            doc.set_automation_shape(
                ctx.track_idx as usize,
                ctx.lane_idx,
                ctx.target,
                tick,
                pending_shape,
            );
            let after = snapshot_lane_events(doc, ctx.track_idx, ctx.lane_idx, ctx.target);
            push_automation_undo(
                doc,
                ctx.track_idx,
                ctx.lane_idx,
                ctx.target,
                before,
                after,
                t!("undo.toggle_anchor_shape").as_ref(),
                snapshot,
            );
        }
        ui.memory_mut(|m| m.data.remove::<SegmentShape>(work_id));
        cleanup_edit_request(ui, salt);
    }
}
