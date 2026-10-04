//! 属性浮动面板（独立视口子窗口）：工程设置。
//!
//! 音轨属性已迁入右侧栏 Info 面板（不再有独立浮窗）。工程设置只在浮窗出现
//! （侧栏不承载）。

use std::cell::RefCell;
use std::rc::Rc;

use eframe::egui;
use rust_i18n::t;

use yinhe_editor_core::document::Document;

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
                            // 与设置窗口一致的控件密度
                            ui.spacing_mut().interact_size.y = 24.0;
                            ui.spacing_mut().item_spacing.y = 4.0;
                            egui::ScrollArea::vertical()
                                .id_salt("project_settings_scroll")
                                .auto_shrink([false; 2])
                                .show(ui, |ui| {
                                    ui.spacing_mut().interact_size.y = 24.0;
                                    ui.spacing_mut().item_spacing.y = 4.0;
                                    crate::dialogs::project_info::show(ui, doc);
                                });
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                *open_rc.borrow_mut() = false;
            }
        },
    );

    if !*open_out.borrow() {
        *open = false;
    }
}
