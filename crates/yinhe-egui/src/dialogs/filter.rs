//! 选择筛选对话框。
//!
//! 在现有选框（矩形空间范围：tick/key/track）之上叠加属性边界：
//! - 音符：音高/轨道/力度/gate，可反选
//! - 自动化：事件类型（target 白名单）、value 范围
//!
//! 应用后 `Selection.filter` 生效，所有选择操作（拖动/删除/复制/统计）
//! 只作用于匹配对象；不物化匹配音符，内存仍是 O(rect 数)。
//! v1 不做逐音符高亮，命中数量由 Info 面板显示。

use eframe::egui;
use rust_i18n::t;
use yinhe_core::{Selection, SelectionFilter, YinModel};
use yinhe_types::AutomationTarget;

/// 自动化事件类型条目。
pub(crate) struct AutomationTargetEntry {
    pub target: AutomationTarget,
    pub label: String,
    pub checked: bool,
}

/// 对话框状态（挂在 App 上，跨帧保留；打开时从当前选区/筛选初始化）。
#[derive(Default)]
pub(crate) struct FilterDialogState {
    pub open: bool,
    /// 本帧刚打开（dialog_dispatch 用于把窗口提升到前台）。
    pub just_opened: bool,

    // ── 音符边界（开关 + 范围）──
    pub key_enabled: bool,
    pub key_lo: u8,
    pub key_hi: u8,
    pub track_enabled: bool,
    pub track_lo: u16,
    pub track_hi: u16,
    pub velocity_enabled: bool,
    pub velocity_lo: u8,
    pub velocity_hi: u8,
    pub gate_enabled: bool,
    pub gate_lo: u32,
    pub gate_hi: u32,
    pub invert: bool,

    // ── 自动化边界 ──
    pub automation_enabled: bool,
    pub automation_targets: Vec<AutomationTargetEntry>,
    pub automation_value_enabled: bool,
    pub automation_value_lo: f32,
    pub automation_value_hi: f32,
}

/// 对话框返回动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FilterDialogAction {
    None,
    /// 应用筛选（保持窗口打开，便于继续调整）。
    Apply,
    /// 应用并关闭。
    Confirm,
    /// 清除筛选（保留选框）。
    Clear,
    /// 取消（不修改）。
    Cancel,
}

impl FilterDialogState {
    /// 打开对话框，从当前选区/筛选与文档自动化类型初始化。
    pub fn open(&mut self, selection: &Selection, model: &YinModel) {
        self.open = true;
        self.just_opened = true;

        let f = &selection.filter;
        self.key_enabled = f.key.is_some();
        let (klo, khi) = f.key.unwrap_or((0, yinhe_types::MAX_KEY));
        self.key_lo = klo;
        self.key_hi = khi;
        self.track_enabled = f.track.is_some();
        let max_track = model.tracks.len().saturating_sub(1) as u16;
        let (tlo, thi) = f.track.unwrap_or((0, max_track));
        self.track_lo = tlo;
        self.track_hi = thi;
        self.velocity_enabled = f.velocity.is_some();
        let (vlo, vhi) = f.velocity.unwrap_or((0, 127));
        self.velocity_lo = vlo;
        self.velocity_hi = vhi;
        self.gate_enabled = f.gate.is_some();
        let (glo, ghi) = f.gate.unwrap_or((1, 100_000));
        self.gate_lo = glo;
        self.gate_hi = ghi;
        self.invert = f.invert;

        let targets = collect_automation_targets(model);
        self.automation_enabled = f.automation_targets.is_some();
        self.automation_targets = targets
            .into_iter()
            .map(|target| {
                let checked = f
                    .automation_targets
                    .as_ref()
                    .map(|list| list.contains(&target))
                    .unwrap_or(true);
                let label = target.display_name();
                AutomationTargetEntry {
                    target,
                    label,
                    checked,
                }
            })
            .collect();
        self.automation_value_enabled = f.automation_value.is_some();
        // 统一参数模型：自动化事件值统一归一化 0..1（Tempo 除外，但筛选按 lane 值比较）。
        let (alo, ahi) = f.automation_value.unwrap_or((0.0, 1.0));
        self.automation_value_lo = alo;
        self.automation_value_hi = ahi;
    }

