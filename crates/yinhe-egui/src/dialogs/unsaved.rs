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
        crate::chrome::dialog::viewport_builder(t!("dialog.unsaved.title").as_ref(), [380.0, 170.0], false),
        move |vctx, _class| {
            let mut close = false;
            if vctx.input(|i| i.viewport().close_requested()) {
                *action_cb.borrow_mut() = Some(Action::Cancel);
                close = true;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::APP_BG,
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, t!("dialog.unsaved.title").as_ref(), &mut close);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(360.0);
                            ui.vertical_centered(|ui| {
                                ui.add_space(8.0);
                                ui.label(t!("dialog.unsaved.message").as_ref());
                                ui.add_space(20.0);
                                ui.horizontal(|ui| {
                                    if ui.button(t!("dialog.unsaved.save").as_ref()).clicked() {
                                        *action_cb.borrow_mut() = Some(Action::Save);
                                        close = true;
                                    }
                                    ui.add_space(8.0);
                                    let discard_btn = ui.button(
                                        egui::RichText::new(t!("dialog.unsaved.discard").as_ref())
                                            .color(egui::Color32::from_rgb(255, 80, 80)),
                                    );
                                    if discard_btn.clicked() {
                                        *action_cb.borrow_mut() = Some(Action::Discard);
                                        close = true;
                                    }
                                    ui.add_space(8.0);
                                    if ui.button(t!("dialog.unsaved.back").as_ref()).clicked() {
                                        *action_cb.borrow_mut() = Some(Action::Cancel);
                                        close = true;
                                    }
                                });
                            });
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        },
    );

    action_rc.borrow_mut().take().unwrap_or(Action::None)
}
