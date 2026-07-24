//! PPQ rescale 进度条 overlay。
//!
//! 当 `App.rescale.rx` 为 `Some` 时显示，参考
//! [`crate::dialogs::export::show_progress_viewport`] 的模式：
//! 独立 viewport + 单进度条 + 取消按钮。

use std::sync::{Arc, Mutex};

use eframe::egui;
use rust_i18n::t;

use yinhe_core::RescaleProgress;

/// 显示 rescale 进度条窗口。用户点击取消或关闭窗口时返回 true。
pub(crate) fn show_viewport(
    ctx: &egui::Context,
    progress: Arc<Mutex<RescaleProgress>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) {
    let ctx_clone = ctx.clone();

    ctx_clone.show_viewport_immediate(
        egui::ViewportId::from_hash_of("rescale_progress_dialog"),
        crate::chrome::dialog::viewport_builder(t!("dialog.rescale.title").as_ref(), [340.0, 140.0], false),
        move |vctx, _class| {
            let state = match progress.lock() {
                Ok(s) => s.clone(),
                Err(_) => return,
            };
            let close_requested = vctx.input(|i| i.viewport().close_requested());
            let mut close = close_requested;
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::APP_BG,
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, t!("dialog.rescale.title").as_ref(), &mut close);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                ui.add_space(4.0);
                                ui.add(
                                    egui::ProgressBar::new(state.progress)
                                        .desired_width(300.0)
                                        .show_percentage(),
                                );
                                ui.add_space(6.0);
                                if !state.label.is_empty() {
                                    ui.label(
                                        egui::RichText::new(&state.label)
                                            .size(11.0)
                                            .color(egui::Color32::from_gray(160)),
                                    );
                                }
                                ui.add_space(8.0);
                                if ui.button(t!("common.cancel").as_ref()).clicked() {
                                    close = true;
                                }
                            });
                        });
                });
            if close {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        },
    );

    ctx.request_repaint();
}
