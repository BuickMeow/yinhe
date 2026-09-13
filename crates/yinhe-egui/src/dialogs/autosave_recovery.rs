//! 自动保存恢复弹窗：启动时发现上次异常退出留下的备份，询问是否恢复。

use eframe::egui;
use rust_i18n::t;

use crate::app::autosave::AutoSaveEntry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecoveryAction {
    None,
    Restore,
    Discard,
}

fn format_saved_at(unix_secs: u64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(unix_secs as i64, 0) {
        chrono::LocalResult::Single(dt) => dt.format("%m-%d %H:%M").to_string(),
        _ => String::new(),
    }
}

/// 显示恢复询问弹窗。`show` 为 false 时直接返回。
pub(crate) fn show_viewport(
    ctx: &egui::Context,
    entries: &[AutoSaveEntry],
    show: &mut bool,
) -> RecoveryAction {
    if !*show {
        return RecoveryAction::None;
    }
    let viewport_id = egui::ViewportId::from_hash_of("autosave_recovery_dialog");
    let title = t!("dialog.autosave_recovery.title");
    let mut action = RecoveryAction::None;

    ctx.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(title.as_ref(), [460.0, 320.0], true),
        |vctx, _class| {
            if vctx.input(|i| i.viewport().close_requested()) {
                // 关闭窗口 = 本次不处理（备份保留，下次启动再询问）
                *show = false;
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
            let mut hide_requested = false;
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::app_bg(),
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    let mut title_close = false;
                    crate::chrome::dialog::title_bar(ui, title.as_ref(), &mut title_close, true);
                    if title_close {
                        *show = false;
                        hide_requested = true;
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
                                    ui.add_space(4.0);
                                    ui.label(t!("dialog.autosave_recovery.desc").as_ref());
                                    ui.add_space(8.0);
                                    egui::ScrollArea::vertical()
                                        .auto_shrink([false, false])
                                        .max_height(ui.available_height())
                                        .show(ui, |ui| {
                                            for entry in entries.iter().take(50) {
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        egui::RichText::new(&entry.name)
                                                            .strong()
                                                            .size(crate::theme::BODY_FONT),
                                                    );
                                                    ui.label(
                                                        egui::RichText::new(format_saved_at(
                                                            entry.saved_at,
                                                        ))
                                                        .size(crate::theme::SMALL_FONT)
                                                        .color(crate::theme::text_muted()),
                                                    );
                                                });
                                                if let Some(orig) = &entry.original {
                                                    ui.label(
                                                        egui::RichText::new(orig)
                                                            .size(crate::theme::SMALL_FONT)
                                                            .color(crate::theme::text_label()),
                                                    );
                                                }
                                                ui.add_space(4.0);
                                            }
                                            if entries.len() > 50 {
                                                ui.label(
                                                    egui::RichText::new(
                                                        t!(
                                                            "dialog.autosave_recovery.more",
                                                            n = entries.len() - 50
                                                        )
                                                        .as_ref(),
                                                    )
                                                    .size(crate::theme::SMALL_FONT)
                                                    .color(crate::theme::text_muted()),
                                                );
                                            }
                                        });
                                },
                                |ui| {
                                    use crate::chrome::dialog_buttons::{
                                        DialogButton, dialog_button_row,
                                    };
                                    ui.add_space(8.0);
                                    let discard = t!("dialog.autosave_recovery.discard");
                                    let restore = t!("dialog.autosave_recovery.restore");
                                    if let Some(idx) = dialog_button_row(
                                        ui,
                                        &[
                                            DialogButton::danger(discard.as_ref()),
                                            DialogButton::primary(restore.as_ref()),
                                        ],
                                    ) {
                                        action = if idx == 0 {
                                            RecoveryAction::Discard
                                        } else {
                                            RecoveryAction::Restore
                                        };
                                        *show = false;
                                        hide_requested = true;
                                    }
                                },
                            );
                        });
                });
            if hide_requested {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        },
    );

    action
}
