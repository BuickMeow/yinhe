use eframe::egui;
use rust_i18n::t;

use crate::app::PendingFileAction;

pub(crate) enum Action {
    None,
    Save,
    Discard,
    Cancel,
}

pub(crate) fn show_viewport(
    ctx: &egui::Context,
    pending_unsaved: &Option<PendingFileAction>,
    save_rx: &Option<std::sync::mpsc::Receiver<()>>,
) -> Action {
    if pending_unsaved.is_none() || save_rx.is_some() {
        return Action::None;
    }
    let viewport_id = egui::ViewportId::from_hash_of("unsaved_dialog");

    let action_rc: std::rc::Rc<std::cell::RefCell<Option<Action>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let action_cb = action_rc.clone();
    let ctx_clone = ctx.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(
            t!("dialog.unsaved.title").as_ref(),
            [340.0, 130.0],
            false,
        ),
        move |vctx, _class| {
            let mut close = false;
            if vctx.input(|i| i.viewport().close_requested()) {
                *action_cb.borrow_mut() = Some(Action::Cancel);
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
                        t!("dialog.unsaved.title").as_ref(),
                        &mut close,
                        false,
                    );
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(316.0);
                            let btn_zone_h = crate::chrome::dialog_buttons::btn_zone_h(ui.ctx());
                            crate::chrome::dialog::content_with_bottom_buttons(
                                ui,
                                btn_zone_h,
                                |ui| {
                                    ui.add_space(8.0);
                                    ui.label(t!("dialog.unsaved.message").as_ref());
                                },
                                |ui| {
                                    use crate::chrome::dialog_buttons::{
                                        DialogButton, dialog_button_row,
                                    };
                                    ui.add_space(8.0);
                                    let discard = t!("dialog.unsaved.discard");
                                    let back = t!("dialog.unsaved.back");
                                    let save = t!("dialog.unsaved.save");
                                    if let Some(idx) = dialog_button_row(
                                        ui,
                                        &[
                                            DialogButton::danger(discard.as_ref()),
                                            DialogButton::secondary(back.as_ref()),
                                            DialogButton::primary(save.as_ref()),
                                        ],
                                    ) {
                                        *action_cb.borrow_mut() = Some(match idx {
                                            0 => Action::Discard,
                                            1 => Action::Cancel,
                                            _ => Action::Save,
                                        });
                                        close = true;
                                    }
                                },
                            );
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        },
    );

    action_rc.borrow_mut().take().unwrap_or(Action::None)
}
