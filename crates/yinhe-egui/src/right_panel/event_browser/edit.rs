//! 右键编辑 popup：在表格的单元格上右键时弹出编辑器。
//!
//! 设计：
//! - cell 右键时把 `EditRequest` 写到 `egui::Id::new((salt, "edit"))`（全局 key，
//!   不用 `ui.id()`，因为 cell 是 child ui，`ui.id()` 与本函数调用处不同）
//! - `apply_*_popups` 每帧 `peek_edit_request` 检查是否有请求，有就显示 popup
//! - popup 内 DragValue 状态持久化到 `egui::Id::new((salt, "state"))`，
//!   避免每帧重建 DragValue 导致拖动时数字不同步
//! - popup 显示期间记录 before 快照，关闭时取 after 对比并 push undo
//! - 音符编辑后更新 `NoteRef` 写回 `EditRequest`，避免下次寻址失效

use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_types::{AutomationTarget, PencilNoteDrag, SegmentShape};

use crate::right_panel::automation_undo::{push_automation_undo, snapshot_lane_events};

use super::state::{EditRequest, NoteRef};
use super::table::{peek_edit_request, remove_edit_request, update_edit_request};

/// popup 内 DragValue 的状态变化或关闭事件。
pub(super) enum PopupAction {
    /// 本帧无变化
    None,
    /// DragValue 值变了（参数是新值）
    Changed(f64),
    /// popup 关闭（lost_focus 或 confirm 按钮）
    Closed,
}

struct PopupConfig<'a> {
    salt: &'a str,
    title: &'a str,
    initial: f64,
    range_min: f64,
    range_max: f64,
    speed: f64,
    fixed_decimals: Option<usize>,
}

/// 渲染数字编辑 popup（Area + DragValue + confirm）。
///
/// DragValue 状态持久化到 `egui::Id::new((salt, "state"))`，每帧从 memory 读出。
/// 这样拖动时 DragValue 内部数字会实时更新，不会因每帧重建而重置。
fn show_number_popup(ui: &mut egui::Ui, cfg: PopupConfig) -> PopupAction {
    let state_id = egui::Id::new((cfg.salt, "state"));
    let popup_id = ui.id().with((cfg.salt, "popup"));

    let mut state = ui.memory(|m| m.data.get_temp::<f64>(state_id).unwrap_or(cfg.initial));
    let mut action = PopupAction::None;
    let mut open = true;
    let popup_pos = ui.clip_rect().min + egui::vec2(20.0, 20.0);

    egui::Area::new(popup_id)
        .order(egui::Order::Foreground)
        .fixed_pos(popup_pos)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(180.0);
                ui.label(egui::RichText::new(cfg.title).strong().size(11.0));
                ui.add_space(2.0);
                let mut dv = crate::widgets::numeric_input::decimal_drag_value(&mut state)
                    .range(cfg.range_min..=cfg.range_max)
                    .speed(cfg.speed);
                if let Some(d) = cfg.fixed_decimals {
                    dv = dv.fixed_decimals(d);
                }
                let resp = ui.add(dv);
                if resp.changed() {
                    action = PopupAction::Changed(state);
                    ui.memory_mut(|m| m.data.insert_temp(state_id, state));
                }
                if resp.lost_focus() {
                    open = false;
                }
                ui.add_space(2.0);
                let confirm = ui.button(t!("common.confirm").as_ref());
                if confirm.clicked() {
                    open = false;
                }
                ui.add_space(2.0);
                if ui.button(t!("common.cancel").as_ref()).clicked() {
                    open = false;
                }
            });
        });

    if !open {
        ui.memory_mut(|m| m.data.remove::<f64>(state_id));
        PopupAction::Closed
    } else {
        action
    }
}

