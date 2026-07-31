//! 自动化锚点信息面板。
//!
//! 显示选中锚点的 Tick / Value / Shape / X1 / Y1 / X2 / Y2 编辑器，
//! 并通过 [`LaneUndoGuard`] 统一管理 DragValue 的 focus/before/after undo 模式。

use eframe::egui;

use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::EditSnapshot;
use yinhe_types::{AutomationEvent, AutomationTarget, SegmentShape};

use rust_i18n::t;

use crate::right_panel::automation_undo::{
    push_automation_actions, push_automation_undo, snapshot_lane_events,
};

use super::InfoContent;

/// 显示自动化锚点信息编辑器。
pub(super) fn show_anchor_info(
    ui: &mut egui::Ui,
    doc: &mut Document,
    track_idx: u16,
    lane_idx: usize,
    _event_idx: usize,
    tick: u32,
    value: f32,
    shape: SegmentShape,
    target: &AutomationTarget,
    info_content: &mut Option<InfoContent>,
) {
    let max_val = target.max_value();

    ui.add_space(4.0);

    // ── 标题 ──
    ui.label(
        egui::RichText::new(t!("anchor.title").as_ref())
            .strong()
            .size(14.0)
            .color(egui::Color32::from_gray(220)),
    );
    ui.add_space(2.0);

    // ── 目标名称（只读） ──
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(t!("anchor.target").as_ref())
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        ui.label(
            egui::RichText::new(target.display_name())
                .size(12.0)
                .color(egui::Color32::from_gray(200)),
        );
    });
    ui.add_space(4.0);

    // ── Tick ──
    let guard = LaneUndoGuard::new(ui, "tick", track_idx, lane_idx, target);
    let mut edit_tick = tick as f64;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(t!("anchor.tick").as_ref())
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        let resp = ui.add(
            crate::widgets::numeric_input::decimal_drag_value(&mut edit_tick)
                .range(0..=u32::MAX as i64)
                .speed(1.0),
        );
        if resp.gained_focus() {
            guard.gained(ui, doc);
        }
        if resp.changed() {
            let new_tick = edit_tick as u32;
            if new_tick != tick {
                doc.apply_automation_edits(vec![yinhe_types::AutomationEdit::Move {
                    track_idx,
                    lane_idx,
                    target: target.clone(),
                    old_tick: tick,
                    new_tick,
                    new_value: value,
                }]);
            }
        }
        if resp.lost_focus() {
            guard.lost(ui, doc, t!("undo.edit_anchor_tick").as_ref());
        }
    });
    ui.add_space(4.0);

    // ── Value ──
    let guard = LaneUndoGuard::new(ui, "val", track_idx, lane_idx, target);
    let mut edit_value = value as f64;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(t!("anchor.value").as_ref())
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        let resp = ui.add(
            crate::widgets::numeric_input::decimal_drag_value(&mut edit_value)
                .range(0.0..=max_val as f64)
                .speed(1.0),
        );
        if resp.gained_focus() {
            guard.gained(ui, doc);
        }
        if resp.changed() {
            let new_value = edit_value as f32;
            if new_value != value {
                doc.apply_automation_edits(vec![yinhe_types::AutomationEdit::Move {
                    track_idx,
                    lane_idx,
                    target: target.clone(),
                    old_tick: tick,
                    new_tick: tick,
                    new_value,
                }]);
            }
        }
        if resp.lost_focus() {
            guard.lost(ui, doc, t!("undo.edit_anchor_value").as_ref());
        }
    });
    ui.add_space(6.0);

    // ── Shape（离散/曲线切换） ──
    ui.separator();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(t!("anchor.shape").as_ref())
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        let is_step = matches!(shape, SegmentShape::Step);
        let mut discrete = is_step;
        let resp = ui.checkbox(&mut discrete, t!("anchor.discrete").as_ref());
        if resp.changed() {
            let before = doc.capture_snapshot();
            let actions =
                doc.apply_automation_edits(vec![yinhe_types::AutomationEdit::CycleShape {
                    track_idx,
                    lane_idx,
                    target: target.clone(),
                    tick,
                }]);
            push_automation_actions(
                doc,
                actions,
                t!("undo.toggle_anchor_shape").as_ref(),
                before,
            );
        }
    });

    // ── X1 / Y1 / X2 / Y2（仅 Curve 模式下显示） ──
    // 偏移量参数化（CSS handle 风格）：
    // - (x1, y1): P1 相对 P0 的偏移，内部 *4 得到实际参数
    // - (x2, y2): P2 相对 P3 的偏移，内部 *4 得到实际参数
    // 每个分量 ∈ [-0.5, 0.5]，0 = 直线（中性）。直线 = (0, 0, 0, 0)。
    if let SegmentShape::Curve { x1, y1, x2, y2 } = shape {
        // X1
        let guard = LaneUndoGuard::new(ui, "x1", track_idx, lane_idx, target);
        let mut edit = x1;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t!("anchor.x1").as_ref())
                    .size(11.0)
                    .color(egui::Color32::GRAY),
            );
            let resp = ui.add(
                crate::widgets::numeric_input::decimal_drag_value(&mut edit)
                    .range(0.0..=0.25)
                    .speed(0.01)
                    .fixed_decimals(2),
            );
            if resp.gained_focus() {
                guard.gained(ui, doc);
            }
            if resp.changed() && edit != x1 {
                doc.set_automation_shape(
                    track_idx as usize,
                    lane_idx,
                    target,
                    tick,
                    SegmentShape::Curve {
                        x1: edit,
                        y1,
                        x2,
                        y2,
                    },
                );
            }
            if resp.lost_focus() {
                guard.lost(ui, doc, t!("undo.edit_anchor_x1").as_ref());
            }
        });
        ui.add_space(2.0);

        // Y1
        let guard = LaneUndoGuard::new(ui, "y1", track_idx, lane_idx, target);
        let mut edit = y1;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t!("anchor.y1").as_ref())
                    .size(11.0)
                    .color(egui::Color32::GRAY),
            );
            let resp = ui.add(
                crate::widgets::numeric_input::decimal_drag_value(&mut edit)
                    .range(-0.5..=0.5)
                    .speed(0.01)
                    .fixed_decimals(2),
            );
            if resp.gained_focus() {
                guard.gained(ui, doc);
            }
            if resp.changed() && edit != y1 {
                doc.set_automation_shape(
                    track_idx as usize,
                    lane_idx,
                    target,
                    tick,
                    SegmentShape::Curve {
                        x1,
                        y1: edit,
                        x2,
                        y2,
                    },
                );
            }
            if resp.lost_focus() {
                guard.lost(ui, doc, t!("undo.edit_anchor_y1").as_ref());
            }
        });
        ui.add_space(2.0);

        // X2
        let guard = LaneUndoGuard::new(ui, "x2", track_idx, lane_idx, target);
        let mut edit = x2;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t!("anchor.x2").as_ref())
                    .size(11.0)
                    .color(egui::Color32::GRAY),
            );
            let resp = ui.add(
                crate::widgets::numeric_input::decimal_drag_value(&mut edit)
                    .range(-0.25..=0.0)
                    .speed(0.01)
                    .fixed_decimals(2),
            );
            if resp.gained_focus() {
                guard.gained(ui, doc);
            }
            if resp.changed() && edit != x2 {
                doc.set_automation_shape(
                    track_idx as usize,
                    lane_idx,
                    target,
                    tick,
                    SegmentShape::Curve {
                        x1,
                        y1,
                        x2: edit,
                        y2,
                    },
                );
            }
            if resp.lost_focus() {
                guard.lost(ui, doc, t!("undo.edit_anchor_x2").as_ref());
            }
        });
        ui.add_space(2.0);

        // Y2
        let guard = LaneUndoGuard::new(ui, "y2", track_idx, lane_idx, target);
        let mut edit = y2;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t!("anchor.y2").as_ref())
                    .size(11.0)
                    .color(egui::Color32::GRAY),
            );
            let resp = ui.add(
                crate::widgets::numeric_input::decimal_drag_value(&mut edit)
                    .range(-0.5..=0.5)
                    .speed(0.01)
                    .fixed_decimals(2),
            );
            if resp.gained_focus() {
                guard.gained(ui, doc);
            }
            if resp.changed() && edit != y2 {
                doc.set_automation_shape(
                    track_idx as usize,
                    lane_idx,
                    target,
                    tick,
                    SegmentShape::Curve {
                        x1,
                        y1,
                        x2,
                        y2: edit,
                    },
                );
            }
            if resp.lost_focus() {
                guard.lost(ui, doc, t!("undo.edit_anchor_y2").as_ref());
            }
        });
        ui.add_space(6.0);
    }

    let shape_desc = match shape {
        SegmentShape::Step => t!("anchor.shape_step_desc"),
        SegmentShape::Curve { .. } => {
            if shape.is_linear() {
                t!("anchor.shape_linear_desc")
            } else {
                t!("anchor.shape_bezier_desc")
            }
        }
    };
    ui.label(
        egui::RichText::new(shape_desc.as_ref())
            .size(10.0)
            .color(egui::Color32::from_gray(140)),
    );

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(6.0);

    if ui
        .add(egui::Button::new(
            egui::RichText::new(t!("common.clear_selection").as_ref()).size(12.0),
        ))
        .clicked()
    {
        *info_content = None;
    }
}

