//! 右键编辑 popup：在表格的单元格上右键时弹出编辑器。
//!
//! 设计（pending + 关闭时一次性 apply）：
//! - cell 右键时把 `EditRequest` 写到 `egui::Id::new((salt, "edit"))`（全局 key，
//!   不用 `ui.id()`，因为 cell 是 child ui，`ui.id()` 与本函数调用处不同）
//! - `apply_*_popups` 每帧 `peek_edit_request` 检查是否有请求，有就显示 popup
//! - popup 打开期间**不修改 Document**：DragValue 状态只写到 egui memory
//!   中的 pending 状态，每帧读出以保持拖动时数字同步
//! - 关闭时一次性 apply：
//!   - `PopupAction::Closed(v)`（确认按钮 / lost_focus / Enter）→ 读 pending 值
//!     应用到 Document，push undo
//!   - `PopupAction::Cancelled`（取消按钮）→ 不碰 Document，不产生 undo，仅清理
//!
//! 按事件类型拆分为子模块：
//! - `automation`: CC/PB/RPN/NRPN/Tempo 的 value/tick/shape
//! - `note`: 音符的 start_tick/end_tick/gate/key/velocity
//! - `timesig`: 拍号的 tick/numerator/denominator
//! - `keysig`: 调号的 tick/sf/mi
//! - `text`: Marker/Lyrics/Chord 的 tick/text

mod automation;
mod keysig;
mod note;
mod pc;
mod text;
mod timesig;

use eframe::egui;

use rust_i18n::t;

use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{
    EditSnapshot, EventListDelta, EventListItem, EventListTarget, UndoAction,
};

use super::bar_lookup::BarLookup;
use super::table::{remove_edit_request, remove_pos_edit_request};

pub(super) use automation::apply_automation_popups;
pub(super) use keysig::apply_keysig_popups;
pub(super) use note::apply_note_popups;
pub(super) use pc::apply_pc_popups;
pub(super) use text::apply_text_popups;
pub(super) use timesig::apply_timesig_popups;

/// popup 关闭事件。
///
/// 打开期间返回 `None`；关闭时区分两种语义：
/// - `Closed(v)`：用户确认（确认按钮 / lost_focus / Enter），携带最终 pending 值，
///   caller 一次性把 v 应用到 Document 并 push undo。
/// - `Cancelled`：用户取消（取消按钮），不碰 Document，不产生 undo，仅清理。
pub(super) enum PopupAction {
    /// 本帧无变化（popup 仍打开）
    None,
    /// popup 关闭并确认，携带最终 pending 值
    Closed(f64),
    /// popup 取消，不 apply
    Cancelled,
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

/// 渲染数字编辑 popup（Area + DragValue + confirm/cancel）。
///
/// DragValue 状态持久化到 `egui::Id::new((salt, "state"))`，每帧从 memory 读出，
/// 拖动时实时更新（不修改 Document）。关闭时返回 `Closed(state)` 携带 pending 值，
/// 或 `Cancelled`（取消按钮）。caller 负责把 pending 应用到 Document。
fn show_number_popup(ui: &mut egui::Ui, cfg: PopupConfig) -> PopupAction {
    let state_id = egui::Id::new((cfg.salt, "state"));
    let popup_id = ui.id().with((cfg.salt, "popup"));

    let mut state = ui.memory(|m| m.data.get_temp::<f64>(state_id).unwrap_or(cfg.initial));
    let mut open = true;
    let mut cancelled = false;
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
                if resp.lost_focus() {
                    open = false;
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
        ui.memory_mut(|m| m.data.remove::<f64>(state_id));
        if cancelled {
            PopupAction::Cancelled
        } else {
            PopupAction::Closed(state)
        }
    } else {
        ui.memory_mut(|m| m.data.insert_temp(state_id, state));
        PopupAction::None
    }
}

/// 下拉选择 popup 的关闭事件。语义与 `PopupAction` 一致。
pub(super) enum ChoicePopupAction<T> {
    None,
    Closed(T),
    Cancelled,
}

/// 渲染下拉选择 popup（Area + ComboBox + confirm/cancel）。
///
/// 选项状态持久化到 `egui::Id::new((salt, "choice"))`，每帧从 memory 读出。
/// 关闭时返回 `Closed(state)` 携带 pending 选项，或 `Cancelled`。
fn show_choice_popup<T: Copy + PartialEq + Send + Sync + 'static>(
    ui: &mut egui::Ui,
    salt: &str,
    title: &str,
    initial: T,
    options: &[T],
    label_of: impl Fn(&T) -> String,
) -> ChoicePopupAction<T> {
    let state_id = egui::Id::new((salt, "choice"));
    let popup_id = ui.id().with((salt, "choice_popup"));

    let mut state = ui.memory(|m| m.data.get_temp::<T>(state_id).unwrap_or(initial));
    let mut open = true;
    let mut cancelled = false;
    let popup_pos = ui.clip_rect().min + egui::vec2(20.0, 20.0);

    egui::Area::new(popup_id)
        .order(egui::Order::Foreground)
        .fixed_pos(popup_pos)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(180.0);
                ui.label(egui::RichText::new(title).strong().size(11.0));
                ui.add_space(2.0);
                let _resp = egui::ComboBox::from_id_salt(salt)
                    .selected_text(label_of(&state))
                    .show_ui(ui, |ui| {
                        for opt in options {
                            ui.selectable_value(&mut state, *opt, label_of(opt));
                        }
                    });
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
        ui.memory_mut(|m| m.data.remove::<T>(state_id));
        if cancelled {
            ChoicePopupAction::Cancelled
        } else {
            ChoicePopupAction::Closed(state)
        }
    } else {
        ui.memory_mut(|m| m.data.insert_temp(state_id, state));
        ChoicePopupAction::None
    }
}

