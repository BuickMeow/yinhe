//! 「调式」模态对话框：左列根音 × 右列音阶（两列各自滚动）+ 钢琴预览。
//!
//! 取代原时间码栏里的两列下拉 popup：音阶变体多（33 个）、且有空间受限的
//! 列表与预览，故升为居中模态。预览复用 `piano_view::keyboard::paint_mini`。
//! 本模块不持有文档状态——确认后由 app 侧把选择写回 `edit`（覆盖 / 工程事件）。

use eframe::egui;
use rust_i18n::t;

use yinhe_types::{NOTE_NAMES, ScaleType};

use crate::chrome::dialog_buttons::{DialogButton, dialog_button_row};
use crate::widgets::{menu, rows};

/// 对话框状态（app 持有）。
pub struct ScaleDialogState {
    pub open: bool,
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
    /// 取消 / 点背景关闭。
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
        let (root, scale) = current.unwrap_or((0, ScaleType::Major));
        self.root = root % 12;
        self.scale = scale;
        self.override_on = override_on;
        self.use_events = use_events;
    }

    pub fn show(&mut self, ctx: &egui::Context) -> ScaleAction {
        if !self.open {
            return ScaleAction::None;
        }

        let modal = egui::Modal::new(egui::Id::new("scale_dialog")).show(ctx, |ui| {
            ui.set_width(520.0);
            rows::section_header(ui, t!("timecode.keysig_dialog_title").as_ref());

            // ── 两列：根音 × 音阶（各自滚动）──
            let list_h = crate::scaling::scaled_font(ctx, 240.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(crate::scaling::scaled_font(ctx, 96.0));
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
                ui.add_space(crate::scaling::scaled_font(ctx, 8.0));
                ui.vertical(|ui| {
                    ui.set_width(crate::scaling::scaled_font(ctx, 180.0));
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
            });

            // ── 钢琴预览：C3..C5，调内音高亮 ──
            ui.add_space(crate::scaling::scaled_font(ctx, 8.0));
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), crate::scaling::scaled_font(ctx, 56.0)),
                egui::Sense::hover(),
            );
            crate::piano_view::keyboard::paint_mini(
                ui.painter(),
                rect,
                48, // C3
                72, // C5
                self.scale.pitch_classes(self.root),
            );

            // ── 名称 + 开关 ──
            ui.add_space(crate::scaling::scaled_font(ctx, 6.0));
            rows::value_row(
                ui,
                t!("timecode.keysig_name").as_ref(),
                format!(
                    "{} {}",
                    NOTE_NAMES[(self.root % 12) as usize],
                    self.scale.english_name()
                ),
            );
            ui.add_space(crate::scaling::scaled_font(ctx, 4.0));
            ui.checkbox(&mut self.override_on, t!("timecode.override"));
            ui.checkbox(&mut self.use_events, t!("timecode.use_events"));

            // ── 按钮 ──
            ui.add_space(crate::scaling::scaled_font(ctx, 8.0));
            dialog_button_row(
                ui,
                &[
                    DialogButton::secondary(t!("common.cancel").as_ref()),
                    DialogButton::primary(t!("common.ok").as_ref()),
                ],
            )
        });

        let mut action = ScaleAction::None;
        match modal.inner {
            Some(1) => {
                action = ScaleAction::Confirm {
                    r#override: self.override_on.then_some((self.root, self.scale)),
                    use_events: self.use_events,
                };
            }
            Some(0) => action = ScaleAction::Cancel,
            _ => {}
        }
        if matches!(action, ScaleAction::None) && modal.backdrop_response.clicked() {
            action = ScaleAction::Cancel;
        }
        if !matches!(action, ScaleAction::None) {
            self.open = false;
        }
        action
    }
}
