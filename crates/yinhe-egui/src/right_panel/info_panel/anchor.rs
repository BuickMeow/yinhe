//! 自动化锚点信息面板。
//!
//! 显示选中锚点的 Tick / Value / Shape / X1 / Y1 / X2 / Y2 编辑器，
//! 并通过 [`LaneUndoGuard`] 统一管理 DragValue 的 focus/before/after undo 模式。

use eframe::egui;

use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{AutomationDelta, UndoAction, UndoEntry};
use yinhe_types::{AutomationEvent, AutomationTarget, SegmentShape};

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
        egui::RichText::new("自动化锚点")
            .strong()
            .size(14.0)
            .color(egui::Color32::from_gray(220)),
    );
    ui.add_space(2.0);

    // ── 目标名称（只读） ──
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("目标:").size(11.0).color(egui::Color32::GRAY));
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
        ui.label(egui::RichText::new("刻度:").size(11.0).color(egui::Color32::GRAY));
        let resp = ui.add(egui::DragValue::new(&mut edit_tick).range(0..=u32::MAX as i64).speed(1.0));
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
            guard.lost(ui, doc, "编辑自动化锚点刻度");
        }
    });
    ui.add_space(4.0);

    // ── Value ──
    let guard = LaneUndoGuard::new(ui, "val", track_idx, lane_idx, target);
    let mut edit_value = value as f64;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("值:").size(11.0).color(egui::Color32::GRAY));
        let resp = ui.add(egui::DragValue::new(&mut edit_value).range(0.0..=max_val as f64).speed(1.0));
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
            guard.lost(ui, doc, "编辑自动化锚点值");
        }
    });
    ui.add_space(6.0);

    // ── Shape（离散/曲线切换） ──
    ui.separator();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("曲线类型:").size(11.0).color(egui::Color32::GRAY));
        let is_step = matches!(shape, SegmentShape::Step);
        let mut discrete = is_step;
        let resp = ui.checkbox(&mut discrete, "离散 (Step)");
        if resp.changed() {
            let actions = doc.apply_automation_edits(vec![yinhe_types::AutomationEdit::CycleShape {
                track_idx,
                lane_idx,
                target: target.clone(),
                tick,
            }]);
            push_undo(doc, actions, "切换锚点形状");
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
            ui.label(egui::RichText::new("X1:").size(11.0).color(egui::Color32::GRAY));
            let resp = ui.add(
                egui::DragValue::new(&mut edit)
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
                    SegmentShape::Curve { x1: edit, y1, x2, y2 },
                );
            }
            if resp.lost_focus() {
                guard.lost(ui, doc, "编辑自动化锚点 X1");
            }
        });
        ui.add_space(2.0);

        // Y1
        let guard = LaneUndoGuard::new(ui, "y1", track_idx, lane_idx, target);
        let mut edit = y1;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Y1:").size(11.0).color(egui::Color32::GRAY));
            let resp = ui.add(
                egui::DragValue::new(&mut edit)
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
                    SegmentShape::Curve { x1, y1: edit, x2, y2 },
                );
            }
            if resp.lost_focus() {
                guard.lost(ui, doc, "编辑自动化锚点 Y1");
            }
        });
        ui.add_space(2.0);

        // X2
        let guard = LaneUndoGuard::new(ui, "x2", track_idx, lane_idx, target);
        let mut edit = x2;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("X2:").size(11.0).color(egui::Color32::GRAY));
            let resp = ui.add(
                egui::DragValue::new(&mut edit)
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
                    SegmentShape::Curve { x1, y1, x2: edit, y2 },
                );
            }
            if resp.lost_focus() {
                guard.lost(ui, doc, "编辑自动化锚点 X2");
            }
        });
        ui.add_space(2.0);

        // Y2
        let guard = LaneUndoGuard::new(ui, "y2", track_idx, lane_idx, target);
        let mut edit = y2;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Y2:").size(11.0).color(egui::Color32::GRAY));
            let resp = ui.add(
                egui::DragValue::new(&mut edit)
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
                    SegmentShape::Curve { x1, y1, x2, y2: edit },
                );
            }
            if resp.lost_focus() {
                guard.lost(ui, doc, "编辑自动化锚点 Y2");
            }
        });
        ui.add_space(6.0);
    }

    let shape_desc = match shape {
        SegmentShape::Step => "离散 (Step) — 值在下一个锚点前保持恒定",
        SegmentShape::Curve { .. } => {
            if shape.is_linear() {
                "曲线 (Linear) — 线性插值"
            } else {
                "曲线 (Bézier) — 拖动锚点间的空心圆调整曲率"
            }
        }
    };
    ui.label(
        egui::RichText::new(shape_desc)
            .size(10.0)
            .color(egui::Color32::from_gray(140)),
    );

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(6.0);

    if ui.add(egui::Button::new(egui::RichText::new("清除选择").size(12.0))).clicked() {
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
/// - [`LaneUndoGuard::lost`] 在 `resp.lost_focus()` 时调用，比较 after 与 before，
///   差异时 push undo entry
///
/// 消除了 Tick / Value / X1 / Y1 / X2 / Y2 六处重复的 undo 样板代码。
struct LaneUndoGuard {
    focus_id: egui::Id,
    before_id: egui::Id,
    track_idx: u16,
    lane_idx: usize,
    target: AutomationTarget,
}

impl LaneUndoGuard {
    fn new(ui: &egui::Ui, key: &str, track_idx: u16, lane_idx: usize, target: &AutomationTarget) -> Self {
        Self {
            focus_id: ui.id().with(key).with("focus"),
            before_id: ui.id().with(key).with("before"),
            track_idx,
            lane_idx,
            target: target.clone(),
        }
    }

    /// 在 DragValue gained_focus 时调用：记录当前 lane events 快照。
    fn gained(&self, ui: &egui::Ui, doc: &Document) {
        let before = snapshot_lane_events(doc, self.track_idx, self.lane_idx, &self.target);
        ui.ctx().data_mut(|d| {
            d.insert_temp(self.before_id, before);
            d.insert_temp(self.focus_id, true);
        });
    }

    /// 在 DragValue lost_focus 时调用：比较 after 与 before，差异时 push undo。
    fn lost(&self, ui: &egui::Ui, doc: &mut Document, label: &'static str) {
        let before = ui.ctx().data(|d| d.get_temp::<Vec<AutomationEvent>>(self.before_id));
        if let Some(before) = before {
            let after = snapshot_lane_events(doc, self.track_idx, self.lane_idx, &self.target);
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::Automation(AutomationDelta {
                        track_idx: self.track_idx as usize,
                        lane_idx: self.lane_idx,
                        target: self.target.clone(),
                        before,
                        after,
                    }),
                    label,
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
        }
        ui.ctx().data_mut(|d| {
            d.remove::<Vec<AutomationEvent>>(self.before_id);
            d.remove::<bool>(self.focus_id);
        });
    }
}

/// 把 `apply_automation_edits` 返回的 actions 包成 UndoEntry push 到 history。
fn push_undo(doc: &mut Document, actions: Vec<UndoAction>, label: &'static str) {
    for action in actions {
        doc.history.push(UndoEntry {
            action,
            label,
            selected: doc.edit.selected.clone(),
            track_selected: doc.edit.track_selected.clone(),
            sel_rect: doc.edit.sel_rect.clone(),
        });
    }
}

/// 按 target 取 lane events 快照（Tempo 走 conductor，其他走 track.automation_lanes）。
fn snapshot_lane_events(
    doc: &Document,
    track_idx: u16,
    lane_idx: usize,
    target: &AutomationTarget,
) -> Vec<AutomationEvent> {
    if matches!(target, AutomationTarget::Tempo) {
        doc.data.model.conductor.tempo.events.clone()
    } else {
        doc.data.model.tracks
            .get(track_idx as usize)
            .and_then(|t| t.automation_lanes.get(lane_idx))
            .map(|l| l.events.clone())
            .unwrap_or_default()
    }
}