// ────────────────────────────────────────────────────────────────
// Automation popups
// ────────────────────────────────────────────────────────────────

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
pub(super) fn apply_automation_popups(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    track_idx: u16,
    lane_idx: usize,
    target: &AutomationTarget,
) {
    let ctx = AutoCtx { track_idx, lane_idx, target };
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::AutoValue { tick, value } => {
            show_auto_value_popup(ui, doc, salt, tick, value, &ctx);
        }
        EditRequest::AutoShape { tick, shape } => {
            show_auto_shape_popup(ui, doc, salt, tick, shape, &ctx);
        }
        EditRequest::AutoTick { tick, value } => {
            show_auto_tick_popup(ui, doc, salt, tick, value, &ctx);
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
    let before = record_lane_before(ui, doc, salt, ctx);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_value").as_ref(),
        initial: value as f64,
        range_min: 0.0,
        range_max: ctx.target.max_value() as f64,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_val) => {
            if (new_val as f32) != value {
                doc.apply_automation_edits(vec![yinhe_types::AutomationEdit::Move {
                    track_idx: ctx.track_idx,
                    lane_idx: ctx.lane_idx,
                    target: ctx.target.clone(),
                    old_tick: tick,
                    new_tick: tick,
                    new_value: new_val as f32,
                }]);
            }
        }
        PopupAction::Closed => {
            finalize_lane_undo(ui, doc, salt, before, ctx,
                t!("undo.edit_anchor_value").as_ref());
        }
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
) {
    let before = record_lane_before(ui, doc, salt, ctx);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_tick").as_ref(),
        initial: tick as f64,
        range_min: 0.0,
        range_max: u32::MAX as f64,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_tick) => {
            let new_tick = new_tick as u32;
            if new_tick != tick {
                doc.apply_automation_edits(vec![yinhe_types::AutomationEdit::Move {
                    track_idx: ctx.track_idx,
                    lane_idx: ctx.lane_idx,
                    target: ctx.target.clone(),
                    old_tick: tick,
                    new_tick,
                    new_value: value,
                }]);
                // 更新 EditRequest 的 tick，避免下次寻址失效
                update_edit_request(ui, salt, EditRequest::AutoTick { tick: new_tick, value });
            }
        }
        PopupAction::Closed => {
            finalize_lane_undo(ui, doc, salt, before, ctx,
                t!("undo.edit_anchor_tick").as_ref());
        }
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
    let before = record_lane_before(ui, doc, salt, ctx);

    let popup_id = ui.id().with((salt, "popup"));
    let work_id = popup_id.with("work");
    let work_shape: SegmentShape = ui.memory(|m| m.data.get_temp(work_id).unwrap_or(shape));
    let mut open = true;
    let popup_pos = ui.clip_rect().min + egui::vec2(20.0, 20.0);

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
                    doc.set_automation_shape(ctx.track_idx as usize, ctx.lane_idx, ctx.target, tick, new_shape);
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
                                crate::widgets::numeric_input::decimal_drag_value(&mut vals[i])
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
                                doc.set_automation_shape(ctx.track_idx as usize, ctx.lane_idx, ctx.target, tick, ns);
                                ui.ctx().memory_mut(|m| m.data.insert_temp(work_id, ns));
                            }
                        });
                    }
                }

                ui.add_space(2.0);
                let confirm = ui.button(t!("common.confirm").as_ref());
                if confirm.clicked() {
                    open = false;
                }
                ui.add_space(2.0);
                if ui.button(t!("common.cancel").as_ref()).clicked() {
                    open = false;
                }
            });
        });

    if !open {
        finalize_lane_undo(ui, doc, salt, before, ctx,
            t!("undo.toggle_anchor_shape").as_ref());
        ui.memory_mut(|m| m.data.remove::<SegmentShape>(work_id));
    }
}

/// popup 显示期间记录 lane events before 快照（仅第一次记录）。
fn record_lane_before(
    ui: &egui::Ui,
    doc: &Document,
    salt: &str,
    ctx: &AutoCtx,
) -> Option<Vec<yinhe_types::AutomationEvent>> {
    let before_id = egui::Id::new((salt, "before"));
    let recorded_id = before_id.with("recorded");
    let recorded = ui.memory(|m| m.data.get_temp::<bool>(recorded_id).unwrap_or(false));
    if !recorded {
        let before = snapshot_lane_events(doc, ctx.track_idx, ctx.lane_idx, ctx.target);
        ui.memory_mut(|m| {
            m.data.insert_temp(before_id, before.clone());
            m.data.insert_temp(recorded_id, true);
        });
        Some(before)
    } else {
        ui.memory(|m| m.data.get_temp::<Vec<yinhe_types::AutomationEvent>>(before_id))
    }
}

/// popup 关闭时取 after 对比，push undo，清除所有 popup 状态。
fn finalize_lane_undo(
    ui: &egui::Ui,
    doc: &mut Document,
    salt: &str,
    before: Option<Vec<yinhe_types::AutomationEvent>>,
    ctx: &AutoCtx,
    label: &str,
) {
    if let Some(before) = before {
        let after = snapshot_lane_events(doc, ctx.track_idx, ctx.lane_idx, ctx.target);
        push_automation_undo(doc, ctx.track_idx, ctx.lane_idx, ctx.target, before, after, label);
    }
    let before_id = egui::Id::new((salt, "before"));
    ui.memory_mut(|m| {
        m.data.remove::<Vec<yinhe_types::AutomationEvent>>(before_id);
        m.data.remove::<bool>(before_id.with("recorded"));
    });
    remove_edit_request(ui, salt);
}

