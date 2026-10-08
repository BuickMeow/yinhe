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
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Selection {
    #[default]
    CustomCc,
    CustomRpn,
    /// 指向 `entries` 的索引。
    Entry(usize),
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
/// 行布局统一「目标在左、控件（步进器/单选）在右」；所有行放在同一个
/// ScrollArea 里（自定义两行是第 0/1 行），保证各行同宽、圆点对齐。
fn content(ui: &mut egui::Ui, state: &mut AutomationPickerState) {
    ui.add_space(4.0);
    let row_h = crate::scaling::scaled_font(ui.ctx(), 30.0);
    let pad = crate::scaling::scaled_font(ui.ctx(), 8.0);
    let label_size = crate::scaling::scaled_font(ui.ctx(), crate::theme::BODY_FONT);

    // 设备自带参数：标题 + 搜索（固定，不随列表滚动）。
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
    ui.add_space(4.0);

    let cc_hint = t!("hint.pr.custom_cc");
    let rpn_hint = t!("hint.pr.custom_rpn");
    let cc_label = t!("arrange.custom_cc");
    let rpn_label = t!("arrange.custom_rpn");

    // 前两行固定为自定义 CC / RPN，其余为设备参数（行 2 起）。
    let total = 2 + state.filtered.len();
    egui::ScrollArea::vertical()
        .id_salt("automation_picker_list")
        .auto_shrink([false, false])
        .max_height(ui.available_height())
        .show_rows(ui, row_h, total, |ui, range| {
            let dev_start = range.start.saturating_sub(2);
            let dev_end = range.end.saturating_sub(2).min(state.filtered.len());
            let vis: Vec<usize> = if dev_start < dev_end {
                state.filtered[dev_start..dev_end].to_vec()
            } else {
                Vec::new()
            };
            let mut vis_iter = vis.iter();
            for row_idx in range.clone() {
                match row_idx {
                    0 => {
                        if pick_row(
                            ui,
                            state.selection == Selection::CustomCc,
                            cc_label.as_ref(),
                            Some((&mut state.custom_cc, cc_hint.as_ref())),
                            row_h,
                            pad,
                            label_size,
                        ) {
                            state.selection = Selection::CustomCc;
                        }
                    }
                    1 => {
                        if pick_row(
                            ui,
                            state.selection == Selection::CustomRpn,
                            rpn_label.as_ref(),
                            Some((&mut state.custom_rpn, rpn_hint.as_ref())),
                            row_h,
                            pad,
                            label_size,
                        ) {
                            state.selection = Selection::CustomRpn;
                        }
                    }
                    _ => {
                        if let Some(&pi) = vis_iter.next() {
                            let label = state.entries[pi].label.clone();
                            if pick_row(
                                ui,
                                state.selection == Selection::Entry(pi),
                                &label,
                                None,
                                row_h,
                                pad,
                                label_size,
                            ) {
                                state.selection = Selection::Entry(pi);
                            }
                        }
                    }
                }
            }
        });
}

/// 一行单选：目标名在左；右侧依次为「可选步进器 + 单选圆点」。
/// 只有单选圆点可点击（返回它是否被点击），整行不可点。
///
/// 用固定高度的 `allocate_ui_with_layout` 行（不用 `scope_builder` 放控件：
/// `scope_builder` 会按子内容高度回退父游标，导致带控件的行变矮、行距不齐）。
fn pick_row(
    ui: &mut egui::Ui,
    selected: bool,
    label: &str,
    stepper: Option<(&mut i32, &str)>,
    row_h: f32,
    pad: f32,
    label_size: f32,
) -> bool {
    let stepper_w = crate::scaling::scaled_font(ui.ctx(), 96.0);
    let outcome = ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), row_h),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let row_rect = ui.max_rect();
            if selected {
                ui.painter()
                    .rect_filled(row_rect, 4.0, crate::theme::selected_bg());
            }
            ui.add_space(pad);
            ui.label(
                egui::RichText::new(label)
                    .size(label_size)
                    .color(if selected {
                        crate::theme::text_bright()
                    } else {
                        crate::theme::text_secondary()
                    }),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(pad);
                let radio = crate::widgets::radio::radio(ui, selected);
                if let Some((value, hint)) = stepper {
                    let sr = crate::widgets::stepper::stepper(value)
                        .range(0..=127)
                        .step(1.0)
                        .width(stepper_w)
                        .show(ui);
                    crate::widgets::hint::hover(ui.ctx(), &sr, hint);
                }
                radio.clicked()
            })
            .inner
        },
    );
    outcome.inner
}

#[cfg(test)]
mod geom_tests {
    use super::*;

    fn init_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        ctx.add_font(egui_material_icons::font_insert());
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
            out.drop_without_applying_deltas();
        }
        ctx
    }

    /// 回归：带步进器的行与普通行必须等高、同距、左缘对齐（曾因用
    /// `scope_builder` 放步进器导致父游标回退，行距变成 27/33 不一致）。
    #[test]
    fn rows_share_pitch_and_left_edge() {
        let ctx = init_ctx();
        let row_h = crate::scaling::scaled_font(&ctx, 30.0);
        let pad = crate::scaling::scaled_font(&ctx, 8.0);
        let mut tops: Vec<f32> = Vec::new();
        let mut minx: Vec<f32> = Vec::new();
        let mut v = 0i32;
        let mut item_spacing_y = 0.0f32;
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(500.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                item_spacing_y = ui.spacing().item_spacing.y;
                egui::ScrollArea::vertical()
                    .id_salt("t")
                    .auto_shrink([false, false])
                    .show_rows(ui, row_h, 6, |ui, range| {
                        for i in range {
                            let cursor = ui.cursor();
                            tops.push(cursor.min.y);
                            minx.push(cursor.min.x);
                            let stepper = if i < 2 { Some((&mut v, "hint")) } else { None };
                            let _ = pick_row(ui, i == 0, "Pitch Bend", stepper, row_h, pad, 12.0);
                        }
                    });
            },
        );
        out.drop_without_applying_deltas();

        let pitches: Vec<f32> = tops.windows(2).map(|w| w[1] - w[0]).collect();
        let expected = row_h + item_spacing_y;
        assert!(
            pitches.iter().all(|p| (p - expected).abs() < 0.5),
            "行距不一致（期望 {expected}）: {pitches:?}"
        );
        assert!(
            minx.iter().all(|x| (x - minx[0]).abs() < 0.5),
            "各行左缘不齐: {minx:?}"
        );
    }
}
