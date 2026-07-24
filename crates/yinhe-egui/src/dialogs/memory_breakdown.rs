use eframe::egui;
use rust_i18n::t;

pub(crate) fn show_viewport(
    ctx: &egui::Context,
    open: &mut bool,
    mem_mb: f64,
    metal_size: u64,
) {
    let viewport_id = egui::ViewportId::from_hash_of("memory_breakdown_dialog");
    if !*open {
        return;
    }

    let snapshot = yinhe_memtrace::Snapshot::capture();
    let open_rc = std::rc::Rc::new(std::cell::RefCell::new(true));
    let ctx_clone = ctx.clone();
    let open_cb = open_rc.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(
            t!("dialog.memory.title").as_ref(),
            crate::theme::MEM_POPUP_SIZE,
            false,
        ),
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
                    crate::chrome::dialog::title_bar(
                        ui,
                        t!("dialog.memory.title").as_ref(),
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
                            egui::ScrollArea::vertical()
                                .auto_shrink([false; 2])
                                .show(ui, |ui| {
                                    if yinhe_memtrace::enabled() {
                                        ui.label(
                                            t!("dialog.memory.allocator", n = format!("{:.1}", snapshot.total_mb())).as_ref(),
                                        );
                                    } else {
                                        ui.label(
                                            egui::RichText::new(
                                                t!("dialog.memory.not_enabled").as_ref(),
                                            )
                                            .color(egui::Color32::from_gray(140)),
                                        );
                                    }
                                    ui.label(
                                        t!("dialog.memory.rss", n = format!("{:.1}", mem_mb)).as_ref(),
                                    );
                                    ui.label(
                                        t!("dialog.memory.gpu", n = format!("{:.1}", snapshot.gpu_mb())).as_ref(),
                                    );

                                    #[cfg(target_os = "macos")]
                                    ui.label(
                                        t!("dialog.memory.metal", n = format!("{:.1}", metal_size as f64 / 1_048_576.0)).as_ref(),
                                    );

                                    if yinhe_memtrace::enabled() {
                                        ui.separator();
                                        ui.heading(t!("dialog.memory.by_subsystem").as_ref());
                                        egui::Grid::new("mem_breakdown_grid")
                                            .num_columns(2)
                                            .spacing([12.0, 8.0])
                                            .show(ui, |ui| {
                                                for tag in yinhe_memtrace::AllocTag::ALL {
                                                    if tag == yinhe_memtrace::AllocTag::Unknown
                                                        && snapshot.get(tag) <= 0
                                                    {
                                                        continue;
                                                    }
                                                    ui.label(tag.name());
                                                    ui.label(format!(
                                                        "{:.1} MB",
                                                        snapshot.mb(tag)
                                                    ));
                                                    ui.end_row();
                                                }
                                            });
                                    }

                                    ui.separator();
                                    ui.small(
                                        t!("dialog.memory.note").as_ref(),
                                    );
                                });
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                *open_cb.borrow_mut() = false;
            }
        },
    );

    if !*open_rc.borrow() {
        *open = false;
    }
}
