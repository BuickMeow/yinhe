//! 「插入」菜单的参数窗口：BPM / 拍号 / 音色（PC）。
//!
//! 三个窗口都是“小参数 + 取消/确定”形态，共用 [`show_param_window`] 壳
//! （独立 viewport；与项目其它对话框一致）。窗口只回传动作，
//! 实际插入由 `app::insert` 落地（含 undo）。

use eframe::egui;
use rust_i18n::t;

use crate::chrome::dialog::{content_with_bottom_buttons, title_bar, viewport_builder};
use crate::chrome::dialog_buttons::{DialogButton, btn_zone_h, dialog_button_row};

/// 参数窗口公共壳：返回 `Some(true)` = 确定、`Some(false)` = 取消/关窗、`None` = 无动作。
fn show_param_window(
    ctx: &egui::Context,
    viewport_id: &str,
    title: &str,
    size: [f32; 2],
    mut content: impl FnMut(&mut egui::Ui),
) -> Option<bool> {
    let mut result = None;
    let mut close = false;
    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of(viewport_id),
        viewport_builder(title, size, false),
        |vctx, _class| {
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            let mut title_close = false;
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::app_bg(),
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    title_bar(ui, title, &mut title_close, false);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            let zone = btn_zone_h(ui.ctx());
                            let mut clicked: Option<usize> = None;
                            content_with_bottom_buttons(
                                ui,
                                zone,
                                |ui| content(ui),
                                |ui| {
                                    let cancel = t!("common.cancel");
                                    let ok = t!("common.ok");
                                    clicked = dialog_button_row(
                                        ui,
                                        &[
                                            DialogButton::secondary(cancel.as_ref()),
                                            DialogButton::primary(ok.as_ref()),
                                        ],
                                    );
                                },
                            );
                            if clicked == Some(1) {
                                result = Some(true);
                            } else if clicked == Some(0) {
                                close = true;
                            }
                        });
                    if title_close {
                        close = true;
                    }
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                if result.is_none() {
                    result = Some(false);
                }
            }
        },
    );
    result
}

// ── BPM ──

/// 「插入 BPM」窗口状态。
#[derive(Default)]
pub struct InsertBpmState {
    pub open: bool,
    /// 打开后首帧把窗口提到最前。
    pub just_opened: bool,
    /// 目标 tick（打开时锁定；拖动指示线不影响已开的窗口）。
    tick: u32,
    bpm: f64,
    /// 位置显示文本（小节.拍.tick）。
    position: String,
}

pub enum InsertBpmAction {
    None,
    /// 确定：在 `tick` 处插入 `bpm` 事件。
    Confirm {
        tick: u32,
        bpm: f64,
    },
    Cancel,
}

impl InsertBpmState {
    pub fn open(&mut self, tick: u32, current_bpm: f64, position: String) {
        self.open = true;
        self.just_opened = true;
        self.tick = tick;
        self.bpm = current_bpm.clamp(0.0, 60_000_000.0);
        self.position = position;
    }

    pub fn show(&mut self, ctx: &egui::Context) -> InsertBpmAction {
        if !self.open {
            return InsertBpmAction::None;
        }
        let title = t!("dialog.insert_bpm.title");
        let mut bpm = self.bpm;
        let result = show_param_window(
            ctx,
            "insert_bpm_dialog",
            title.as_ref(),
            [340.0, 196.0],
            |ui| {
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(t!("dialog.insert_bpm.position", pos = &self.position))
                        .size(crate::theme::SMALL_FONT)
                        .color(crate::theme::text_muted()),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(t!("menu.insert.bpm"));
                    ui.add(
                        egui::DragValue::new(&mut bpm)
                            .range(0.0..=60_000_000.0)
                            .speed(0.5)
                            .max_decimals(2),
                    );
                    ui.label("BPM");
                });
            },
        );
        self.bpm = bpm;
        match result {
            Some(true) => InsertBpmAction::Confirm {
                tick: self.tick,
                bpm: self.bpm,
            },
            Some(false) => InsertBpmAction::Cancel,
            None => InsertBpmAction::None,
        }
    }
}

// ── 拍号 ──

/// 「插入拍号」窗口状态。
#[derive(Default)]
pub struct InsertTimeSigState {
    pub open: bool,
    pub just_opened: bool,
    tick: u32,
    numerator: u8,
    denominator: u8,
    position: String,
}

