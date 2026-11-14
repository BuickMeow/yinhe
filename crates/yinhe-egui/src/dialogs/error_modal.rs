//! 阻塞式错误窗口：一次显示队列中的一条错误，确认后显示下一条。
//!
//! 用 `egui::Modal`（主窗口内全屏遮罩）而不是独立 viewport：独立窗口在
//! macOS 上不会阻止用户继续操作主窗口，达不到“阻碍当前工作流程”的目的。

use eframe::egui;
use rust_i18n::t;

use crate::chrome::dialog_buttons::{DialogButton, dialog_button_row};
use crate::widgets::toast::model::ErrorReport;

/// 返回 true 表示用户确认并关闭了当前错误。
pub(crate) fn show(ctx: &egui::Context, err: &ErrorReport) -> bool {
    let mut close = false;
    let mut copy = false;

    egui::Modal::new(egui::Id::unique("yinhe_error_modal")).show(ctx, |ui| {
        ui.set_width(480.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(&err.title)
                    .size(crate::theme::SUB_TITLE_FONT)
                    .strong()
                    .color(crate::theme::text_primary()),
            )
            .selectable(false),
        );
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .max_height(260.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&err.message)
                            .size(crate::theme::SMALL_FONT)
                            .color(crate::theme::text_secondary()),
                    )
                    .selectable(true)
                    .wrap(),
                );
            });
        ui.add_space(12.0);

        let copy_label = t!("common.copy").to_string();
        let ok_label = t!("common.ok").to_string();
        let buttons = [
            DialogButton::secondary(&copy_label),
            DialogButton::primary(&ok_label),
        ];
        if let Some(idx) = dialog_button_row(ui, &buttons) {
            match idx {
                0 => copy = true,
                1 => close = true,
                _ => {}
            }
        }
    });

    if copy {
        ctx.copy_text(format!("{}\n\n{}", err.title, err.message));
    }
    close
}
