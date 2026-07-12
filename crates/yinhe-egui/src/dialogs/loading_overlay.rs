use std::cell::Cell;
use std::rc::Rc;

use eframe::egui;
use egui_material_icons::icons::*;

use yinhe_editor_core::progress::SharedProgress;

/// Returns `true` if the user closed the loading window (i.e. requested cancel).
pub(crate) fn show_viewport(ctx: &egui::Context, progress: SharedProgress) -> bool {
    let cancel_flag = Rc::new(Cell::new(false));
    let cancel_cb = cancel_flag.clone();
    let ctx_clone = ctx.clone();

    ctx_clone.show_viewport_immediate(
        egui::ViewportId::from_hash_of("loading_overlay_dialog"),
        crate::chrome::dialog::viewport_builder("正在加载", [380.0, 160.0], false),
        move |vctx, _class| {
            let close_requested = vctx.input(|i| i.viewport().close_requested());
            let mut close = close_requested;
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::APP_BG,
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, "正在加载", &mut close);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            let p = match progress.lock() {
                                Ok(p) => p.clone(),
                                Err(_) => return,
                            };
                            if !p.visible {
                                return;
                            }
                            for stage in &p.stages {
                                ui.horizontal(|ui| {
                                    let icon = match stage.status {
                                        yinhe_editor_core::progress::StageStatus::Done => {
                                            ICON_CHECK_CIRCLE
                                        }
                                        yinhe_editor_core::progress::StageStatus::Active => {
                                            ICON_SYNC
                                        }
                                        yinhe_editor_core::progress::StageStatus::Pending => {
                                            ICON_RADIO_BUTTON_UNCHECKED
                                        }
                                    };
                                    ui.label(icon.rich_text().size(14.0));
                                    ui.add(
                                        egui::ProgressBar::new(stage.progress)
                                            .desired_width(200.0)
                                            .show_percentage(),
                                    );
                                    ui.label(
                                        egui::RichText::new(&stage.label).size(12.0),
                                    );
                                });
                                if !stage.detail.is_empty() {
                                    ui.label(
                                        egui::RichText::new(&stage.detail)
                                            .size(10.0)
                                            .color(egui::Color32::GRAY),
                                    );
                                }
                            }
                        });
                });
            if close {
                cancel_cb.set(true);
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        },
    );

    ctx.request_repaint();
    cancel_flag.get()
}