// ────────────────────────────────────────────────────────────────
// 工具函数
// ────────────────────────────────────────────────────────────────

/// DragValue 的 lane undo guard。
///
/// 统一管理自动化锚点编辑器的 focus/before/after undo 模式：
/// - [`LaneUndoGuard::gained`] 在 `resp.gained_focus()` 时调用，记录 lane events 快照
///   + 界面状态快照（`EditSnapshot`）
/// - [`LaneUndoGuard::lost`] 在 `resp.lost_focus()` 时调用，比较 after 与 before，
///   差异时 push undo entry
///
/// 消除了 Tick / Value / X1 / Y1 / X2 / Y2 六处重复的 undo 样板代码。
struct LaneUndoGuard {
    focus_id: egui::Id,
    before_id: egui::Id,
    snapshot_id: egui::Id,
    track_idx: u16,
    lane_idx: usize,
    target: AutomationTarget,
}

impl LaneUndoGuard {
    fn new(
        ui: &egui::Ui,
        key: &str,
        track_idx: u16,
        lane_idx: usize,
        target: &AutomationTarget,
    ) -> Self {
        Self {
            focus_id: ui.id().with(key).with("focus"),
            before_id: ui.id().with(key).with("before"),
            snapshot_id: ui.id().with(key).with("snap"),
            track_idx,
            lane_idx,
            target: target.clone(),
        }
    }