pub enum InsertTimeSigAction {
    None,
    /// 确定：在 `tick` 处插入拍号事件。
    Confirm {
        tick: u32,
        numerator: u8,
        denominator: u8,
    },
    Cancel,
}

/// 拍号分母可选值（2 的幂，与 SMF 表示一致）。
const DENOMINATORS: [u8; 6] = [1, 2, 4, 8, 16, 32];

impl InsertTimeSigState {
    pub fn open(&mut self, tick: u32, numerator: u8, denominator: u8, position: String) {
        self.open = true;
        self.just_opened = true;
        self.tick = tick;
        self.numerator = numerator.clamp(1, 32);
        self.denominator = denominator.max(1);
        self.position = position;
    }

    pub fn show(&mut self, ctx: &egui::Context) -> InsertTimeSigAction {
        if !self.open {
            return InsertTimeSigAction::None;
        }
        let title = t!("dialog.insert_timesig.title");
        let mut numerator = self.numerator;
        let mut denominator = self.denominator;
        let result = show_param_window(
            ctx,
            "insert_timesig_dialog",
            title.as_ref(),
            [340.0, 208.0],
            |ui| {
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(t!("dialog.insert_timesig.position", pos = &self.position))
                        .size(crate::theme::SMALL_FONT)
                        .color(crate::theme::text_muted()),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(t!("quantize.numerator"));
                    ui.add(egui::DragValue::new(&mut numerator).range(1..=32));
                    ui.add_space(12.0);
                    ui.label(t!("quantize.denominator"));
                    egui::ComboBox::from_id_salt("insert_timesig_den")
                        .selected_text(denominator.to_string())
                        .show_ui(ui, |ui| {
                            for d in DENOMINATORS {
                                ui.selectable_value(&mut denominator, d, d.to_string());
                            }
                        });
                });
            },
        );
        self.numerator = numerator;
        self.denominator = denominator;
        match result {
            Some(true) => InsertTimeSigAction::Confirm {
                tick: self.tick,
                numerator: self.numerator,
                denominator: self.denominator,
            },
            Some(false) => InsertTimeSigAction::Cancel,
            None => InsertTimeSigAction::None,
        }
    }
}

// ── 音色（PC）──

/// 「插入音色（PC）」窗口状态。
#[derive(Default)]
pub struct InsertPcState {
    pub open: bool,
    pub just_opened: bool,
    tick: u32,
    program: u8,
    position: String,
    /// 目标轨道描述（“Piano 等 3 条轨道”）。
    targets: String,
    /// 目标轨道（打开时锁定；确认后按其批量插入）。
    pub(crate) tracks: Vec<u16>,
}

pub enum InsertPcAction {
    None,
    /// 确定：在 `tick` 处向打开时锁定的轨道插入 PC 事件。
    Confirm {
        tick: u32,
        program: u8,
    },
    Cancel,
}

impl InsertPcState {
    pub fn open(&mut self, tick: u32, position: String, targets: String, tracks: Vec<u16>) {
        self.open = true;
        self.just_opened = true;
        self.tick = tick;
        self.program = 0;
        self.position = position;
        self.targets = targets;
        self.tracks = tracks;
    }

    pub fn show(&mut self, ctx: &egui::Context) -> InsertPcAction {
        if !self.open {
            return InsertPcAction::None;
        }
        let title = t!("dialog.insert_pc.title");
        let mut program = self.program;
        let result = show_param_window(
            ctx,
            "insert_pc_dialog",
            title.as_ref(),
            [340.0, 208.0],
            |ui| {
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(t!("dialog.insert_pc.position", pos = &self.position))
                        .size(crate::theme::SMALL_FONT)
                        .color(crate::theme::text_muted()),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(t!("event_browser.header.program"));
                    ui.add(egui::DragValue::new(&mut program).range(0..=127));
                });
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(&self.targets)
                        .size(crate::theme::SMALL_FONT)
                        .color(crate::theme::text_muted()),
                );
            },
        );
        self.program = program;
        match result {
            Some(true) => InsertPcAction::Confirm {
                tick: self.tick,
                program: self.program,
            },
            Some(false) => InsertPcAction::Cancel,
            None => InsertPcAction::None,
        }
    }
}
