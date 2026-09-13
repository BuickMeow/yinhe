use eframe::egui;
use rust_i18n::t;

pub(crate) fn show_viewport(ctx: &egui::Context, error: &mut Option<String>) {
    let msg = match error {
        Some(m) => m.clone(),
        None => return,
    };
    let viewport_id = egui::ViewportId::from_hash_of("load_error_dialog");

    let open = std::rc::Rc::new(std::cell::RefCell::new(true));
    let open_cb = open.clone();
    let ctx_clone = ctx.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(
            t!("dialog.load_error.title").as_ref(),
            [420.0, 120.0],
            false,
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
                        t!("dialog.load_error.title").as_ref(),
                        &mut close,
                    );
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(396.0);
                            let btn_zone_h = crate::chrome::dialog_buttons::btn_zone_h(ui.ctx());
                            crate::chrome::dialog::content_with_bottom_buttons(
                                ui,
                                btn_zone_h,
                                |ui| {
                                    ui.add_space(8.0);
                                    ui.label(&msg);
                                },
                                |ui| {
                                    use crate::chrome::dialog_buttons::{
                                        DialogButton, dialog_button_row,
                                    };
                                    ui.add_space(8.0);
                                    let ok = t!("dialog.load_error.ok");
                                    if dialog_button_row(ui, &[DialogButton::primary(ok.as_ref())])
                                        .is_some()
                                    {
                                        close = true;
                                    }
                                },
                            );
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                *open_cb.borrow_mut() = false;
            }
        },
    );

    if !*open.borrow() {
        *error = None;
    }
}
