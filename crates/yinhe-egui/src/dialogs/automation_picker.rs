//! 「添加自动化」窗口（独立 OS viewport）。
//!
//! 自动化属于**乐器设备**（XSynth 或插件），不属于音轨：
//! - 左列：自定义 CC / 自定义 RPN 两个网格（8 列 × 16 行 = 128 格，格子内写编号，
//!   点亮 = 已有对应 lane，点击增/删）；
//! - 右列：该设备自带的参数（XSynth 内置参数 / 插件参数），带搜索。
//!
//! 窗口不修改模型：条目标签与目标在打开时（App 侧）收集，点击返回 [`Toggle`]
//! 动作，由 App 落模型并回写点亮状态。

use std::collections::HashSet;

use eframe::egui;
use rust_i18n::t;
use yinhe_types::AutomationTarget;

/// 网格列数（8 列 × 16 行 = 128 格）。
const GRID_COLS: usize = 8;
const GRID_ROWS: usize = 16;
const GRID_CELL_H: f32 = 16.0;
const GRID_GAP: f32 = 1.0;

/// 窗口里的一个可添加项（右侧「设备自带参数」）。
pub(crate) struct AutomationEntry {
    pub target: AutomationTarget,
    /// 显示名（含 CC 名 / 模块）。
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
    /// 右侧「设备自带参数」条目。
    entries: Vec<AutomationEntry>,
    /// 当前已存在的自动化目标（左网格 + 右列表共用）。
    existing: HashSet<AutomationTarget>,
    /// 搜索过滤后的条目索引。
    filtered: Vec<usize>,
    search: String,
    filter_dirty: bool,
}

/// 用户动作（窗口不修改模型，交给 App 处理）。
pub(crate) enum AutomationPickerAction {
    None,
    /// 添加/移除该目标的 AM lane。
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
        existing: HashSet<AutomationTarget>,
    ) {
        self.open = true;
        self.just_opened = true;
        self.track_idx = track_idx;
        self.title = title;
        self.device_name = device_name;
        self.entries = entries;
        self.existing = existing;
        self.search.clear();
        self.filter_dirty = true;
        self.rebuild_filter();
    }

    /// App 落模型后回写点亮状态。
    pub(crate) fn set_existing(&mut self, target: &AutomationTarget, existing: bool) {
        if existing {
            self.existing.insert(target.clone());
        } else {
            self.existing.remove(target);
        }
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
}