    /// 生成 `SelectionFilter`。
    pub fn build_filter(&self) -> SelectionFilter {
        let mut f = SelectionFilter::default();
        if self.key_enabled {
            f.key = Some((self.key_lo.min(self.key_hi), self.key_lo.max(self.key_hi)));
        }
        if self.track_enabled {
            f.track = Some((
                self.track_lo.min(self.track_hi),
                self.track_lo.max(self.track_hi),
            ));
        }
        if self.velocity_enabled {
            f.velocity = Some((
                self.velocity_lo.min(self.velocity_hi),
                self.velocity_lo.max(self.velocity_hi),
            ));
        }
        if self.gate_enabled {
            f.gate = Some((
                self.gate_lo.min(self.gate_hi),
                self.gate_lo.max(self.gate_hi),
            ));
        }
        if self.automation_enabled {
            f.automation_targets = Some(
                self.automation_targets
                    .iter()
                    .filter(|e| e.checked)
                    .map(|e| e.target.clone())
                    .collect(),
            );
        }
        if self.automation_value_enabled {
            f.automation_value = Some((
                self.automation_value_lo.min(self.automation_value_hi),
                self.automation_value_lo.max(self.automation_value_hi),
            ));
        }
        f.invert = self.invert;
        f
    }
}

/// 收集文档中实际存在的自动化 target（Tempo + 各轨 lane）。
fn collect_automation_targets(model: &YinModel) -> Vec<AutomationTarget> {
    let mut targets: Vec<AutomationTarget> = Vec::new();
    if !model.conductor.tempo.events.is_empty() {
        targets.push(AutomationTarget::Tempo);
    }
    for track in &model.tracks {
        for lane in &track.automation_lanes {
            if !targets.iter().any(|t| t == &lane.target) {
                targets.push(lane.target.clone());
            }
        }
    }
    targets
}

/// 显示筛选对话框。返回用户动作。
pub(crate) fn show_viewport(
    ctx: &egui::Context,
    state: &mut FilterDialogState,
    num_tracks: usize,
) -> FilterDialogAction {
    let viewport_id = egui::ViewportId::from_hash_of("selection_filter_dialog");
    let title = t!("dialog.filter.title");
    let mut action = FilterDialogAction::None;
    let mut close = false;

    ctx.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(title.as_ref(), [420.0, 540.0], true),
        |vctx, _class| {
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::app_bg(),
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    let mut title_close = false;
                    crate::chrome::dialog::title_bar(ui, title.as_ref(), &mut title_close, true);
                    if title_close {
                        close = true;
                    }
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            let btn_zone_h = crate::chrome::dialog_buttons::btn_zone_h(ui.ctx());
                            crate::chrome::dialog::content_with_bottom_buttons(
                                ui,
                                btn_zone_h,
                                |ui| {
                                    egui::ScrollArea::vertical()
                                        .auto_shrink([false, false])
                                        .max_height(ui.available_height())
                                        .show(ui, |ui| {
                                            show_note_section(ui, state, num_tracks);
                                            ui.add_space(6.0);
                                            show_automation_section(ui, state);
                                        });
                                },
                                |ui| {
                                    use crate::chrome::dialog_buttons::{
                                        DialogButton, dialog_button_row,
                                    };
                                    ui.add_space(8.0);
                                    let clear = t!("dialog.filter.clear");
                                    let cancel = t!("dialog.filter.cancel");
                                    let apply = t!("dialog.filter.apply");
                                    let ok = t!("dialog.filter.ok");
                                    if let Some(idx) = dialog_button_row(
                                        ui,
                                        &[
                                            DialogButton::secondary(clear.as_ref()),
                                            DialogButton::secondary(cancel.as_ref()),
                                            DialogButton::secondary(apply.as_ref()),
                                            DialogButton::primary(ok.as_ref()),
                                        ],
                                    ) {
                                        action = match idx {
                                            0 => FilterDialogAction::Clear,
                                            1 => FilterDialogAction::Cancel,
                                            2 => FilterDialogAction::Apply,
                                            _ => FilterDialogAction::Confirm,
                                        };
                                    }
                                },
                            );
                        });
                });
            if close && action == FilterDialogAction::None {
                action = FilterDialogAction::Cancel;
            }
            if matches!(
                action,
                FilterDialogAction::Confirm | FilterDialogAction::Cancel
            ) {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        },
    );

    action
}