// ────────────────────────────────────────────────────────────────
// Note popups
// ────────────────────────────────────────────────────────────────

/// 处理音符的 start_tick / end_tick / gate / key / velocity 编辑 popup。
pub(super) fn apply_note_popups(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
) {
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::NoteStartTick { note } => show_note_start_tick_popup(ui, doc, salt, note),
        EditRequest::NoteEndTick { note } => show_note_end_tick_popup(ui, doc, salt, note),
        EditRequest::NoteGate { note } => show_note_gate_popup(ui, doc, salt, note),
        EditRequest::NoteKey { note } => show_note_key_popup(ui, doc, salt, note),
        EditRequest::NoteVelocity { note } => show_note_velocity_popup(ui, doc, salt, note),
        _ => {}
    }
}

fn show_note_start_tick_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    note: NoteRef,
) {
    let before = record_note_before(ui, doc, salt, &note);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_tick").as_ref(),
        initial: note.start_tick as f64,
        range_min: 0.0,
        range_max: u32::MAX as f64,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_tick) => {
            let new_tick = new_tick as u32;
            if new_tick != note.start_tick {
                let delta_ticks = new_tick as i64 - note.start_tick as i64;
                doc.pencil_drag_note(&PencilNoteDrag::Move {
                    track: note.track,
                    start_tick: note.start_tick,
                    key: note.key,
                    delta_ticks,
                    delta_keys: 0,
                });
                // 保持 gate 不变：end_tick 跟着平移
                let new_end = note.end_tick as i64 + delta_ticks;
                let new_end = new_end.max(0) as u32;
                // 更新 NoteRef，下次寻址用新值
                update_edit_request(ui, salt, EditRequest::NoteStartTick {
                    note: NoteRef { start_tick: new_tick, end_tick: new_end, ..note },
                });
            }
        }
        PopupAction::Closed => {
            finalize_note_undo(ui, doc, salt, before, &note, t!("undo.edit_anchor_tick").as_ref());
        }
        PopupAction::None => {}
    }
}

fn show_note_end_tick_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    note: NoteRef,
) {
    let before = record_note_before(ui, doc, salt, &note);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_end_tick").as_ref(),
        initial: note.end_tick as f64,
        range_min: (note.start_tick + 1) as f64,
        range_max: u32::MAX as f64,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_end) => {
            let new_end = new_end as u32;
            if new_end != note.end_tick {
                doc.pencil_drag_note(&PencilNoteDrag::ResizeRight {
                    track: note.track,
                    start_tick: note.start_tick,
                    key: note.key,
                    new_end_tick: new_end,
                });
                update_edit_request(ui, salt, EditRequest::NoteEndTick {
                    note: NoteRef { end_tick: new_end, ..note },
                });
            }
        }
        PopupAction::Closed => {
            finalize_note_undo(ui, doc, salt, before, &note, t!("undo.edit_anchor_tick").as_ref());
        }
        PopupAction::None => {}
    }
}

fn show_note_gate_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    note: NoteRef,
) {
    let gate = note.end_tick.saturating_sub(note.start_tick);
    let before = record_note_before(ui, doc, salt, &note);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_gate").as_ref(),
        initial: gate as f64,
        range_min: 1.0,
        range_max: u32::MAX as f64,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_gate) => {
            let new_gate = new_gate as u32;
            let new_end = note.start_tick + new_gate;
            if new_end != note.end_tick {
                doc.pencil_drag_note(&PencilNoteDrag::ResizeRight {
                    track: note.track,
                    start_tick: note.start_tick,
                    key: note.key,
                    new_end_tick: new_end,
                });
                update_edit_request(ui, salt, EditRequest::NoteGate {
                    note: NoteRef { end_tick: new_end, ..note },
                });
            }
        }
        PopupAction::Closed => {
            finalize_note_undo(ui, doc, salt, before, &note, t!("undo.edit_anchor_tick").as_ref());
        }
        PopupAction::None => {}
    }
}

