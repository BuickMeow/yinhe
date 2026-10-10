//! 「调式」对话框（系统级窗口 / Viewport）：左列根音 × 右列音阶（各自滚动）
//! + 纵向钢琴预览。
//!
//! 与项目其它对话框一致地使用独立 OS 窗口（`show_viewport_immediate`），
//! 而非应用内模态。预览复用 `piano_view::keyboard::paint_mini`（纵向、随主题）。
//! 本模块不持有文档状态——确认后由 app 侧把选择写回 `edit`（覆盖 / 工程事件）。

use eframe::egui;
use rust_i18n::t;

use yinhe_types::{NOTE_NAMES, ScaleType};

use crate::chrome::dialog::{content_with_bottom_buttons, title_bar, viewport_builder};
use crate::chrome::dialog_buttons::{DialogButton, btn_zone_h, dialog_button_row};
use crate::widgets::{menu, rows};

pub const VIEWPORT_ID: &str = "scale_dialog";

/// 对话框状态（app 持有）。
pub struct ScaleDialogState {
    pub open: bool,
    /// 本帧刚打开（dispatch 用于把窗口提升到前台）。
    pub just_opened: bool,
    root: u8,
    scale: ScaleType,
    /// 覆盖显示（勾选 = 用当前选择覆盖工程事件）。
    override_on: bool,
    /// 使用工程事件（未覆盖时读工程调号事件）。
    use_events: bool,
}

impl Default for ScaleDialogState {
    fn default() -> Self {
        Self {
            open: false,
            just_opened: false,
            root: 0,
            scale: ScaleType::Major,
            override_on: true,
            use_events: true,
        }
    }
}

/// 对话框本帧的结果。
pub enum ScaleAction {
    None,
    /// 确定：写回覆盖值 / 是否用工程事件。
    Confirm {
        r#override: Option<(u8, ScaleType)>,
        use_events: bool,
    },
    /// 取消 / 关窗。
    Cancel,
}

impl ScaleDialogState {
    /// 打开对话框，并以当前调式初始化选择。
    pub fn open_at(
        &mut self,
        current: Option<(u8, ScaleType)>,
        override_on: bool,
        use_events: bool,
    ) {
        self.open = true;
        self.just_opened = true;
        let (root, scale) = current.unwrap_or((0, ScaleType::Major));
        self.root = root % 12;
        self.scale = scale;
        self.override_on = override_on;
        self.use_events = use_events;
    }

    pub fn show_viewport(&mut self, ctx: &egui::Context) -> ScaleAction {
        let viewport_id = egui::ViewportId::from_hash_of(VIEWPORT_ID);
        let title = t!("timecode.keysig_dialog_title");

        let mut clicked: Option<usize> = None; // 0=取消, 1=确定
        let mut close = false;

        ctx.show_viewport_immediate(
            viewport_id,
            viewport_builder(title.as_ref(), [560.0, 560.0], true),
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
                        title_bar(ui, title.as_ref(), &mut title_close, true);
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
                                let zone = btn_zone_h(ui.ctx());
                                content_with_bottom_buttons(
                                    ui,
                                    zone,
                                    |ui| {
                                        ui.with_layout(
                                            egui::Layout::top_down(egui::Align::Min),
                                            |ui| {
                                                self.body(ui);
                                            },
                                        );
                                    },
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
                            });
                    });
                if close || clicked.is_some() {
                    vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
            },
        );

        let mut action = ScaleAction::None;
        match clicked {
            Some(1) => {
                action = ScaleAction::Confirm {
                    r#override: self.override_on.then_some((self.root, self.scale)),
                    use_events: self.use_events,
                };
            }
            Some(0) => action = ScaleAction::Cancel,
            _ => {}
        }
        if matches!(action, ScaleAction::None) && close {
            action = ScaleAction::Cancel;
        }
        if !matches!(action, ScaleAction::None) {
            self.open = false;
        }
        action
    }

    /// 对话框主体：两列滚动 + 纵向钢琴预览 + 名称 + 开关。
    fn body(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.set_width(ui.available_width());
        rows::section_header(ui, t!("timecode.keysig_dialog_title").as_ref());

        let list_h = crate::scaling::scaled_font(&ctx, 260.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(crate::scaling::scaled_font(&ctx, 96.0));
                ui.label(
                    egui::RichText::new(t!("timecode.root").as_ref())
                        .color(crate::theme::text_label()),
                );
                egui::ScrollArea::vertical()
                    .id_salt("scale_root_list")
                    .max_height(list_h)
                    .show(ui, |ui| {
                        for root in 0..12u8 {
                            let selected = root == self.root;
                            if ui
                                .add(menu::menu_item_button(
                                    ui,
                                    selected,
                                    NOTE_NAMES[(root % 12) as usize],
                                ))
                                .clicked()
                            {
                                self.root = root;
                            }
                        }
                    });
            });
            ui.add_space(crate::scaling::scaled_font(&ctx, 8.0));
            ui.vertical(|ui| {
                ui.set_width(crate::scaling::scaled_font(&ctx, 200.0));
                ui.label(
                    egui::RichText::new(t!("timecode.scale").as_ref())
                        .color(crate::theme::text_label()),
                );
                egui::ScrollArea::vertical()
                    .id_salt("scale_name_list")
                    .max_height(list_h)
                    .show(ui, |ui| {
                        for &scale in ScaleType::ALL {
                            let selected = scale == self.scale;
                            if ui
                                .add(menu::menu_item_button(ui, selected, scale.english_name()))
                                .clicked()
                            {
                                self.scale = scale;
                            }
                        }
                    });
            });
            // 右侧：纵向钢琴预览（随主题明暗/配色）。
            ui.add_space(crate::scaling::scaled_font(&ctx, 12.0));
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(crate::scaling::scaled_font(&ctx, 60.0), list_h),
                egui::Sense::hover(),
            );
            crate::piano_view::keyboard::paint_mini(
                ui.painter(),
                rect,
                48, // C3
                72, // C5
                self.scale.pitch_classes(self.root),
            );
        });

        ui.add_space(crate::scaling::scaled_font(&ctx, 8.0));
        rows::value_row(
            ui,
            t!("timecode.keysig_name").as_ref(),
            format!(
                "{} {}",
                NOTE_NAMES[(self.root % 12) as usize],
                self.scale.english_name()
            ),
        );
        ui.add_space(crate::scaling::scaled_font(&ctx, 6.0));
        ui.checkbox(&mut self.override_on, t!("timecode.override"));
        ui.checkbox(&mut self.use_events, t!("timecode.use_events"));
    }
}