/// 分组标题。
fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(text)
            .strong()
            .size(crate::scaling::scaled_font(
                ui.ctx(),
                crate::theme::SUB_TITLE_FONT,
            )),
    );
    ui.add_space(6.0);
}

/// 属性边界行：自绘开关在左，标题跟随，范围控件右对齐。
fn bound_row(
    ui: &mut egui::Ui,
    title: &str,
    enabled: &mut bool,
    add_range: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        crate::widgets::switch::switch(ui, enabled);
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(title)
                .size(crate::scaling::scaled_font(
                    ui.ctx(),
                    crate::theme::SMALL_FONT,
                ))
                .color(if *enabled {
                    crate::theme::text_primary()
                } else {
                    crate::theme::text_muted()
                }),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(*enabled, |ui| add_range(ui));
        });
    });
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);
}

fn drag_u32(ui: &mut egui::Ui, v: &mut u32, max: u32) {
    ui.add(egui::DragValue::new(v).range(0..=max).speed(1.0));
}

fn show_note_section(ui: &mut egui::Ui, state: &mut FilterDialogState, num_tracks: usize) {
    section_title(ui, t!("dialog.filter.note_bounds").as_ref());
    let max_track = num_tracks.saturating_sub(1) as u32;

    bound_row(
        ui,
        t!("dialog.filter.key").as_ref(),
        &mut state.key_enabled,
        |ui| {
            let mut hi = state.key_hi as u32;
            drag_u32(ui, &mut hi, 127);
            state.key_hi = hi as u8;
            ui.label("~");
            let mut lo = state.key_lo as u32;
            drag_u32(ui, &mut lo, 127);
            state.key_lo = lo as u8;
        },
    );
    bound_row(
        ui,
        t!("dialog.filter.track").as_ref(),
        &mut state.track_enabled,
        |ui| {
            let mut hi = state.track_hi as u32;
            drag_u32(ui, &mut hi, max_track);
            state.track_hi = hi as u16;
            ui.label("~");
            let mut lo = state.track_lo as u32;
            drag_u32(ui, &mut lo, max_track);
            state.track_lo = lo as u16;
        },
    );
    bound_row(
        ui,
        t!("dialog.filter.velocity").as_ref(),
        &mut state.velocity_enabled,
        |ui| {
            let mut hi = state.velocity_hi as u32;
            drag_u32(ui, &mut hi, 127);
            state.velocity_hi = hi as u8;
            ui.label("~");
            let mut lo = state.velocity_lo as u32;
            drag_u32(ui, &mut lo, 127);
            state.velocity_lo = lo as u8;
        },
    );
    bound_row(
        ui,
        t!("dialog.filter.gate").as_ref(),
        &mut state.gate_enabled,
        |ui| {
            ui.add(
                egui::DragValue::new(&mut state.gate_hi)
                    .range(1..=u32::MAX)
                    .speed(1.0),
            );
            ui.label("~");
            ui.add(
                egui::DragValue::new(&mut state.gate_lo)
                    .range(1..=u32::MAX)
                    .speed(1.0),
            );
        },
    );
    ui.horizontal(|ui| {
        crate::widgets::switch::switch(ui, &mut state.invert);
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(t!("dialog.filter.invert").as_ref())
                .size(crate::scaling::scaled_font(
                    ui.ctx(),
                    crate::theme::SMALL_FONT,
                ))
                .color(if state.invert {
                    crate::theme::text_primary()
                } else {
                    crate::theme::text_muted()
                }),
        );
    });
}

