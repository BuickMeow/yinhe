//! 通用错误弹窗：所有阻塞式错误（加载/导出/缩放/窗口创建失败等）共用，
//! 只换标题与消息，交互与排版完全一致。

use eframe::egui;
use rust_i18n::t;

/// 一次错误提示的内容。标题已由调用方按场景翻译好。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ErrorDialog {
    pub title: String,
    pub message: String,
}

impl ErrorDialog {
    pub fn open(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
        }
    }
}

pub(crate) fn show_viewport(ctx: &egui::Context, state: &mut Option<ErrorDialog>) {
    let Some(dialog) = state.clone() else {
        return;
    };
    let viewport_id = egui::ViewportId::from_hash_of("error_dialog");

    let open = std::rc::Rc::new(std::cell::RefCell::new(true));
    let open_cb = open.clone();
    let ctx_clone = ctx.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(&dialog.title, [420.0, 150.0], false),
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
                    crate::chrome::dialog::title_bar(ui, &dialog.title, &mut close);
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
                                    egui::ScrollArea::vertical()
                                        .auto_shrink([false, false])
                                        .max_height(ui.available_height())
                                        .show(ui, |ui| {
                                            ui.add_space(8.0);
                                            ui.label(&dialog.message);
                                        });
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
        *state = None;
    }
}