/// 翻转一个目标并返回对应动作（同时更新本地点亮集合）。
fn toggle(state: &mut AutomationPickerState, target: AutomationTarget) -> AutomationPickerAction {
    let add = !state.existing.contains(&target);
    if add {
        state.existing.insert(target.clone());
    } else {
        state.existing.remove(&target);
    }
    AutomationPickerAction::Toggle {
        track_idx: state.track_idx,
        target,
        add,
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
        crate::chrome::dialog::viewport_builder(title.as_ref(), [860.0, 640.0], false),
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
                            ui.columns(2, |cols| {
                                egui::ScrollArea::vertical()
                                    .id_salt("custom_grids_scroll")
                                    .auto_shrink([false, false])
                                    .show(&mut cols[0], |ui| {
                                        custom_grids(ui, state, &mut action);
                                    });
                                device_params(&mut cols[1], state, &mut action);
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

/// 左列：自定义 CC / 自定义 RPN 两个 8×16 网格。
fn custom_grids(
    ui: &mut egui::Ui,
    state: &mut AutomationPickerState,
    action: &mut AutomationPickerAction,
) {
    ui.label(egui::RichText::new(t!("arrange.custom_cc")).strong().size(
        crate::scaling::scaled_font(ui.ctx(), crate::theme::SUB_TITLE_FONT),
    ));
    ui.add_space(4.0);
    if let Some(i) = number_grid(ui, &state.existing, |i| AutomationTarget::CC {
        controller: i as u8,
    }) {
        *action = toggle(
            state,
            AutomationTarget::CC {
                controller: i as u8,
            },
        );
    }

    ui.add_space(12.0);
    ui.label(egui::RichText::new(t!("arrange.custom_rpn")).strong().size(
        crate::scaling::scaled_font(ui.ctx(), crate::theme::SUB_TITLE_FONT),
    ));
    ui.add_space(4.0);
    if let Some(i) = number_grid(ui, &state.existing, |i| AutomationTarget::Rpn {
        parameter: i as u16,
    }) {
        *action = toggle(
            state,
            AutomationTarget::Rpn {
                parameter: i as u16,
            },
        );
    }
}

/// 8 列 × 16 行 = 128 格网格；返回本帧被点击的格号。
fn number_grid(
    ui: &mut egui::Ui,
    existing: &HashSet<AutomationTarget>,
    make_target: impl Fn(usize) -> AutomationTarget,
) -> Option<usize> {
    let gap = crate::scaling::scaled_font(ui.ctx(), GRID_GAP);
    let cell_h = crate::scaling::scaled_font(ui.ctx(), GRID_CELL_H);
    let cell_w =
        ((ui.available_width() - gap * (GRID_COLS as f32 - 1.0)) / GRID_COLS as f32).max(1.0);
    let font = egui::FontId::proportional(crate::scaling::scaled_font(
        ui.ctx(),
        crate::theme::SMALL_LABEL_FONT,
    ));
    let lit_bg = crate::theme::accent_active();
    let lit_fg = crate::theme::contrast_fg();
    let idle_bg = crate::theme::btn_bg();
    let idle_fg = crate::theme::text_secondary();
    let mut clicked = None;

    for row in 0..GRID_ROWS {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for col in 0..GRID_COLS {
                let i = row * GRID_COLS + col;
                let target = make_target(i);
                let lit = existing.contains(&target);
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(cell_w, cell_h), egui::Sense::click());
                let bg = if lit {
                    lit_bg
                } else if resp.hovered() {
                    crate::theme::hover_color(idle_bg)
                } else {
                    idle_bg
                };
                let fg = if lit { lit_fg } else { idle_fg };
                let painter = ui.painter();
                painter.rect_filled(rect, 2.0, bg);
                painter.rect_stroke(
                    rect,
                    2.0,
                    crate::widgets::control::control_stroke(true, false),
                    egui::StrokeKind::Inside,
                );
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{i}"),
                    font.clone(),
                    fg,
                );
                crate::widgets::hint::hover(ui.ctx(), &resp, target.display_name());
                if resp.clicked() {
                    clicked = Some(i);
                }
            }
        });
        if row + 1 < GRID_ROWS {
            ui.add_space(gap);
        }
    }
    clicked
}

/// 右列：设备自带参数列表（搜索 + 开关）。
fn device_params(
    ui: &mut egui::Ui,
    state: &mut AutomationPickerState,
    action: &mut AutomationPickerAction,
) {
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
    ui.separator();

    if state.entries.is_empty() {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(t!("dialog.automation.empty"))
                .size(crate::theme::SMALL_FONT)
                .color(crate::theme::text_muted()),
        );
        return;
    }

    let row_h = crate::scaling::scaled_font(ui.ctx(), 30.0);
    egui::ScrollArea::vertical()
        .id_salt("automation_picker_list")
        .auto_shrink([false, false])
        .max_height(ui.available_height())
        .show_rows(ui, row_h, state.filtered.len(), |ui, range| {
            let total = state.filtered.len();
            let pis: Vec<usize> = state.filtered[range.clone()].to_vec();
            for (offset, &pi) in pis.iter().enumerate() {
                let row_idx = range.start + offset;
                let label = state.entries[pi].label.clone();
                let target = state.entries[pi].target.clone();
                let mut on = state.existing.contains(&target);
                let mut toggled = false;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label).size(crate::scaling::scaled_font(
                        ui.ctx(),
                        crate::theme::BODY_FONT,
                    )));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::widgets::switch::switch(ui, &mut on).changed() {
                            toggled = true;
                        }
                    });
                });
                if toggled {
                    *action = toggle(state, target);
                }
                // 行间分割线：末尾不再画一条多余的分割线。
                if row_idx + 1 < total {
                    ui.add_space(2.0);
                    ui.separator();
                    ui.add_space(2.0);
                }
            }
        });
}