fn show_automation_section(ui: &mut egui::Ui, state: &mut FilterDialogState) {
    section_title(ui, t!("dialog.filter.automation").as_ref());
    if state.automation_targets.is_empty() {
        ui.label(
            egui::RichText::new(t!("dialog.filter.no_automation").as_ref())
                .size(crate::theme::SMALL_FONT)
                .color(crate::theme::text_muted()),
        );
        return;
    }
    bound_row(
        ui,
        t!("dialog.filter.automation_targets").as_ref(),
        &mut state.automation_enabled,
        |_ui| {},
    );
    if state.automation_enabled {
        ui.indent("filter_automation_targets", |ui| {
            for entry in &mut state.automation_targets {
                ui.checkbox(&mut entry.checked, entry.label.as_str());
            }
        });
        ui.add_space(4.0);
    }
    bound_row(
        ui,
        t!("dialog.filter.value").as_ref(),
        &mut state.automation_value_enabled,
        |ui| {
            ui.add(
                egui::DragValue::new(&mut state.automation_value_hi)
                    .speed(0.01)
                    .fixed_decimals(3),
            );
            ui.label("~");
            ui.add(
                egui::DragValue::new(&mut state.automation_value_lo)
                    .speed(0.01)
                    .fixed_decimals(3),
            );
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use yinhe_types::{AutomationEvent, AutomationLane, SegmentShape};

    fn model_with_cc() -> YinModel {
        let mut track = yinhe_core::TrackData::new(0, 0);
        track.automation_lanes.push(AutomationLane {
            target: AutomationTarget::CC { controller: 1 },
            track: 0,
            events: vec![AutomationEvent {
                tick: 0,
                value: 10.0,
                shape: SegmentShape::Step,
            }],
        });
        YinModel {
            conductor: Arc::new(yinhe_core::ConductorData {
                tempo: AutomationLane {
                    target: AutomationTarget::Tempo,
                    track: 0,
                    events: vec![AutomationEvent {
                        tick: 0,
                        value: 120.0,
                        shape: SegmentShape::Step,
                    }],
                },
                ..Default::default()
            }),
            tracks: vec![Arc::new(track)],
            ..Default::default()
        }
    }

    #[test]
    fn open_restores_existing_filter() {
        let model = model_with_cc();
        let mut selection = Selection::default();
        selection.add_rect_track(100, 500, 60, 70, 0, 0);
        selection.filter.key = Some((55, 65));
        selection.filter.velocity = Some((10, 20));
        selection.filter.gate = Some((30, 40));
        selection.filter.invert = true;
        selection.filter.automation_targets = Some(vec![AutomationTarget::CC { controller: 1 }]);

        let mut state = FilterDialogState::default();
        state.open(&selection, &model);

        assert!(state.key_enabled);
        assert_eq!((state.key_lo, state.key_hi), (55, 65));
        assert!(state.velocity_enabled);
        assert_eq!((state.velocity_lo, state.velocity_hi), (10, 20));
        assert!(state.gate_enabled);
        assert_eq!((state.gate_lo, state.gate_hi), (30, 40));
        assert!(state.invert);
        assert!(state.automation_enabled);
        assert_eq!(state.automation_targets.len(), 2, "Tempo + CC1");
        let cc = state
            .automation_targets
            .iter()
            .find(|e| matches!(e.target, AutomationTarget::CC { controller: 1 }))
            .unwrap();
        assert!(cc.checked);
        let tempo = state
            .automation_targets
            .iter()
            .find(|e| matches!(e.target, AutomationTarget::Tempo))
            .unwrap();
        assert!(!tempo.checked, "白名单只含 CC1");
    }

    #[test]
    fn build_filter_roundtrip() {
        let model = model_with_cc();
        let selection = Selection::default();
        let mut state = FilterDialogState::default();
        state.open(&selection, &model);
        state.key_enabled = true;
        state.key_lo = 80;
        state.key_hi = 40; // 反着填也应规范化为 (40, 80)
        state.velocity_enabled = true;
        state.velocity_lo = 90;
        state.velocity_hi = 10; // 反着填也应规范化为 (10, 90)
        state.gate_enabled = true;
        state.gate_lo = 5;
        state.gate_hi = 100;
        state.invert = true;
        state.automation_value_enabled = true;
        state.automation_value_lo = 5.0;
        state.automation_value_hi = 1.0;

        let f = state.build_filter();
        assert_eq!(f.key, Some((40, 80)));
        assert_eq!(f.velocity, Some((10, 90)));
        assert_eq!(f.gate, Some((5, 100)));
        assert!(f.invert);
        assert_eq!(f.automation_value, Some((1.0, 5.0)));
        assert!(f.automation_targets.is_none(), "未启用类型白名单");
    }

    #[test]
    fn disabled_bounds_are_none() {
        let model = model_with_cc();
        let selection = Selection::default();
        let mut state = FilterDialogState::default();
        state.open(&selection, &model);
        state.key_enabled = false;
        state.track_enabled = false;
        state.velocity_enabled = false;
        state.gate_enabled = false;

        let f = state.build_filter();
        assert!(f.key.is_none());
        assert!(f.track.is_none());
        assert!(f.velocity.is_none());
        assert!(f.gate.is_none());
    }
}