fn show_note_key_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    note: NoteRef,
) {
    let before = record_note_before(ui, doc, salt, &note);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_key").as_ref(),
        initial: note.key as f64,
        range_min: 0.0,
        range_max: 127.0,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_key) => {
            let new_key = new_key as i32;
            if new_key != note.key as i32 {
                let delta_keys = new_key - note.key as i32;
                doc.pencil_drag_note(&PencilNoteDrag::Move {
                    track: note.track,
                    start_tick: note.start_tick,
                    key: note.key,
                    delta_ticks: 0,
                    delta_keys,
                });
                update_edit_request(ui, salt, EditRequest::NoteKey {
                    note: NoteRef { key: new_key as u8, ..note },
                });
            }
        }
        PopupAction::Closed => {
            finalize_note_undo(ui, doc, salt, before, &note, t!("undo.edit_anchor_tick").as_ref());
        }
        PopupAction::None => {}
    }
}

fn show_note_velocity_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    note: NoteRef,
) {
    let before = record_note_before(ui, doc, salt, &note);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_velocity").as_ref(),
        initial: note.velocity as f64,
        range_min: 0.0,
        range_max: 127.0,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_vel) => {
            let new_vel = new_vel as u8;
            if new_vel != note.velocity {
                doc.set_note_velocity(note.track, note.start_tick, note.key, new_vel);
                update_edit_request(ui, salt, EditRequest::NoteVelocity {
                    note: NoteRef { velocity: new_vel, ..note },
                });
            }
        }
        PopupAction::Closed => {
            finalize_note_undo(ui, doc, salt, before, &note, t!("undo.edit_anchor_tick").as_ref());
        }
        PopupAction::None => {}
    }
}

/// popup 显示期间记录音符 before 快照（仅第一次记录）。
fn record_note_before(
    ui: &egui::Ui,
    doc: &Document,
    salt: &str,
    note: &NoteRef,
) -> Option<(yinhe_types::Note, u8)> {
    let before_id = egui::Id::new((salt, "before"));
    let recorded_id = before_id.with("recorded");
    let recorded = ui.memory(|m| m.data.get_temp::<bool>(recorded_id).unwrap_or(false));
    if !recorded {
        let model = &doc.data.model;
        let k = note.key as usize;
        let before = model.notes.get(k)
            .and_then(|bucket| bucket.iter().find(|n| n.id == note.id))
            .copied();
        if let Some(before) = before {
            ui.memory_mut(|m| {
                m.data.insert_temp(before_id, (before, note.key));
                m.data.insert_temp(recorded_id, true);
            });
            Some((before, note.key))
        } else {
            None
        }
    } else {
        ui.memory(|m| m.data.get_temp::<(yinhe_types::Note, u8)>(before_id))
    }
}

/// popup 关闭时取 after 对比，push undo，清除所有 popup 状态。
fn finalize_note_undo(
    ui: &egui::Ui,
    doc: &mut Document,
    salt: &str,
    before: Option<(yinhe_types::Note, u8)>,
    note: &NoteRef,
    label: &str,
) {
    use yinhe_editor_core::history::{NoteDelta, UndoAction, UndoEntry};

    if let Some((before_note, before_key)) = before {
        let model = &doc.data.model;
        let k = note.key as usize;
        if let Some(after_note) = model.notes.get(k)
            .and_then(|bucket| bucket.iter().find(|n| n.id == note.id))
            .copied()
        {
            // Note 未实现 PartialEq，按字段比较（id 不变，只比可变字段）
            let changed = before_note.start_tick != after_note.start_tick
                || before_note.end_tick != after_note.end_tick
                || before_note.velocity != after_note.velocity
                || before_note.track != after_note.track
                || before_key != note.key;
            if changed {
                doc.history.push(UndoEntry {
                    action: UndoAction::Notes(NoteDelta {
                        before: vec![(before_note, before_key)],
                        after: vec![(after_note, note.key)],
                    }),
                    label: label.to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
        }
    }
    let before_id = egui::Id::new((salt, "before"));
    ui.memory_mut(|m| {
        m.data.remove::<(yinhe_types::Note, u8)>(before_id);
        m.data.remove::<bool>(before_id.with("recorded"));
    });
    remove_edit_request(ui, salt);
}

// ────────────────────────────────────────────────────────────────
// TimeSig popups
// ────────────────────────────────────────────────────────────────

/// 处理拍号的 tick / numerator / denominator 编辑 popup。
pub(super) fn apply_timesig_popups(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
) {
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::TimeSigTick { tick } => show_timesig_tick_popup(ui, doc, salt, tick),
        EditRequest::TimeSigNumerator { tick } => show_timesig_num_popup(ui, doc, salt, tick),
        EditRequest::TimeSigDenominator { tick } => show_timesig_den_popup(ui, doc, salt, tick),
        _ => {}
    }
}

fn show_timesig_tick_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
) {
    let before = record_timesig_before(ui, doc, salt);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_tick").as_ref(),
        initial: tick as f64,
        range_min: 0.0,
        range_max: u32::MAX as f64,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_tick) => {
            let new_tick = new_tick as u32;
            if new_tick != tick {
                let model = &doc.data.model;
                if let Some(e) = model.conductor.time_sig.iter().find(|e| e.tick == tick) {
                    let _ = doc.set_time_sig_event(tick, new_tick, e.numerator, e.denominator);
                }
                update_edit_request(ui, salt, EditRequest::TimeSigTick { tick: new_tick });
            }
        }
        PopupAction::Closed => {
            finalize_timesig_undo(ui, doc, salt, before, t!("undo.edit_timesig").as_ref());
        }
        PopupAction::None => {}
    }
}

