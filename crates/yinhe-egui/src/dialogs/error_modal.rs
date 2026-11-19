//! 阻塞式错误窗口：一次显示队列中的一条错误，确认后显示下一条。
//!
//! 用 `egui::Modal`（主窗口内全屏遮罩）而不是独立 viewport：独立窗口在
//! macOS 上不会阻止用户继续操作主窗口，达不到“阻碍当前工作流程”的目的。

use eframe::egui;
use rust_i18n::t;

use crate::chrome::dialog_buttons::{DialogButton, dialog_button_row_responses};
use crate::widgets::toast::model::ErrorReport;

/// 返回 true 表示用户确认并关闭了当前错误。
pub(crate) fn show(ctx: &egui::Context, err: &ErrorReport) -> bool {
    let mut close = false;
    let mut copy = false;

    let modal = egui::Modal::new(egui::Id::unique("yinhe_error_modal")).show(ctx, |ui| {
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
        let responses = dialog_button_row_responses(ui, &buttons);
        if responses.first().is_some_and(|resp| resp.clicked()) {
            copy = true;
        }
        if responses.get(1).is_some_and(|resp| resp.clicked()) {
            close = true;
        }

        // 弹窗打开后把焦点送到「确定」；如果用户已经 Tab 到「复制」，
        // 任一按钮有焦点就不再抢。没有这一步，Tab 会从背后的主界面开始。
        if !responses.iter().any(|resp| resp.has_focus())
            && let Some(ok) = responses.get(1)
        {
            ok.request_focus();
        }
    });

    // 弹窗本身作为 AlertDialog + assertive live region：标题/正文出现时
    // 读屏主动播报，而不是等用户主动浏览。
    modal.response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::AlertDialog, true, err.title.clone())
    });
    ctx.accesskit_node_builder(modal.response.id, |node| {
        node.set_live(egui::accesskit::Live::Assertive);
    });

    // 键盘退出：阻塞式弹窗也应该能用 Esc 关闭。
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }

    if copy {
        ctx.copy_text(format!("{}\n\n{}", err.title, err.message));
    }
    close
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    /// 错误弹窗必须是 AlertDialog，且打开后焦点落在「确定」。
    #[test]
    fn error_modal_is_alert_and_focuses_ok() {
        let err = ErrorReport {
            title: "保存失败".to_string(),
            message: "磁盘已满".to_string(),
        };
        let mut harness = Harness::builder()
            .allow_missing_glyphs()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui_state(
                |ui, err: &mut ErrorReport| {
                    show(ui.ctx(), err);
                },
                err,
            );
        harness.step();
        harness.step();

        // 找不到 AlertDialog 会直接 panic。
        harness.get_by_role(egui::Role::AlertDialog);

        let ok_label = Box::leak(t!("common.ok").to_string().into_boxed_str());
        assert!(
            harness.get_by_label(ok_label).is_focused(),
            "错误弹窗打开后焦点应落在「确定」"
        );
    }
}
