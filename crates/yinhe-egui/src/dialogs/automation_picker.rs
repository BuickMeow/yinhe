//! 「添加自动化」窗口（独立 OS viewport）。
//!
//! 自动化属于**乐器设备**（XSynth 或插件），不属于音轨。窗口是**单选**选择器：
//! - 自定义 CC：数字步进器选控制器号；
//! - 自定义 RPN：数字步进器选参数号；
//! - 设备自带参数（含 Pitch Bend、XSynth 内置参数 / 插件参数），可搜索。
//!
//! 选中一项后点底部「添加」：由 App 落模型（建一条 AM lane）并关窗。
//! 窗口本身不修改模型，只返回 [`Toggle`] 动作。

use eframe::egui;
use rust_i18n::t;
use yinhe_types::AutomationTarget;

/// 单选状态。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Selection {
    CustomCc,
    CustomRpn,
    /// 指向 `entries` 的索引。
    Entry(usize),
}

impl Default for Selection {
    fn default() -> Self {
        Selection::CustomCc
    }
}

/// 窗口里的一个可添加项（设备自带参数）。
pub(crate) struct AutomationEntry {
    pub target: AutomationTarget,
    /// 显示名（含 CC/RPN 名 / 模块）。
    pub label: String,
}

/// 窗口状态（挂在 App 上，跨帧保留；打开时重置）。
#[derive(Default)]
pub(crate) struct AutomationPickerState {
    pub open: bool,
    /// 打开后首帧把窗口提到最前。
    pub just_opened: bool,
    /// 目标轨道（lane 建在该轨上）。
    pub track_idx: usize,
    /// 窗口标题里的轨道名。
    title: String,
    /// 设备名（XSynth / Serum 2）。
    device_name: String,
    /// 设备自带参数条目。
    entries: Vec<AutomationEntry>,
    /// 搜索过滤后的条目索引。
    filtered: Vec<usize>,
    search: String,
    filter_dirty: bool,
    /// 当前选中项。
    selection: Selection,
    /// 自定义 CC 控制器号。
    custom_cc: i32,
    /// 自定义 RPN 参数号。
    custom_rpn: i32,
}

/// 用户动作（窗口不修改模型，交给 App 处理）。
pub(crate) enum AutomationPickerAction {
    None,
    /// 添加该目标的 AM lane。
    Toggle {
        track_idx: usize,
        target: AutomationTarget,
        add: bool,
    },
    Close,
}

impl AutomationPickerState {
    /// 打开窗口并填入条目。
    pub(crate) fn open(
        &mut self,
        track_idx: usize,
        title: String,
        device_name: String,
        entries: Vec<AutomationEntry>,
    ) {
        self.open = true;
        self.just_opened = true;
        self.track_idx = track_idx;
        self.title = title;
        self.device_name = device_name;
        self.entries = entries;
        self.search.clear();
        self.filter_dirty = true;
        self.selection = Selection::default();
        self.rebuild_filter();
    }

    fn rebuild_filter(&mut self) {
        let needle = self.search.trim().to_lowercase();
        self.filtered.clear();
        if needle.is_empty() {
            self.filtered.extend(0..self.entries.len());
        } else {
            self.filtered.extend(
                self.entries
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| e.label.to_lowercase().contains(&needle).then_some(i)),
            );
        }
        self.filter_dirty = false;
    }

    /// 当前选中项对应的目标（越界返回 None）。
    fn selected_target(&self) -> Option<AutomationTarget> {
        match self.selection {
            Selection::CustomCc => Some(AutomationTarget::CC {
                controller: self.custom_cc.clamp(0, 127) as u8,
            }),
            Selection::CustomRpn => Some(AutomationTarget::Rpn {
                parameter: self.custom_rpn.clamp(0, 127) as u16,
            }),
            Selection::Entry(i) => self.entries.get(i).map(|e| e.target.clone()),
        }
    }
}