fn show_timesig_num_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
) {
    let before = record_timesig_before(ui, doc, salt);
    let numerator = doc.data.model.conductor.time_sig.iter()
        .find(|e| e.tick == tick).map(|e| e.numerator).unwrap_or(4);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_numerator").as_ref(),
        initial: numerator as f64,
        range_min: 1.0,
        range_max: 255.0,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_num) => {
            let new_num = new_num as u8;
            if new_num != numerator {
                let model = &doc.data.model;
                if let Some(e) = model.conductor.time_sig.iter().find(|e| e.tick == tick) {
                    let _ = doc.set_time_sig_event(tick, tick, new_num, e.denominator);
                }
            }
        }
        PopupAction::Closed => {
            finalize_timesig_undo(ui, doc, salt, before, t!("undo.edit_timesig").as_ref());
        }
        PopupAction::None => {}
    }
}

fn show_timesig_den_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
) {
    let before = record_timesig_before(ui, doc, salt);
    let denominator = doc.data.model.conductor.time_sig.iter()
        .find(|e| e.tick == tick).map(|e| e.denominator).unwrap_or(2);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_denominator").as_ref(),
        initial: denominator as f64,
        range_min: 0.0,
        range_max: 8.0,  // 2^8 = 256
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_den) => {
            let new_den = new_den as u8;
            if new_den != denominator {
                let model = &doc.data.model;
                if let Some(e) = model.conductor.time_sig.iter().find(|e| e.tick == tick) {
                    let _ = doc.set_time_sig_event(tick, tick, e.numerator, new_den);
                }
            }
        }
        PopupAction::Closed => {
            finalize_timesig_undo(ui, doc, salt, before, t!("undo.edit_timesig").as_ref());
        }
        PopupAction::None => {}
    }
}

/// popup 显示期间记录 time_sig before 快照（仅第一次记录）。
fn record_timesig_before(
    ui: &egui::Ui,
    doc: &Document,
    salt: &str,
) -> Option<Vec<yinhe_types::TimeSigEvent>> {
    let before_id = egui::Id::new((salt, "before"));
    let recorded_id = before_id.with("recorded");
    let recorded = ui.memory(|m| m.data.get_temp::<bool>(recorded_id).unwrap_or(false));
    if !recorded {
        let before = doc.data.model.conductor.time_sig.clone();
        ui.memory_mut(|m| {
            m.data.insert_temp(before_id, before.clone());
            m.data.insert_temp(recorded_id, true);
        });
        Some(before)
    } else {
        ui.memory(|m| m.data.get_temp::<Vec<yinhe_types::TimeSigEvent>>(before_id))
    }
}

/// popup 关闭时取 after 对比，push undo，清除所有 popup 状态。
fn finalize_timesig_undo(
    ui: &egui::Ui,
    doc: &mut Document,
    salt: &str,
    before: Option<Vec<yinhe_types::TimeSigEvent>>,
    label: &str,
) {
    use yinhe_editor_core::history::{UndoAction, UndoEntry};
    if let Some(before) = before {
        let after = doc.data.model.conductor.time_sig.clone();
        if before != after {
            doc.history.push(UndoEntry {
                action: UndoAction::TimeSig { old: before, new: after },
                label: label.to_string(),
                selected: doc.edit.selected.clone(),
                track_selected: doc.edit.track_selected.clone(),
                sel_rect: doc.edit.sel_rect.clone(),
            });
        }
    }
    let before_id = egui::Id::new((salt, "before"));
    ui.memory_mut(|m| {
        m.data.remove::<Vec<yinhe_types::TimeSigEvent>>(before_id);
        m.data.remove::<bool>(before_id.with("recorded"));
    });
    remove_edit_request(ui, salt);
}
