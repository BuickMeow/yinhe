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
mod text;
mod timesig;

use eframe::egui;

use rust_i18n::t;

pub(super) use automation::apply_automation_popups;
pub(super) use keysig::apply_keysig_popups;
pub(super) use note::apply_note_popups;
pub(super) use text::apply_text_popups;
pub(super) use timesig::apply_timesig_popups;

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

/// 下拉选择 popup 的动作。
pub(super) enum ChoicePopupAction<T> {
    None,
    Changed(T),
    Closed,
}

/// 渲染下拉选择 popup（Area + ComboBox + confirm）。
///
/// 选项状态持久化到 `egui::Id::new((salt, "choice"))`，每帧从 memory 读出。
/// `label_of` 把选项值转为显示文本。
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
    let mut action = ChoicePopupAction::None;
    let mut open = true;
    let popup_pos = ui.clip_rect().min + egui::vec2(20.0, 20.0);

    egui::Area::new(popup_id)
        .order(egui::Order::Foreground)
        .fixed_pos(popup_pos)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(180.0);
                ui.label(egui::RichText::new(title).strong().size(11.0));
                ui.add_space(2.0);
                let resp = egui::ComboBox::from_id_salt(salt)
                    .selected_text(label_of(&state))
                    .show_ui(ui, |ui| {
                        for opt in options {
                            ui.selectable_value(&mut state, *opt, label_of(opt));
                        }
                    });
                if resp.response.changed() {
                    action = ChoicePopupAction::Changed(state);
                    ui.memory_mut(|m| m.data.insert_temp(state_id, state));
                }
                ui.add_space(2.0);
                if ui.button(t!("common.confirm").as_ref()).clicked() {
                    open = false;
                }
                ui.add_space(2.0);
                if ui.button(t!("common.cancel").as_ref()).clicked() {
                    open = false;
                }
            });
        });

    if !open {
        ui.memory_mut(|m| m.data.remove::<T>(state_id));
        ChoicePopupAction::Closed
    } else {
        action
    }
}