/// 显示「添加自动化」窗口。
pub(crate) fn show_viewport(
    ctx: &egui::Context,
    state: &mut AutomationPickerState,
) -> AutomationPickerAction {
    let viewport_id = egui::ViewportId::from_hash_of("automation_picker");
    let title = t!("dialog.automation.title", ch = state.title.as_str());
    let mut action = AutomationPickerAction::None;
    let mut closed = false;

    ctx.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(title.as_ref(), [520.0, 560.0], false),
        |vctx, _class| {
            if vctx.input(|i| i.viewport().close_requested()) {
                closed = true;
            }
            let mut close = closed;
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::app_bg(),
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, title.as_ref(), &mut close, false);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            if state.filter_dirty {
                                state.rebuild_filter();
                            }
                            let btn_zone = crate::chrome::dialog_buttons::btn_zone_h(ui.ctx());
                            // 内容区（按钮行以上）。
                            ui.allocate_ui_with_layout(
                                egui::vec2(
                                    ui.available_width(),
                                    (ui.available_height() - btn_zone).max(0.0),
                                ),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| content(ui, state),
                            );
                            // 底部按钮行（贴底、右对齐）。
                            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                                use crate::chrome::dialog_buttons::{
                                    DialogButton, dialog_button_row,
                                };
                                let clicked = dialog_button_row(
                                    ui,
                                    &[
                                        DialogButton::secondary(t!("common.cancel").as_ref()),
                                        DialogButton::primary(t!("dialog.automation.add").as_ref()),
                                    ],
                                );
                                match clicked {
                                    Some(0) => close = true,
                                    Some(1) => {
                                        if let Some(target) = state.selected_target() {
                                            action = AutomationPickerAction::Toggle {
                                                track_idx: state.track_idx,
                                                target,
                                                add: true,
                                            };
                                        }
                                        close = true;
                                    }
                                    _ => {}
                                }
                            });
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                closed = true;
            }
        },
    );

    if closed {
        state.open = false;
        AutomationPickerAction::Close
    } else {
        action
    }
}

/// 内容区：自定义 CC / 自定义 RPN 两行 + 设备参数单选列表。
fn content(ui: &mut egui::Ui, state: &mut AutomationPickerState) {
    ui.add_space(4.0);
    let title_size = crate::scaling::scaled_font(ui.ctx(), crate::theme::SUB_TITLE_FONT);

    // 自定义 CC：单选 + 控制器号步进器。
    ui.horizontal(|ui| {
        ui.radio_value(
            &mut state.selection,
            Selection::CustomCc,
            egui::RichText::new(t!("arrange.custom_cc")).size(title_size),
        );
        crate::widgets::stepper::stepper(&mut state.custom_cc)
            .range(0..=127)
            .step(1.0)
            .width(96.0)
            .show(ui);
    });
    ui.add_space(4.0);

    // 自定义 RPN：单选 + 参数号步进器。
    ui.horizontal(|ui| {
        ui.radio_value(
            &mut state.selection,
            Selection::CustomRpn,
            egui::RichText::new(t!("arrange.custom_rpn")).size(title_size),
        );
        crate::widgets::stepper::stepper(&mut state.custom_rpn)
            .range(0..=127)
            .step(1.0)
            .width(96.0)
            .show(ui);
    });
    ui.add_space(6.0);
    ui.separator();

    // 设备自带参数：标题 + 搜索。
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{} · {}",
                state.device_name,
                t!("dialog.automation.count", n = state.entries.len())
            ))
            .size(crate::theme::SMALL_FONT)
            .color(crate::theme::text_muted()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::widgets::text_input::control_text_input(
                ui,
                &mut state.search,
                140.0,
                "auto_picker_search",
                Some(t!("mix.search").as_ref()),
            )
            .changed()
            {
                state.filter_dirty = true;
            }
        });
    });
    ui.add_space(2.0);

    if state.entries.is_empty() {
        ui.label(
            egui::RichText::new(t!("dialog.automation.empty"))
                .size(crate::theme::SMALL_FONT)
                .color(crate::theme::text_muted()),
        );
        return;
    }

    let row_h = crate::scaling::scaled_font(ui.ctx(), 24.0);
    egui::ScrollArea::vertical()
        .id_salt("automation_picker_list")
        .auto_shrink([false, false])
        .max_height(ui.available_height())
        .show_rows(ui, row_h, state.filtered.len(), |ui, range| {
            let pis: Vec<usize> = state.filtered[range.clone()].to_vec();
            for &pi in &pis {
                let label = state.entries[pi].label.clone();
                ui.radio_value(&mut state.selection, Selection::Entry(pi), label);
            }
        });
}
