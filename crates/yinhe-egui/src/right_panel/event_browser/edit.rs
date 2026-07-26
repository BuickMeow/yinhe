//! 右键编辑 popup：在表格的 value / shape 单元格上右键时弹出编辑器。
//!
//! popup 用 `egui::Area` 实现，固定在表格区域左上角附近。
//! 编辑过程中实时 `apply_automation_edits`，关闭时比较 before/after 并 push undo。

use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_types::{AutomationTarget, SegmentShape};

use crate::right_panel::automation_undo::{push_automation_undo, snapshot_lane_events};

/// 在表格下方检测是否有待编辑的单元格，弹出 popup 编辑 value / shape 并应用到 doc。
///
/// `val_salt` / `shape_salt` 与 `cell_*_editable` 用的 id_salt 一致。
///
/// **关键**：edit key 用 `egui::Id::new((salt, "edit"))` 全局 id，**不**用 `ui.id()`。
/// 原因：cell 内的 `ui.id()` 与本函数调用处的 `ui.id()` 不同（cell 是 child ui），
/// 用 `ui.id()` 会导致 write/read key 不匹配，popup 永远不触发。
pub(super) fn apply_automation_popups(
    ui: &mut egui::Ui,
    doc: &mut Document,
    val_salt: &str,
    shape_salt: &str,
    track_idx: u16,
    lane_idx: usize,
    target: &AutomationTarget,
) {
    // ── 值 popup ──
    let val_edit_id = egui::Id::new((val_salt, "edit"));
    let val_req: Option<(usize, u32, f32)> = ui.memory(|m| m.data.get_temp(val_edit_id));
    if let Some((row_idx, tick, value)) = val_req {
        let max_val = target.max_value();
        let popup_id = ui.id().with((val_salt, "popup")).with(row_idx);
        let mut edit = value as f64;
        let mut open = true;
        let popup_pos = ui.clip_rect().min + egui::vec2(20.0, 20.0);
        egui::Area::new(popup_id)
            .order(egui::Order::Foreground)
            .fixed_pos(popup_pos)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(180.0);
                    ui.label(egui::RichText::new(t!("event_browser.edit_value").as_ref()).strong().size(11.0));
                    ui.add_space(2.0);
                    let resp = ui.add(
                        egui::DragValue::new(&mut edit)
                            .range(0.0..=max_val as f64)
                            .speed(1.0),
                    );
                    if resp.gained_focus() {
                        let before = snapshot_lane_events(doc, track_idx, lane_idx, target);
                        ui.ctx().memory_mut(|m| m.data.insert_temp(popup_id.with("before"), before));
                    }
                    if resp.changed() && (edit as f32) != value {
                        doc.apply_automation_edits(vec![yinhe_types::AutomationEdit::Move {
                            track_idx,
                            lane_idx,
                            target: target.clone(),
                            old_tick: tick,
                            new_tick: tick,
                            new_value: edit as f32,
                        }]);
                    }
                    if resp.lost_focus() {
                        let before: Option<Vec<yinhe_types::AutomationEvent>> =
                            ui.memory(|m| m.data.get_temp(popup_id.with("before")));
                        if let Some(before) = before {
                            let after = snapshot_lane_events(doc, track_idx, lane_idx, target);
                            push_automation_undo(doc, track_idx, lane_idx, target, before, after, t!("undo.edit_anchor_value").as_ref());
                        }
                        open = false;
                    }
                    ui.add_space(2.0);
                    if ui.button(t!("common.confirm").as_ref()).clicked() {
                        open = false;
                    }
                });
            });
        if !open {
            ui.memory_mut(|m| {
                m.data.remove::<(usize, u32, f32)>(val_edit_id);
                m.data.remove::<Vec<yinhe_types::AutomationEvent>>(popup_id.with("before"));
            });
        }
    }

    // ── 形状 popup ──
    let shape_edit_id = egui::Id::new((shape_salt, "edit"));
    let shape_req: Option<(usize, u32, SegmentShape)> = ui.memory(|m| m.data.get_temp(shape_edit_id));
    if let Some((row_idx, tick, shape)) = shape_req {
        let popup_id = ui.id().with((shape_salt, "popup")).with(row_idx);
        let popup_pos = ui.clip_rect().min + egui::vec2(20.0, 20.0);
        let mut open = true;

        let work_id = popup_id.with("work");
        let work_shape: SegmentShape = ui.memory(|m| m.data.get_temp(work_id).unwrap_or(shape));

        let before_id = popup_id.with("before");
        let before: Option<Vec<yinhe_types::AutomationEvent>> = ui.memory(|m| m.data.get_temp(before_id));
        if before.is_none() {
            let b = snapshot_lane_events(doc, track_idx, lane_idx, target);
            ui.ctx().memory_mut(|m| m.data.insert_temp(before_id, b));
        }

        egui::Area::new(popup_id)
            .order(egui::Order::Foreground)
            .fixed_pos(popup_pos)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(220.0);
                    ui.label(egui::RichText::new(t!("event_browser.edit_shape").as_ref()).strong().size(11.0));
                    ui.add_space(2.0);

                    let mut is_step = matches!(work_shape, SegmentShape::Step);
                    if ui.checkbox(&mut is_step, t!("event_browser.shape_step").as_ref()).changed() {
                        let new_shape = if is_step { SegmentShape::Step } else { SegmentShape::linear_curve() };
                        doc.set_automation_shape(track_idx as usize, lane_idx, target, tick, new_shape);
                        ui.ctx().memory_mut(|m| m.data.insert_temp(work_id, new_shape));
                    }

                    if let SegmentShape::Curve { x1, y1, x2, y2 } = work_shape {
                        ui.add_space(2.0);
                        // ranges 与 anchor.rs 一致：x1 ∈ [0, 0.25], y1/y2 ∈ [-0.5, 0.5], x2 ∈ [-0.25, 0]
                        let ranges: [(f32, f32); 4] = [
                            (0.0, 0.25),
                            (-0.5, 0.5),
                            (-0.25, 0.0),
                            (-0.5, 0.5),
                        ];
                        let labels = ["X1", "Y1", "X2", "Y2"];
                        let mut vals = [x1, y1, x2, y2];
                        for i in 0..4 {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(labels[i]).size(11.0).color(egui::Color32::GRAY));
                                let resp = ui.add(
                                    egui::DragValue::new(&mut vals[i])
                                        .range(ranges[i].0 as f64..=ranges[i].1 as f64)
                                        .speed(0.01)
                                        .fixed_decimals(2),
                                );
                                if resp.changed() {
                                    let ns = match i {
                                        0 => SegmentShape::Curve { x1: vals[0], y1, x2, y2 },
                                        1 => SegmentShape::Curve { x1, y1: vals[1], x2, y2 },
                                        2 => SegmentShape::Curve { x1, y1, x2: vals[2], y2 },
                                        _ => SegmentShape::Curve { x1, y1, x2, y2: vals[3] },
                                    };
                                    doc.set_automation_shape(track_idx as usize, lane_idx, target, tick, ns);
                                    ui.ctx().memory_mut(|m| m.data.insert_temp(work_id, ns));
                                }
                            });
                        }
                    }

                    ui.add_space(2.0);
                    if ui.button(t!("common.confirm").as_ref()).clicked() {
                        open = false;
                    }
                });
            });
        if !open {
            let before: Option<Vec<yinhe_types::AutomationEvent>> = ui.memory(|m| m.data.get_temp(before_id));
            if let Some(before) = before {
                let after = snapshot_lane_events(doc, track_idx, lane_idx, target);
                push_automation_undo(doc, track_idx, lane_idx, target, before, after, t!("undo.toggle_anchor_shape").as_ref());
            }
            ui.memory_mut(|m| {
                m.data.remove::<(usize, u32, SegmentShape)>(shape_edit_id);
                m.data.remove::<SegmentShape>(work_id);
                m.data.remove::<Vec<yinhe_types::AutomationEvent>>(before_id);
            });
        }
    }
}
