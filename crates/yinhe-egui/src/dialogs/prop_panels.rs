//! 属性浮动面板（独立视口子窗口）：工程设置。
//!
//! 音轨属性已迁入右侧栏 Info 面板（不再有独立浮窗）。工程设置只在浮窗出现
//! （侧栏不承载）。编辑期用草稿缓存，「保存」才写回 doc，支持「取消」。

use std::cell::RefCell;
use std::rc::Rc;

use eframe::egui;
use rust_i18n::t;

use yinhe_editor_core::document::Document;

use crate::dialogs::project_info::ProjectSettingsDraft;

/// 工程设置浮窗（内容排版对齐设置窗口）。
pub(crate) fn show_project_settings_viewport(
    ctx: &egui::Context,
    doc: &mut Document,
    open: &mut bool,
) {
    if !*open {
        return;
    }
    let viewport_id = egui::ViewportId::from_hash_of("project_settings_dialog");
    let open_rc = Rc::new(RefCell::new(true));
    let open_out = Rc::clone(&open_rc);
    let ctx_clone = ctx.clone();
    let draft_key = egui::Id::new("project_settings_draft");

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(
            t!("dialog.project_settings.title").as_ref(),
            crate::theme::PROJECT_SETTINGS_POPUP_SIZE,
            true,
        ),
        move |vctx, _class| {
            let mut close = false;
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }

            // 草稿：首次打开从 doc 读入，之后每帧在 memory 中持久，供「取消」丢弃。
            let mut draft = vctx
                .data(|d| d.get_temp::<ProjectSettingsDraft>(draft_key))
                .unwrap_or_else(|| ProjectSettingsDraft::from_doc(doc));
            let mut save = false;
            let mut cancel = false;

            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::app_bg(),
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(
                        ui,
                        t!("dialog.project_settings.title").as_ref(),
                        &mut close,
                        true,
                    );
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            ui.spacing_mut().interact_size.y = 24.0;
                            ui.spacing_mut().item_spacing.y = 4.0;
                            let btn_zone_h = crate::chrome::dialog_buttons::btn_zone_h(ui.ctx());
                            crate::chrome::dialog::content_with_bottom_buttons(
                                ui,
                                btn_zone_h,
                                |ui| {
                                    egui::ScrollArea::vertical()
                                        .id_salt("project_settings_scroll")
                                        .auto_shrink([false; 2])
                                        .show(ui, |ui| {
                                            ui.spacing_mut().interact_size.y = 24.0;
                                            ui.spacing_mut().item_spacing.y = 4.0;
                                            crate::dialogs::project_info::show(ui, &mut draft);
                                        });
                                },
                                |ui| {
                                    use crate::chrome::dialog_buttons::{
                                        DialogButton, dialog_button_row,
                                    };
                                    let cancel_txt = t!("common.cancel");
                                    let save_txt = t!("common.save");
                                    if let Some(idx) = dialog_button_row(
                                        ui,
                                        &[
                                            DialogButton::secondary(cancel_txt.as_ref()),
                                            DialogButton::primary(save_txt.as_ref()),
                                        ],
                                    ) {
                                        if idx == 0 {
                                            cancel = true;
                                        } else {
                                            save = true;
                                        }
                                    }
                                },
                            );
                        });
                });

            // 存回草稿（下帧继续编辑）。
            vctx.data_mut(|d| d.insert_temp(draft_key, draft.clone()));

            if save {
                let save_id = egui::Id::new("project_settings_save").value();
                crate::dialogs::project_info::commit_draft(vctx, doc, &draft, save_id);
                vctx.data_mut(|d| d.remove::<ProjectSettingsDraft>(draft_key));
                *open_rc.borrow_mut() = false;
                close = true;
            } else if cancel || close {
                // 取消 / 关闭窗口：丢弃草稿，不改 doc。
                vctx.data_mut(|d| d.remove::<ProjectSettingsDraft>(draft_key));
                *open_rc.borrow_mut() = false;
                close = true;
            }

            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        },
    );

    if !*open_out.borrow() {
        *open = false;
    }
}