    /// 在 DragValue gained_focus 时调用：记录当前 lane events 快照 + 界面状态快照。
    fn gained(&self, ui: &egui::Ui, doc: &Document) {
        let before = snapshot_lane_events(doc, self.track_idx, self.lane_idx, &self.target);
        let snapshot = doc.capture_snapshot();
        ui.ctx().data_mut(|d| {
            d.insert_temp(self.before_id, before);
            d.insert_temp(self.snapshot_id, snapshot);
            d.insert_temp(self.focus_id, true);
        });
    }

    /// 在 DragValue lost_focus 时调用：比较 after 与 before，差异时 push undo。
    fn lost(&self, ui: &egui::Ui, doc: &mut Document, label: &str) {
        let before = ui
            .ctx()
            .data(|d| d.get_temp::<Vec<AutomationEvent>>(self.before_id));
        if let Some(before) = before {
            let snapshot = ui
                .ctx()
                .data(|d| d.get_temp::<EditSnapshot>(self.snapshot_id));
            if let Some(snapshot) = snapshot {
                let after = snapshot_lane_events(doc, self.track_idx, self.lane_idx, &self.target);
                push_automation_undo(
                    doc,
                    self.track_idx,
                    self.lane_idx,
                    &self.target,
                    before,
                    after,
                    label,
                    snapshot,
                );
            }
        }
        ui.ctx().data_mut(|d| {
            d.remove::<Vec<AutomationEvent>>(self.before_id);
            d.remove::<EditSnapshot>(self.snapshot_id);
            d.remove::<bool>(self.focus_id);
        });
    }
}
