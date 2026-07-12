use eframe::egui;

pub(crate) fn show_viewport(ctx: &egui::Context, error: &mut Option<String>) {
    let msg = match error {
        Some(m) => m.clone(),
        None => return,
    };

    let open = std::rc::Rc::new(std::cell::RefCell::new(true));
    let open_cb = open.clone();
    let ctx_clone = ctx.clone();

    ctx_clone.show_viewport_immediate(
        egui::ViewportId::from_hash_of("load_error_dialog"),
        crate::chrome::dialog::viewport_builder("无法打开文件", [420.0, 120.0], false),
        move |vctx, _class| {
            let mut close = false;
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::APP_BG,
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, "无法打开文件", &mut close);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(420.0);
                            ui.label(&msg);
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if ui.button("确定").clicked() {
                                    close = true;
                                }
                            });
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