/// tick 类 popup 的统一入口：根据请求来源选择位置 popup（双 DragValue）或数字 popup。
///
/// `bar_lookup` 为 `Some` 表示请求来自 position 列（`(salt, "edit_pos")` key），
/// 弹位置编辑器；`None` 表示来自 tick 列，弹单数字编辑器。
/// `min_tick` 约束最小 tick（如音符 end_tick 必须 > start_tick）。
pub(super) fn show_tick_popup(
    ui: &mut egui::Ui,
    salt: &str,
    title: &str,
    tick: u32,
    min_tick: u32,
    bar_lookup: Option<&BarLookup>,
) -> PopupAction {
    match bar_lookup {
        Some(bl) => show_position_popup(ui, salt, title, bl, tick, min_tick),
        None => show_number_popup(
            ui,
            PopupConfig {
                salt,
                title,
                initial: tick as f64,
                range_min: min_tick as f64,
                range_max: u32::MAX as f64,
                speed: 1.0,
                fixed_decimals: None,
            },
        ),
    }
}

/// 渲染位置编辑 popup（Area + 小节/小节内 tick 两个 DragValue + confirm/cancel）。
///
/// 状态存当前 tick（f64）到 `(salt, "pos_state")`，每帧换算为 (小节, 小节内 tick)
/// 显示；两个 DragValue 修改本地副本后用**值比较**检测变化（不依赖 `resp.changed()`，
/// 后者在 Area 中不可靠），再经 `BarLookup::position_to_tick` 换算回 tick。
/// 关闭时返回 `Closed(tick)` 携带 pending tick，或 `Cancelled`。
pub(super) fn show_position_popup(
    ui: &mut egui::Ui,
    salt: &str,
    title: &str,
    bar_lookup: &BarLookup,
    current_tick: u32,
    min_tick: u32,
) -> PopupAction {
    let state_id = egui::Id::new((salt, "pos_state"));
    let popup_id = ui.id().with((salt, "pos_popup"));

    let mut tick_f = ui.memory(|m| {
        m.data
            .get_temp::<f64>(state_id)
            .unwrap_or(current_tick as f64)
    });
    let mut open = true;
    let mut cancelled = false;
    let popup_pos = ui.clip_rect().min + egui::vec2(20.0, 20.0);

    egui::Area::new(popup_id)
        .order(egui::Order::Foreground)
        .fixed_pos(popup_pos)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(200.0);
                ui.label(egui::RichText::new(title).strong().size(11.0));
                ui.add_space(2.0);
                let (bar, tick_in_bar) = bar_lookup.tick_to_position(tick_f.max(0.0) as u32);
                let mut bar_f = bar as f64;
                let mut tib_f = tick_in_bar as f64;
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("小节")
                            .size(11.0)
                            .color(egui::Color32::GRAY),
                    );
                    ui.add(
                        crate::widgets::numeric_input::decimal_drag_value(&mut bar_f)
                            .range(1.0..=u32::MAX as f64)
                            .speed(1.0)
                            .fixed_decimals(0),
                    );
                    ui.label(
                        egui::RichText::new("/")
                            .size(11.0)
                            .color(egui::Color32::GRAY),
                    );
                    ui.add(
                        crate::widgets::numeric_input::decimal_drag_value(&mut tib_f)
                            .range(0.0..=u32::MAX as f64)
                            .speed(1.0)
                            .fixed_decimals(0),
                    );
                });
                // 值比较：任一 dragvalue 变了就换算回 tick
                if bar_f != bar as f64 || tib_f != tick_in_bar as f64 {
                    tick_f = bar_lookup
                        .position_to_tick(bar_f.max(1.0) as u32, tib_f.max(0.0) as u32)
                        as f64;
                    tick_f = tick_f.max(min_tick as f64);
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
        ui.memory_mut(|m| m.data.remove::<f64>(state_id));
        if cancelled {
            PopupAction::Cancelled
        } else {
            PopupAction::Closed(tick_f)
        }
    } else {
        ui.memory_mut(|m| m.data.insert_temp(state_id, tick_f));
        PopupAction::None
    }
}

// ---- 共享 helper（供各子模块复用，避免重复样板）----

/// 构造并 push `UndoAction::EventList`（before != after 时才 push）。
/// 供 keysig/timesig/text/pc 共用：它们的 undo 都是"某事件列表整体替换"。
/// `snapshot` 必须是编辑**前**捕获的界面状态快照。
pub(super) fn push_event_list_undo(
    doc: &mut Document,
    target: EventListTarget,
    before: Vec<EventListItem>,
    after: Vec<EventListItem>,
    label: &str,
    snapshot: EditSnapshot,
) {
    if before != after {
        doc.push_undo(
            UndoAction::EventList(EventListDelta {
                target,
                old: before,
                new: after,
            }),
            label,
            snapshot,
        );
    }
}

/// 清除 EditRequest（普通 + 位置）。popup 关闭（无论确认或取消）后调用。
pub(super) fn cleanup_edit_request(ui: &egui::Ui, salt: &str) {
    remove_edit_request(ui, salt);
    remove_pos_edit_request(ui, salt);
}
